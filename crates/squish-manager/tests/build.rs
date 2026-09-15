//! 构建用例的跨 crate 契约测试。 / Cross-crate contract tests for the build use case.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use squish_kernel::{CancellationToken, EventSink, InvocationContext, Kernel, SinkError};
use squish_manager::{
    ArtifactLocator, Effect, InvocationSettings, ManagerCapability, PlannedWork,
    ProvenanceRelation, ResolveRequest, ResolvedDependencies, ServiceError, Services,
    StorageLayout, build,
};
use squish_project::{LockedPackage, LockedSource, Lockfile, ResolutionMode};
use squish_protocol::{
    BuildRequest, EmitKind, Event, EventPayload, InvocationId, LockMode, OperationRequest,
    OperationResult, ProfileName, ProjectPath, WorkspaceScope,
};
use squish_publish::FileArtifactPublisher;
use squish_repository::PackageLocation;
use squish_store::Cas;
use tempfile::TempDir;

struct LocalServices;

struct IgnoreEvents;
impl EventSink for IgnoreEvents {
    fn emit(&self, _event: Event) -> Result<(), SinkError> {
        Ok(())
    }
}

struct CancelOnPlanReady(CancellationToken);

impl EventSink for CancelOnPlanReady {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        if matches!(event.payload, EventPayload::PlanReady { .. }) {
            self.0.cancel();
        }
        Ok(())
    }
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<Event>>);
impl EventSink for RecordingEvents {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl Services for LocalServices {
    fn storage_layout(&self, project: &Path) -> Result<StorageLayout, ServiceError> {
        Ok(StorageLayout::project_local_for_tests(project))
    }

    fn materialize_locked(
        &self,
        _project: &Path,
        _lock: &Lockfile,
        _mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Ok(Vec::new())
    }

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        let manifest = request
            .manifests
            .values()
            .find(|manifest| manifest.package.is_some())
            .expect("fixture has a package");
        let package = manifest.package.as_ref().unwrap();
        Ok(ResolvedDependencies {
            lockfile: Lockfile {
                lock_version: squish_project::LOCK_VERSION,
                resolver_version: "test/1".into(),
                manifest_digest: request.manifest_digest.into(),
                packages: vec![LockedPackage {
                    id: "fixture-local".into(),
                    name: package.name.clone(),
                    version: package.version.clone(),
                    source: LockedSource::Path {
                        path: ".".into(),
                        mutable: true,
                    },
                    manifest_digest:
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into(),
                    dependencies: BTreeMap::new(),
                }],
            },
            packages: Vec::new(),
        })
    }
}

fn fixture() -> (TempDir, BuildRequest) {
    let temp = TempDir::new_in(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".temp"),
    )
    .expect("workspace test directory exists");
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        r#"manifest-version = 1
[package]
name = "fixture"
version = "1.0.0"

[target.chat]
entry = "src/main.xml"
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("src/main.xml"),
        r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><message>Hello world</message></xs:entry>"#,
    )
    .unwrap();
    let request = BuildRequest {
        project: ProjectPath::new(temp.path().display().to_string()).unwrap(),
        scope: WorkspaceScope::Current,
        targets: Vec::new(),
        profile: ProfileName::new("dev").unwrap(),
        arguments: BTreeMap::new(),
        emit: vec![EmitKind::Prompt, EmitKind::DebugInfo, EmitKind::BinaryIr],
        lock: LockMode::Update,
    };
    (temp, request)
}

#[test]
fn prepared_plan_has_one_compile_per_sealed_source_and_effectful_publish() {
    let (_temp, request) = fixture();
    let prepared = build::prepare(&request, &LocalServices).unwrap();
    let works: Vec<_> = prepared.plan().works().map(|(_, work)| work).collect();

    assert_eq!(
        works
            .iter()
            .filter(|work| matches!(work, build::BuildWork::Compile { .. }))
            .count(),
        1
    );
    assert!(
        works
            .iter()
            .any(|work| work.effect() == Effect::WriteEffect)
    );
    assert!(
        works
            .iter()
            .filter(|work| work.effect() != Effect::Transform)
            .all(|work| !work.effect().cacheable())
    );
}

