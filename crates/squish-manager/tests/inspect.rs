use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use sha2::{Digest as _, Sha256};
use squish_manager::build::{
    BuildActionFact, BuildRecordV2, BuildResultSource, BuildTerminalState, CatalogArtifact,
    RecordedGeneration, encode_build_record,
};
use squish_manager::{
    ArtifactLocator, InspectSubject, InvocationSettings, ManagerCapability,
    ProvenanceNonApplicability, ProvenanceRelation, ResolveRequest, ResolvedDependencies,
    ServiceError, Services, StorageLayout,
    inspect::{inspect_value, prepare, project_plan},
};
use squish_project::{Lockfile, ResolutionMode};
use squish_protocol::{
    ActionKeyId, Artifact, ArtifactId, ArtifactKind, CachedAction, Digest, DigestAlgorithm, Event,
    EventPayload, InspectRequest, InspectResult, InspectView, InvocationId, JobId,
    OperationRequest, PlanDigest, PlanId, PlanInspection, PlanMode, ProjectPath, TargetName,
};
use squish_repository::PackageLocation;
use tempfile::TempDir;

#[derive(Default)]
struct FakeServices {
    allowed_root: Mutex<Option<PathBuf>>,
    blobs: Mutex<BTreeMap<String, Vec<u8>>>,
    cache: Mutex<Vec<CachedAction>>,
    artifacts: Mutex<BTreeMap<ArtifactId, Artifact>>,
    paths: Mutex<BTreeMap<ArtifactLocator, Artifact>>,
    links: Mutex<BTreeMap<TargetName, Artifact>>,
    evidence: Mutex<BTreeMap<ArtifactId, Vec<Artifact>>>,
    relations: Mutex<BTreeMap<ArtifactId, ProvenanceRelation>>,
    plan: Mutex<Option<PlanInspection>>,
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

impl squish_kernel::EventSink for Events {
    fn emit(&self, event: Event) -> Result<(), squish_kernel::SinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl Services for FakeServices {
    fn storage_layout(&self, project: &Path) -> Result<StorageLayout, ServiceError> {
        if self
            .allowed_root
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|allowed| allowed != project)
        {
            return Err(ServiceError::new(
                "wrong_project_layout",
                "storage layout does not authorize this project",
            ));
        }
        Ok(StorageLayout::project_local_for_tests(project))
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
        Err(ServiceError::new("unused", "not configured"))
    }

    fn cache_records(&self, _: &Path) -> Result<Vec<CachedAction>, ServiceError> {
        Ok(self.cache.lock().unwrap().clone())
    }

    fn read_blob(&self, _: &Path, digest: &Digest) -> Result<Option<Vec<u8>>, ServiceError> {
        Ok(self.blobs.lock().unwrap().get(&digest.hex()).cloned())
    }

    fn artifact(&self, _: &Path, id: &ArtifactId) -> Result<Option<Artifact>, ServiceError> {
        Ok(self.artifacts.lock().unwrap().get(id).cloned())
    }

    fn artifact_at(
        &self,
        _: &Path,
        locator: &ArtifactLocator,
    ) -> Result<Option<Artifact>, ServiceError> {
        Ok(self.paths.lock().unwrap().get(locator).cloned())
    }

    fn link_map(&self, _: &Path, target: &TargetName) -> Result<Option<Artifact>, ServiceError> {
        Ok(self.links.lock().unwrap().get(target).cloned())
    }

    fn provenance_evidence(
        &self,
        _: &Path,
        id: &ArtifactId,
    ) -> Result<ProvenanceRelation, ServiceError> {
        if let Some(relation) = self.relations.lock().unwrap().get(id).cloned() {
            return Ok(relation);
        }
        Ok(ProvenanceRelation::Evidence(
            self.evidence
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .unwrap_or_default(),
        ))
    }

    fn planned_actions(&self, _: &Path) -> Result<Option<PlanInspection>, ServiceError> {
        Ok(self.plan.lock().unwrap().clone())
    }
}

fn project() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("xmlsquish.toml"),
        r#"
manifest-version = 1

[package]
name = "demo"
version = "1.0.0"

[target.main]
entry = "src/main.xml"
"#,
    )
    .unwrap();
    root
}

fn request(root: &TempDir, view: InspectView) -> InspectRequest {
    InspectRequest {
        project: ProjectPath::new(root.path().to_string_lossy()).unwrap(),
        view,
    }
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(DigestAlgorithm::Sha256, Sha256::digest(bytes).to_vec()).unwrap()
}

