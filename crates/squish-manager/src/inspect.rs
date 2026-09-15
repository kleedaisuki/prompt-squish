//! 持久管理器状态的类型化查询。 / Typed inspection of persistent manager state.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

use sha2::{Digest as _, Sha256};
use squish_build::{
    Action, ActionResult, BuildPlan, Dispatch, KeyRecipe, ResourceClass, Resources,
};
use squish_ir::{
    ArtifactDigest, ContainerKind, SECTION_DOCUMENT, SECTION_LINKED_IMAGE, decode_container,
    decode_debug_bundle, decode_linked_document, decode_linked_image, decode_static_link_map,
    decode_unit_container,
};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_project::{LOCK_FILE_NAME, Lockfile, ResolutionMode};
use squish_protocol::{
    ActionId, ActionKeyId, ActionKind, Artifact, ArtifactKind, CacheInspection, Digest,
    DigestAlgorithm, InspectRequest, InspectResult, InspectView, IrInspection, JobId,
    LinkInspection, OperationKind, OperationResult, PackageName, Phase, PlanInspection, PlanMode,
    PlannedAction, PlanningAttemptId, PlanningStepId, PlanningStepKind, ProjectInspection,
    ProvenanceInspection, SourceInspection, TargetName,
};
use squish_repository::{Discovery, ProjectRepository, ProjectSnapshot};
use squish_source::{FileSourceProvider, SnapshotBuilder, SourceIdentity};

use crate::orchestrator::{
    self, CachedResult, PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
};
use crate::{ArtifactLocator, InspectSubject, ProvenanceNonApplicability, ProvenanceRelation};
use crate::{Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan, Services};

/// 查询动作；视图本身是工作身份的一部分。 / Inspection work whose view is part of its identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectWork {
    view: InspectView,
}

impl PlannedWork for InspectWork {
    fn kind(&self) -> ActionKind {
        inspect_kind(&self.view)
    }

    fn effect(&self) -> Effect {
        Effect::ReadEffect
    }
}

/// 从真实查询工作构造单一路径计划。 / Builds the single-path plan from the actual inspection work.
pub fn prepare(view: InspectView) -> Result<PreparedPlan<InspectWork>, ManagerError> {
    let id = action_id("inspect.read")?;
    let kind = inspect_kind(&view);
    let key = KeyRecipe::new(
        "xmlsquish-inspect-v1",
        Vec::new(),
        BTreeMap::from([("view".into(), view_key(&view))]),
    )
    .map_err(|error| manager_error("XS3400", Phase::Orchestrate, "invalid inspect key", error))?;
    let action = Action {
        id: id.clone(),
        key,
        kind,
        class: ResourceClass::Io,
        resources: Resources::new(0, 1, 0),
        dependencies: Vec::new(),
        outputs: Vec::new(),
    };
    let graph = BuildPlan::new([action]).map_err(|error| {
        manager_error("XS3400", Phase::Orchestrate, "invalid inspect plan", error)
    })?;
    PreparedPlan::new(graph, BTreeMap::from([(id, InspectWork { view })]))
        .map_err(|error| manager_error("XS3400", Phase::Orchestrate, "invalid inspect work", error))
}

/// 使用封闭身份投影实际冻结计划，而非重建或重新散列近似图。 / Projects the actual frozen plan with its sealed identity rather than rebuilding or rehashing an approximation.
pub fn project_plan<W: PlannedWork>(
    identity: &PlanInspection,
    plan: &PreparedPlan<W>,
) -> PlanInspection {
    let actions = plan
        .graph()
        .actions()
        .map(|action| PlannedAction {
            action: action.id.clone(),
            kind: action.kind,
            action_key: action
                .key
                .materialize(&action.kind, &action.outputs, |_| None)
                .and_then(|key| ActionKeyId::new(key.as_str()).ok()),
            dependencies: action.dependencies.clone(),
        })
        .collect();
    PlanInspection {
        job: identity.job.clone(),
        plan: identity.plan.clone(),
        digest: identity.digest.clone(),
        mode: identity.mode,
        actions,
    }
}