#[test]
fn complete_build_publishes_prompt_debug_and_ir_from_cas() {
    let (temp, request) = fixture();
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("complete-build-test").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("expected build result")
    };

    assert_eq!(result.published.len(), 1);
    assert!(
        result.build_record.is_some(),
        "successful builds persist their build record"
    );
    let layout = StorageLayout::project_local_for_tests(temp.path());
    let catalog = build::read_current_build_catalog(&layout)
        .unwrap()
        .expect("successful build publishes its current catalog record");
    assert_eq!(Some(&catalog.record_artifact), result.build_record.as_ref());
    assert_eq!(catalog.record.schema, 2);
    assert_eq!(
        catalog.record.plan.actions.len(),
        catalog.record.actions.len()
    );
    let artifacts = &result.published[0].artifacts;
    assert!(
        artifacts
            .iter()
            .any(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::Prompt))
    );
    assert!(
        artifacts
            .iter()
            .any(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::DebugInfo))
    );
    assert!(
        artifacts
            .iter()
            .any(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::BinaryIr))
    );
    let publication_root = temp.path().join("target/xmlsquish");
    let publisher = FileArtifactPublisher::open(
        &publication_root,
        Cas::open(temp.path().join(".cache/xmlsquish/cas")).unwrap(),
    )
    .unwrap();
    let generation = publisher
        .current_generation("fixture:chat")
        .unwrap()
        .unwrap();
    assert_eq!(generation.artifacts, *artifacts);
    assert_eq!(generation.artifacts.len(), 5);
    let prompt = artifacts
        .iter()
        .find(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::Prompt))
        .unwrap();
    assert_eq!(
        fs::read_to_string(publication_root.join(&prompt.uri)).unwrap(),
        "<message> Hello world </message>"
    );
    let debug = artifacts
        .iter()
        .find(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::DebugInfo))
        .unwrap();
    let debug = fs::read(publication_root.join(&debug.uri)).unwrap();
    squish_ir::decode_debug_bundle(&debug)
        .expect("published debug bundle is self-contained and valid");
    let link_map = artifacts
        .iter()
        .find(|artifact| {
            matches!(&artifact.kind, squish_protocol::ArtifactKind::Other(name) if name == "static-link-map")
        })
        .expect("target generation contains its typed static link map");
    squish_ir::decode_static_link_map(&fs::read(publication_root.join(&link_map.uri)).unwrap())
        .expect("published static link map uses the public canonical codec");
    assert_eq!(
        catalog
            .link_map(&squish_protocol::TargetName::new("chat").unwrap())
            .unwrap(),
        Some(link_map)
    );
    let recorded_prompt = catalog
        .record
        .targets
        .iter()
        .flat_map(|target| &target.artifacts)
        .find(|item| item.artifact.id == prompt.id)
        .unwrap();
    let locator = ArtifactLocator::new(recorded_prompt.destination.clone()).unwrap();
    assert_eq!(catalog.artifact_at(&locator), Some(prompt));
    let ProvenanceRelation::Evidence(evidence) = catalog.provenance_relation(&prompt.id) else {
        panic!("prompt has typed provenance evidence")
    };
    assert!(
        evidence
            .iter()
            .any(|item| item.id == catalog.record_artifact.id)
    );
    assert!(
        evidence
            .iter()
            .any(|item| matches!(item.kind, squish_protocol::ArtifactKind::DebugInfo))
    );
}

#[test]
fn build_catalog_distinguishes_absence_from_corruption_and_rejects_traversal() {
    let (temp, request) = fixture();
    let layout = StorageLayout::project_local_for_tests(temp.path());
    assert!(
        build::read_current_build_catalog(&layout)
            .unwrap()
            .is_none()
    );
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("catalog-corruption-test").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("catalog fixture build must produce a typed result")
    };
    assert_eq!(result.published.len(), 1);
    let snapshot = build::read_current_build_catalog(&layout).unwrap().unwrap();
    let wire = serde_json::to_value(&snapshot.record).unwrap();
    for invalid in [
        "../escape",
        r"a\..\escape",
        "C:escape",
        "//server/share",
        "a//b",
        "./a",
    ] {
        let mut invalid_wire = wire.clone();
        invalid_wire["targets"][0]["artifacts"][0]["destination"] = serde_json::json!(invalid);
        assert!(
            build::decode_build_record(&serde_json::to_vec(&invalid_wire).unwrap()).is_err(),
            "noncanonical catalog path `{invalid}` must be rejected on every host"
        );
    }
    let mut unknown_field = wire;
    unknown_field["future_field"] = serde_json::json!(true);
    assert!(
        build::decode_build_record(&serde_json::to_vec(&unknown_field).unwrap()).is_err(),
        "schema 2 must not silently discard unknown semantic fields"
    );
    fs::write(
        layout.catalog_root().join(&snapshot.record_artifact.uri),
        b"corrupt",
    )
    .unwrap();
    assert!(matches!(
        build::read_current_build_catalog(&layout),
        Err(build::BuildCatalogError::Corrupt(_))
    ));
}

