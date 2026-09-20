use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use squish_kernel::{CancellationToken, EventSink, InvocationContext, Kernel, SinkError};
use squish_manager::{
    InvocationSettings, ManagerCapability, ProjectBuildLayout, ProjectCreationLocation,
    ProjectCreationStatus, ResolveRequest, ResolvedDependencies, ServiceError, Services,
};
use squish_project::{Lockfile, ResolutionMode};
use squish_protocol::{
    ActionKind, Event, EventPayload, InvocationId, NewRequest, OperationRequest, OperationResult,
    ProjectDestination, VcsChoice,
};
use squish_repository::{
    CreateProjectRequest, CreatedProject, FaultInjector, PackageLocation, ProjectVcs,
};

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().expect("event mutex poisoned").push(event);
        Ok(())
    }
}

struct LateCancellationEvents {
    events: Mutex<Vec<Event>>,
    token: CancellationToken,
}

impl EventSink for LateCancellationEvents {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        let terminal = matches!(event.payload, EventPayload::ActionSucceeded { .. });
        self.events
            .lock()
            .expect("event mutex poisoned")
            .push(event);
        if terminal {
            self.token.cancel();
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Completion {
    Created,
    Cancelled,
    CreatedThenCancel,
    CommittedFailure,
    CommittedFailureThenCancel,
    WrongDestination,
}

struct CreationServices {
    destination: PathBuf,
    completion: Completion,
    request: Mutex<Option<CreateProjectRequest>>,
}

impl Services for CreationServices {
    fn locate_project_creation(
        &self,
        _: &Path,
        _: VcsChoice,
    ) -> Result<ProjectCreationLocation, ServiceError> {
        Ok(ProjectCreationLocation {
            destination: self.destination.clone(),
            vcs: ProjectVcs::None,
            workspace: None,
        })
    }

    fn create_project(
        &self,
        request: &CreateProjectRequest,
        cancellation: CancellationToken,
        _: Arc<dyn FaultInjector>,
    ) -> Result<ProjectCreationStatus, ServiceError> {
        *self.request.lock().unwrap() = Some(request.clone());
        match self.completion {
            Completion::Cancelled => {
                cancellation.cancel();
                Ok(ProjectCreationStatus::Cancelled)
            }
            Completion::CreatedThenCancel => {
                cancellation.cancel();
                Ok(ProjectCreationStatus::Created(CreatedProject {
                    destination: request.destination.clone(),
                    workspace_updated: false,
                }))
            }
            Completion::CommittedFailure => Ok(ProjectCreationStatus::CommittedFailure(
                ServiceError::new("committed", "recovery remains required"),
            )),
            Completion::CommittedFailureThenCancel => {
                cancellation.cancel();
                Ok(ProjectCreationStatus::CommittedFailure(ServiceError::new(
                    "committed",
                    "recovery remains required",
                )))
            }
            Completion::Created => Ok(ProjectCreationStatus::Created(CreatedProject {
                destination: request.destination.clone(),
                workspace_updated: false,
            })),
            Completion::WrongDestination => Ok(ProjectCreationStatus::Created(CreatedProject {
                destination: request.destination.with_extension("different"),
                workspace_updated: false,
            })),
        }
    }

    fn storage_layout(&self, _: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Err(ServiceError::new(
            "unused",
            "new must not request project storage",
        ))
    }

    fn materialize_locked(
        &self,
        _: &Path,
        _: &Lockfile,
        _: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Err(ServiceError::new(
            "unused",
            "new must not materialize dependencies",
        ))
    }

    fn resolve(&self, _: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        Err(ServiceError::new(
            "unused",
            "new must not resolve dependencies",
        ))
    }
}

fn run(
    completion: Completion,
) -> (
    squish_kernel::DispatchOutcome,
    Arc<Events>,
    Arc<CreationServices>,
) {
    let destination = std::env::temp_dir().join("xmlsquish-manager-new-contract");
    run_at(completion, destination)
}

fn run_at(
    completion: Completion,
    destination: PathBuf,
) -> (
    squish_kernel::DispatchOutcome,
    Arc<Events>,
    Arc<CreationServices>,
) {
    let services = Arc::new(CreationServices {
        destination: destination.clone(),
        completion,
        request: Mutex::new(None),
    });
    struct Shared(Arc<CreationServices>);
    impl Services for Shared {
        fn locate_project_creation(
            &self,
            destination: &Path,
            vcs: VcsChoice,
        ) -> Result<ProjectCreationLocation, ServiceError> {
            self.0.locate_project_creation(destination, vcs)
        }
        fn create_project(
            &self,
            request: &CreateProjectRequest,
            cancellation: CancellationToken,
            faults: Arc<dyn FaultInjector>,
        ) -> Result<ProjectCreationStatus, ServiceError> {
            self.0.create_project(request, cancellation, faults)
        }
        fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
            self.0.storage_layout(project)
        }
        fn materialize_locked(
            &self,
            project: &Path,
            lock: &Lockfile,
            mode: ResolutionMode,
        ) -> Result<Vec<PackageLocation>, ServiceError> {
            self.0.materialize_locked(project, lock, mode)
        }
        fn resolve(
            &self,
            request: ResolveRequest<'_>,
        ) -> Result<ResolvedDependencies, ServiceError> {
            self.0.resolve(request)
        }
    }
    let manager = ManagerCapability::new(Shared(services.clone()), InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("new-contract").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    let outcome = kernel
        .dispatch(
            &OperationRequest::New(NewRequest {
                destination: ProjectDestination::new(destination),
                name: None,
                vcs: Some(VcsChoice::None),
            }),
            &context,
        )
        .unwrap();
    (outcome, events, services)
}

#[test]
fn new_uses_one_truthful_write_action_and_returns_canonical_files() {
    let (outcome, events, services) = run(Completion::Created);
    assert_eq!(outcome.summary.status.code(), 0);
    assert_eq!(outcome.summary.totals.succeeded, 1);
    let OperationResult::New(result) = outcome.result else {
        panic!("new result expected")
    };
    assert_eq!(result.package.as_str(), "xmlsquish-manager-new-contract");
    assert_eq!(
        result
            .created
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["src/prompt.xml", "xmlsquish.toml"]
    );
    assert!(
        result
            .created
            .iter()
            .all(|file| file.size > 0 && file.digest.bytes().len() == 32)
    );
    let request = services.request.lock().unwrap();
    let request = request.as_ref().unwrap();
    assert_eq!(request.files.len(), 2);
    assert_eq!(request.vcs, ProjectVcs::None);
    let declared = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ActionDeclared { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(declared, [ActionKind::CreateProject]);
}

#[test]
fn predecision_cancellation_is_action_cancellation_without_root_failure() {
    let (outcome, events, _) = run(Completion::Cancelled);
    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable { .. }
    ));
    assert_eq!(outcome.summary.status.code(), 130);
    assert_eq!(outcome.summary.root_failures, 0);
    assert_eq!(outcome.summary.totals.cancelled, 1);
    assert!(
        events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.payload, EventPayload::ActionCancelled { .. }))
    );
}

