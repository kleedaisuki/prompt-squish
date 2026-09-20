//! 构建用例的跨 crate 契约测试。 / Cross-crate contract tests for the build use case.

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use squish_build::{ArtifactRead, PublicationTargetId};
use squish_kernel::{CancellationToken, EventSink, InvocationContext, Kernel, SinkError};
use squish_manager::{
    ArtifactLocator, Effect, InvocationSettings, ManagerCapability, PlannedWork,
    ProjectBuildLayout, ProvenanceRelation, ResolveRequest, ResolvedDependencies, ServiceError,
    Services, build,
};
use squish_project::{LockedPackage, LockedSource, Lockfile, ResolutionMode};
use squish_protocol::{
    BuildRequest, EmitKind, Event, EventPayload, InvocationId, LockMode, OperationRequest,
    OperationResult, ProfileName, ProjectPath, WorkspaceScope,
};
use squish_publish::{
    DurablePoint, FileArtifactPublisher, NoopObserver, PublishEvent, PublishObserver,
};
use squish_repository::PackageLocation;
use squish_store::Cas;
use tempfile::TempDir;

use common::{MemoryBuildRuntime, RuntimeFault, TestBuildRuntime};

struct LocalServices;

struct IgnoreEvents;
impl EventSink for IgnoreEvents {
    fn emit(&self, _event: Event) -> Result<(), SinkError> {
        Ok(())
    }
}