/// 通过统一调度器执行查询并返回严格匹配的领域结果。 / Executes inspection through the common scheduler and returns a strictly matching result.
pub fn execute<S: Services>(
    request: &InspectRequest,
    services: &S,
    settings: &InvocationSettings,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = JobId::new(format!("inspect:{}", context.id())).expect("invocation ID is non-empty");
    let attempt = PlanningAttemptId::new("inspect-attempt-1").expect("non-empty constant");
    let mut planning = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(value) => value,
        Err(_) => return unavailable(job),
    };
    let repository = match planning.step(
        planning_step("inspect.locate"),
        PlanningStepKind::Locate,
        || {
            validate_subject(&request.view, settings.inspect_subject.as_ref())?;
            let repository = ProjectRepository::discover(Discovery::Explicit(PathBuf::from(
                request.project.as_str(),
            )))
            .map_err(|error| {
                manager_error(
                    "XS3401",
                    Phase::Discover,
                    "could not discover project",
                    error,
                )
            })?;
            services
                .storage_layout(repository.root())
                .map_err(|error| service_error_at(Phase::Discover, error))?;
            Ok(repository)
        },
    ) {
        Ok(value) => value,
        Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
    };
    let preplanned = match &request.view {
        InspectView::Project | InspectView::Source(_) => {
            let locations = match planning.step(
                planning_step("inspect.fetch"),
                PlanningStepKind::Fetch,
                || snapshot_locations(&repository, services),
            ) {
                Ok(value) => value,
                Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
            };
            let snapshot = match planning.step(
                planning_step("inspect.snapshot"),
                PlanningStepKind::Snapshot,
                || freeze_snapshot(&repository, &locations),
            ) {
                Ok(value) => value,
                Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
            };
            if matches!(request.view, InspectView::Project) {
                match planning.step(
                    planning_step("inspect.scan"),
                    PlanningStepKind::Scan,
                    || project_from_snapshot(&snapshot).map(InspectResult::Project),
                ) {
                    Ok(value) => Some(value),
                    Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
                }
            } else {
                let InspectView::Source(opaque) = &request.view else {
                    unreachable!("match arm restricts the view")
                };
                let result = match planning.step(
                    planning_step("inspect.scan"),
                    PlanningStepKind::Scan,
                    || {
                        let id = SourceIdentity::try_from_protocol(opaque).map_err(|error| {
                            manager_error(
                                "XS3441",
                                Phase::Snapshot,
                                "invalid source identity",
                                error,
                            )
                        })?;
                        if snapshot.lockfile().is_none() {
                            return Err(ManagerError::new(
                                "XS3442",
                                Phase::Snapshot,
                                "source inspection requires an exact lockfile",
                            ));
                        }
                        sealed_source(&snapshot, &locations, &id).map(InspectResult::Source)
                    },
                ) {
                    Ok(value) => value,
                    Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
                };
                Some(result)
            }
        }
        _ => None,
    };
    let plan = match planning.step(
        planning_step("inspect.validate-plan"),
        PlanningStepKind::ValidatePlan,
        || prepare(request.view.clone()),
    ) {
        Ok(value) => value,
        Err(failure) => return planning.unavailable(OperationKind::Inspect, &failure),
    };
    let seed = inspection_seed(repository.root(), request, preplanned.as_ref());
    let sealed = match planning.seal(plan, &seed, PlanMode::Execute) {
        Ok(value) => value,
        Err(failure) => {
            return orchestrator::planning_outcome(job, OperationKind::Inspect, &failure);
        }
    };
    let executor = Inspector {
        services,
        settings,
        root: repository.root().to_path_buf(),
        preplanned,
        result: Mutex::new(None),
    };
    match orchestrator::run(sealed, &executor, context, settings) {
        Ok(report) => OperationOutcome {
            job,
            result: executor
                .result
                .lock()
                .expect("inspect result mutex poisoned")
                .take()
                .map_or(
                    OperationResult::Unavailable {
                        kind: OperationKind::Inspect,
                    },
                    OperationResult::Inspect,
                ),
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        },
        // `run` may already have published `PlanReady`; emitting a synthetic failure plan here
        // would itself violate the lifecycle. The kernel observes the original sink/lifecycle
        // error recorded by `InvocationContext`. / `run` 可能已经发布 `PlanReady`；此处再发布
        // 合成失败计划会破坏生命周期。内核会读取 `InvocationContext` 记录的原始错误。
        Err(_) => unavailable(job),
    }
}

struct Inspector<'a, S> {
    services: &'a S,
    settings: &'a InvocationSettings,
    root: PathBuf,
    preplanned: Option<InspectResult>,
    result: Mutex<Option<InspectResult>>,
}

