//! 依赖编辑的完整候选事务。 / Complete candidate transactions for dependency edits.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use squish_build::{
    Action, ActionResult, BuildPlan, Dispatch, InputRef, KeyRecipe, ResourceClass, Resources,
};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_project::{
    CandidateManifest, DependencyDetail, DependencySpec, GitReference as ProjectGitReference,
    LOCK_FILE_NAME, Lockfile, Manifest, MutationFile, MutationKind, MutationPlan, MutationPlanner,
    ResolutionMode, TransactionId,
};
use squish_protocol::{
    ActionId, ActionKind, ActionTotals, AddRequest, AddResult, DependencyKind, DependencySource,
    Digest, DigestAlgorithm, GitReference, JobId, LockMode, OpaqueSourceId, OperationKind,
    OperationResult, Phase, PlanMode, PlanningAttemptId, PlanningStepId, PlanningStepKind,
    ProjectStateDigests, RemoveRequest, RemoveResult, SupersedeReason,
};
use squish_repository::{ManifestSnapshot, ProjectRepository, ProjectSnapshot, RepositoryError};
use squish_source::{LogicalPath, PackageId, SourceId};

use crate::{
    DurabilityPorts, Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan,
    ResolveRequest, Services,
    orchestrator::{
        self, CachedResult, PlanningFailure, PlanningRecorder, ResolvedInputs, WorkDisposition,
        WorkExecutor,
    },
};

const RETRIES: u8 = 4;

/// 候选已封闭后的唯一可执行工作。 / The sole executable work after sealing a candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationWork {
    /// 可恢复地提交清单与锁。 / Recoverably commit manifest and lock.
    CommitTransaction {
        /// 只验证而不写权威文件。 / Validate without writing authoritative files.
        dry_run: bool,
    },
}

impl PlannedWork for MutationWork {
    fn kind(&self) -> ActionKind {
        match self {
            Self::CommitTransaction { .. } => ActionKind::CommitTransaction,
        }
    }

    fn effect(&self) -> Effect {
        match self {
            Self::CommitTransaction { dry_run: true } => Effect::Coordination,
            Self::CommitTransaction { dry_run: false } => Effect::WriteEffect,
        }
    }
}

/// 构造只含已封闭提交的执行图。 / Builds an execution graph containing only the sealed commit.
pub fn plan(dry_run: bool, identity: &Digest) -> Result<PreparedPlan<MutationWork>, ManagerError> {
    let id = action_id("mutation.commit")?;
    let actions = [action(
        id.clone(),
        ActionKind::CommitTransaction,
        Vec::new(),
        vec![InputRef::Blob(identity.clone())],
    )?];
    let work = BTreeMap::from([(id, MutationWork::CommitTransaction { dry_run })]);
    PreparedPlan::new(
        BuildPlan::new(actions)
            .map_err(|e| err("XS3200", Phase::Manage, "invalid mutation graph", e))?,
        work,
    )
    .map_err(|e| err("XS3200", Phase::Manage, "invalid mutation work", e))
}

/// 执行 add 候选事务；worker 只发布结构化事件。 / Executes an add candidate transaction; the worker only emits structured events.
pub(crate) fn add_with_durability(
    request: &AddRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let intent = match MutationIntent::add(request) {
        Ok(intent) => intent,
        Err(error) => {
            return orchestrator::fail(
                mutation_job(context),
                OperationKind::Add,
                ActionKind::Snapshot,
                error,
                context,
                settings,
            );
        }
    };
    execute(intent, services, settings, durability, context)
}

/// 执行 remove 候选事务；worker 不直接打印。 / Executes a remove candidate transaction; the worker never prints.
pub(crate) fn remove_with_durability(
    request: &RemoveRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let intent = match MutationIntent::remove(request) {
        Ok(intent) => intent,
        Err(error) => {
            return orchestrator::fail(
                mutation_job(context),
                OperationKind::Remove,
                ActionKind::Snapshot,
                error,
                context,
                settings,
            );
        }
    };
    execute(intent, services, settings, durability, context)
}

