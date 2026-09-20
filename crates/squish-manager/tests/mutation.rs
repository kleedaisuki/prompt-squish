use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use squish_kernel::{CancellationToken, Capability, EventSink, InvocationContext, SinkError};
use squish_manager::{
    InvocationSettings, ManagerCapability, ProjectBuildLayout, ResolveRequest,
    ResolvedDependencies, ServiceError, Services,
};
use squish_project::{LOCK_VERSION, Lockfile, ResolutionMode};
use squish_protocol::{
    AddRequest, DependencyKind, DependencyName, DependencySource, Event, EventPayload,
    InvocationId, LockMode, OperationRequest, OperationResult, PackageName, ProjectPath,
    RegistryName, RemoveRequest, VersionRequirement,
};
use squish_repository::PackageLocation;
use tempfile::TempDir;

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct Resolver;

impl Services for Resolver {
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

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        Ok(ResolvedDependencies {
            lockfile: Lockfile {
                lock_version: LOCK_VERSION,
                resolver_version: "mutation-test/1".into(),
                manifest_digest: request.manifest_digest.into(),
                packages: Vec::new(),
            },
            packages: Vec::new(),
        })
    }
}

struct FailingResolver;

impl Services for FailingResolver {
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
        Err(ServiceError::new(
            "test_resolve",
            "resolver rejected candidate",
        ))
    }
}

struct RacingResolver {
    manifest: std::path::PathBuf,
    changed: AtomicBool,
}

impl Services for RacingResolver {
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

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        if !self.changed.swap(true, Ordering::SeqCst) {
            let mut source = fs::read_to_string(&self.manifest).unwrap();
            source.push_str("\n# concurrent but valid edit\n");
            fs::write(&self.manifest, source).unwrap();
        }
        Ok(ResolvedDependencies {
            lockfile: Lockfile {
                lock_version: LOCK_VERSION,
                resolver_version: "racing-test/1".into(),
                manifest_digest: request.manifest_digest.into(),
                packages: Vec::new(),
            },
            packages: Vec::new(),
        })
    }
}

fn context() -> InvocationContext {
    InvocationContext::new(
        InvocationId::new("mutation-test").unwrap(),
        CancellationToken::default(),
        Arc::new(Events::default()),
    )
}

fn manifest(dependency: &str) -> String {
    format!(
        "# keep this comment\nmanifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n\n[dependencies]\n{dependency}"
    )
}

#[test]
fn dry_run_resolves_candidate_but_does_not_commit() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("xmlsquish.toml");
    let before = manifest("");
    fs::write(&path, &before).unwrap();
    let manager = ManagerCapability::new(Resolver, InvocationSettings::default());
    let request = AddRequest {
        project: ProjectPath::new(temp.path().to_string_lossy()).unwrap(),
        package: Some(PackageName::new("demo").unwrap()),
        dependency: DependencyName::new("dep").unwrap(),
        rename: None,
        source: DependencySource::Registry {
            registry: Some(RegistryName::new("test").unwrap()),
            version: VersionRequirement::new("^1").unwrap(),
        },
        kind: DependencyKind::Normal,
        features: Vec::new(),
        no_default_features: false,
        optional: false,
        lock: LockMode::Update,
        dry_run: true,
    };
    let outcome = manager.execute(&OperationRequest::Add(request), &context());

    assert!(
        matches!(outcome.result, OperationResult::Add(ref result) if result.dry_run && result.before != result.after)
    );
    assert_eq!(fs::read_to_string(path).unwrap(), before);
    assert!(!temp.path().join("xmlsquish.lock").exists());
}

#[test]
fn remove_commits_manifest_and_lock_and_reports_static_imports() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        manifest("dep = \"1\" # preserve neighbor\n"),
    )
    .unwrap();
    fs::write(
        temp.path().join("src/main.xml"),
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><xs:import src="pkg:dep/main"/></xs:module>"#,
    ).unwrap();
    let manager = ManagerCapability::new(Resolver, InvocationSettings::default());
    let request = RemoveRequest {
        project: ProjectPath::new(temp.path().to_string_lossy()).unwrap(),
        package: Some(PackageName::new("demo").unwrap()),
        dependency: DependencyName::new("dep").unwrap(),
        kind: DependencyKind::Normal,
        lock: LockMode::Update,
        dry_run: false,
    };
    let outcome = manager.execute(&OperationRequest::Remove(request), &context());

    assert!(
        matches!(outcome.result, OperationResult::Remove(ref result) if result.affected_sources.iter().any(|id| id.as_str() == "xmlsquish://demo/src/main.xml"))
    );
    let after = fs::read_to_string(temp.path().join("xmlsquish.toml")).unwrap();
    assert!(after.contains("# keep this comment"));
    assert!(!after.contains("dep ="));
    assert!(temp.path().join("xmlsquish.lock").exists());
}

#[test]
fn resolver_failure_is_attributed_to_resolve_candidate_stage() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("xmlsquish.toml"), manifest("")).unwrap();
    let manager = ManagerCapability::new(FailingResolver, InvocationSettings::default());
    let request = AddRequest {
        project: ProjectPath::new(temp.path().to_string_lossy()).unwrap(),
        package: Some(PackageName::new("demo").unwrap()),
        dependency: DependencyName::new("dep").unwrap(),
        rename: None,
        source: DependencySource::Registry {
            registry: None,
            version: VersionRequirement::new("1").unwrap(),
        },
        kind: DependencyKind::Normal,
        features: Vec::new(),
        no_default_features: false,
        optional: false,
        lock: LockMode::Update,
        dry_run: true,
    };
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("stage-failure").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );

    let outcome = manager.execute(&OperationRequest::Add(request), &context);

    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable { .. }
    ));
    let events = events.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        EventPayload::PlanningStepFailed { step, .. } if step.as_str() == "resolve-candidate"
    )));
    assert!(events.iter().any(|event| matches!(
        &event.payload,
        EventPayload::PlanningStepStarted { step, kind: squish_protocol::PlanningStepKind::Resolve, .. }
            if step.as_str() == "resolve-candidate"
    )));
}

#[test]
fn contention_supersedes_sealed_plan_and_replans_fresh_candidate() {
    let temp = TempDir::new().unwrap();
    let manifest_path = temp.path().join("xmlsquish.toml");
    fs::write(&manifest_path, manifest("")).unwrap();
    let manager = ManagerCapability::new(
        RacingResolver {
            manifest: manifest_path.clone(),
            changed: AtomicBool::new(false),
        },
        InvocationSettings::default(),
    );
    let request = AddRequest {
        project: ProjectPath::new(temp.path().to_string_lossy()).unwrap(),
        package: Some(PackageName::new("demo").unwrap()),
        dependency: DependencyName::new("dep").unwrap(),
        rename: None,
        source: DependencySource::Registry {
            registry: None,
            version: VersionRequirement::new("1").unwrap(),
        },
        kind: DependencyKind::Normal,
        features: Vec::new(),
        no_default_features: false,
        optional: false,
        lock: LockMode::Update,
        dry_run: false,
    };
    let events = Arc::new(Events::default());
    let context = InvocationContext::new(
        InvocationId::new("contention").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );

    let outcome = manager.execute(&OperationRequest::Add(request), &context);

    assert!(matches!(outcome.result, OperationResult::Add(_)));
    assert!(fs::read_to_string(manifest_path).unwrap().contains("dep ="));
    let events = events.0.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::ActionSuperseded { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event.payload,
        EventPayload::PlanClosed {
            reason: squish_protocol::PlanCloseReason::Superseded,
            ..
        }
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::PlanningStarted { .. }))
            .count(),
        2
    );
}