impl<S: Services> WorkExecutor<InspectWork> for Inspector<'_, S> {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &InspectWork,
        _: &PreparedPlan<InspectWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        work: &InspectWork,
        _: &PreparedPlan<InspectWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("XS3499", "inspection cancelled").into();
        }
        let result = self.preplanned.clone().map_or_else(
            || inspect_at_root(&work.view, self.services, self.settings, &self.root),
            Ok,
        );
        let completed = match result {
            Ok(value) => {
                *self.result.lock().expect("inspect result mutex poisoned") = Some(value);
                ActionResult::success()
            }
            Err(error) => ActionResult::failure(error.code(), error.message()),
        };
        completed.into()
    }

    fn record(&self, _: &Dispatch, _: &InspectWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

/// 计算一个查询视图；该入口便于适配器契约测试且不发布事件。 / Computes one inspection view without publishing events, enabling adapter contract tests.
pub fn inspect_value<S: Services>(
    request: &InspectRequest,
    view: &InspectView,
    services: &S,
    settings: &InvocationSettings,
) -> Result<InspectResult, ManagerError> {
    let repository =
        ProjectRepository::discover(Discovery::Explicit(PathBuf::from(request.project.as_str())))
            .map_err(|error| {
            manager_error(
                "XS3401",
                Phase::Discover,
                "could not discover project",
                error,
            )
        })?;
    validate_subject(view, settings.inspect_subject.as_ref())?;
    inspect_at_repository(view, services, settings, &repository)
}

fn inspect_at_repository<S: Services>(
    view: &InspectView,
    services: &S,
    settings: &InvocationSettings,
    repository: &ProjectRepository,
) -> Result<InspectResult, ManagerError> {
    match view {
        InspectView::Project => project(repository, services).map(InspectResult::Project),
        InspectView::Plan => persisted_plan(repository.root(), services).map(InspectResult::Plan),
        InspectView::Cache => {
            cache(repository.root(), services, settings).map(InspectResult::Cache)
        }
        InspectView::Ir(id) => ir(repository.root(), id, services).map(InspectResult::Ir),
        InspectView::Link(target) => {
            link(repository.root(), target, services).map(InspectResult::Link)
        }
        InspectView::Source(source_id) => {
            source(repository, source_id, services).map(InspectResult::Source)
        }
        InspectView::Provenance(id) => {
            provenance(repository.root(), id, services).map(InspectResult::Provenance)
        }
        InspectView::Artifact(path) => {
            artifact_provenance(repository.root(), path, services).map(InspectResult::Provenance)
        }
    }
}

fn inspect_at_root<S: Services>(
    view: &InspectView,
    services: &S,
    settings: &InvocationSettings,
    root: &Path,
) -> Result<InspectResult, ManagerError> {
    match view {
        InspectView::Plan => persisted_plan(root, services).map(InspectResult::Plan),
        InspectView::Cache => cache(root, services, settings).map(InspectResult::Cache),
        InspectView::Ir(id) => ir(root, id, services).map(InspectResult::Ir),
        InspectView::Link(target) => link(root, target, services).map(InspectResult::Link),
        InspectView::Provenance(id) => {
            provenance(root, id, services).map(InspectResult::Provenance)
        }
        InspectView::Artifact(path) => {
            artifact_provenance(root, path, services).map(InspectResult::Provenance)
        }
        InspectView::Project | InspectView::Source(_) => Err(ManagerError::new(
            "XS3407",
            Phase::Orchestrate,
            "snapshot inspection was not frozen during planning",
        )),
    }
}

fn inspection_seed(
    root: &Path,
    request: &InspectRequest,
    preplanned: Option<&InspectResult>,
) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"xmlsquish\0inspect-planning-snapshot\0v2");
    hasher.update(root.to_string_lossy().as_bytes());
    hasher
        .update(&serde_json::to_vec(&request.view).expect("InspectView serialization cannot fail"));
    if let Some(value) = preplanned {
        hasher.update(&serde_json::to_vec(value).expect("InspectResult serialization cannot fail"));
    }
    Digest::new(
        DigestAlgorithm::Blake3,
        hasher.finalize().as_bytes().to_vec(),
    )
    .expect("BLAKE3 digest has canonical length")
}

fn planning_step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("planning step constants are non-empty")
}

fn project<S: Services>(
    repository: &ProjectRepository,
    services: &S,
) -> Result<ProjectInspection, ManagerError> {
    let (snapshot, _) = complete_snapshot(repository, services)?;
    project_from_snapshot(&snapshot)
}

fn project_from_snapshot(snapshot: &ProjectSnapshot) -> Result<ProjectInspection, ManagerError> {
    let mut packages = Vec::new();
    let mut targets = Vec::new();
    for item in snapshot.manifests() {
        if let Some(package) = &item.manifest.package {
            packages.push(PackageName::new(package.name.clone()).map_err(|error| {
                manager_error("XS3403", Phase::Snapshot, "invalid package identity", error)
            })?);
        }
        for target in item.manifest.targets.keys() {
            targets.push(TargetName::new(target.clone()).map_err(|error| {
                manager_error("XS3403", Phase::Snapshot, "invalid target identity", error)
            })?);
        }
    }
    packages.sort();
    targets.sort();
    Ok(ProjectInspection { packages, targets })
}