#[test]
fn successful_execute_returns_the_persisted_build_record() {
    let (temp, request) = fixture();
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("build-record-test").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let operation = OperationRequest::Build(request);
    let outcome = kernel.dispatch(&operation, &context).unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("expected build result")
    };
    assert!(result.build_record.is_some());
    let publication_root = temp.path().join("target/xmlsquish");
    let cold_bytes: Vec<_> = result.published[0]
        .artifacts
        .iter()
        .map(|artifact| fs::read(publication_root.join(&artifact.uri)).unwrap())
        .collect();
    let warm_events = Arc::new(RecordingEvents::default());
    let warm = InvocationContext::new(
        InvocationId::new("build-record-warm-test").unwrap(),
        CancellationToken::default(),
        warm_events.clone(),
    );
    let warm_manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let warm_capabilities: [&dyn squish_kernel::Capability; 1] = [&warm_manager];
    let warm_kernel = Kernel::new(&warm_capabilities).unwrap();
    let second = warm_kernel.dispatch(&operation, &warm).unwrap();
    let OperationResult::Build(result) = second.result else {
        panic!("expected warm build result")
    };
    assert!(
        result.build_record.is_some(),
        "cache hydration preserves the complete result"
    );
    let warm_bytes: Vec<_> = result.published[0]
        .artifacts
        .iter()
        .map(|artifact| fs::read(publication_root.join(&artifact.uri)).unwrap())
        .collect();
    assert_eq!(warm_bytes, cold_bytes);
    let cold_debug = cold_bytes
        .iter()
        .find_map(|bytes| squish_ir::decode_debug_bundle(bytes).ok())
        .unwrap();
    let warm_debug = warm_bytes
        .iter()
        .find_map(|bytes| squish_ir::decode_debug_bundle(bytes).ok())
        .unwrap();
    assert_eq!(warm_debug, cold_debug);
    let cached: Vec<_> = warm_events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::CacheHit { action, .. } => Some(action.as_str().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(cached.len(), 4);
    for phase in ["compile", "link", "instantiate", "backend"] {
        let prefix = format!("build:{phase}:");
        assert!(
            cached.iter().any(|action| action.starts_with(&prefix)),
            "warm build must hydrate the {phase} transform from its persistent cache"
        );
    }
    assert!(
        cached
            .iter()
            .all(|action| !action.starts_with("build:publish:")
                && !action.starts_with("build:build-record:")),
        "effectful publication and build-record actions must never enter the action index"
    );
}

#[test]
fn compile_failure_persists_terminal_action_facts() {
    let (temp, request) = fixture();
    fs::write(temp.path().join("src/main.xml"), "<broken").unwrap();
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("compile-failure-record").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("execution failure still has a typed build result")
    };
    let result_record = result
        .build_record
        .expect("a sealed failed plan persists its aggregate record");
    let catalog =
        build::read_current_build_catalog(&StorageLayout::project_local_for_tests(temp.path()))
            .unwrap()
            .unwrap();
    assert_eq!(catalog.record_artifact, result_record);
    assert!(catalog.record.actions.iter().any(|fact| {
        fact.kind == squish_protocol::ActionKind::Compile
            && matches!(fact.state, build::BuildTerminalState::Failed { .. })
    }));
    assert!(catalog.record.actions.iter().any(|fact| {
        matches!(
            fact.state,
            build::BuildTerminalState::Blocked { .. } | build::BuildTerminalState::Cancelled
        )
    }));
}