fn artifact(id: &str, kind: ArtifactKind, bytes: &[u8]) -> Artifact {
    Artifact {
        id: ArtifactId::new(id).unwrap(),
        kind,
        uri: format!("cas://{id}"),
        size: bytes.len() as u64,
        digest: digest(bytes),
    }
}

fn insert_blob(services: &FakeServices, bytes: &[u8]) -> Digest {
    let value = digest(bytes);
    services
        .blobs
        .lock()
        .unwrap()
        .insert(value.hex(), bytes.to_vec());
    value
}

fn projection(view: InspectView) -> PlanInspection {
    let identity = PlanInspection {
        job: JobId::new("inspect-job").unwrap(),
        plan: PlanId::new("inspect-plan").unwrap(),
        digest: PlanDigest::new(digest(b"sealed inspect plan")),
        mode: PlanMode::Execute,
        actions: Vec::new(),
    };
    project_plan(&identity, &prepare(view).unwrap())
}

#[test]
fn project_and_plan_use_frozen_typed_data() {
    let root = project();
    let services = FakeServices::default();
    let project_request = request(&root, InspectView::Project);
    let result = inspect_value(
        &project_request,
        &project_request.view,
        &services,
        &InvocationSettings::default(),
    )
    .unwrap();
    let InspectResult::Project(value) = result else {
        panic!("wrong view")
    };
    assert_eq!(value.packages[0].as_str(), "demo");
    assert_eq!(value.targets[0].as_str(), "main");

    let plan_request = request(&root, InspectView::Plan);
    let exact = projection(plan_request.view.clone());
    *services.plan.lock().unwrap() = Some(exact.clone());
    assert_eq!(
        inspect_value(
            &plan_request,
            &plan_request.view,
            &services,
            &InvocationSettings::default()
        )
        .unwrap(),
        InspectResult::Plan(exact)
    );
}

#[test]
fn capability_uses_v2_planning_and_one_inspect_execution_action() {
    let root = project();
    let events = Arc::new(Events::default());
    let manager = ManagerCapability::new(FakeServices::default(), InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = squish_kernel::Kernel::new(&capabilities).unwrap();
    let context = squish_kernel::InvocationContext::new(
        InvocationId::new("inspect-lifecycle").unwrap(),
        squish_kernel::CancellationToken::default(),
        events.clone(),
    );
    let outcome = kernel
        .dispatch(
            &OperationRequest::Inspect(request(&root, InspectView::Project)),
            &context,
        )
        .unwrap();
    assert!(matches!(
        outcome.result,
        squish_protocol::OperationResult::Inspect(InspectResult::Project(_))
    ));
    let events = events.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event.payload,
        EventPayload::PlanningStepStarted {
            kind: squish_protocol::PlanningStepKind::Locate,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event.payload,
        EventPayload::PlanningStepStarted {
            kind: squish_protocol::PlanningStepKind::Snapshot,
            ..
        }
    )));
    let declared: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ActionDeclared { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(declared, [squish_protocol::ActionKind::Inspect]);
    assert!(events.iter().any(|event| matches!(
        event.payload,
        EventPayload::PlanClosed {
            reason: squish_protocol::PlanCloseReason::Executed,
            ..
        }
    )));
}

#[test]
fn capability_rejects_storage_layout_for_another_project_during_locate() {
    let root = project();
    let services = FakeServices::default();
    *services.allowed_root.lock().unwrap() = Some(root.path().join("another-project"));
    let manager = ManagerCapability::new(services, InvocationSettings::default());
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = squish_kernel::Kernel::new(&capabilities).unwrap();
    let events = Arc::new(Events::default());
    let context = squish_kernel::InvocationContext::new(
        InvocationId::new("inspect-wrong-layout").unwrap(),
        squish_kernel::CancellationToken::default(),
        events.clone(),
    );
    let outcome = kernel
        .dispatch(
            &OperationRequest::Inspect(request(&root, InspectView::Project)),
            &context,
        )
        .unwrap();
    assert!(matches!(
        outcome.result,
        squish_protocol::OperationResult::Unavailable {
            kind: squish_protocol::OperationKind::Inspect
        }
    ));
    assert_eq!(outcome.summary.root_failures, 1);
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(
        &event.payload,
        EventPayload::PlanningFailed { diagnostic, .. }
            if diagnostic.code == "wrong_project_layout"
    )));
}