fn persisted_plan<S: Services>(root: &Path, services: &S) -> Result<PlanInspection, ManagerError> {
    let plan = services
        .planned_actions(root)
        .map_err(|error| service_error_at(Phase::Orchestrate, error))?
        .ok_or_else(|| {
            ManagerError::new(
                "XS3404",
                Phase::Orchestrate,
                "no persistent prepared plan exists",
            )
        })?;
    if plan.mode == PlanMode::Legacy {
        return Err(ManagerError::new(
            "XS3405",
            Phase::Orchestrate,
            "legacy lossy plan projection is not an inspectable prepared plan",
        ));
    }
    let mut seen = BTreeSet::new();
    for action in &plan.actions {
        if matches!(
            action.kind,
            ActionKind::Resolve
                | ActionKind::Snapshot
                | ActionKind::Scan
                | ActionKind::ResolveCandidate
        ) {
            return Err(ManagerError::new(
                "XS3405",
                Phase::Orchestrate,
                format!(
                    "prepared plan declares planning-only work as action `{}`",
                    action.action
                ),
            ));
        }
        if !seen.insert(action.action.clone()) {
            return Err(ManagerError::new(
                "XS3405",
                Phase::Orchestrate,
                format!("prepared plan repeats action `{}`", action.action),
            ));
        }
        if action
            .dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(ManagerError::new(
                "XS3405",
                Phase::Orchestrate,
                format!("action `{}` has non-canonical dependencies", action.action),
            ));
        }
        if let Some(missing) = action
            .dependencies
            .iter()
            .find(|dependency| !seen.contains(*dependency))
        {
            return Err(ManagerError::new(
                "XS3405",
                Phase::Orchestrate,
                format!(
                    "action `{}` has absent, cyclic, or non-topological dependency `{missing}`",
                    action.action
                ),
            ));
        }
    }
    Ok(plan)
}

fn cache<S: Services>(
    root: &Path,
    services: &S,
    settings: &InvocationSettings,
) -> Result<CacheInspection, ManagerError> {
    let mut actions = services
        .cache_records(root)
        .map_err(|error| service_error_at(Phase::Cache, error))?;
    if let Some(InspectSubject::CacheKey(subject)) = settings.inspect_subject.as_ref() {
        actions.retain(|record| &record.action_key == subject);
        if actions.is_empty() {
            return Err(ManagerError::new(
                "XS3411",
                Phase::Cache,
                format!("cache action `{subject}` was not found"),
            ));
        }
    }
    actions.sort_by(|a, b| a.action_key.cmp(&b.action_key));
    let mut seen = BTreeSet::new();
    for record in &actions {
        if !seen.insert(record.action_key.clone()) {
            return Err(ManagerError::new(
                "XS3412",
                Phase::Cache,
                format!("duplicate cache action `{}`", record.action_key),
            ));
        }
        validate_digest_blob(root, &record.result_digest, None, services)?;
        let mut outputs = BTreeSet::new();
        for artifact in &record.outputs {
            if !outputs.insert(artifact.id.clone()) {
                return Err(ManagerError::new(
                    "XS3412",
                    Phase::Cache,
                    format!(
                        "cache action `{}` repeats output `{}`",
                        record.action_key, artifact.id
                    ),
                ));
            }
            validate_artifact(root, artifact, services)?;
        }
    }
    Ok(CacheInspection { actions })
}

fn ir<S: Services>(
    root: &Path,
    id: &squish_protocol::ArtifactId,
    services: &S,
) -> Result<IrInspection, ManagerError> {
    let artifact = required_artifact(root, id, services)?;
    if artifact.kind != ArtifactKind::BinaryIr {
        return Err(ManagerError::new(
            "XS3421",
            Phase::Analyze,
            "requested artifact is not binary IR",
        ));
    }
    let bytes = validate_artifact(root, &artifact, services)?;
    validate_ir_bytes(&bytes)?;
    Ok(IrInspection { artifact })
}

fn validate_ir_bytes(bytes: &[u8]) -> Result<(), ManagerError> {
    let container = decode_container(bytes)
        .map_err(|error| manager_error("XS3422", Phase::Analyze, "invalid IR container", error))?;
    match container.kind() {
        ContainerKind::Module | ContainerKind::Entry => {
            decode_unit_container(bytes).map_err(|error| {
                manager_error("XS3422", Phase::Analyze, "invalid unit IR", error)
            })?;
        }
        ContainerKind::LinkedImage => {
            decode_section(
                &container,
                SECTION_LINKED_IMAGE,
                decode_linked_image,
                "linked image",
            )?;
        }
        ContainerKind::LinkedDocument => {
            decode_section(
                &container,
                SECTION_DOCUMENT,
                decode_linked_document,
                "linked document",
            )?;
        }
        _ => {
            return Err(ManagerError::new(
                "XS3421",
                Phase::Analyze,
                "artifact container is not an inspectable IR object",
            ));
        }
    }
    Ok(())
}