#[derive(Clone)]
struct MutationIntent {
    project: squish_protocol::ProjectPath,
    package: Option<String>,
    dependency: squish_protocol::DependencyName,
    mode: LockMode,
    dry_run: bool,
    kind: MutationKind,
    edit: IntentEdit,
}

#[derive(Clone)]
enum IntentEdit {
    Add(DependencySpec),
    Remove,
}

impl MutationIntent {
    fn add(request: &AddRequest) -> Result<Self, ManagerError> {
        require_normal(request.kind)?;
        Ok(Self {
            project: request.project.clone(),
            package: request.package.as_ref().map(|p| p.as_str().into()),
            dependency: request.dependency.clone(),
            mode: request.lock,
            dry_run: request.dry_run,
            kind: MutationKind::AddDependency {
                alias: request.dependency.as_str().into(),
            },
            edit: IntentEdit::Add(dependency_spec(request)?),
        })
    }

    fn remove(request: &RemoveRequest) -> Result<Self, ManagerError> {
        require_normal(request.kind)?;
        Ok(Self {
            project: request.project.clone(),
            package: request.package.as_ref().map(|p| p.as_str().into()),
            dependency: request.dependency.clone(),
            mode: request.lock,
            dry_run: request.dry_run,
            kind: MutationKind::RemoveDependency {
                alias: request.dependency.as_str().into(),
            },
            edit: IntentEdit::Remove,
        })
    }

    fn operation_kind(&self) -> OperationKind {
        match self.edit {
            IntentEdit::Add(_) => OperationKind::Add,
            IntentEdit::Remove => OperationKind::Remove,
        }
    }
}

struct CandidatePrepared {
    manifests: BTreeMap<String, Manifest>,
    manifest: Vec<u8>,
    digest: Digest,
    affected: Vec<OpaqueSourceId>,
    before: ProjectStateDigests,
    selected_path: PathBuf,
    selected_digest: String,
}

#[derive(Default)]
struct MutationState {
    repository: Option<ProjectRepository>,
    locations: Vec<squish_repository::PackageLocation>,
    snapshot: Option<ProjectSnapshot>,
    candidate: Option<CandidatePrepared>,
    transaction: Option<MutationPlan>,
    result: Option<OperationResult>,
}

fn load_stage(
    state: &mut MutationState,
    intent: &MutationIntent,
    services: &dyn Services,
    durability: &DurabilityPorts,
) -> Result<(), ManagerError> {
    let repository = ProjectRepository::discover_with_faults(
        squish_repository::Discovery::Explicit(PathBuf::from(intent.project.as_str())),
        durability.repository(),
    )
    .map_err(|e| err("XS3201", Phase::Discover, "could not load project", e))?;
    services
        .storage_layout(repository.root())
        .map_err(|error| {
            err(
                error.code(),
                Phase::Discover,
                "project storage layout is unavailable",
                &error,
            )
        })?;
    state.repository = Some(repository);
    Ok(())
}

fn recover_stage(state: &MutationState) -> Result<(), ManagerError> {
    state
        .repository
        .as_ref()
        .expect("Locate precedes Recover")
        .recover()
        .map_err(|e| err("XS3202", Phase::Snapshot, "could not recover project", e))
}

fn materialize_stage(
    state: &mut MutationState,
    intent: &MutationIntent,
    services: &dyn Services,
) -> Result<(), ManagerError> {
    let repository = state.repository.as_ref().expect("Recover precedes Fetch");
    state.locations = bootstrap_locations(repository, intent.mode, services)?;
    Ok(())
}

fn snapshot_stage(state: &mut MutationState) -> Result<(), ManagerError> {
    let repository = state.repository.as_ref().expect("Fetch precedes Snapshot");
    let snapshot = repository
        .snapshot_with_locations(&state.locations)
        .map_err(|e| err("XS3203", Phase::Snapshot, "could not snapshot project", e))?;
    state.snapshot = Some(snapshot);
    state.candidate = None;
    state.transaction = None;
    state.result = None;
    Ok(())
}