#[test]
fn cache_is_sorted_filtered_and_every_blob_is_verified() {
    let root = project();
    let services = FakeServices::default();
    let output_bytes = b"output";
    insert_blob(&services, output_bytes);
    let output = artifact("out", ArtifactKind::Prompt, output_bytes);
    for key in ["b", "a"] {
        let record = format!("record-{key}").into_bytes();
        let result_digest = insert_blob(&services, &record);
        services.cache.lock().unwrap().push(CachedAction {
            action_key: ActionKeyId::new(key).unwrap(),
            result_digest,
            outputs: vec![output.clone()],
        });
    }
    let req = request(&root, InspectView::Cache);
    let result = inspect_value(&req, &req.view, &services, &InvocationSettings::default()).unwrap();
    let InspectResult::Cache(value) = result else {
        panic!("wrong view")
    };
    assert_eq!(
        value
            .actions
            .iter()
            .map(|a| a.action_key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );

    let selected = InvocationSettings {
        inspect_subject: Some(InspectSubject::CacheKey(ActionKeyId::new("b").unwrap())),
        ..InvocationSettings::default()
    };
    let InspectResult::Cache(selected_result) =
        inspect_value(&req, &req.view, &services, &selected).unwrap()
    else {
        panic!("wrong view")
    };
    assert_eq!(selected_result.actions.len(), 1);
    assert_eq!(selected_result.actions[0].action_key.as_str(), "b");

    let wrong_subject = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(
            ArtifactLocator::new("target/main.prompt").unwrap(),
        )),
        ..InvocationSettings::default()
    };
    assert!(inspect_value(&req, &req.view, &services, &wrong_subject).is_err());

    services.blobs.lock().unwrap().remove(&output.digest.hex());
    assert!(inspect_value(&req, &req.view, &services, &InvocationSettings::default()).is_err());
}

#[test]
fn link_decodes_the_persistent_static_map() {
    let root = project();
    let services = FakeServices::default();
    let bytes = squish_ir::encode_static_link_map(&squish_ir::StaticLinkMap::default());
    insert_blob(&services, &bytes);
    let item = artifact("main.link", ArtifactKind::Metadata, &bytes);
    let target = TargetName::new("main").unwrap();
    services
        .links
        .lock()
        .unwrap()
        .insert(target.clone(), item.clone());
    let req = request(&root, InspectView::Link(target.clone()));
    assert_eq!(
        inspect_value(&req, &req.view, &services, &InvocationSettings::default()).unwrap(),
        InspectResult::Link(squish_protocol::LinkInspection {
            target,
            link_map: item,
        })
    );
}

#[test]
fn ir_and_source_reject_wrong_kind_and_noncanonical_identity() {
    let root = project();
    let services = FakeServices::default();
    let bytes = b"not ir";
    insert_blob(&services, bytes);
    let item = artifact("prompt", ArtifactKind::Prompt, bytes);
    services
        .artifacts
        .lock()
        .unwrap()
        .insert(item.id.clone(), item.clone());
    let req = request(&root, InspectView::Ir(item.id));
    assert!(inspect_value(&req, &req.view, &services, &InvocationSettings::default()).is_err());

    let source = squish_protocol::OpaqueSourceId::new("src/main.xml").unwrap();
    let req = request(&root, InspectView::Source(source));
    assert!(inspect_value(&req, &req.view, &services, &InvocationSettings::default()).is_err());
}

#[test]
fn source_reads_digest_and_size_from_the_sealed_project_snapshot() {
    let root = project();
    fs::create_dir(root.path().join("src")).unwrap();
    let source_bytes = b"<prompt/>";
    fs::write(root.path().join("src/main.xml"), source_bytes).unwrap();
    let repository = squish_repository::ProjectRepository::discover(
        squish_repository::Discovery::Explicit(root.path().to_path_buf()),
    )
    .unwrap();
    let before = repository.snapshot().unwrap();
    let manifest = &before.manifests()[0];
    fs::write(
        root.path().join("xmlsquish.lock"),
        format!(
            r#"
lock-version = 1
resolver-version = "test-v1"
manifest-digest = "{}"

[[package]]
id = "demo"
name = "demo"
version = "1.0.0"
manifest-digest = "{}"

[package.source]
kind = "workspace"
member = "."
mutable = true
"#,
            before.manifest_digest(),
            manifest.digest
        ),
    )
    .unwrap();

    let services = FakeServices::default();
    let id = squish_protocol::OpaqueSourceId::new("xmlsquish://demo/src/main.xml").unwrap();
    let req = request(&root, InspectView::Source(id.clone()));
    let result = inspect_value(&req, &req.view, &services, &InvocationSettings::default()).unwrap();
    assert_eq!(
        result,
        InspectResult::Source(squish_protocol::SourceInspection {
            source: id,
            digest: squish_source::SourceDigest::of(source_bytes).to_protocol(),
            size: source_bytes.len() as u64,
        })
    );
}