fn decode_section<T, E: std::fmt::Display>(
    container: &squish_ir::Container,
    tag: u32,
    decode: impl FnOnce(&[u8]) -> Result<T, E>,
    name: &str,
) -> Result<(), ManagerError> {
    let payload = container
        .sections()
        .iter()
        .find(|section| section.tag == tag)
        .ok_or_else(|| {
            ManagerError::new(
                "XS3422",
                Phase::Analyze,
                format!("{name} section is missing"),
            )
        })?;
    decode(&payload.payload).map_err(|error| {
        manager_error("XS3422", Phase::Analyze, &format!("invalid {name}"), error)
    })?;
    Ok(())
}

fn link<S: Services>(
    root: &Path,
    target: &TargetName,
    services: &S,
) -> Result<LinkInspection, ManagerError> {
    let artifact = services
        .link_map(root, target)
        .map_err(|error| service_error_at(Phase::Link, error))?
        .ok_or_else(|| {
            ManagerError::new(
                "XS3431",
                Phase::Link,
                format!("no persistent link map exists for target `{target}`"),
            )
        })?;
    if artifact.kind != ArtifactKind::Metadata
        && !matches!(&artifact.kind, ArtifactKind::Other(name) if name == "static-link-map")
    {
        return Err(ManagerError::new(
            "XS3432",
            Phase::Link,
            "link-map catalog returned a wrong-kind artifact",
        ));
    }
    let bytes = validate_artifact(root, &artifact, services)?;
    decode_static_link_map(&bytes)
        .map_err(|error| manager_error("XS3432", Phase::Link, "invalid static link map", error))?;
    Ok(LinkInspection {
        target: target.clone(),
        link_map: artifact,
    })
}

fn source<S: Services>(
    repository: &ProjectRepository,
    opaque: &squish_protocol::OpaqueSourceId,
    services: &S,
) -> Result<SourceInspection, ManagerError> {
    let id = SourceIdentity::try_from_protocol(opaque).map_err(|error| {
        manager_error("XS3441", Phase::Snapshot, "invalid source identity", error)
    })?;
    let (snapshot, locations) = complete_snapshot(repository, services)?;
    if snapshot.lockfile().is_none() {
        return Err(ManagerError::new(
            "XS3442",
            Phase::Snapshot,
            "source inspection requires an exact lockfile",
        ));
    }
    sealed_source(&snapshot, &locations, &id)
}

fn complete_snapshot<S: Services>(
    repository: &ProjectRepository,
    services: &S,
) -> Result<(ProjectSnapshot, Vec<squish_repository::PackageLocation>), ManagerError> {
    let locations = snapshot_locations(repository, services)?;
    let snapshot = freeze_snapshot(repository, &locations)?;
    Ok((snapshot, locations))
}

fn snapshot_locations<S: Services>(
    repository: &ProjectRepository,
    services: &S,
) -> Result<Vec<squish_repository::PackageLocation>, ManagerError> {
    let lock_path = repository.root().join(LOCK_FILE_NAME);
    Ok(match std::fs::read_to_string(&lock_path) {
        Ok(bytes) => {
            let lock = Lockfile::parse(&bytes).map_err(|error| {
                manager_error("XS3402", Phase::Snapshot, "invalid project lockfile", error)
            })?;
            services
                .materialize_locked(repository.root(), &lock, ResolutionMode::Frozen)
                .map_err(|error| service_error_at(Phase::Resolve, error))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(manager_error(
                "XS3402",
                Phase::Snapshot,
                "could not read project lockfile",
                error,
            ));
        }
    })
}

fn freeze_snapshot(
    repository: &ProjectRepository,
    locations: &[squish_repository::PackageLocation],
) -> Result<ProjectSnapshot, ManagerError> {
    repository
        .snapshot_with_locations(locations)
        .map_err(|error| {
            manager_error(
                "XS3402",
                Phase::Snapshot,
                "could not read project snapshot",
                error,
            )
        })
}