fn candidate_stage(state: &mut MutationState, intent: &MutationIntent) -> Result<(), ManagerError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .expect("Snapshot precedes PrepareCandidate");
    let selected = select_manifest(snapshot, intent.package.as_deref())?;
    let source = std::str::from_utf8(&selected.bytes)
        .map_err(|e| err("XS3204", Phase::Manage, "manifest is not UTF-8", e))?;
    let editable = CandidateManifest::parse(source)
        .map_err(|e| err("XS3205", Phase::Manage, "dependency edit is invalid", e))?;
    let edit = match &intent.edit {
        IntentEdit::Add(spec) => editable.add(intent.dependency.as_str(), spec.clone(), true),
        IntentEdit::Remove => editable.remove(intent.dependency.as_str()),
    }
    .map_err(|e| err("XS3205", Phase::Manage, "dependency edit is invalid", e))?;
    let affected = if matches!(intent.edit, IntentEdit::Remove) {
        affected_sources(selected, snapshot.root(), intent.dependency.as_str())?
    } else {
        Vec::new()
    };
    let set = candidate_set(snapshot, &selected.path, edit.after.as_bytes())?;
    state.candidate = Some(CandidatePrepared {
        manifests: set.manifests,
        manifest: set.manifest,
        digest: set.digest,
        affected,
        before: state_digests(snapshot)?,
        selected_path: selected.path.clone(),
        selected_digest: selected.digest.clone(),
    });
    Ok(())
}

fn resolve_stage(
    state: &mut MutationState,
    intent: &MutationIntent,
    services: &dyn Services,
) -> Result<(), ManagerError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .expect("Snapshot precedes ResolveCandidate");
    let candidate = state
        .candidate
        .as_ref()
        .expect("Snapshot creates candidate");
    let digest_text = format!("blake3:{}", candidate.digest.hex());
    let resolved = services
        .resolve(ResolveRequest {
            manifests: &candidate.manifests,
            manifest_digest: &digest_text,
            prior_lock: snapshot.lockfile(),
            mode: resolution_mode(intent.mode),
        })
        .map_err(|e| err(e.code(), Phase::Resolve, "candidate resolution failed", &e))?;
    let lock_bytes = resolved
        .lockfile
        .to_toml()
        .map_err(|e| {
            err(
                "XS3206",
                Phase::Resolve,
                "could not encode candidate lock",
                e,
            )
        })?
        .into_bytes();
    let transaction = mutation_plan(CandidateTransaction {
        snapshot,
        selected_path: &candidate.selected_path,
        selected_digest: &candidate.selected_digest,
        manifest: candidate.manifest.clone(),
        digest: candidate.digest.clone(),
        lock: &resolved.lockfile,
        lock_bytes,
        kind: intent.kind.clone(),
    })?;
    let after = project_state(
        &digest_text,
        Some(
            &transaction
                .files
                .iter()
                .find(|f| f.path == Path::new(LOCK_FILE_NAME))
                .expect("lock candidate exists")
                .candidate_digest,
        ),
    )?;
    state.result = Some(match &intent.edit {
        IntentEdit::Add(_) => OperationResult::Add(AddResult {
            dependency: intent.dependency.clone(),
            before: candidate.before.clone(),
            after,
            dry_run: intent.dry_run,
        }),
        IntentEdit::Remove => OperationResult::Remove(RemoveResult {
            dependency: intent.dependency.clone(),
            before: candidate.before.clone(),
            after,
            dry_run: intent.dry_run,
            affected_sources: candidate.affected.clone(),
        }),
    });
    state.transaction = Some(transaction);
    Ok(())
}

fn state_digests(snapshot: &ProjectSnapshot) -> Result<ProjectStateDigests, ManagerError> {
    project_state(
        snapshot.manifest_digest(),
        snapshot.lockfile().map(|_| snapshot.lock_digest()),
    )
}

