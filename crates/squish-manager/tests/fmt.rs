use std::{
    path::Path,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use squish_kernel::{CancellationToken, EventSink, InvocationContext, Kernel, SinkError};
use squish_manager::{
    InvocationSettings, ManagerCapability, ResolveRequest, ResolvedDependencies, ServiceError,
    Services, StorageLayout,
};
use squish_project::{Lockfile, ResolutionMode};
use squish_protocol::{
    Event, EventPayload, FormatRequest, FormatSelection, InvocationId, OperationRequest,
    ProjectPath, WorkspaceScope,
};
use squish_repository::PackageLocation;
use squish_xml_front::DSL_NAMESPACE;

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().expect("event mutex poisoned").push(event);
        Ok(())
    }
}

struct FormatBarrierEvents {
    events: Mutex<Vec<Event>>,
    started: AtomicUsize,
    both_started: Barrier,
    release: Barrier,
}

impl FormatBarrierEvents {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            started: AtomicUsize::new(0),
            both_started: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

impl EventSink for FormatBarrierEvents {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        let is_format_start = matches!(
            &event.payload,
            EventPayload::ActionStarted { action, .. }
                if action.as_str().starts_with("format.10.source")
        );
        self.events
            .lock()
            .expect("event mutex poisoned")
            .push(event);
        if is_format_start && self.started.fetch_add(1, Ordering::AcqRel) + 1 == 2 {
            self.both_started.wait();
            self.release.wait();
        }
        Ok(())
    }
}

struct UnusedServices;

impl Services for UnusedServices {
    fn storage_layout(&self, project: &Path) -> Result<StorageLayout, ServiceError> {
        Ok(StorageLayout::project_local_for_tests(project))
    }

    fn materialize_locked(
        &self,
        _: &Path,
        _: &Lockfile,
        _: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Err(ServiceError::new("unused", "unexpected materialization"))
    }

    fn resolve(&self, _: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        Err(ServiceError::new("unused", "unexpected resolution"))
    }
}

#[test]
fn discovery_failure_is_a_planning_failure_without_fake_actions() {
    let directory = tempfile::tempdir().unwrap();
    let request = OperationRequest::Format(FormatRequest {
        project: ProjectPath::new(directory.path().to_string_lossy()).unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: None,
        check: true,
        diff: false,
    });
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("fmt-failure").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    let manager = ManagerCapability::with_default_settings(UnusedServices);
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let outcome = Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&request, &context)
        .unwrap();

    assert_eq!(outcome.summary.totals.total(), 0);
    assert_eq!(outcome.summary.root_failures, 1);
    let events = events.0.lock().unwrap();
    assert!(matches!(
        &events[0].payload,
        EventPayload::PlanningStarted { .. }
    ));
    assert!(matches!(
        &events[1].payload,
        EventPayload::PlanningStepStarted {
            kind: squish_protocol::PlanningStepKind::Locate,
            ..
        }
    ));
    assert!(matches!(
        &events[2].payload,
        EventPayload::PlanningStepFailed { .. }
    ));
    assert!(matches!(
        &events[3].payload,
        EventPayload::PlanningFailed { .. }
    ));
    assert!(!events.iter().any(|event| matches!(
        &event.payload,
        EventPayload::ActionDeclared { .. }
            | EventPayload::ActionStarted { .. }
            | EventPayload::ActionSucceeded { .. }
            | EventPayload::ActionFailed { .. }
    )));
}

#[test]
fn write_ignores_remote_lock_and_returns_semantically_checked_diff() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"app\"\nversion = \"1.0.0\"\nsource-root = \"src\"\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("xmlsquish.lock"),
        r#"lock-version = 1