fn sealed_source(
    snapshot: &ProjectSnapshot,
    locations: &[squish_repository::PackageLocation],
    id: &SourceIdentity,
) -> Result<SourceInspection, ManagerError> {
    let sources = snapshot.owned_sources(locations).map_err(|error| {
        manager_error(
            "XS3443",
            Phase::Snapshot,
            "could not enumerate sealed sources",
            error,
        )
    })?;
    let mut builder = SnapshotBuilder::new(FileSourceProvider);
    for source in sources {
        builder.load(source.id, source.locator).map_err(|error| {
            manager_error(
                "XS3443",
                Phase::Snapshot,
                "could not load sealed source",
                error,
            )
        })?;
    }
    let sealed = builder.seal().map_err(|error| {
        manager_error(
            "XS3443",
            Phase::Snapshot,
            "could not seal project sources",
            error,
        )
    })?;
    let blob = sealed.get(id).ok_or_else(|| {
        ManagerError::new(
            "XS3444",
            Phase::Snapshot,
            format!("source `{id}` is not part of the sealed project"),
        )
    })?;
    Ok(SourceInspection {
        source: id.to_protocol(),
        digest: blob.digest().to_protocol(),
        size: blob.bytes().len() as u64,
    })
}

fn provenance<S: Services>(
    root: &Path,
    id: &squish_protocol::ArtifactId,
    services: &S,
) -> Result<ProvenanceInspection, ManagerError> {
    let artifact = required_artifact(root, id, services)?;
    provenance_for_artifact(root, artifact, services)
}

/// 将项目路径选择器解析为权威产物，再以真实身份查询证据。 /
/// Resolves a project-path selector to an authoritative artifact, then queries evidence with
/// the artifact's real identity.
fn artifact_provenance<S: Services>(
    root: &Path,
    path: &squish_protocol::ProjectPath,
    services: &S,
) -> Result<ProvenanceInspection, ManagerError> {
    let locator = ArtifactLocator::new(path.as_str()).map_err(|error| {
        manager_error(
            "XS3424",
            Phase::Cache,
            "invalid artifact path selector",
            error,
        )
    })?;
    let canonical_locator = validate_locator(root, &locator)?;
    let artifact = services
        .artifact_at(root, &canonical_locator)
        .map_err(|error| service_error_at(Phase::Cache, error))?
        .ok_or_else(|| {
            ManagerError::new(
                "XS3420",
                Phase::Cache,
                format!(
                    "artifact path `{}` was not found",
                    locator.as_path().display()
                ),
            )
        })?;
    provenance_for_artifact(root, artifact, services)
}

/// 验证已解析产物并按其内容身份加载来源证据。 /
/// Validates a resolved artifact and loads provenance evidence by its content identity.
fn provenance_for_artifact<S: Services>(
    root: &Path,
    artifact: Artifact,
    services: &S,
) -> Result<ProvenanceInspection, ManagerError> {
    let id = artifact.id.clone();
    let product = validate_artifact(root, &artifact, services)?;
    let relation = services
        .provenance_evidence(root, &id)
        .map_err(|error| service_error_at(Phase::Analyze, error))?;
    let mut evidence = match relation {
        ProvenanceRelation::Evidence(evidence) => evidence,
        ProvenanceRelation::NotApplicable(reason) => {
            validate_non_applicability(reason, &artifact, &product)?;
            return Ok(ProvenanceInspection {
                artifact,
                evidence: Vec::new(),
            });
        }
    };
    if evidence.is_empty() {
        return Err(ManagerError::new(
            "XS3451",
            Phase::Analyze,
            format!("artifact `{id}` has no persistent provenance evidence"),
        ));
    }
    evidence.sort_by(|a, b| a.id.cmp(&b.id));
    let mut seen = BTreeSet::new();
    for item in &evidence {
        if !seen.insert(item.id.clone()) {
            return Err(ManagerError::new(
                "XS3452",
                Phase::Analyze,
                format!("duplicate provenance artifact `{}`", item.id),
            ));
        }
        let bytes = validate_artifact(root, item, services)?;
        validate_evidence(item, &bytes, &artifact, &product)?;
    }
    Ok(ProvenanceInspection { artifact, evidence })
}

fn validate_subject(
    view: &InspectView,
    subject: Option<&InspectSubject>,
) -> Result<(), ManagerError> {
    let valid = matches!(
        (view, subject),
        (_, None) | (InspectView::Cache, Some(InspectSubject::CacheKey(_)))
    );
    if valid {
        Ok(())
    } else {
        Err(ManagerError::new(
            "XS3406",
            Phase::Orchestrate,
            "typed inspection subject does not match the requested view",
        ))
    }
}

fn validate_locator(
    root: &Path,
    locator: &ArtifactLocator,
) -> Result<ArtifactLocator, ManagerError> {
    #[cfg(windows)]
    {
        validate_windows_locator(root, locator)
    }
    #[cfg(not(windows))]
    {
        let path = locator.as_path();
        let relative = if path.is_absolute() {
            path.strip_prefix(root).map_err(|_| {
                ManagerError::new(
                    "XS3424",
                    Phase::Cache,
                    "artifact path is outside the project",
                )
            })?
        } else {
            path
        };
        validate_relative_locator(relative)?;
        relative_locator(relative)
    }
}