fn commit_stage(state: &MutationState, intent: &MutationIntent) -> Result<bool, ManagerError> {
    if intent.dry_run {
        return Ok(false);
    }
    let repository = state
        .repository
        .as_ref()
        .expect("planning loaded repository");
    let transaction = state
        .transaction
        .as_ref()
        .expect("planning resolved transaction");
    match repository.commit(transaction) {
        Ok(()) => Ok(false),
        Err(RepositoryError::Contended(_)) => Ok(true),
        Err(error) => Err(err(
            "XS3207",
            Phase::Publish,
            "mutation transaction failed",
            error,
        )),
    }
}

fn bootstrap_locations(
    repository: &ProjectRepository,
    mode: LockMode,
    services: &dyn Services,
) -> Result<Vec<squish_repository::PackageLocation>, ManagerError> {
    let path = repository.root().join(LOCK_FILE_NAME);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(err(
                "XS3203",
                Phase::Snapshot,
                "could not read prior lock",
                error,
            ));
        }
    };
    let source = std::str::from_utf8(&bytes)
        .map_err(|error| err("XS3203", Phase::Snapshot, "prior lock is not UTF-8", error))?;
    let lock = Lockfile::parse(source)
        .map_err(|error| err("XS3203", Phase::Snapshot, "prior lock is invalid", error))?;
    services
        .materialize_locked(repository.root(), &lock, resolution_mode(mode))
        .map_err(|error| {
            err(
                error.code(),
                Phase::Resolve,
                "could not materialize prior lock",
                &error,
            )
        })
}

struct CandidateSet {
    manifests: BTreeMap<String, Manifest>,
    manifest: Vec<u8>,
    digest: Digest,
}

fn candidate_set(
    snapshot: &ProjectSnapshot,
    changed: &Path,
    candidate: &[u8],
) -> Result<CandidateSet, ManagerError> {
    let mut bytes = BTreeMap::<String, Vec<u8>>::new();
    let mut manifests = BTreeMap::new();
    for item in snapshot.manifests() {
        let content = if item.path == changed {
            candidate
        } else {
            &item.bytes
        };
        let path = slash(&item.path);
        let source = std::str::from_utf8(content)
            .map_err(|e| err("XS3204", Phase::Manage, "manifest is not UTF-8", e))?;
        manifests.insert(
            path.clone(),
            Manifest::parse(source)
                .map_err(|e| err("XS3205", Phase::Manage, "candidate manifest is invalid", e))?,
        );
        bytes.insert(path, content.to_vec());
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish\0manifest-set\0");
    for (path, content) in bytes {
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(&(content.len() as u64).to_le_bytes());
        hash.update(&content);
    }
    let digest = Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec())
        .expect("BLAKE3 is 32 bytes");
    Ok(CandidateSet {
        manifests,
        manifest: candidate.to_vec(),
        digest,
    })
}

struct CandidateTransaction<'a> {
    snapshot: &'a ProjectSnapshot,
    selected_path: &'a Path,
    selected_digest: &'a str,
    manifest: Vec<u8>,
    digest: Digest,
    lock: &'a Lockfile,
    lock_bytes: Vec<u8>,
    kind: MutationKind,
}

fn mutation_plan(candidate: CandidateTransaction<'_>) -> Result<MutationPlan, ManagerError> {
    let lock_path = PathBuf::from(LOCK_FILE_NAME);
    let files = vec![
        mutation_file(
            candidate.selected_path,
            candidate.selected_digest.into(),
            candidate.manifest,
        )?,
        mutation_file(
            &lock_path,
            candidate.snapshot.lock_digest().into(),
            candidate.lock_bytes,
        )?,
    ];
    let alias = match &candidate.kind {
        MutationKind::AddDependency { alias } | MutationKind::RemoveDependency { alias } => alias,
    };
    let id = TransactionId(format!(
        "dependency-{}-{}",
        alias,
        &candidate.digest.hex()[..16]
    ));
    MutationPlanner::plan(
        id,
        candidate.kind,
        format!("blake3:{}", candidate.digest.hex()),
        candidate.lock,
        candidate.snapshot.read_set().clone(),
        files,
    )
    .map_err(|e| err("XS3208", Phase::Manage, "invalid mutation transaction", e))
}