#[test]
fn provenance_parses_build_record_and_rejects_empty_evidence() {
    let root = project();
    let services = FakeServices::default();
    let product_bytes = b"hello";
    insert_blob(&services, product_bytes);
    let mut product = artifact("demo:main:prompt", ArtifactKind::Prompt, product_bytes);
    product.uri = "generations/generation-1/prompt".into();
    services
        .artifacts
        .lock()
        .unwrap()
        .insert(product.id.clone(), product.clone());
    let plan = projection(InspectView::Project);
    let planned = plan.actions[0].clone();
    let link_bytes = squish_ir::encode_static_link_map(&squish_ir::StaticLinkMap::default());
    let mut link_map = artifact(
        "demo:main:zz-static-link-map",
        ArtifactKind::Other("static-link-map".into()),
        &link_bytes,
    );
    link_map.uri = "generations/generation-1/static-link-map".into();
    let record = BuildRecordV2 {
        schema: 2,
        job: plan.job.clone(),
        plan,
        actions: vec![BuildActionFact {
            action: planned.action,
            kind: planned.kind,
            dependencies: planned.dependencies,
            state: BuildTerminalState::Succeeded,
            key: planned.action_key,
            outputs: Vec::new(),
            source: Some(BuildResultSource::Worker),
        }],
        targets: vec![RecordedGeneration {
            target_id: "demo:main".into(),
            generation_id: "generation-1".into(),
            manifest: "generations/generation-1/manifest.json".into(),
            artifacts: vec![
                CatalogArtifact {
                    artifact: product.clone(),
                    destination: "target/main.prompt".into(),
                },
                CatalogArtifact {
                    artifact: link_map,
                    destination: "target/main.xsmap".into(),
                },
            ],
        }],
        catalog_target: squish_manager::build::BUILD_CATALOG_TARGET.into(),
    };
    let record_bytes = encode_build_record(&record).unwrap();
    insert_blob(&services, &record_bytes);
    let evidence = artifact("build-record", ArtifactKind::Metadata, &record_bytes);
    services
        .evidence
        .lock()
        .unwrap()
        .insert(product.id.clone(), vec![evidence.clone()]);
    let req = request(&root, InspectView::Provenance(product.id.clone()));
    let result = inspect_value(&req, &req.view, &services, &InvocationSettings::default()).unwrap();
    assert_eq!(
        result,
        InspectResult::Provenance(squish_protocol::ProvenanceInspection {
            artifact: product.clone(),
            evidence: vec![evidence.clone()],
        })
    );

    let locator = ArtifactLocator::new("target/main.prompt").unwrap();
    services
        .paths
        .lock()
        .unwrap()
        .insert(locator.clone(), product.clone());
    let path_settings = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(locator)),
        ..InvocationSettings::default()
    };
    assert!(inspect_value(&req, &req.view, &services, &path_settings).is_ok());

    let escaping = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(
            ArtifactLocator::new("../outside.prompt").unwrap(),
        )),
        ..InvocationSettings::default()
    };
    assert!(inspect_value(&req, &req.view, &services, &escaping).is_err());

    services
        .artifacts
        .lock()
        .unwrap()
        .insert(evidence.id.clone(), evidence.clone());
    services.relations.lock().unwrap().insert(
        evidence.id.clone(),
        ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::SelfDescribingBuildRecord),
    );
    let self_request = request(&root, InspectView::Provenance(evidence.id));
    let InspectResult::Provenance(self_result) = inspect_value(
        &self_request,
        &self_request.view,
        &services,
        &InvocationSettings::default(),
    )
    .unwrap() else {
        panic!("wrong view")
    };
    assert!(self_result.evidence.is_empty());

    services.evidence.lock().unwrap().clear();
    assert!(inspect_value(&req, &req.view, &services, &InvocationSettings::default()).is_err());
}