#[cfg(windows)]
fn validate_windows_locator(
    root: &Path,
    locator: &ArtifactLocator,
) -> Result<ArtifactLocator, ManagerError> {
    let path = locator.as_path();
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        manager_error(
            "XS3424",
            Phase::Cache,
            "could not canonicalize project root",
            error,
        )
    })?;
    if path.is_absolute() {
        let canonical = std::fs::canonicalize(path).map_err(|error| {
            manager_error(
                "XS3424",
                Phase::Cache,
                "absolute artifact path does not name an existing object",
                error,
            )
        })?;
        let relative = require_contained(&canonical_root, &canonical)?;
        return relative_locator(relative);
    }
    validate_relative_locator(path)?;
    let joined = root.join(path);
    let anchor = canonical_artifact_anchor(&joined)?;
    require_contained(&canonical_root, &anchor)?;
    Ok(locator.clone())
}

fn validate_relative_locator(path: &Path) -> Result<(), ManagerError> {
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => depth += 1,
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if depth > 0 => depth -= 1,
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(ManagerError::new(
                    "XS3424",
                    Phase::Cache,
                    "artifact path escapes the project",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn canonical_artifact_anchor(path: &Path) -> Result<PathBuf, ManagerError> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        match std::fs::canonicalize(current) {
            Ok(value) => return Ok(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                candidate = current.parent();
            }
            Err(error) => {
                return Err(manager_error(
                    "XS3424",
                    Phase::Cache,
                    "could not resolve artifact path",
                    error,
                ));
            }
        }
    }
    Err(ManagerError::new(
        "XS3424",
        Phase::Cache,
        "artifact path has no existing filesystem anchor",
    ))
}

#[cfg(windows)]
fn require_contained<'a>(root: &Path, artifact: &'a Path) -> Result<&'a Path, ManagerError> {
    artifact.strip_prefix(root).map_err(|_| {
        ManagerError::new(
            "XS3424",
            Phase::Cache,
            "artifact path is outside the project",
        )
    })
}

fn relative_locator(path: &Path) -> Result<ArtifactLocator, ManagerError> {
    ArtifactLocator::new(path.to_path_buf()).map_err(|error| {
        manager_error(
            "XS3424",
            Phase::Cache,
            "artifact path does not identify a project-relative object",
            error,
        )
    })
}

fn validate_evidence(
    evidence: &Artifact,
    bytes: &[u8],
    subject: &Artifact,
    subject_bytes: &[u8],
) -> Result<(), ManagerError> {
    match &evidence.kind {
        ArtifactKind::DebugInfo => {
            let bundle = decode_debug_bundle(bytes).map_err(|error| {
                manager_error("XS3453", Phase::Analyze, "invalid psdbg evidence", error)
            })?;
            let expected = ArtifactDigest::of(subject_bytes);
            if bundle.artifact.digest != expected || bundle.artifact.byte_len != subject.size {
                return Err(ManagerError::new(
                    "XS3454",
                    Phase::Analyze,
                    "psdbg evidence describes a different artifact",
                ));
            }
        }
        ArtifactKind::Metadata => validate_build_record(bytes, subject)?,
        _ => {
            return Err(ManagerError::new(
                "XS3453",
                Phase::Analyze,
                "provenance evidence is neither psdbg nor a build record",
            ));
        }
    }
    Ok(())
}

fn validate_build_record(bytes: &[u8], subject: &Artifact) -> Result<(), ManagerError> {
    let record = crate::build::decode_build_record(bytes)
        .map_err(|error| manager_error("XS3453", Phase::Analyze, "invalid build record", error))?;
    let names_subject = record
        .targets
        .iter()
        .flat_map(|target| target.artifacts.iter().map(|item| &item.artifact))
        .any(|artifact| {
            artifact.id == subject.id
                && artifact.digest == subject.digest
                && artifact.size == subject.size
        });
    if !names_subject {
        return Err(ManagerError::new(
            "XS3454",
            Phase::Analyze,
            "build record does not name the requested artifact",
        ));
    }
    Ok(())
}