fn mutation_file(
    path: &Path,
    expected_digest: String,
    candidate: Vec<u8>,
) -> Result<MutationFile, ManagerError> {
    let candidate_digest = ProjectRepository::candidate_digest(path, &candidate)
        .map_err(|e| err("XS3208", Phase::Manage, "invalid mutation candidate", e))?;
    Ok(MutationFile {
        path: path.into(),
        expected_digest,
        candidate,
        candidate_digest,
    })
}

fn select_manifest<'a>(
    snapshot: &'a ProjectSnapshot,
    package: Option<&str>,
) -> Result<&'a ManifestSnapshot, ManagerError> {
    let matches: Vec<_> = snapshot
        .manifests()
        .iter()
        .filter(|item| {
            item.manifest
                .package
                .as_ref()
                .is_some_and(|p| package.is_none_or(|name| p.name == name))
        })
        .collect();
    match matches.as_slice() {
        [item] => Ok(item),
        [] => Err(ManagerError::new(
            "XS3209",
            Phase::Manage,
            package.map_or("current project has no package".into(), |p| {
                format!("unknown workspace package `{p}`")
            }),
        )),
        _ => Err(ManagerError::new(
            "XS3209",
            Phase::Manage,
            "workspace has multiple packages; select one explicitly",
        )),
    }
}

fn dependency_spec(request: &AddRequest) -> Result<DependencySpec, ManagerError> {
    let mut detail = DependencyDetail {
        package: request.rename.as_ref().map(|v| v.as_str().into()),
        optional: request.optional,
        features: request.features.iter().map(|v| v.as_str().into()).collect(),
        default_features: !request.no_default_features,
        ..DependencyDetail::default()
    };
    match &request.source {
        DependencySource::Path { path } => detail.path = Some(path.as_str().into()),
        DependencySource::Registry { registry, version } => {
            detail.registry = registry.as_ref().map(|v| v.as_str().into());
            detail.version = Some(
                version
                    .as_str()
                    .parse()
                    .map_err(|e| err("XS3210", Phase::Manage, "invalid version requirement", e))?,
            );
        }
        DependencySource::Workspace { package } => {
            detail.workspace = true;
            detail.package = Some(package.as_str().into());
        }
        DependencySource::Git {
            repository,
            reference,
        } => {
            detail.git = Some(repository.as_str().into());
            detail.git_reference = match reference {
                GitReference::Revision(v) => ProjectGitReference {
                    rev: Some(v.as_str().into()),
                    ..Default::default()
                },
                GitReference::Branch(v) => ProjectGitReference {
                    branch: Some(v.as_str().into()),
                    ..Default::default()
                },
                GitReference::Tag(v) => ProjectGitReference {
                    tag: Some(v.as_str().into()),
                    ..Default::default()
                },
            };
        }
    }
    Ok(DependencySpec::Detail(Box::new(detail)))
}