#[test]
fn postdecision_cancellation_preserves_created_result_and_defers_interrupt() {
    let (outcome, events, _) = run(Completion::CreatedThenCancel);
    assert!(matches!(outcome.result, OperationResult::New(_)));
    assert_eq!(outcome.summary.totals.succeeded, 1);
    assert_eq!(outcome.summary.root_failures, 0);
    assert_eq!(outcome.summary.status.code(), 130);
    let events = events.0.lock().unwrap();
    let deferred = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::CancellationDeferred { .. }))
        .unwrap();
    let succeeded = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::ActionSucceeded { .. }))
        .unwrap();
    assert_eq!(deferred + 1, succeeded);
}

#[test]
fn invalid_inferred_leaf_is_a_domain_failure_before_action_declaration() {
    let destination = std::env::temp_dir().join("not a package");
    let (outcome, events, services) = run_at(Completion::Created, destination);
    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable { .. }
    ));
    assert_eq!(outcome.summary.root_failures, 1);
    assert_eq!(outcome.summary.totals.total(), 0);
    assert!(services.request.lock().unwrap().is_none());
    assert!(
        !events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.payload, EventPayload::ActionDeclared { .. }))
    );
}

#[test]
fn token_arriving_after_action_terminal_does_not_rewrite_committed_success() {
    let destination = std::env::temp_dir().join("xmlsquish-manager-late-cancel");
    let services = CreationServices {
        destination: destination.clone(),
        completion: Completion::Created,
        request: Mutex::new(None),
    };
    let manager = ManagerCapability::with_default_settings(services);
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let token = CancellationToken::default();
    let events = Arc::new(LateCancellationEvents {
        events: Mutex::new(Vec::new()),
        token: token.clone(),
    });
    let context = InvocationContext::new(
        InvocationId::new("new-late-cancel").unwrap(),
        token,
        events.clone(),
    );
    let outcome = kernel
        .dispatch(
            &OperationRequest::New(NewRequest {
                destination: ProjectDestination::new(destination),
                name: None,
                vcs: Some(VcsChoice::None),
            }),
            &context,
        )
        .unwrap();
    assert!(matches!(outcome.result, OperationResult::New(_)));
    assert_eq!(outcome.summary.status.code(), 0);
    assert!(!events.events.lock().unwrap().iter().any(|event| matches!(
        event.payload,
        EventPayload::CancellationDeferred { .. } | EventPayload::ActionCancelled { .. }
    )));
}

#[test]
fn committed_failure_preserves_failure_and_optional_deferred_cancellation() {
    let (failed, failed_events, _) = run(Completion::CommittedFailure);
    assert!(matches!(failed.result, OperationResult::Unavailable { .. }));
    assert_eq!(failed.summary.root_failures, 1);
    assert_eq!(failed.summary.status.code(), 1);
    assert!(
        !failed_events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event.payload, EventPayload::CancellationDeferred { .. }))
    );

    let (cancelled, cancelled_events, _) = run(Completion::CommittedFailureThenCancel);
    assert!(matches!(
        cancelled.result,
        OperationResult::Unavailable { .. }
    ));
    assert_eq!(cancelled.summary.root_failures, 1);
    assert_eq!(cancelled.summary.status.code(), 130);
    let events = cancelled_events.0.lock().unwrap();
    let deferred = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::CancellationDeferred { .. }))
        .unwrap();
    let failed_terminal = events
        .iter()
        .position(|event| matches!(event.payload, EventPayload::ActionFailed { .. }))
        .unwrap();
    assert_eq!(deferred + 1, failed_terminal);
}

#[test]
fn publisher_receipt_cannot_redirect_the_created_result() {
    let (outcome, events, _) = run(Completion::WrongDestination);
    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable { .. }
    ));
    assert_eq!(outcome.summary.root_failures, 1);
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(
        &event.payload,
        EventPayload::ActionFailed { diagnostic, .. } if diagnostic.code == "XS3505"
    )));
}