#[test]
fn cancellation_after_plan_seal_persists_cancelled_action_facts() {
    let (temp, request) = fixture();
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let token = CancellationToken::default();
    let context = InvocationContext::new(
        InvocationId::new("pre-execution-cancel-record").unwrap(),
        token.clone(),
        Arc::new(CancelOnPlanReady(token)),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("a cancelled sealed plan still has a typed build result")
    };
    assert_eq!(outcome.summary.status.code(), 130);
    assert!(result.build_record.is_some());
    let record =
        build::read_current_build_record(&StorageLayout::project_local_for_tests(temp.path()))
            .unwrap()
            .unwrap();
    assert!(
        record
            .actions
            .iter()
            .any(|fact| matches!(fact.state, build::BuildTerminalState::Cancelled))
    );
    assert!(
        record
            .actions
            .iter()
            .all(|fact| !matches!(fact.state, build::BuildTerminalState::Succeeded))
    );
}

#[test]
fn failed_and_cancelled_attempts_retain_the_previous_target_generation() {
    let (temp, request) = fixture();
    let layout = StorageLayout::project_local_for_tests(temp.path());
    let target = squish_protocol::TargetName::new("chat").unwrap();

    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("catalog-continuity-success").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    kernel
        .dispatch(&OperationRequest::Build(request.clone()), &context)
        .unwrap();
    let good_map = build::read_current_build_catalog(&layout)
        .unwrap()
        .unwrap()
        .link_map(&target)
        .unwrap()
        .unwrap()
        .clone();

    fs::write(temp.path().join("src/main.xml"), "<broken").unwrap();
    let failed_manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let failed_capabilities: [&dyn squish_kernel::Capability; 1] = [&failed_manager];
    let failed_kernel = Kernel::new(&failed_capabilities).unwrap();
    let failed_context = InvocationContext::new(
        InvocationId::new("catalog-continuity-failure").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    failed_kernel
        .dispatch(&OperationRequest::Build(request.clone()), &failed_context)
        .unwrap();
    let failed = build::read_current_build_catalog(&layout).unwrap().unwrap();
    assert_eq!(failed.link_map(&target).unwrap(), Some(&good_map));
    assert!(
        failed
            .record
            .actions
            .iter()
            .any(|fact| matches!(fact.state, build::BuildTerminalState::Failed { .. }))
    );

    let token = CancellationToken::default();
    let cancelled_manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let cancelled_capabilities: [&dyn squish_kernel::Capability; 1] = [&cancelled_manager];
    let cancelled_kernel = Kernel::new(&cancelled_capabilities).unwrap();
    let cancelled_context = InvocationContext::new(
        InvocationId::new("catalog-continuity-cancel").unwrap(),
        token.clone(),
        Arc::new(CancelOnPlanReady(token)),
    );
    cancelled_kernel
        .dispatch(&OperationRequest::Build(request), &cancelled_context)
        .unwrap();
    let cancelled = build::read_current_build_catalog(&layout).unwrap().unwrap();
    assert_eq!(cancelled.link_map(&target).unwrap(), Some(&good_map));
    assert!(
        cancelled
            .record
            .actions
            .iter()
            .any(|fact| matches!(fact.state, build::BuildTerminalState::Cancelled))
    );
}

#[test]
fn same_semantic_targets_restore_single_flight_results_for_each_owner() {
    let (temp, request) = fixture();
    let manifest = fs::read_to_string(temp.path().join("xmlsquish.toml")).unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        format!(
            "{manifest}\n[target.second]\nentry = \"src/main.xml\"\noutput = \"second.prompt\"\n"
        ),
    )
    .unwrap();
    let manager = ManagerCapability::new(
        LocalServices,
        InvocationSettings {
            jobs: 4,
            ..Default::default()
        },
    );
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("single-flight-targets").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("expected build result")
    };
    let record =
        build::read_current_build_record(&StorageLayout::project_local_for_tests(temp.path()))
            .unwrap()
            .unwrap();
    assert_eq!(
        result.published.len(),
        2,
        "action facts: {:?}",
        record.actions
    );
    assert!(
        result
            .published
            .iter()
            .all(|target| !target.artifacts.is_empty())
    );
}