fn affected_sources(
    selected: &ManifestSnapshot,
    root: &Path,
    alias: &str,
) -> Result<Vec<OpaqueSourceId>, ManagerError> {
    let Some(package) = &selected.manifest.package else {
        return Ok(Vec::new());
    };
    let mut paths = BTreeSet::new();
    collect_xml(
        &selected.package_dir.join(&package.source_root),
        &selected.package_dir,
        &mut paths,
    )
    .map_err(|e| err("XS3211", Phase::Scan, "could not scan package sources", e))?;
    paths.extend(selected.manifest.targets.values().map(|v| v.entry.clone()));
    paths.extend(selected.manifest.exports.values().cloned());
    let package_id = PackageId::new(&package.name)
        .map_err(|e| err("XS3211", Phase::Scan, "invalid package source identity", e))?;
    let mut result = Vec::new();
    for path in paths {
        let absolute = selected.package_dir.join(&path);
        let bytes = match fs::read(&absolute) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(err(
                    "XS3211",
                    Phase::Scan,
                    "could not read package source",
                    e,
                ));
            }
        };
        if imports_alias(&bytes, alias) {
            let logical = LogicalPath::new(slash(&path))
                .map_err(|e| err("XS3211", Phase::Scan, "invalid package source path", e))?;
            result.push(SourceId::new(package_id.clone(), logical).to_protocol());
        }
    }
    result.sort();
    result.dedup();
    let _ = root; // Root participates in repository ownership validation before this scan.
    Ok(result)
}

fn imports_alias(bytes: &[u8], alias: &str) -> bool {
    let mut reader = NsReader::from_reader(bytes);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if element.name().local_name().as_ref() == b"import" =>
            {
                let (namespace, _) = reader.resolve_element(element.name());
                if matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == squish_xml_front::DSL_NAMESPACE.as_bytes())
                {
                    for attribute in element.attributes().flatten() {
                        if attribute.key.local_name().as_ref() == b"src"
                            && attribute
                                .decode_and_unescape_value(reader.decoder())
                                .ok()
                                .is_some_and(|value| {
                                    value
                                        .strip_prefix("pkg:")
                                        .and_then(|v| v.split_once('/'))
                                        .is_some_and(|(found, _)| found == alias)
                                })
                        {
                            return true;
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => return false,
            _ => {}
        }
    }
}

fn collect_xml(dir: &Path, package_dir: &Path, out: &mut BTreeSet<PathBuf>) -> std::io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut entries = entries.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let ty = entry.file_type()?;
        if ty.is_dir() {
            collect_xml(&entry.path(), package_dir, out)?;
        } else if ty.is_file()
            && entry.path().extension().is_some_and(|v| v == "xml")
            && !entry.path().to_string_lossy().ends_with(".i.xml")
            && !entry.path().to_string_lossy().ends_with(".o.xml")
            && let Ok(path) = entry.path().strip_prefix(package_dir)
        {
            out.insert(path.into());
        }
    }
    Ok(())
}

fn mutation_job(context: &InvocationContext) -> JobId {
    JobId::new(format!("{}-mutation", context.id())).expect("invocation ID is non-empty")
}

