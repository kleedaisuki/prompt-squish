//! 项目清理编排契约。 / Project-clean orchestration contracts.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use squish_kernel::{CancellationToken, Capability, EventSink, InvocationContext, SinkError};
use squish_manager::{
    InvocationSettings, ManagerCapability, ProjectBuildLayout, ProjectCleanStatus, ResolveRequest,
    ResolvedDependencies, ServiceError, Services,
};
use squish_project::{Lockfile, ResolutionMode};
use squish_protocol::{
    ActionKind, CleanRequest, CleanResult, Event, EventPayload, InvocationId, OperationRequest,
    OperationResult, ProjectPath,
};
use squish_repository::PackageLocation;
use tempfile::TempDir;

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().expect("events poisoned").push(event);
        Ok(())
    }
}

struct Cleaner {
    result: Result<ProjectCleanStatus, ServiceError>,
    roots: Mutex<Vec<PathBuf>>,
    cancel_during: bool,
}

impl Cleaner {
    fn successful(result: CleanResult) -> Self {
        Self {
            result: Ok(ProjectCleanStatus::Cleaned(result)),
            roots: Mutex::new(Vec::new()),
            cancel_during: false,
        }
    }
}

impl Services for Cleaner {
    fn clean_project(
        &self,
        project_root: &Path,
        cancellation: CancellationToken,
    ) -> Result<ProjectCleanStatus, ServiceError> {
        self.roots
            .lock()
            .expect("roots poisoned")
            .push(project_root.to_owned());
        if self.cancel_during {
            cancellation.cancel();
        }
        self.result.clone()
    }

    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Ok(ProjectBuildLayout::project_local_for_tests(project))
    }

    fn materialize_locked(
        &self,
        _: &Path,
        _: &Lockfile,
        _: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Ok(Vec::new())
    }

    fn resolve(&self, _: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        Err(ServiceError::new("unused", "resolver is unused by clean"))
    }
}

fn project() -> TempDir {
    let temp = TempDir::new().unwrap();
    std::fs::write(
        temp.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    temp
}

fn request(project: &Path) -> OperationRequest {
    OperationRequest::Clean(CleanRequest {
        project: ProjectPath::new(project.to_string_lossy()).unwrap(),
    })
}

fn context(cancellation: CancellationToken) -> (InvocationContext, Arc<Events>) {
    let events = Arc::new(Events::default());
    (
        InvocationContext::new(
            InvocationId::new("clean-test").unwrap(),
            cancellation,
            events.clone(),
        ),
        events,
    )
}

#[test]
fn clean_returns_typed_stats_and_one_structured_action() {
    let temp = project();
    let expected = CleanResult {
        build_files: 7,
        build_bytes: 1_024,
        invalid_dependency_entries: 2,
        invalid_dependency_bytes: 8_192,
        busy_dependency_entries: 0,
    };
    let services = Cleaner::successful(expected);
    let manager = ManagerCapability::new(services, InvocationSettings::default());
    let (context, events) = context(CancellationToken::default());

    let outcome = manager.execute(&request(temp.path()), &context);

    assert_eq!(outcome.result, OperationResult::Clean(expected));
    assert_eq!(outcome.totals.succeeded, 1);
    assert_eq!(outcome.root_failures, 0);
    assert!(!outcome.cancelled);
    assert_eq!(
        manager.services().roots.lock().unwrap().as_slice(),
        [std::fs::canonicalize(temp.path()).unwrap()]
    );
    let declared = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(
                event.payload,
                EventPayload::ActionDeclared {
                    kind: ActionKind::Clean,
                    ..
                }
            )
        })
        .count();
    assert_eq!(declared, 1);
}

#[test]
fn no_op_clean_is_a_successful_zero_result() {
    let temp = project();
    let manager = ManagerCapability::new(
        Cleaner::successful(CleanResult::default()),
        InvocationSettings::default(),
    );
    let (context, _) = context(CancellationToken::default());

    let outcome = manager.execute(&request(temp.path()), &context);

    assert_eq!(
        outcome.result,
        OperationResult::Clean(CleanResult::default())
    );
    assert_eq!(outcome.totals.succeeded, 1);
}

#[test]
fn service_failure_is_a_structured_action_failure() {
    let temp = project();
    let manager = ManagerCapability::new(
        Cleaner {
            result: Err(ServiceError::new("clean_failed", "cleaner failed")),
            roots: Mutex::new(Vec::new()),
            cancel_during: false,
        },
        InvocationSettings::default(),
    );
    let (context, _) = context(CancellationToken::default());

    let outcome = manager.execute(&request(temp.path()), &context);

    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable {
            kind: squish_protocol::OperationKind::Clean
        }
    ));
    assert_eq!(outcome.totals.failed, 1);
    assert_eq!(outcome.root_failures, 1);
}

#[test]
fn committed_failure_preserves_partial_stats_and_fails_the_action() {
    let temp = project();
    let partial = CleanResult {
        build_files: 2,
        build_bytes: 64,
        invalid_dependency_entries: 0,
        invalid_dependency_bytes: 0,
        busy_dependency_entries: 1,
    };
    let manager = ManagerCapability::new(
        Cleaner {
            result: Ok(ProjectCleanStatus::CommittedFailure {
                result: partial,
                error: ServiceError::new("cache_clean_failed", "cache unavailable"),
            }),
            roots: Mutex::new(Vec::new()),
            cancel_during: false,
        },
        InvocationSettings::default(),
    );
    let (context, _) = context(CancellationToken::default());

    let outcome = manager.execute(&request(temp.path()), &context);

    assert_eq!(outcome.result, OperationResult::Clean(partial));
    assert_eq!(outcome.totals.failed, 1);
    assert_eq!(outcome.root_failures, 1);
}

#[test]
fn cancellation_after_clean_commit_preserves_stats_and_is_deferred() {
    let temp = project();
    let expected = CleanResult {
        build_files: 1,
        build_bytes: 9,
        invalid_dependency_entries: 0,
        invalid_dependency_bytes: 0,
        busy_dependency_entries: 0,
    };
    let manager = ManagerCapability::new(
        Cleaner {
            result: Ok(ProjectCleanStatus::Cleaned(expected)),
            roots: Mutex::new(Vec::new()),
            cancel_during: true,
        },
        InvocationSettings::default(),
    );
    let (context, events) = context(CancellationToken::default());

    let outcome = manager.execute(&request(temp.path()), &context);

    assert_eq!(outcome.result, OperationResult::Clean(expected));
    assert!(outcome.cancelled);
    assert_eq!(outcome.totals.succeeded, 1);
    let events = events.0.lock().unwrap();
    let deferred = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::CancellationDeferred { .. }))
        .expect("committed cancellation must be deferred");
    let succeeded = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::ActionSucceeded { .. }))
        .expect("committed clean must succeed");
    assert_eq!(deferred + 1, succeeded);
}

#[test]
fn pre_cancelled_clean_never_calls_the_destructive_port() {
    let temp = project();
    let manager = ManagerCapability::new(
        Cleaner::successful(CleanResult::default()),
        InvocationSettings::default(),
    );
    let cancellation = CancellationToken::default();
    cancellation.cancel();
    let (context, _) = context(cancellation);

    let outcome = manager.execute(&request(temp.path()), &context);

    assert!(outcome.cancelled);
    assert!(outcome.result.is_unavailable());
    assert!(manager.services().roots.lock().unwrap().is_empty());
}