#[test]
fn same_named_workspace_targets_keep_distinct_internal_owners() {
    let temp = TempDir::new_in(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".temp"),
    )
    .unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        "manifest-version = 1\n[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    for package in ["a", "b"] {
        let root = temp.path().join(package);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("xmlsquish.toml"), format!("manifest-version = 1\n[package]\nname = \"{package}\"\nversion = \"1.0.0\"\n[target.chat]\nentry = \"src/main.xml\"\noutput = \"{package}.prompt\"\n")).unwrap();
        fs::write(
            root.join("src/main.xml"),
            r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><x/></xs:entry>"#,
        )
        .unwrap();
    }
    let request = BuildRequest {
        project: ProjectPath::new(temp.path().display().to_string()).unwrap(),
        scope: WorkspaceScope::Workspace,
        targets: Vec::new(),
        profile: ProfileName::new("dev").unwrap(),
        arguments: BTreeMap::new(),
        emit: vec![EmitKind::Prompt],
        lock: LockMode::Update,
    };
    let prepared = build::prepare(&request, &WorkspaceServices).unwrap();
    let mut owners: Vec<_> = prepared
        .plan()
        .works()
        .filter_map(|(_, work)| match work {
            build::BuildWork::Link { target, .. } => Some(target.as_str()),
            _ => None,
        })
        .collect();
    owners.sort_unstable();
    assert_eq!(owners, ["a:chat", "b:chat"]);
    let compile_keys: std::collections::BTreeSet<_> = prepared
        .plan()
        .graph()
        .actions()
        .filter(|action| action.kind == squish_protocol::ActionKind::Compile)
        .map(|action| {
            action
                .key
                .materialize(&action.kind, &action.outputs, |_| None)
                .unwrap()
                .as_str()
                .to_owned()
        })
        .collect();
    assert_eq!(
        compile_keys.len(),
        2,
        "SourceKey participates even for equal source bytes"
    );
    let manager = ManagerCapability::new(WorkspaceServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("ambiguous-workspace-target").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let catalog =
        build::read_current_build_catalog(&StorageLayout::project_local_for_tests(temp.path()))
            .unwrap()
            .unwrap();
    assert!(matches!(
        catalog.link_map(&squish_protocol::TargetName::new("chat").unwrap()),
        Err(build::BuildRecordQueryError::AmbiguousTarget(_))
    ));
}

#[test]
fn link_key_changes_when_resolution_manifest_changes_with_equal_source_bytes() {
    let (temp, request) = fixture();
    let first = build::prepare(&request, &LocalServices).unwrap();
    let first_key = link_key(&first);
    let manifest = fs::read_to_string(temp.path().join("xmlsquish.toml")).unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        format!("{manifest}\n[profile.unused]\noptimization = \"none\"\n"),
    )
    .unwrap();
    let second = build::prepare(&request, &LocalServices).unwrap();
    assert_ne!(first_key, link_key(&second));
}

fn link_key(prepared: &build::PreparedBuild) -> String {
    let placeholder =
        squish_protocol::Digest::new(squish_protocol::DigestAlgorithm::Blake3, vec![0; 32])
            .unwrap();
    let action = prepared
        .plan()
        .graph()
        .actions()
        .find(|action| action.kind == squish_protocol::ActionKind::Link)
        .unwrap();
    action
        .key
        .materialize(&action.kind, &action.outputs, |_| Some(placeholder.clone()))
        .unwrap()
        .as_str()
        .to_owned()
}

struct WorkspaceServices;
impl Services for WorkspaceServices {
    fn storage_layout(&self, project: &Path) -> Result<StorageLayout, ServiceError> {
        Ok(StorageLayout::project_local_for_tests(project))
    }

    fn materialize_locked(
        &self,
        _project: &Path,
        _lock: &Lockfile,
        _mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Ok(Vec::new())
    }
    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        let packages = request.manifests.iter().filter_map(|(path, manifest)| manifest.package.as_ref().map(|package| {
            let member = Path::new(path).parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
            LockedPackage {
                id: package.name.clone(), name: package.name.clone(), version: package.version.clone(),
                source: LockedSource::Workspace { member, mutable: true },
                manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                dependencies: BTreeMap::new(),
            }
        })).collect();
        Ok(ResolvedDependencies {
            lockfile: Lockfile {
                lock_version: squish_project::LOCK_VERSION,
                resolver_version: "test/1".into(),
                manifest_digest: request.manifest_digest.into(),
                packages,
            },
            packages: Vec::new(),
        })
    }
}