fn execute(
    intent: MutationIntent,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = mutation_job(context);
    let kind = intent.operation_kind();
    for attempt_index in 0..=RETRIES {
        let attempt = PlanningAttemptId::new(format!("mutation-attempt-{}", attempt_index + 1))
            .expect("attempt ID is non-empty");
        let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
            Ok(recorder) => recorder,
            Err(_) => return unavailable(job, kind, context.is_cancelled()),
        };
        let mut state = MutationState::default();
        if let Err(failure) = recorder.step(planning_step("load"), PlanningStepKind::Locate, || {
            load_stage(&mut state, &intent, services, durability)
        }) {
            return recorder.unavailable(kind, &failure);
        }
        if let Err(failure) =
            recorder.step(planning_step("recover"), PlanningStepKind::Recover, || {
                recover_stage(&state)
            })
        {
            return recorder.unavailable(kind, &failure);
        }
        if let Err(failure) = recorder.step(
            planning_step("materialize-locked"),
            PlanningStepKind::Fetch,
            || materialize_stage(&mut state, &intent, services),
        ) {
            return recorder.unavailable(kind, &failure);
        }
        if let Err(failure) = recorder.step(
            planning_step("snapshot"),
            PlanningStepKind::Snapshot,
            || snapshot_stage(&mut state),
        ) {
            return recorder.unavailable(kind, &failure);
        }
        if let Err(failure) = recorder.step(
            planning_step("prepare-candidate"),
            PlanningStepKind::PrepareCandidate,
            || candidate_stage(&mut state, &intent),
        ) {
            return recorder.unavailable(kind, &failure);
        }
        if let Err(failure) = recorder.step(
            planning_step("resolve-candidate"),
            PlanningStepKind::Resolve,
            || resolve_stage(&mut state, &intent, services),
        ) {
            return recorder.unavailable(kind, &failure);
        }
        let snapshot_digest = sealed_digest(&state);
        let prepared = match recorder.step(
            planning_step("validate-plan"),
            PlanningStepKind::ValidatePlan,
            || plan(intent.dry_run, &snapshot_digest),
        ) {
            Ok(prepared) => prepared,
            Err(failure) => return recorder.unavailable(kind, &failure),
        };
        let sealed = match recorder.seal(prepared, &snapshot_digest, PlanMode::Execute) {
            Ok(sealed) => sealed,
            Err(failure) => return unavailable_for_failure(job, kind, &failure),
        };
        let executor = MutationExecutor {
            intent: intent.clone(),
            state: std::sync::Mutex::new(state),
        };
        let report = match orchestrator::run(sealed, &executor, context, settings) {
            Ok(report) => report,
            Err(_) => return unavailable(job, kind, context.is_cancelled()),
        };
        if report.superseded == Some(SupersedeReason::AuthoritativeRevisionChanged) {
            continue;
        }
        let result = executor
            .state
            .lock()
            .unwrap()
            .result
            .clone()
            .unwrap_or(OperationResult::Unavailable { kind });
        return OperationOutcome {
            job,
            result,
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        };
    }
    let attempt = PlanningAttemptId::new(format!("mutation-attempt-{}", RETRIES + 2))
        .expect("attempt ID is non-empty");
    let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(recorder) => recorder,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    let exhausted: Result<(), PlanningFailure> = recorder.step(
        planning_step("retry-budget"),
        PlanningStepKind::ValidatePlan,
        || {
            Err(ManagerError::new(
                "XS3207",
                Phase::Publish,
                "project kept changing while committing the dependency mutation",
            ))
        },
    );
    let failure = exhausted.expect_err("retry-budget validation always fails");
    recorder.unavailable(kind, &failure)
}

fn sealed_digest(state: &MutationState) -> Digest {
    let transaction = state
        .transaction
        .as_ref()
        .expect("resolution created transaction");
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish\0sealed-mutation\0v1\0");
    let (tag, alias) = match &transaction.kind {
        MutationKind::AddDependency { alias } => (b"add".as_slice(), alias.as_str()),
        MutationKind::RemoveDependency { alias } => (b"remove".as_slice(), alias.as_str()),
    };
    hash_field(&mut hash, tag);
    hash_field(&mut hash, alias.as_bytes());
    hash_field(&mut hash, transaction.manifest_digest.as_bytes());
    let mut observations: Vec<_> = transaction
        .observations
        .iter()
        .map(|(path, digest)| (slash(path), digest))
        .collect();
    observations.sort_by(|left, right| left.0.cmp(&right.0));
    hash.update(&(observations.len() as u64).to_le_bytes());
    for (path, digest) in observations {
        hash_field(&mut hash, path.as_bytes());
        hash_field(&mut hash, digest.as_bytes());
    }
    let mut files: Vec<_> = transaction
        .files
        .iter()
        .map(|file| (slash(&file.path), file))
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    hash.update(&(files.len() as u64).to_le_bytes());
    for (path, file) in files {
        hash_field(&mut hash, path.as_bytes());
        hash_field(&mut hash, file.expected_digest.as_bytes());
        hash_field(&mut hash, file.candidate_digest.as_bytes());
        hash_field(&mut hash, &file.candidate);
    }
    Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec())
        .expect("BLAKE3 is 32 bytes")
}

fn hash_field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn unavailable(job: JobId, kind: OperationKind, cancelled: bool) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable { kind },
        totals: ActionTotals::default(),
        root_failures: u64::from(!cancelled),
        cancelled,
    }
}