fn validate_non_applicability(
    reason: ProvenanceNonApplicability,
    artifact: &Artifact,
    bytes: &[u8],
) -> Result<(), ManagerError> {
    match reason {
        ProvenanceNonApplicability::SelfDescribingBuildRecord => {
            if artifact.kind != ArtifactKind::Metadata {
                return Err(non_applicability_mismatch(artifact));
            }
            crate::build::decode_build_record(bytes).map_err(|error| {
                manager_error(
                    "XS3453",
                    Phase::Analyze,
                    "invalid self-describing build record",
                    error,
                )
            })?;
        }
        ProvenanceNonApplicability::SelfDescribingEvidence => match &artifact.kind {
            ArtifactKind::DebugInfo => {
                decode_debug_bundle(bytes).map_err(|error| {
                    manager_error(
                        "XS3453",
                        Phase::Analyze,
                        "invalid self-describing psdbg",
                        error,
                    )
                })?;
            }
            ArtifactKind::BinaryIr => validate_ir_bytes(bytes)?,
            ArtifactKind::Other(name) if name == "static-link-map" => {
                decode_static_link_map(bytes).map_err(|error| {
                    manager_error(
                        "XS3453",
                        Phase::Analyze,
                        "invalid self-describing link map",
                        error,
                    )
                })?;
            }
            _ => return Err(non_applicability_mismatch(artifact)),
        },
        ProvenanceNonApplicability::UnsupportedKind => {
            if !matches!(&artifact.kind, ArtifactKind::Other(name) if name != "static-link-map") {
                return Err(non_applicability_mismatch(artifact));
            }
        }
    }
    Ok(())
}

fn non_applicability_mismatch(artifact: &Artifact) -> ManagerError {
    ManagerError::new(
        "XS3455",
        Phase::Analyze,
        format!(
            "provenance non-applicability reason does not match artifact `{}`",
            artifact.id
        ),
    )
}

fn required_artifact<S: Services>(
    root: &Path,
    id: &squish_protocol::ArtifactId,
    services: &S,
) -> Result<Artifact, ManagerError> {
    services
        .artifact(root, id)
        .map_err(|error| service_error_at(Phase::Cache, error))?
        .ok_or_else(|| {
            ManagerError::new(
                "XS3420",
                Phase::Cache,
                format!("artifact `{id}` was not found"),
            )
        })
}

fn validate_artifact<S: Services>(
    root: &Path,
    artifact: &Artifact,
    services: &S,
) -> Result<Vec<u8>, ManagerError> {
    validate_digest_blob(root, &artifact.digest, Some(artifact.size), services)
}

fn validate_digest_blob<S: Services>(
    root: &Path,
    digest: &Digest,
    size: Option<u64>,
    services: &S,
) -> Result<Vec<u8>, ManagerError> {
    let bytes = services
        .read_blob(root, digest)
        .map_err(|error| service_error_at(Phase::Cache, error))?
        .ok_or_else(|| {
            ManagerError::new(
                "XS3460",
                Phase::Cache,
                format!("blob {} is missing or corrupt", digest.hex()),
            )
        })?;
    if size.is_some_and(|expected| expected != bytes.len() as u64) {
        return Err(ManagerError::new(
            "XS3461",
            Phase::Cache,
            "blob size does not match its artifact record",
        ));
    }
    let actual = match digest.algorithm() {
        DigestAlgorithm::Blake3 => blake3::hash(&bytes).as_bytes().to_vec(),
        DigestAlgorithm::Sha256 => Sha256::digest(&bytes).to_vec(),
        DigestAlgorithm::Other(name) => {
            return Err(ManagerError::new(
                "XS3462",
                Phase::Cache,
                format!("unsupported digest algorithm `{name}`"),
            ));
        }
    };
    if actual != digest.bytes() {
        return Err(ManagerError::new(
            "XS3461",
            Phase::Cache,
            "blob digest does not match its record",
        ));
    }
    Ok(bytes)
}

fn inspect_kind(_view: &InspectView) -> ActionKind {
    ActionKind::Inspect
}

fn view_key(view: &InspectView) -> String {
    match view {
        InspectView::Project => "project".into(),
        InspectView::Plan => "plan".into(),
        InspectView::Cache => "cache".into(),
        InspectView::Ir(id) => format!("ir:{id}"),
        InspectView::Link(id) => format!("link:{id}"),
        InspectView::Source(id) => format!("source:{id}"),
        InspectView::Provenance(id) => format!("provenance:{id}"),
        InspectView::Artifact(path) => format!("artifact:{}", path.as_str()),
    }
}

fn action_id(value: &str) -> Result<ActionId, ManagerError> {
    ActionId::new(value)
        .map_err(|error| manager_error("XS3400", Phase::Orchestrate, "invalid action ID", error))
}

fn service_error_at(phase: Phase, error: crate::ServiceError) -> ManagerError {
    ManagerError::new(error.code(), phase, error.message())
}

fn manager_error(
    code: &str,
    phase: Phase,
    context: &str,
    error: impl std::fmt::Display,
) -> ManagerError {
    ManagerError::new(code, phase, format!("{context}: {error}"))
}

fn unavailable(job: JobId) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::Inspect,
        },
        totals: Default::default(),
        root_failures: 0,
        cancelled: false,
    }
}