fn read_generation_member(
    publisher: &FileArtifactPublisher<Cas>,
    generation: &squish_build::CommittedGeneration,
    locator: &str,
) -> Vec<u8> {
    let path = squish_build::PublicationPath::new(locator).unwrap();
    let mut bytes = Vec::new();
    assert!(matches!(
        publisher
            .read_generation_artifact(&generation.identity, &path, &mut bytes)
            .unwrap(),
        ArtifactRead::Verified(_)
    ));
    bytes
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

struct RecordAndCancelOnPlanReady {
    token: CancellationToken,
    events: Mutex<Vec<Event>>,
}

impl EventSink for RecordAndCancelOnPlanReady {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        if matches!(event.payload, EventPayload::PlanReady { .. }) {
            self.token.cancel();
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

struct RestoreCatalogOnFailure {
    catalog_root: std::path::PathBuf,
    events: Mutex<Vec<Event>>,
}

impl EventSink for RestoreCatalogOnFailure {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        if matches!(event.payload, EventPayload::FinalizationFailed { .. }) {
            let backup = self.catalog_root.with_extension("faulted-catalog");
            fs::remove_file(&self.catalog_root).unwrap();
            fs::rename(backup, &self.catalog_root).unwrap();
        }
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

struct BreakCatalogOnTargetCommit {
    catalog_root: std::path::PathBuf,
    broken: std::sync::atomic::AtomicBool,
}

impl PublishObserver for BreakCatalogOnTargetCommit {
    fn observe(&self, event: &PublishEvent) {
        if !matches!(
            event,
            PublishEvent::DurablePoint(DurablePoint::CurrentSwitched)
        ) || self.broken.swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let backup = self.catalog_root.with_extension("faulted-catalog");
        fs::rename(&self.catalog_root, &backup).unwrap();
        fs::write(&self.catalog_root, b"fault").unwrap();
    }
}

struct FaultingServices;

impl Services for FaultingServices {
    fn open_build_runtime(
        &self,
        project: &Path,
    ) -> Result<Arc<dyn squish_manager::BuildRuntime>, ServiceError> {
        let layout = ProjectBuildLayout::project_local_for_tests(project);
        let observer = Arc::new(BreakCatalogOnTargetCommit {
            catalog_root: layout.catalog_root().to_path_buf(),
            broken: std::sync::atomic::AtomicBool::new(false),
        });
        TestBuildRuntime::with_observers(&layout, observer, Arc::new(NoopObserver))
            .map(|runtime| Arc::new(runtime) as Arc<dyn squish_manager::BuildRuntime>)
            .map_err(|error| ServiceError::new(error.code(), error.message()))
    }

    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        LocalServices.storage_layout(project)
    }

    fn materialize_locked(
        &self,
        project: &Path,
        lock: &Lockfile,
        mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        LocalServices.materialize_locked(project, lock, mode)
    }

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        LocalServices.resolve(request)
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
    fn open_build_runtime(
        &self,
        project: &Path,
    ) -> Result<Arc<dyn squish_manager::BuildRuntime>, ServiceError> {
        let layout = ProjectBuildLayout::project_local_for_tests(project);
        TestBuildRuntime::open(&layout)
            .map(|runtime| Arc::new(runtime) as Arc<dyn squish_manager::BuildRuntime>)
            .map_err(|error| ServiceError::new(error.code(), error.message()))
    }

    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Ok(ProjectBuildLayout::project_local_for_tests(project))
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

fn runtime(layout: &ProjectBuildLayout) -> TestBuildRuntime {
    TestBuildRuntime::open(layout).expect("test runtime opens")
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

#[derive(Clone)]
struct MemoryServices {
    runtime: MemoryBuildRuntime,
}

impl Services for MemoryServices {
    fn open_build_runtime(
        &self,
        _project: &Path,
    ) -> Result<Arc<dyn squish_manager::BuildRuntime>, ServiceError> {
        Ok(Arc::new(self.runtime.clone()))
    }

    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        LocalServices.storage_layout(project)
    }

    fn materialize_locked(
        &self,
        project: &Path,
        lock: &Lockfile,
        mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        LocalServices.materialize_locked(project, lock, mode)
    }

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        LocalServices.resolve(request)
    }
}

fn run_memory_build(
    request: BuildRequest,
    runtime: MemoryBuildRuntime,
    invocation: &str,
) -> squish_kernel::DispatchOutcome {
    let manager = ManagerCapability::new(MemoryServices { runtime }, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let context = InvocationContext::new(
        InvocationId::new(invocation).unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap()
}

#[test]
fn memory_runtime_completes_build_and_faults_are_capability_specific() {
    let (_temp, request) = fixture();
    let runtime = MemoryBuildRuntime::new();
    let outcome = run_memory_build(request.clone(), runtime.clone(), "memory-runtime-success");
    assert!(
        matches!(outcome.result, OperationResult::Build(_)),
        "outcome: {outcome:?}"
    );
    assert!(
        build::read_current_build_catalog(&runtime)
            .unwrap()
            .is_some()
    );

    let compile_fault = run_memory_build(
        request.clone(),
        MemoryBuildRuntime::new().with_fault(RuntimeFault::Compile),
        "memory-runtime-compile-fault",
    );
    assert!(compile_fault.summary.totals.failed >= 1);

    let catalog_fault = run_memory_build(
        request,
        MemoryBuildRuntime::new().with_fault(RuntimeFault::PublishCatalog),
        "memory-runtime-catalog-fault",
    );
    assert_eq!(catalog_fault.summary.totals.failed, 0);
    assert_eq!(catalog_fault.summary.root_failures, 1);
    assert!(matches!(
        catalog_fault.result,
        OperationResult::Unavailable { .. }
    ));
}

#[test]
fn catalog_runtime_preserves_corruption_vs_storage_classification() {
    let corrupt = MemoryBuildRuntime::new().with_fault(RuntimeFault::ReadCatalogCorrupt);
    assert!(matches!(
        build::read_current_build_catalog(&corrupt),
        Err(build::BuildCatalogError::Corrupt(message)) if message.contains("corrupt catalog")
    ));

    let storage = MemoryBuildRuntime::new().with_fault(RuntimeFault::ReadCatalogStorage);
    assert!(matches!(
        build::read_current_build_catalog(&storage),
        Err(build::BuildCatalogError::Storage(message)) if message.contains("storage failure")
    ));
}

#[test]
fn runtime_descriptor_identity_changes_the_sealed_plan() {
    let (_temp, request) = fixture();
    let baseline = build::prepare(
        &request,
        &MemoryServices {
            runtime: MemoryBuildRuntime::new(),
        },
    )
    .unwrap()
    .plan()
    .graph()
    .semantic_digest();
    let mut descriptor = squish_manager::BuildRuntimeDescriptor {
        frontend_abi: "xmlsquish.xml/2-test".into(),
        linker_abi: "xmlsquish.link/1".into(),
        evaluator_abi: "xmlsquish.instantiate/1".into(),
        document_abi: squish_backend::DOCUMENT_ABI.into(),
    };
    let changed = build::prepare(
        &request,
        &MemoryServices {
            runtime: MemoryBuildRuntime::new().with_descriptor(descriptor.clone()),
        },
    )
    .unwrap()
    .plan()
    .graph()
    .semantic_digest();
    assert_ne!(baseline, changed);

    for descriptor in [
        squish_manager::BuildRuntimeDescriptor {
            frontend_abi: "xmlsquish.xml/1".into(),
            linker_abi: "xmlsquish.link/2-test".into(),
            evaluator_abi: "xmlsquish.instantiate/1".into(),
            document_abi: squish_backend::DOCUMENT_ABI.into(),
        },
        squish_manager::BuildRuntimeDescriptor {
            frontend_abi: "xmlsquish.xml/1".into(),
            linker_abi: "xmlsquish.link/1".into(),
            evaluator_abi: "xmlsquish.instantiate/2-test".into(),
            document_abi: squish_backend::DOCUMENT_ABI.into(),
        },
        squish_manager::BuildRuntimeDescriptor {
            frontend_abi: "xmlsquish.xml/1".into(),
            linker_abi: "xmlsquish.link/1".into(),
            evaluator_abi: "xmlsquish.instantiate/1".into(),
            document_abi: "xmlsquish.document.v2-test".into(),
        },
    ] {
        let digest = build::prepare(
            &request,
            &MemoryServices {
                runtime: MemoryBuildRuntime::new().with_descriptor(descriptor),
            },
        )
        .unwrap()
        .plan()
        .graph()
        .semantic_digest();
        assert_ne!(baseline, digest);
    }

    descriptor.frontend_abi = "xmlsquish.xml/1".into();
    let restored = build::prepare(
        &request,
        &MemoryServices {
            runtime: MemoryBuildRuntime::new().with_descriptor(descriptor),
        },
    )
    .unwrap()
    .plan()
    .graph()
    .semantic_digest();
    assert_eq!(baseline, restored);
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
fn emit_set_is_order_insensitive_and_plan_sensitive() {
    let (_temp, mut request) = fixture();
    request.emit = vec![EmitKind::Prompt, EmitKind::DebugInfo];
    let first_request = request.clone();
    let first = build::prepare(&request, &LocalServices)
        .unwrap()
        .plan()
        .graph()
        .semantic_digest();
    request.emit = vec![EmitKind::DebugInfo, EmitKind::Prompt, EmitKind::Prompt];
    let reordered_request = request.clone();
    let reordered = build::prepare(&request, &LocalServices)
        .unwrap()
        .plan()
        .graph()
        .semantic_digest();
    assert_eq!(first, reordered);
    request.emit = vec![EmitKind::Prompt];
    let prompt_request = request.clone();
    let prompt_only = build::prepare(&request, &LocalServices)
        .unwrap()
        .plan()
        .graph()
        .semantic_digest();
    request.emit = vec![EmitKind::Prompt, EmitKind::BinaryIr];
    let ir_request = request.clone();
    let with_ir = build::prepare(&request, &LocalServices)
        .unwrap()
        .plan()
        .graph()
        .semantic_digest();
    assert_ne!(first, prompt_only);
    assert_ne!(prompt_only, with_ir);
    let first_plan = observed_plan_digest(&first_request, "emit-plan-a");
    let reordered_plan = observed_plan_digest(&reordered_request, "emit-plan-b");
    let prompt_plan = observed_plan_digest(&prompt_request, "emit-plan-c");
    let ir_plan = observed_plan_digest(&ir_request, "emit-plan-d");
    assert_eq!(first_plan, reordered_plan);
    assert_ne!(first_plan, prompt_plan);
    assert_ne!(prompt_plan, ir_plan);
}

fn observed_plan_digest(request: &BuildRequest, invocation: &str) -> squish_protocol::PlanDigest {
    let events = Arc::new(RecordingEvents::default());
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new(invocation).unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    kernel
        .dispatch(&OperationRequest::Build(request.clone()), &context)
        .unwrap();
    events
        .0
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::PlanReady { digest, .. } => Some(digest.clone()),
            _ => None,
        })
        .unwrap()
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
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let catalog = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .expect("successful build publishes its current catalog record");
    assert_eq!(Some(&catalog.record_artifact), result.build_record.as_ref());
    assert_eq!(catalog.record.schema, 3);
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
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let publisher = FileArtifactPublisher::open(
        layout.artifacts_root(),
        Cas::open(layout.cas_root()).unwrap(),
    )
    .unwrap();
    let generation = publisher
        .current_generation(&PublicationTargetId::new("fixture:chat").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(generation.artifacts.len(), artifacts.len());
    assert_eq!(generation.artifacts.len(), 5);
    let prompt = artifacts
        .iter()
        .find(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::Prompt))
        .unwrap();
    assert_eq!(
        String::from_utf8(read_generation_member(
            &publisher,
            &generation,
            prompt.locator.as_str()
        ))
        .unwrap(),
        "<message> Hello world </message>"
    );
    let debug = artifacts
        .iter()
        .find(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::DebugInfo))
        .unwrap();
    let debug = read_generation_member(&publisher, &generation, debug.locator.as_str());
    squish_ir::decode_debug_bundle(&debug)
        .expect("published debug bundle is self-contained and valid");
    let link_map = artifacts
        .iter()
        .find(|artifact| {
            matches!(&artifact.kind, squish_protocol::ArtifactKind::Other(name) if name == "static-link-map")
        })
        .expect("target generation contains its typed static link map");
    squish_ir::decode_static_link_map(&read_generation_member(
        &publisher,
        &generation,
        link_map.locator.as_str(),
    ))
    .expect("published static link map uses the public canonical codec");
    assert_eq!(
        catalog
            .link_map(&squish_protocol::TargetName::new("chat").unwrap())
            .unwrap(),
        Some(squish_protocol::Artifact {
            id: link_map.id.clone(),
            kind: link_map.kind.clone(),
            uri: link_map.locator.as_str().into(),
            size: link_map.size,
            digest: link_map.digest.clone()
        })
    );
    let recorded_prompt = catalog
        .record
        .targets
        .iter()
        .flat_map(|target| &target.artifacts)
        .find(|item| item.descriptor.id == prompt.id)
        .unwrap();
    let locator = ArtifactLocator::new(recorded_prompt.destination.as_str()).unwrap();
    assert_eq!(catalog.artifact_at(&locator).unwrap().id, prompt.id);
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
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    assert!(
        build::read_current_build_catalog(&runtime(&layout))
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
    let snapshot = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .unwrap();
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
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let publisher = FileArtifactPublisher::open(
        layout.artifacts_root(),
        Cas::open(layout.cas_root()).unwrap(),
    )
    .unwrap();
    let cold_generation = publisher
        .current_generation(&PublicationTargetId::new(&result.published[0].target_id).unwrap())
        .unwrap()
        .unwrap();
    let cold_bytes: Vec<_> = result.published[0]
        .artifacts
        .iter()
        .map(|artifact| {
            read_generation_member(&publisher, &cold_generation, artifact.locator.as_str())
        })
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
    let warm_generation = publisher
        .current_generation(&PublicationTargetId::new(&result.published[0].target_id).unwrap())
        .unwrap()
        .unwrap();
    let warm_bytes: Vec<_> = result.published[0]
        .artifacts
        .iter()
        .map(|artifact| {
            read_generation_member(&publisher, &warm_generation, artifact.locator.as_str())
        })
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
    let catalog = build::read_current_build_catalog(&runtime(
        &ProjectBuildLayout::project_local_for_tests(temp.path()),
    ))
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
    let record = build::read_current_build_record(&runtime(
        &ProjectBuildLayout::project_local_for_tests(temp.path()),
    ))
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
fn catalog_finalization_failure_is_typed_and_counted() {
    let (temp, request) = fixture();
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let events = Arc::new(RestoreCatalogOnFailure {
        catalog_root: layout.catalog_root().to_path_buf(),
        events: Mutex::new(Vec::new()),
    });
    let manager = ManagerCapability::new(FaultingServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("catalog-finalization-failure").unwrap(),
        CancellationToken::default(),
        events.clone(),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    assert!(matches!(
        outcome.result,
        OperationResult::Unavailable {
            kind: squish_protocol::OperationKind::Build
        }
    ));
    assert_eq!(outcome.summary.root_failures, 1);
    let events = events.events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::FinalizationStarted { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::FinalizationFailed { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.payload, EventPayload::FinalizationSucceeded { .. }))
    );
}

#[test]
fn planning_recovery_adopts_a_verified_newer_typed_generation() {
    let (temp, request) = fixture();
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let run = |invocation: &str, sink: Arc<dyn EventSink>| {
        let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
        let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
        let kernel = Kernel::new(&capabilities).unwrap();
        let context = InvocationContext::new(
            InvocationId::new(invocation).unwrap(),
            CancellationToken::default(),
            sink,
        );
        kernel
            .dispatch(&OperationRequest::Build(request.clone()), &context)
            .unwrap()
    };

    run("typed-recovery-g1", Arc::new(IgnoreEvents));
    let g1 = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .unwrap()
        .record
        .targets[0]
        .generation_id;
    fs::write(
        temp.path().join("src/main.xml"),
        r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><message>Version two</message></xs:entry>"#,
    )
    .unwrap();
    let fault = Arc::new(RestoreCatalogOnFailure {
        catalog_root: layout.catalog_root().to_path_buf(),
        events: Mutex::new(Vec::new()),
    });
    let manager = ManagerCapability::new(FaultingServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let context = InvocationContext::new(
        InvocationId::new("typed-recovery-g2-fault").unwrap(),
        CancellationToken::default(),
        fault,
    );
    let failed = Kernel::new(&capabilities)
        .unwrap()
        .dispatch(&OperationRequest::Build(request.clone()), &context)
        .unwrap();
    assert_eq!(failed.summary.root_failures, 1);

    let publisher = FileArtifactPublisher::open(
        layout.artifacts_root(),
        Cas::open(layout.cas_root()).unwrap(),
    )
    .unwrap();
    let target = PublicationTargetId::new("fixture:chat").unwrap();
    let g2 = publisher
        .current_generation(&target)
        .unwrap()
        .unwrap()
        .identity
        .generation;
    assert_ne!(g1, g2);
    assert!(matches!(
        build::read_current_build_catalog(&runtime(&layout)),
        Err(build::BuildCatalogError::Historical { .. })
    ));

    let token = CancellationToken::default();
    let events = Arc::new(RecordAndCancelOnPlanReady {
        token: token.clone(),
        events: Mutex::new(Vec::new()),
    });
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("typed-recovery-adopt").unwrap(),
        token,
        events.clone(),
    );
    kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    assert_eq!(
        build::read_current_build_catalog(&runtime(&layout))
            .unwrap()
            .unwrap()
            .record
            .targets[0]
            .generation_id,
        g2
    );
    assert!(events.events.lock().unwrap().iter().any(|event| matches!(
        &event.payload,
        EventPayload::PlanningStepSucceeded { step, .. }
            if step.as_str() == "recover-build-catalog"
    )));
}

#[test]
fn failed_and_cancelled_attempts_retain_the_previous_target_generation() {
    let (temp, request) = fixture();
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
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
    let good_map = build::read_current_build_catalog(&runtime(&layout))
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
    let failed = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .unwrap();
    assert_eq!(failed.link_map(&target).unwrap(), Some(good_map.clone()));
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
    let cancelled = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.link_map(&target).unwrap(), Some(good_map.clone()));
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
    let record = build::read_current_build_record(&runtime(
        &ProjectBuildLayout::project_local_for_tests(temp.path()),
    ))
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
fn prompt_collision_restores_matching_backend_evidence_for_follower() {
    let (temp, mut request) = fixture();
    fs::write(
        temp.path().join("src/ma.xml"),
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:macro name="m:f"><message>Hello world</message></xs:macro></xs:module>"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("src/a.xml"),
        r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><xs:import src="ma.xml"/><message>Hello world</message></xs:entry>"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("src/b.xml"),
        r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><message>Hello world</message></xs:entry>"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("xmlsquish.toml"),
        r#"manifest-version = 1
[package]
name = "fixture"
version = "1.0.0"

[target.a]
entry = "src/a.xml"
output = "a.prompt"

[target.b]
entry = "src/b.xml"
output = "b.prompt"

[target.c]
entry = "src/b.xml"
output = "c.prompt"
"#,
    )
    .unwrap();
    request.emit = vec![EmitKind::Prompt, EmitKind::DebugInfo];
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
        InvocationId::new("backend-evidence-collision").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let catalog = build::read_current_build_catalog(&runtime(&layout))
        .unwrap()
        .unwrap();
    assert_eq!(
        catalog.record.targets.len(),
        3,
        "collision fixture action facts: {:?}",
        catalog.record.actions
    );
    let publisher = FileArtifactPublisher::open(
        layout.artifacts_root(),
        Cas::open(layout.cas_root()).unwrap(),
    )
    .unwrap();
    let read_kind = |target: &str, predicate: &dyn Fn(&squish_protocol::ArtifactKind) -> bool| {
        let generation = publisher
            .current_generation(&PublicationTargetId::new(target).unwrap())
            .unwrap()
            .unwrap();
        let member = generation
            .artifacts
            .iter()
            .find(|item| predicate(&item.descriptor.kind))
            .unwrap();
        read_generation_member(&publisher, &generation, member.path.as_str())
    };
    let a = read_kind("fixture:a", &|kind| {
        matches!(kind, squish_protocol::ArtifactKind::DebugInfo)
    });
    let b = read_kind("fixture:b", &|kind| {
        matches!(kind, squish_protocol::ArtifactKind::DebugInfo)
    });
    let c = read_kind("fixture:c", &|kind| {
        matches!(kind, squish_protocol::ArtifactKind::DebugInfo)
    });
    assert_ne!(a, b, "different entry identities have distinct provenance");
    assert_eq!(b, c, "single-flight follower C must retain B's evidence");
    let link = |kind: &squish_protocol::ArtifactKind| matches!(kind, squish_protocol::ArtifactKind::Other(name) if name == "static-link-map");
    let a_map = read_kind("fixture:a", &link);
    let b_map = read_kind("fixture:b", &link);
    let c_map = read_kind("fixture:c", &link);
    assert_ne!(a_map, b_map, "different imports have distinct static maps");
    assert_eq!(b_map, c_map, "C must retain B's restored static map");
}

#[test]
fn semantic_example_publishes_fully_traceable_debug_bundle() {
    let temp = TempDir::new_in(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".temp"),
    )
    .unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/semantic");
    fs::create_dir_all(temp.path().join("parts")).unwrap();
    for relative in [
        "xmlsquish.toml",
        "prompt.xml",
        "parts/hello.xml",
        "parts/another.xml",
    ] {
        fs::copy(example.join(relative), temp.path().join(relative)).unwrap();
    }
    let request = BuildRequest {
        project: ProjectPath::new(temp.path().display().to_string()).unwrap(),
        scope: WorkspaceScope::Current,
        targets: Vec::new(),
        profile: ProfileName::new("dev").unwrap(),
        arguments: BTreeMap::new(),
        emit: vec![EmitKind::Prompt, EmitKind::DebugInfo],
        lock: LockMode::Update,
    };
    let manager = ManagerCapability::new(LocalServices, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities).unwrap();
    let context = InvocationContext::new(
        InvocationId::new("semantic-example-debug").unwrap(),
        CancellationToken::default(),
        Arc::new(IgnoreEvents),
    );
    let outcome = kernel
        .dispatch(&OperationRequest::Build(request), &context)
        .unwrap();
    let OperationResult::Build(result) = outcome.result else {
        panic!("semantic example returns a typed build result")
    };
    let layout = ProjectBuildLayout::project_local_for_tests(temp.path());
    let record = build::read_current_build_record(&runtime(&layout))
        .unwrap()
        .unwrap();
    assert_eq!(
        result.published.len(),
        1,
        "semantic example action facts: {:?}",
        record.actions
    );
    let debug = result.published[0]
        .artifacts
        .iter()
        .find(|artifact| matches!(artifact.kind, squish_protocol::ArtifactKind::DebugInfo))
        .unwrap();
    let publisher = FileArtifactPublisher::open(
        layout.artifacts_root(),
        Cas::open(layout.cas_root()).unwrap(),
    )
    .unwrap();
    let generation = publisher
        .current_generation(&PublicationTargetId::new(&result.published[0].target_id).unwrap())
        .unwrap()
        .unwrap();
    let bytes = read_generation_member(&publisher, &generation, debug.locator.as_str());
    let bundle = squish_ir::decode_debug_bundle(&bytes).unwrap();
    assert!(!bundle.artifact_map.entries.is_empty());
    assert!(bundle.artifact_map.entries.iter().all(|entry| {
        test_origin_reaches_archive(
            entry.origin,
            &bundle.expansion_trace,
            &bundle.source_archives,
            &mut BTreeSet::new(),
        )
    }));
}

fn test_origin_reaches_archive(
    id: squish_ir::OriginNodeId,
    trace: &squish_ir::ExpansionTrace,
    archives: &[squish_ir::SourceArchiveReference],
    seen: &mut BTreeSet<u32>,
) -> bool {
    if !seen.insert(id.0) {
        return false;
    }
    use squish_ir::OriginNode;
    match trace.origins.get(id.0 as usize) {
        Some(OriginNode::SourceSpan { origin }) => archives.iter().any(|archive| {
            archive.object == origin.object
                && archive
                    .origins
                    .entries
                    .get(origin.local.0 as usize)
                    .is_some()
        }),
        Some(OriginNode::DecodedSegment { map, segment_index }) => archives.iter().any(|archive| {
            archive.object == map.object
                && archive
                    .origins
                    .decoded_values
                    .get(map.local.0 as usize)
                    .and_then(|value| value.segments.get(*segment_index as usize))
                    .is_some()
        }),
        Some(OriginNode::Import { child, .. } | OriginNode::RegexCapture { input: child, .. }) => {
            test_origin_reaches_archive(*child, trace, archives, seen)
        }
        Some(OriginNode::Concat { ordered_inputs }) => ordered_inputs
            .iter()
            .any(|parent| test_origin_reaches_archive(*parent, trace, archives, seen)),
        Some(OriginNode::BackendTransform { inputs, .. } | OriginNode::Fused { inputs, .. }) => {
            inputs
                .iter()
                .any(|edge| test_origin_reaches_archive(edge.parent, trace, archives, seen))
        }
        Some(OriginNode::Synthetic {
            nearest: Some(parent),
            ..
        }) => test_origin_reaches_archive(*parent, trace, archives, seen),
        _ => false,
    }
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
    let catalog = build::read_current_build_catalog(&runtime(
        &ProjectBuildLayout::project_local_for_tests(temp.path()),
    ))
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
    fn open_build_runtime(
        &self,
        project: &Path,
    ) -> Result<Arc<dyn squish_manager::BuildRuntime>, ServiceError> {
        let layout = ProjectBuildLayout::project_local_for_tests(project);
        TestBuildRuntime::open(&layout)
            .map(|runtime| Arc::new(runtime) as Arc<dyn squish_manager::BuildRuntime>)
            .map_err(|error| ServiceError::new(error.code(), error.message()))
    }

    fn storage_layout(&self, project: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Ok(ProjectBuildLayout::project_local_for_tests(project))
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