resolver-version = "test/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
id = "remote@1"
name = "remote"
version = "1.0.0"
manifest-digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
source = { kind = "registry", registry = "test", checksum = "sha256:cccccccccccccccccccccccccccccccc" }
"#,
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    let source = directory.path().join("src/main.xml");
    std::fs::write(
        &source,
        format!("<xs:entry   xmlns:xs = \"{DSL_NAMESPACE}\">hello</xs:entry>"),
    )
    .unwrap();
    let request = OperationRequest::Format(FormatRequest {
        project: ProjectPath::new(directory.path().to_string_lossy()).unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: None,
        check: false,
        diff: true,
    });
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("fmt-write").unwrap(),
        CancellationToken::default(),
        events,
    );
    let manager = ManagerCapability::with_default_settings(UnusedServices);
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let outcome = Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&request, &context)
        .unwrap();
    let squish_protocol::OperationResult::Format(result) = outcome.result else {
        panic!("format should succeed")
    };

    assert_eq!(result.selected.len(), 1);
    assert_eq!(result.changed, result.selected);
    assert_eq!(result.diffs.len(), 1);
    assert_eq!(
        result.diffs[0].kind,
        squish_protocol::ArtifactKind::Other("format-diff+json".into())
    );
    assert_eq!(
        std::fs::read_to_string(source).unwrap(),
        format!("<xs:entry xmlns:xs=\"{DSL_NAMESPACE}\">hello</xs:entry>")
    );
}

#[test]
fn format_workers_cross_barrier_and_one_failure_prevents_batch_write() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[package]\nname = \"app\"\nversion = \"1.0.0\"\nsource-root = \"src\"\n",
    )
    .unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    let valid = directory.path().join("src/a.xml");
    let invalid = directory.path().join("src/z.xml");
    let original = format!("<xs:entry   xmlns:xs = \"{DSL_NAMESPACE}\">ok</xs:entry>");
    std::fs::write(&valid, &original).unwrap();
    std::fs::write(&invalid, "<plain   value = \"not-an-entry\"/>").unwrap();
    let request = OperationRequest::Format(FormatRequest {
        project: ProjectPath::new(directory.path().to_string_lossy()).unwrap(),
        scope: WorkspaceScope::Current,
        selection: FormatSelection::All,
        style: None,
        check: false,
        diff: false,
    });
    let events = Arc::new(FormatBarrierEvents::new());
    let context = InvocationContext::new(
        InvocationId::new("fmt-atomic-failure").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    let run = std::thread::spawn(move || {
        let manager = ManagerCapability::new(
            UnusedServices,
            InvocationSettings {
                jobs: 2,
                ..InvocationSettings::default()
            },
        );
        let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
        Kernel::new(&capabilities)
            .unwrap()
            .dispatch(&request, &context)
            .unwrap()
    });
    events.both_started.wait();
    assert!(events.events.lock().unwrap().iter().all(|event| !matches!(
        &event.payload,
        EventPayload::ActionSucceeded { action, .. } | EventPayload::ActionFailed { action, .. }
            if action.as_str().starts_with("format.10.source")
    )));
    events.release.wait();
    let outcome = run.join().unwrap();

    assert_eq!(outcome.summary.totals.failed, 1);
    assert_eq!(outcome.summary.totals.blocked, 1);
    let events = events.events.lock().unwrap();
    let second_started = events
        .iter()
        .rposition(|event| matches!(&event.payload, EventPayload::ActionStarted { action, .. } if action.as_str().starts_with("format.10.source")))
        .unwrap();
    let first_format_terminal = events
        .iter()
        .position(|event| {
            matches!(&event.payload,
            EventPayload::ActionSucceeded { action, .. } | EventPayload::ActionFailed { action, .. }
            if action.as_str().starts_with("format.10.source"))
        })
        .unwrap();
    assert!(
        second_started < first_format_terminal,
        "both independent format workers must be dispatched before either completes"
    );
    drop(events);
    assert_eq!(std::fs::read_to_string(valid).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(invalid).unwrap(),
        "<plain   value = \"not-an-entry\"/>"
    );
}