#[test]
fn provenance_accepts_only_kind_correct_typed_non_applicability() {
    let root = project();
    let services = FakeServices::default();
    let map_bytes = squish_ir::encode_static_link_map(&squish_ir::StaticLinkMap::default());
    insert_blob(&services, &map_bytes);
    let map = artifact(
        "main:static-link-map",
        ArtifactKind::Other("static-link-map".into()),
        &map_bytes,
    );
    services
        .artifacts
        .lock()
        .unwrap()
        .insert(map.id.clone(), map.clone());
    services.relations.lock().unwrap().insert(
        map.id.clone(),
        ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::SelfDescribingEvidence),
    );
    let map_request = request(&root, InspectView::Provenance(map.id.clone()));
    assert_eq!(
        inspect_value(
            &map_request,
            &map_request.view,
            &services,
            &InvocationSettings::default()
        )
        .unwrap(),
        InspectResult::Provenance(squish_protocol::ProvenanceInspection {
            artifact: map,
            evidence: Vec::new(),
        })
    );

    let opaque_bytes = b"opaque extension";
    insert_blob(&services, opaque_bytes);
    let opaque = artifact(
        "opaque",
        ArtifactKind::Other("vendor-object".into()),
        opaque_bytes,
    );
    services
        .artifacts
        .lock()
        .unwrap()
        .insert(opaque.id.clone(), opaque.clone());
    services.relations.lock().unwrap().insert(
        opaque.id.clone(),
        ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::UnsupportedKind),
    );
    let opaque_request = request(&root, InspectView::Provenance(opaque.id));
    assert!(
        inspect_value(
            &opaque_request,
            &opaque_request.view,
            &services,
            &InvocationSettings::default()
        )
        .is_ok()
    );

    let prompt_bytes = b"prompt";
    insert_blob(&services, prompt_bytes);
    let prompt = artifact("wrong-reason", ArtifactKind::Prompt, prompt_bytes);
    services
        .artifacts
        .lock()
        .unwrap()
        .insert(prompt.id.clone(), prompt.clone());
    services.relations.lock().unwrap().insert(
        prompt.id.clone(),
        ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::UnsupportedKind),
    );
    let prompt_request = request(&root, InspectView::Provenance(prompt.id));
    assert!(
        inspect_value(
            &prompt_request,
            &prompt_request.view,
            &services,
            &InvocationSettings::default()
        )
        .is_err()
    );

    let outside = tempfile::tempdir().unwrap();
    let outside_path = outside.path().join("artifact.bin");
    fs::write(&outside_path, b"outside").unwrap();
    let outside_settings = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(
            ArtifactLocator::new(outside_path).unwrap(),
        )),
        ..InvocationSettings::default()
    };
    let error = inspect_value(
        &prompt_request,
        &prompt_request.view,
        &services,
        &outside_settings,
    )
    .unwrap_err();
    assert_eq!(error.code(), "XS3424");
}

#[cfg(windows)]
#[test]
fn ordinary_absolute_artifact_matches_verbatim_project_root_but_missing_absolute_fails() {
    let root = project();
    let services = FakeServices::default();
    let bytes = b"opaque artifact";
    let ordinary = root.path().join("artifact.bin");
    fs::write(&ordinary, bytes).unwrap();
    assert!(
        !ordinary.to_string_lossy().starts_with(r"\\?\"),
        "fixture must exercise an ordinary Win32 path"
    );
    assert!(
        root.path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .starts_with(r"\\?\"),
        "canonical project root must use the verbatim namespace"
    );

    insert_blob(&services, bytes);
    let artifact = artifact(
        "opaque-absolute",
        ArtifactKind::Other("vendor-object".into()),
        bytes,
    );
    let catalog_locator = ArtifactLocator::new("artifact.bin").unwrap();
    services
        .paths
        .lock()
        .unwrap()
        .insert(catalog_locator, artifact.clone());
    services.relations.lock().unwrap().insert(
        artifact.id.clone(),
        ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::UnsupportedKind),
    );
    let request = request(&root, InspectView::Provenance(artifact.id.clone()));
    let settings = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(
            ArtifactLocator::new(ordinary).unwrap(),
        )),
        ..InvocationSettings::default()
    };
    assert!(inspect_value(&request, &request.view, &services, &settings).is_ok());

    let missing = ArtifactLocator::new(root.path().join("missing.bin")).unwrap();
    let missing_settings = InvocationSettings {
        inspect_subject: Some(InspectSubject::ArtifactPath(missing)),
        ..InvocationSettings::default()
    };
    let error = inspect_value(&request, &request.view, &services, &missing_settings).unwrap_err();
    assert_eq!(error.code(), "XS3424");
}