fn unavailable_for_failure(
    job: JobId,
    kind: OperationKind,
    failure: &PlanningFailure,
) -> OperationOutcome {
    unavailable(job, kind, matches!(failure, PlanningFailure::Cancelled))
}

fn planning_step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("static planning step ID is non-empty")
}

struct MutationExecutor {
    intent: MutationIntent,
    state: std::sync::Mutex<MutationState>,
}

impl WorkExecutor<MutationWork> for MutationExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &MutationWork,
        _: &PreparedPlan<MutationWork>,
        _inputs: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }

    fn execute(
        &self,
        _: &Dispatch,
        work: &MutationWork,
        _: &PreparedPlan<MutationWork>,
        _inputs: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("manager_cancelled", "mutation was cancelled").into();
        }
        let state = self.state.lock().unwrap();
        match work {
            MutationWork::CommitTransaction { .. } => match commit_stage(&state, &self.intent) {
                Ok(false) => ActionResult::success().into(),
                Ok(true) => WorkDisposition::Superseded {
                    reason: SupersedeReason::AuthoritativeRevisionChanged,
                    events: Vec::new(),
                },
                Err(error) => ActionResult::failure(error.code(), error.message()).into(),
            },
        }
    }

    fn record(&self, _: &Dispatch, _: &MutationWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

fn require_normal(kind: DependencyKind) -> Result<(), ManagerError> {
    if kind == DependencyKind::Normal {
        Ok(())
    } else {
        Err(ManagerError::new(
            "XS3212",
            Phase::Manage,
            "development and build dependency tables are not supported by manifest version 1",
        ))
    }
}
fn resolution_mode(mode: LockMode) -> ResolutionMode {
    match mode {
        LockMode::Update => ResolutionMode::Online,
        LockMode::Locked => ResolutionMode::Locked,
        LockMode::Offline => ResolutionMode::Offline,
        LockMode::Frozen => ResolutionMode::Frozen,
    }
}
fn project_state(manifest: &str, lock: Option<&str>) -> Result<ProjectStateDigests, ManagerError> {
    Ok(ProjectStateDigests {
        manifest: parse_digest(manifest)?,
        lock: lock.map(parse_digest).transpose()?,
    })
}
fn parse_digest(value: &str) -> Result<Digest, ManagerError> {
    let (_, hex) = value.split_once(':').ok_or_else(|| {
        ManagerError::new(
            "XS3213",
            Phase::Manage,
            "repository returned a malformed digest",
        )
    })?;
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or(""), 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            err(
                "XS3213",
                Phase::Manage,
                "repository returned a malformed digest",
                e,
            )
        })?;
    Digest::new(DigestAlgorithm::Blake3, bytes).map_err(|e| {
        err(
            "XS3213",
            Phase::Manage,
            "repository returned a malformed digest",
            e,
        )
    })
}
fn slash(path: &Path) -> String {
    path.components()
        .map(|v| v.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
fn action_id(value: &str) -> Result<ActionId, ManagerError> {
    ActionId::new(value).map_err(|e| err("XS3200", Phase::Manage, "invalid mutation action ID", e))
}
fn action(
    id: ActionId,
    kind: ActionKind,
    dependencies: Vec<ActionId>,
    inputs: Vec<InputRef>,
) -> Result<Action, ManagerError> {
    let options = BTreeMap::from([("stage".into(), id.as_str().into())]);
    Ok(Action {
        id,
        key: KeyRecipe::new("xmlsquish-mutation-v2", inputs, options)
            .map_err(|e| err("XS3200", Phase::Manage, "invalid mutation action key", e))?,
        kind,
        class: ResourceClass::Io,
        resources: Resources::new(0, 1, 0),
        dependencies,
        outputs: Vec::new(),
    })
}
fn err(code: &str, phase: Phase, context: &str, error: impl std::fmt::Display) -> ManagerError {
    ManagerError::new(code, phase, format!("{context}: {error}"))
}
