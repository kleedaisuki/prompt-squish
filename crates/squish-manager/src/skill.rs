//! Transactional Agent Skill dependencies and the separately bundled tool skill.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use fs2::FileExt;
use serde::Deserialize;

use squish_build::{ActionResult, Dispatch};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_project::{
    CandidateManifest, GitReference as ProjectGitReference, LOCK_FILE_NAME, LockedSkill,
    LockedSkillSource, Lockfile, MANIFEST_FILE_NAME, Manifest, MutationFile, MutationKind,
    MutationPlan, MutationPlanner, ResolutionMode, SkillSpec, TransactionId,
};
use squish_protocol::{
    ActionTotals, AddSkillRequest, AddSkillResult, Digest, DigestAlgorithm, GitReference,
    InstallSkillRequest, InstallSkillResult, JobId, LockMode, OperationKind, OperationResult,
    Phase, PlanMode, PlanningAttemptId, PlanningStepId, PlanningStepKind, ProjectDestination,
    ProjectStateDigests, RemoveSkillRequest, RemoveSkillResult, SkillName, SkillSource,
    SupersedeReason, SyncSkillsRequest, SyncSkillsResult,
};
use squish_repository::{
    Discovery, ProjectRepository, ProjectSnapshot, RepositoryError, SkillDirectoryUpdate,
    skill_marker, skill_tree_digest,
};

use crate::{
    DurabilityPorts, InvocationSettings, ManagerError, PreparedPlan, ResolvedSkillSource, Services,
    mutation,
    orchestrator::{
        self, CachedResult, PlanningFailure, PlanningRecorder, ResolvedInputs, WorkDisposition,
        WorkExecutor,
    },
};

const RETRIES: u8 = 4;
const MARKER: &str = ".xmlsquish-managed";

/// Add or update a skill declaration, its exact lock entry, and its installed copy.
pub(crate) fn add(
    request: &AddSkillRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    mutate(
        SkillIntent::Add(request.clone()),
        services,
        settings,
        durability,
        context,
    )
}

/// Remove a skill declaration, its lock entry, and only its owned installed copy.
pub(crate) fn remove(
    request: &RemoveSkillRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    mutate(
        SkillIntent::Remove(request.clone()),
        services,
        settings,
        durability,
        context,
    )
}

#[derive(Clone)]
enum SkillIntent {
    Add(AddSkillRequest),
    Remove(RemoveSkillRequest),
}

impl SkillIntent {
    fn kind(&self) -> OperationKind {
        match self {
            Self::Add(_) => OperationKind::AddSkill,
            Self::Remove(_) => OperationKind::RemoveSkill,
        }
    }
    fn project(&self) -> &squish_protocol::ProjectPath {
        match self {
            Self::Add(v) => &v.project,
            Self::Remove(v) => &v.project,
        }
    }
    fn name(&self) -> &SkillName {
        match self {
            Self::Add(v) => &v.skill,
            Self::Remove(v) => &v.skill,
        }
    }
    fn mode(&self) -> LockMode {
        match self {
            Self::Add(v) => v.lock,
            Self::Remove(v) => v.lock,
        }
    }
    fn dry_run(&self) -> bool {
        match self {
            Self::Add(v) => v.dry_run,
            Self::Remove(v) => v.dry_run,
        }
    }
}

#[derive(Default)]
struct SkillState {
    repository: Option<ProjectRepository>,
    snapshot: Option<ProjectSnapshot>,
    transaction: Option<MutationPlan>,
    update: Option<SkillDirectoryUpdate>,
    result: Option<OperationResult>,
    identity: Option<Digest>,
}

fn mutate(
    intent: SkillIntent,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = job(context, "skill-mutation");
    let kind = intent.kind();
    for index in 0..=RETRIES {
        let attempt = PlanningAttemptId::new(format!("skill-attempt-{}", index + 1)).unwrap();
        let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
            Ok(v) => v,
            Err(_) => return unavailable(job, kind, context.is_cancelled()),
        };
        let mut state = SkillState::default();
        if let Err(f) = recorder.step(step("locate"), PlanningStepKind::Locate, || {
            let repo = ProjectRepository::discover_with_faults(
                Discovery::Explicit(PathBuf::from(intent.project().as_str())),
                durability.repository(),
            )
            .map_err(|e| error("XS3301", Phase::Discover, "could not load project", e))?;
            services.storage_layout(repo.root()).map_err(|e| {
                error(
                    e.code(),
                    Phase::Discover,
                    "project storage layout is unavailable",
                    &e,
                )
            })?;
            state.repository = Some(repo);
            Ok(())
        }) {
            return recorder.unavailable(kind, &f);
        }
        if let Err(f) = recorder.step(step("recover"), PlanningStepKind::Recover, || {
            state
                .repository
                .as_ref()
                .unwrap()
                .recover()
                .map_err(|e| error("XS3302", Phase::Snapshot, "could not recover project", e))
        }) {
            return recorder.unavailable(kind, &f);
        }
        if let Err(f) = recorder.step(step("snapshot"), PlanningStepKind::Snapshot, || {
            state.snapshot = Some(
                state
                    .repository
                    .as_ref()
                    .unwrap()
                    .snapshot_workspace()
                    .map_err(|e| {
                        error("XS3303", Phase::Snapshot, "could not snapshot project", e)
                    })?,
            );
            Ok(())
        }) {
            return recorder.unavailable(kind, &f);
        }
        if let Err(f) = recorder.step(
            step("prepare-candidate"),
            PlanningStepKind::PrepareCandidate,
            || prepare(&mut state, &intent, services),
        ) {
            return recorder.unavailable(kind, &f);
        }
        let identity = state.identity.clone().expect("candidate identity set");
        let prepared = match recorder.step(
            step("validate-plan"),
            PlanningStepKind::ValidatePlan,
            || mutation::plan(intent.dry_run(), &identity),
        ) {
            Ok(v) => v,
            Err(f) => return recorder.unavailable(kind, &f),
        };
        let sealed = match recorder.seal(prepared, &identity, PlanMode::Execute) {
            Ok(v) => v,
            Err(f) => return unavailable_for_failure(job, kind, &f),
        };
        let executor = SkillExecutor {
            state: Mutex::new(state),
            dry_run: intent.dry_run(),
        };
        let report = match orchestrator::run(sealed, &executor, context, settings) {
            Ok(v) => v,
            Err(_) => return unavailable(job, kind, context.is_cancelled()),
        };
        if report.superseded == Some(SupersedeReason::AuthoritativeRevisionChanged) {
            continue;
        }
        let result = if report.root_failures == 0 && !report.cancelled {
            executor
                .state
                .lock()
                .unwrap()
                .result
                .clone()
                .unwrap_or(OperationResult::Unavailable { kind })
        } else {
            OperationResult::Unavailable { kind }
        };
        return OperationOutcome {
            job,
            result,
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        };
    }
    let attempt = PlanningAttemptId::new("skill-retry-exhausted").unwrap();
    let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(v) => v,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    let failure: Result<(), PlanningFailure> =
        recorder.step(step("retry-budget"), PlanningStepKind::ValidatePlan, || {
            Err(ManagerError::new(
                "XS3309",
                Phase::Publish,
                "project kept changing while committing skill dependency",
            ))
        });
    recorder.unavailable(kind, &failure.expect_err("retry budget fails"))
}

fn prepare(
    state: &mut SkillState,
    intent: &SkillIntent,
    services: &dyn Services,
) -> Result<(), ManagerError> {
    let snapshot = state
        .snapshot
        .as_ref()
        .expect("snapshot precedes candidate");
    let observed_manifests: BTreeMap<String, Manifest> = snapshot
        .manifests()
        .iter()
        .map(|item| {
            (
                item.path.to_string_lossy().replace('\\', "/"),
                item.manifest.clone(),
            )
        })
        .collect();
    validate_skill_lock(&observed_manifests, snapshot.lockfile())?;
    let root_manifest = snapshot
        .manifests()
        .iter()
        .find(|m| m.path == Path::new(MANIFEST_FILE_NAME))
        .ok_or_else(|| {
            ManagerError::new("XS3304", Phase::Manage, "workspace root manifest missing")
        })?;
    let source = std::str::from_utf8(&root_manifest.bytes)
        .map_err(|e| error("XS3304", Phase::Manage, "root manifest is not UTF-8", e))?;
    let editable = CandidateManifest::parse(source)
        .map_err(|e| error("XS3304", Phase::Manage, "root manifest is invalid", e))?;
    let name = intent.name().as_str();
    let edit = match intent {
        SkillIntent::Add(request) => {
            editable.add_skill(name, spec(&request.source, snapshot.root())?, true)
        }
        SkillIntent::Remove(_) => editable.remove_skill(name),
    }
    .map_err(|e| error("XS3304", Phase::Manage, "skill edit is invalid", e))?;
    let candidate = mutation::candidate_set(snapshot, &root_manifest.path, edit.after.as_bytes())?;
    let digest_text = format!("blake3:{}", candidate.digest.hex());
    let old_lock = snapshot.lockfile();
    let prior = old_lock.and_then(|lock| lock.skills.iter().find(|skill| skill.name == name));
    let old_spec = root_manifest.manifest.skills.get(name);
    let mut lock = services
        .resolve(crate::ResolveRequest {
            manifests: &candidate.manifests,
            manifest_digest: &digest_text,
            prior_lock: old_lock,
            mode: resolution_mode(intent.mode()),
        })
        .map_err(|e| error(e.code(), Phase::Resolve, "package resolution failed", &e))?
        .lockfile;
    validate_preserved_skill_lock(old_lock, &lock)?;
    let mut update = SkillDirectoryUpdate {
        name: name.into(),
        expected_digest: prior.map(|s| s.digest.clone()),
        files: None,
    };
    match intent {
        SkillIntent::Add(request) => {
            let spec = spec(&request.source, snapshot.root())?;
            let pinned = if old_spec == Some(&spec) && intent.mode() != LockMode::Update {
                prior.and_then(|skill| match &skill.source {
                    LockedSkillSource::Git { revision, .. } => Some(revision.as_str()),
                    _ => None,
                })
            } else {
                None
            };
            let normalized_source = source_from_spec(&spec)?;
            let resolved = services
                .resolve_skill_source(
                    snapshot.root(),
                    &normalized_source,
                    pinned,
                    resolution_mode(intent.mode()),
                )
                .map_err(|e| {
                    error(
                        e.code(),
                        Phase::Resolve,
                        "skill source resolution failed",
                        &e,
                    )
                })?;
            validate_tree(name, &resolved)?;
            let digest = skill_tree_digest(&resolved.files)
                .map_err(|e| error("XS3305", Phase::Resolve, "invalid skill tree", e))?;
            let locked = locked_skill(name, &spec, &resolved, digest.clone())?;
            lock.skills.retain(|skill| skill.name != name);
            lock.skills.push(locked);
            let mut files = resolved.files;
            files.insert(PathBuf::from(MARKER), skill_marker(name, &digest));
            update.files = Some(files);
        }
        SkillIntent::Remove(_) => {
            if prior.is_none() {
                return Err(ManagerError::new(
                    "XS3306",
                    Phase::Manage,
                    "skill is declared but missing from xmlsquish.lock",
                ));
            }
            lock.skills.retain(|skill| skill.name != name);
        }
    }
    lock.manifest_digest = digest_text.clone();
    lock.skills.sort_by(|a, b| a.name.cmp(&b.name));
    lock.validate()
        .map_err(|e| error("XS3306", Phase::Resolve, "invalid skill lock", e))?;
    let lock_bytes = lock
        .to_toml()
        .map_err(|e| error("XS3306", Phase::Resolve, "could not encode skill lock", e))?
        .into_bytes();
    if matches!(intent.mode(), LockMode::Locked | LockMode::Frozen)
        && (candidate.manifest != root_manifest.bytes.as_ref()
            || snapshot
                .lock_bytes()
                .is_none_or(|old| old != lock_bytes.as_slice()))
    {
        return Err(ManagerError::new(
            "XS3307",
            Phase::Resolve,
            "--locked/--frozen forbids changing manifest or lock",
        ));
    }
    let before = project_state(
        snapshot.manifest_digest(),
        old_lock.map(|_| snapshot.lock_digest()),
    )?;
    let manifest_file = mutation_file(
        &root_manifest.path,
        &root_manifest.digest,
        candidate.manifest,
    )?;
    let lock_file = mutation_file(
        Path::new(LOCK_FILE_NAME),
        snapshot.lock_digest(),
        lock_bytes,
    )?;
    let after = project_state(&digest_text, Some(&lock_file.candidate_digest))?;
    let mutation_kind = match intent {
        SkillIntent::Add(_) => MutationKind::AddSkill { name: name.into() },
        SkillIntent::Remove(_) => MutationKind::RemoveSkill { name: name.into() },
    };
    let id = TransactionId(format!("skill-{}-{}", name, &candidate.digest.hex()[..16]));
    let transaction = MutationPlanner::plan(
        id,
        mutation_kind,
        digest_text,
        &lock,
        snapshot.read_set().clone(),
        vec![manifest_file, lock_file],
    )
    .map_err(|e| error("XS3308", Phase::Manage, "invalid skill transaction", e))?;
    let actual = state
        .repository
        .as_ref()
        .unwrap()
        .skill_directory_digest(name)
        .map_err(|e| {
            error(
                "XS3308",
                Phase::Manage,
                "unsafe or unowned skill destination",
                e,
            )
        })?;
    if actual != update.expected_digest {
        return Err(ManagerError::new(
            "XS3308",
            Phase::Manage,
            "installed skill differs from its locked, manager-owned snapshot",
        ));
    }
    state.identity = Some(sealed_identity(&transaction, &update));
    state.result = Some(match intent {
        SkillIntent::Add(_) => OperationResult::AddSkill(AddSkillResult {
            skill: intent.name().clone(),
            before,
            after,
            dry_run: intent.dry_run(),
        }),
        SkillIntent::Remove(_) => OperationResult::RemoveSkill(RemoveSkillResult {
            skill: intent.name().clone(),
            before,
            after,
            dry_run: intent.dry_run(),
        }),
    });
    state.transaction = Some(transaction);
    state.update = Some(update);
    Ok(())
}

fn spec(source: &SkillSource, root: &Path) -> Result<SkillSpec, ManagerError> {
    Ok(match source {
        SkillSource::Path { path } => {
            let supplied = Path::new(path.as_str());
            let absolute = if supplied.is_absolute() {
                supplied.to_path_buf()
            } else {
                root.join(supplied)
            };
            reject_source_aliases(&absolute)?;
            let canonical = absolute.canonicalize().map_err(|e| {
                error(
                    "XS3305",
                    Phase::Manage,
                    "local skill directory is unavailable",
                    e,
                )
            })?;
            let destination = root.join(".agents").join("skills");
            if canonical.starts_with(&destination) || destination.starts_with(&canonical) {
                return Err(ManagerError::new(
                    "XS3305",
                    Phase::Manage,
                    "skill source cannot overlap managed installation directories",
                ));
            }
            SkillSpec::path(relative_path(root, &canonical)?)
        }
        SkillSource::Git {
            repository,
            reference,
            subdir,
        } => {
            let reference = match reference {
                GitReference::Head => ProjectGitReference::default(),
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
            SkillSpec::git(
                repository.as_str(),
                reference,
                subdir.as_ref().map(|path| PathBuf::from(path.as_str())),
            )
        }
    })
}

fn reject_source_aliases(path: &Path) -> Result<(), ManagerError> {
    let mut ancestor = PathBuf::new();
    for part in path.components() {
        ancestor.push(part.as_os_str());
        let metadata = fs::symlink_metadata(&ancestor).map_err(|e| {
            error(
                "XS3305",
                Phase::Manage,
                "could not inspect local skill path",
                e,
            )
        })?;
        if is_alias(&metadata) {
            return Err(ManagerError::new(
                "XS3305",
                Phase::Manage,
                format!(
                    "local skill path crosses a filesystem alias at `{}`",
                    ancestor.display()
                ),
            ));
        }
    }
    Ok(())
}

fn relative_path(root: &Path, absolute: &Path) -> Result<PathBuf, ManagerError> {
    let left: Vec<_> = root.components().collect();
    let right: Vec<_> = absolute.components().collect();
    if left.first() != right.first() {
        return Err(ManagerError::new(
            "XS3305",
            Phase::Manage,
            "local skill must be on the same filesystem volume as the project",
        ));
    }
    let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..left.len() {
        relative.push("..");
    }
    for component in &right[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Ok(relative)
}

fn locked_skill(
    name: &str,
    spec: &SkillSpec,
    resolved: &ResolvedSkillSource,
    digest: String,
) -> Result<LockedSkill, ManagerError> {
    let source = if let Some(path) = &spec.path {
        if resolved.revision.is_some() {
            return Err(ManagerError::new(
                "XS3305",
                Phase::Resolve,
                "local skill unexpectedly returned a Git revision",
            ));
        }
        LockedSkillSource::Path {
            path: path.clone(),
            mutable: true,
        }
    } else {
        let revision = resolved.revision.clone().ok_or_else(|| {
            ManagerError::new(
                "XS3305",
                Phase::Resolve,
                "Git skill did not resolve to an exact commit",
            )
        })?;
        LockedSkillSource::Git {
            repository: spec.git.clone().unwrap(),
            revision,
            subdir: spec.subdir.clone(),
            reference: spec.git_reference.clone(),
        }
    };
    Ok(LockedSkill {
        name: name.into(),
        source,
        digest,
    })
}

fn validate_tree(name: &str, tree: &ResolvedSkillSource) -> Result<(), ManagerError> {
    if tree.files.contains_key(Path::new(MARKER)) {
        return Err(ManagerError::new(
            "XS3305",
            Phase::Resolve,
            "skill source contains reserved ownership marker",
        ));
    }
    for path in tree.files.keys() {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(ManagerError::new(
                "XS3305",
                Phase::Resolve,
                format!("unsafe skill file path `{}`", path.display()),
            ));
        }
    }
    let bytes = tree.files.get(Path::new("SKILL.md")).ok_or_else(|| {
        ManagerError::new("XS3305", Phase::Resolve, "skill source has no SKILL.md")
    })?;
    validate_frontmatter(name, bytes)?;
    Ok(())
}

fn validate_frontmatter(name: &str, bytes: &[u8]) -> Result<(), ManagerError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| error("XS3305", Phase::Resolve, "SKILL.md is not UTF-8", e))?;
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(ManagerError::new(
            "XS3305",
            Phase::Resolve,
            "SKILL.md must start with YAML frontmatter",
        ));
    }
    let mut frontmatter = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        if frontmatter.len().saturating_add(line.len() + 1) > 65_536 {
            return Err(ManagerError::new(
                "XS3305",
                Phase::Resolve,
                "SKILL.md frontmatter exceeds 64 KiB",
            ));
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    if !closed {
        return Err(ManagerError::new(
            "XS3305",
            Phase::Resolve,
            "SKILL.md frontmatter has no closing delimiter",
        ));
    }
    #[derive(Deserialize)]
    struct RequiredFields {
        name: String,
        description: String,
    }
    let parsed: RequiredFields = yaml_serde::from_str(&frontmatter).map_err(|e| {
        error(
            "XS3305",
            Phase::Resolve,
            "invalid SKILL.md YAML frontmatter",
            e,
        )
    })?;
    if parsed.name != name
        || parsed.description.trim().is_empty()
        || parsed.description.chars().count() > 1024
    {
        return Err(ManagerError::new(
            "XS3305",
            Phase::Resolve,
            "SKILL.md requires unique matching `name` and 1–1024 character `description` frontmatter",
        ));
    }
    Ok(())
}

fn mutation_file(
    path: &Path,
    expected_digest: &str,
    candidate: Vec<u8>,
) -> Result<MutationFile, ManagerError> {
    let candidate_digest = ProjectRepository::candidate_digest(path, &candidate)
        .map_err(|e| error("XS3308", Phase::Manage, "invalid mutation candidate", e))?;
    Ok(MutationFile {
        path: path.into(),
        expected_digest: expected_digest.into(),
        candidate,
        candidate_digest,
    })
}

fn sealed_identity(transaction: &MutationPlan, update: &SkillDirectoryUpdate) -> Digest {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish\0skill-mutation\0v1\0");
    hash.update(transaction.manifest_digest.as_bytes());
    for file in &transaction.files {
        hash.update(file.path.to_string_lossy().as_bytes());
        hash.update(&file.candidate);
    }
    hash.update(update.name.as_bytes());
    hash.update(update.expected_digest.as_deref().unwrap_or("").as_bytes());
    if let Some(files) = &update.files {
        for (path, bytes) in files {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update(bytes);
        }
    }
    Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec()).unwrap()
}

struct SkillExecutor {
    state: Mutex<SkillState>,
    dry_run: bool,
}

impl WorkExecutor<mutation::MutationWork> for SkillExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }
    fn execute(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("manager_cancelled", "skill mutation was cancelled")
                .into();
        }
        if self.dry_run {
            return ActionResult::success().into();
        }
        let state = self.state.lock().unwrap();
        let repo = state.repository.as_ref().unwrap();
        match repo.commit_with_skill(
            state.transaction.as_ref().unwrap(),
            state.update.as_ref().unwrap(),
        ) {
            Ok(()) => ActionResult::success().into(),
            Err(RepositoryError::Contended(_)) => WorkDisposition::Superseded {
                reason: SupersedeReason::AuthoritativeRevisionChanged,
                events: Vec::new(),
            },
            Err(e) => {
                ActionResult::failure("XS3309", format!("skill transaction failed: {e}")).into()
            }
        }
    }
    fn record(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

fn job(context: &InvocationContext, suffix: &str) -> JobId {
    JobId::new(format!("{}-{suffix}", context.id())).unwrap()
}
fn step(name: &str) -> PlanningStepId {
    PlanningStepId::new(name).unwrap()
}
fn resolution_mode(mode: LockMode) -> ResolutionMode {
    match mode {
        LockMode::Update => ResolutionMode::Online,
        LockMode::Locked => ResolutionMode::Locked,
        LockMode::Offline => ResolutionMode::Offline,
        LockMode::Frozen => ResolutionMode::Frozen,
    }
}
fn error(code: &str, phase: Phase, context: &str, cause: impl std::fmt::Display) -> ManagerError {
    ManagerError::new(code, phase, format!("{context}: {cause}"))
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
fn project_state(manifest: &str, lock: Option<&str>) -> Result<ProjectStateDigests, ManagerError> {
    Ok(ProjectStateDigests {
        manifest: parse_digest(manifest)?,
        lock: lock.map(parse_digest).transpose()?,
    })
}
fn parse_digest(value: &str) -> Result<Digest, ManagerError> {
    let (_, hex) = value.split_once(':').ok_or_else(|| {
        ManagerError::new(
            "XS3310",
            Phase::Manage,
            "repository returned malformed digest",
        )
    })?;
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or(""), 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            error(
                "XS3310",
                Phase::Manage,
                "repository returned malformed digest",
                e,
            )
        })?;
    Digest::new(DigestAlgorithm::Blake3, bytes).map_err(|e| {
        error(
            "XS3310",
            Phase::Manage,
            "repository returned malformed digest",
            e,
        )
    })
}

const BUNDLED_SKILL: &str = include_str!("../../../SKILL.md");
const BUNDLED_NAME: &str = "prompt-squish";
const INSTALL_BACKUP: &str = ".prompt-squish.xmlsquish-backup";
const INSTALL_STAGING: &str = ".prompt-squish.xmlsquish-staging";

fn bundled_marker() -> Result<Vec<u8>, ManagerError> {
    let files = BTreeMap::from([(PathBuf::from("SKILL.md"), BUNDLED_SKILL.as_bytes().to_vec())]);
    let digest = skill_tree_digest(&files)
        .map_err(|e| error("XS3321", Phase::Manage, "invalid bundled skill", e))?;
    Ok(format!("xmlsquish bundled skill v1\n{digest}\n").into_bytes())
}

/// Install the embedded xmlsquish guide independently of project dependency state.
pub(crate) fn install(
    request: &InstallSkillRequest,
    settings: &InvocationSettings,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = job(context, "install-skill");
    let kind = OperationKind::InstallSkill;
    let attempt = PlanningAttemptId::new("install-skill-attempt-1").unwrap();
    let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(v) => v,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    let destination = match recorder.step(step("locate"), PlanningStepKind::Locate, || {
        install_destination(request)
    }) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let changed = match recorder.step(
        step("prepare-candidate"),
        PlanningStepKind::PrepareCandidate,
        || {
            validate_frontmatter(BUNDLED_NAME, BUNDLED_SKILL.as_bytes())?;
            inspect_bundle(&destination, request.force)
        },
    ) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let identity = {
        let mut hash = blake3::Hasher::new();
        hash.update(b"xmlsquish\0bundled-skill\0v1\0");
        hash.update(destination.to_string_lossy().as_bytes());
        hash.update(BUNDLED_SKILL.as_bytes());
        Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec()).unwrap()
    };
    let prepared = match recorder.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || mutation::plan(request.dry_run, &identity),
    ) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let sealed = match recorder.seal(prepared, &identity, PlanMode::Execute) {
        Ok(v) => v,
        Err(f) => return unavailable_for_failure(job, kind, &f),
    };
    let executor = InstallExecutor {
        destination: destination.clone(),
        changed,
        force: request.force,
        dry_run: request.dry_run,
    };
    let report = match orchestrator::run(sealed, &executor, context, settings) {
        Ok(v) => v,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    OperationOutcome {
        job,
        result: if report.root_failures == 0 && !report.cancelled {
            OperationResult::InstallSkill(InstallSkillResult {
                destination: ProjectDestination::new(destination),
                changed,
                dry_run: request.dry_run,
            })
        } else {
            OperationResult::Unavailable { kind }
        },
        totals: report.totals,
        root_failures: report.root_failures,
        cancelled: report.cancelled,
    }
}

fn install_destination(request: &InstallSkillRequest) -> Result<PathBuf, ManagerError> {
    let root = if let Some(project) = &request.project {
        ProjectRepository::discover(Discovery::Explicit(PathBuf::from(project.as_str())))
            .map_err(|e| error("XS3320", Phase::Discover, "could not locate project", e))?
            .root()
            .to_path_buf()
    } else {
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
        } else {
            std::env::var_os("HOME")
        };
        let home = home.ok_or_else(|| {
            ManagerError::new("XS3320", Phase::Discover, "home directory is not available")
        })?;
        PathBuf::from(home)
            .canonicalize()
            .map_err(|e| error("XS3320", Phase::Discover, "home directory is invalid", e))?
    };
    let agents = root.join(".agents");
    let skills = agents.join("skills");
    reject_alias(&agents)?;
    reject_alias(&skills)?;
    let destination = skills.join(BUNDLED_NAME);
    reject_alias(&destination)?;
    Ok(destination)
}

fn reject_alias(path: &Path) -> Result<(), ManagerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_dir() || is_alias(&metadata) => Err(ManagerError::new(
            "XS3321",
            Phase::Manage,
            format!(
                "Agent Skills path `{}` is not an ordinary directory",
                path.display()
            ),
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(
            "XS3321",
            Phase::Manage,
            "could not inspect Agent Skills path",
            e,
        )),
    }
}

#[cfg(windows)]
fn is_alias(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_alias(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn inspect_bundle(destination: &Path, force: bool) -> Result<bool, ManagerError> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(error("XS3321", Phase::Manage, "could not inspect skill", e)),
    };
    if !metadata.is_dir() || is_alias(&metadata) {
        return Err(ManagerError::new(
            "XS3321",
            Phase::Manage,
            "bundled skill destination is not an ordinary directory",
        ));
    }
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(destination)
        .map_err(|e| error("XS3321", Phase::Manage, "could not list installed skill", e))?
    {
        let entry = entry
            .map_err(|e| error("XS3321", Phase::Manage, "could not list installed skill", e))?;
        let name = entry.file_name();
        if name != "SKILL.md" && name != MARKER {
            return Err(ManagerError::new(
                "XS3321",
                Phase::Manage,
                "bundled skill contains extra files; refusing to overwrite them",
            ));
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| {
            error(
                "XS3321",
                Phase::Manage,
                "could not inspect installed file",
                e,
            )
        })?;
        if !metadata.is_file() || is_alias(&metadata) {
            return Err(ManagerError::new(
                "XS3321",
                Phase::Manage,
                "bundled skill contains a link or non-regular file",
            ));
        }
        if metadata.len() > 1_048_576 {
            return Err(ManagerError::new(
                "XS3321",
                Phase::Manage,
                "bundled skill file exceeds the inspection limit",
            ));
        }
        files.insert(
            PathBuf::from(name),
            fs::read(entry.path())
                .map_err(|e| error("XS3321", Phase::Manage, "could not read installed file", e))?,
        );
    }
    let marker = files.get(Path::new(MARKER)).ok_or_else(|| {
        ManagerError::new(
            "XS3321",
            Phase::Manage,
            "existing skill is not manager-owned",
        )
    })?;
    let actual = skill_tree_digest(&files)
        .map_err(|e| error("XS3321", Phase::Manage, "invalid installed skill tree", e))?;
    if marker != &format!("xmlsquish bundled skill v1\n{actual}\n").into_bytes() {
        return Err(ManagerError::new(
            "XS3321",
            Phase::Manage,
            "existing bundled skill was modified or is not manager-owned",
        ));
    }
    let installed = files.get(Path::new("SKILL.md")).ok_or_else(|| {
        ManagerError::new("XS3321", Phase::Manage, "installed skill lacks SKILL.md")
    })?;
    if installed == BUNDLED_SKILL.as_bytes() {
        return Ok(false);
    }
    if !force {
        return Err(ManagerError::new(
            "XS3321",
            Phase::Manage,
            "bundled skill differs; pass --force to update the owned installation",
        ));
    }
    Ok(true)
}

fn publish_bundle(destination: &Path, force: bool) -> Result<(), ManagerError> {
    let parent = destination.parent().expect("skill has a parent");
    reject_alias(parent.parent().expect("skills has an .agents parent"))?;
    reject_alias(parent)?;
    fs::create_dir_all(parent).map_err(|e| {
        error(
            "XS3322",
            Phase::Publish,
            "could not create Agent Skills directory",
            e,
        )
    })?;
    reject_alias(parent)?;
    let lock_path = parent.join(".prompt-squish.xmlsquish.lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|e| error("XS3322", Phase::Publish, "could not open install lock", e))?;
    lock.lock_exclusive()
        .map_err(|e| error("XS3322", Phase::Publish, "could not lock install", e))?;
    let backup = parent.join(INSTALL_BACKUP);
    let staging = parent.join(INSTALL_STAGING);
    recover_bundle(destination, &backup, &staging)?;
    if !inspect_bundle(destination, force)? {
        return Ok(());
    }
    fs::create_dir(&staging)
        .map_err(|e| error("XS3322", Phase::Publish, "could not stage bundled skill", e))?;
    let staged = (|| -> Result<(), ManagerError> {
        write_synced(&staging.join("SKILL.md"), BUNDLED_SKILL.as_bytes())?;
        write_synced(&staging.join(MARKER), &bundled_marker()?)?;
        sync_dir(&staging);
        if destination.exists() {
            fs::rename(destination, &backup).map_err(|e| {
                error(
                    "XS3322",
                    Phase::Publish,
                    "could not back up bundled skill",
                    e,
                )
            })?;
        }
        if let Err(e) = fs::rename(&staging, destination) {
            if backup.exists() {
                let _ = fs::rename(&backup, destination);
            }
            return Err(error(
                "XS3322",
                Phase::Publish,
                "could not publish bundled skill",
                e,
            ));
        }
        sync_dir(parent);
        if backup.exists() {
            fs::remove_dir_all(&backup).map_err(|e| {
                error(
                    "XS3322",
                    Phase::Publish,
                    "could not retire old bundled skill",
                    e,
                )
            })?;
        }
        Ok(())
    })();
    if staged.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    staged
}

fn recover_bundle(destination: &Path, backup: &Path, staging: &Path) -> Result<(), ManagerError> {
    if backup.exists() {
        inspect_bundle(backup, true)?;
        if destination.exists() {
            inspect_bundle(destination, true)?;
            fs::remove_dir_all(backup).map_err(|e| {
                error(
                    "XS3322",
                    Phase::Publish,
                    "could not retire install backup",
                    e,
                )
            })?;
        } else {
            fs::rename(backup, destination).map_err(|e| {
                error(
                    "XS3322",
                    Phase::Publish,
                    "could not recover install backup",
                    e,
                )
            })?;
        }
    }
    if staging.exists() {
        inspect_bundle(staging, true)?;
        fs::remove_dir_all(staging).map_err(|e| {
            error(
                "XS3322",
                Phase::Publish,
                "could not remove abandoned stage",
                e,
            )
        })?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ManagerError> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            error(
                "XS3322",
                Phase::Publish,
                "could not create staged skill file",
                e,
            )
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            error(
                "XS3322",
                Phase::Publish,
                "could not sync staged skill file",
                e,
            )
        })
}

fn sync_dir(path: &Path) {
    if let Ok(directory) = fs::File::open(path) {
        let _ = directory.sync_all();
    }
}

struct InstallExecutor {
    destination: PathBuf,
    changed: bool,
    force: bool,
    dry_run: bool,
}

impl WorkExecutor<mutation::MutationWork> for InstallExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }
    fn execute(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("manager_cancelled", "install-skill was cancelled")
                .into();
        }
        if self.dry_run || !self.changed {
            return ActionResult::success().into();
        }
        match publish_bundle(&self.destination, self.force) {
            Ok(()) => ActionResult::success().into(),
            Err(e) => ActionResult::failure(e.code(), e.message()).into(),
        }
    }
    fn record(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

/// Reconcile an existing exact lock with project-visible skill directories.
///
/// This never changes the manifest or lock. A missing copy is installed from its pinned
/// source; a divergent copy is a conflict rather than an invitation to erase user edits.
pub(crate) fn sync(
    request: &SyncSkillsRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    sync_attempt(request, services, settings, durability, context, 0, 0, 0)
}

fn sync_attempt(
    request: &SyncSkillsRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
    attempt_index: u8,
    installed_before: u64,
    removed_before: u64,
) -> OperationOutcome {
    let job = job(context, "sync-skills");
    let kind = OperationKind::SyncSkills;
    let attempt =
        PlanningAttemptId::new(format!("sync-skills-attempt-{}", attempt_index + 1)).unwrap();
    let mut recorder = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(v) => v,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    let repo = match recorder.step(step("locate"), PlanningStepKind::Locate, || {
        ProjectRepository::discover_with_faults(
            Discovery::Explicit(PathBuf::from(request.project.as_str())),
            durability.repository(),
        )
        .map_err(|e| error("XS3330", Phase::Discover, "could not load project", e))
    }) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    if let Err(f) = recorder.step(step("recover"), PlanningStepKind::Recover, || {
        repo.recover()
            .map_err(|e| error("XS3330", Phase::Snapshot, "could not recover project", e))
    }) {
        return recorder.unavailable(kind, &f);
    }
    let snapshot = match recorder.step(step("snapshot"), PlanningStepKind::Snapshot, || {
        repo.snapshot_workspace()
            .map_err(|e| error("XS3330", Phase::Snapshot, "could not snapshot project", e))
    }) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let work = match recorder.step(
        step("prepare-candidate"),
        PlanningStepKind::PrepareCandidate,
        || prepare_sync(&repo, &snapshot, request, services),
    ) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let identity = sync_identity(&snapshot, &work);
    let prepared = match recorder.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || mutation::plan(request.dry_run, &identity),
    ) {
        Ok(v) => v,
        Err(f) => return recorder.unavailable(kind, &f),
    };
    let sealed = match recorder.seal(prepared, &identity, PlanMode::Execute) {
        Ok(v) => v,
        Err(f) => return unavailable_for_failure(job, kind, &f),
    };
    let result = SyncSkillsResult {
        installed: installed_before + work.pending.len() as u64,
        updated: 0,
        removed: removed_before + work.stale.len() as u64,
        unchanged: work.unchanged.saturating_sub(installed_before),
        dry_run: request.dry_run,
    };
    let expected_manifest_digest = snapshot
        .manifests()
        .iter()
        .find(|m| m.path == Path::new(MANIFEST_FILE_NAME))
        .expect("validated snapshot has a root manifest")
        .digest
        .clone();
    let executor = SyncExecutor {
        repo,
        work,
        dry_run: request.dry_run,
        expected_manifest_digest,
        progress: Mutex::new((0, 0)),
    };
    let report = match orchestrator::run(sealed, &executor, context, settings) {
        Ok(v) => v,
        Err(_) => return unavailable(job, kind, context.is_cancelled()),
    };
    if report.superseded == Some(SupersedeReason::AuthoritativeRevisionChanged) {
        if attempt_index < RETRIES {
            let (installed, removed) = *executor.progress.lock().unwrap();
            return sync_attempt(
                request,
                services,
                settings,
                durability,
                context,
                attempt_index + 1,
                installed_before + installed,
                removed_before + removed,
            );
        }
        let exhausted = PlanningAttemptId::new("sync-skills-retry-exhausted").unwrap();
        let mut recorder = match PlanningRecorder::start(job.clone(), exhausted, context) {
            Ok(v) => v,
            Err(_) => return unavailable(job, kind, context.is_cancelled()),
        };
        let failure: Result<(), PlanningFailure> =
            recorder.step(step("retry-budget"), PlanningStepKind::ValidatePlan, || {
                Err(ManagerError::new(
                    "XS3333",
                    Phase::Publish,
                    "project kept changing while synchronizing skill projections",
                ))
            });
        return recorder.unavailable(kind, &failure.expect_err("retry budget fails"));
    }
    OperationOutcome {
        job,
        result: if report.root_failures == 0 && !report.cancelled && report.superseded.is_none() {
            OperationResult::SyncSkills(result)
        } else {
            OperationResult::Unavailable { kind }
        },
        totals: report.totals,
        root_failures: report.root_failures,
        cancelled: report.cancelled,
    }
}

struct SyncWork {
    pending: Vec<(String, String, BTreeMap<PathBuf, Vec<u8>>)>,
    stale: Vec<(String, String)>,
    unchanged: u64,
}

fn prepare_sync(
    repo: &ProjectRepository,
    snapshot: &ProjectSnapshot,
    request: &SyncSkillsRequest,
    services: &dyn Services,
) -> Result<SyncWork, ManagerError> {
    let manifests: BTreeMap<String, Manifest> = snapshot
        .manifests()
        .iter()
        .map(|item| {
            (
                item.path.to_string_lossy().replace('\\', "/"),
                item.manifest.clone(),
            )
        })
        .collect();
    validate_skill_lock(&manifests, snapshot.lockfile())?;
    let manifest = &snapshot
        .manifests()
        .iter()
        .find(|m| m.path == Path::new(MANIFEST_FILE_NAME))
        .ok_or_else(|| {
            ManagerError::new(
                "XS3331",
                Phase::Manage,
                "workspace root manifest is missing",
            )
        })?
        .manifest;
    let owned = repo.managed_skill_names().map_err(|e| {
        error(
            "XS3332",
            Phase::Manage,
            "could not inspect managed skills",
            e,
        )
    })?;
    let stale: Vec<_> = owned
        .into_iter()
        .filter(|(name, _)| !manifest.skills.contains_key(name))
        .collect();
    let Some(lock) = snapshot.lockfile() else {
        if manifest.skills.is_empty() {
            return Ok(SyncWork {
                pending: Vec::new(),
                stale,
                unchanged: 0,
            });
        }
        return Err(ManagerError::new(
            "XS3331",
            Phase::Resolve,
            "project declares skills but has no xmlsquish.lock",
        ));
    };
    if lock.manifest_digest != snapshot.manifest_digest() {
        return Err(ManagerError::new(
            "XS3331",
            Phase::Resolve,
            "xmlsquish.lock does not match the current manifest set",
        ));
    }
    if manifest.skills.len() != lock.skills.len() {
        return Err(ManagerError::new(
            "XS3331",
            Phase::Resolve,
            "skill declarations and lock entries differ",
        ));
    }
    let mut pending = Vec::new();
    let mut unchanged = 0;
    for (name, spec) in &manifest.skills {
        let locked = lock
            .skills
            .iter()
            .find(|item| &item.name == name)
            .ok_or_else(|| {
                ManagerError::new(
                    "XS3331",
                    Phase::Resolve,
                    format!("skill `{name}` has no lock entry"),
                )
            })?;
        if !lock_matches_spec(locked, spec) {
            return Err(ManagerError::new(
                "XS3331",
                Phase::Resolve,
                format!("skill `{name}` source differs from lock"),
            ));
        }
        match repo
            .skill_directory_digest(name)
            .map_err(|e| error("XS3332", Phase::Manage, "unsafe installed skill", e))?
        {
            Some(actual) if actual == locked.digest => {
                unchanged += 1;
                continue;
            }
            Some(_) => {
                return Err(ManagerError::new(
                    "XS3332",
                    Phase::Manage,
                    format!("installed skill `{name}` differs from its locked content"),
                ));
            }
            None => {}
        }
        let source = source_from_spec(spec)?;
        let pinned = match &locked.source {
            LockedSkillSource::Git { revision, .. } => Some(revision.as_str()),
            _ => None,
        };
        let resolved = services
            .resolve_skill_source(
                snapshot.root(),
                &source,
                pinned,
                resolution_mode(request.lock),
            )
            .map_err(|e| error(e.code(), Phase::Resolve, "locked skill fetch failed", &e))?;
        validate_tree(name, &resolved)?;
        let digest = skill_tree_digest(&resolved.files)
            .map_err(|e| error("XS3332", Phase::Resolve, "invalid locked skill tree", e))?;
        if digest != locked.digest {
            return Err(ManagerError::new(
                "XS3332",
                Phase::Resolve,
                format!("source for skill `{name}` differs from the exact lock"),
            ));
        }
        let mut files = resolved.files;
        files.insert(PathBuf::from(MARKER), skill_marker(name, &digest));
        pending.push((name.clone(), locked.digest.clone(), files));
    }
    Ok(SyncWork {
        pending,
        stale,
        unchanged,
    })
}

fn lock_matches_spec(locked: &LockedSkill, spec: &SkillSpec) -> bool {
    match (&locked.source, &spec.path, &spec.git) {
        (
            LockedSkillSource::Path {
                path,
                mutable: true,
            },
            Some(want),
            None,
        ) => path == want,
        (
            LockedSkillSource::Git {
                repository,
                subdir,
                reference,
                ..
            },
            None,
            Some(want),
        ) => repository == want && subdir == &spec.subdir && reference == &spec.git_reference,
        _ => false,
    }
}

/// Ensure the skill lock represents the current root declarations exactly.
///
/// XML package resolution is independent of skill source resolution; this guard prevents a
/// package add/remove/build from silently carrying stale skill pins after a hand-edited manifest.
pub(crate) fn validate_skill_lock(
    manifests: &BTreeMap<String, Manifest>,
    lock: Option<&Lockfile>,
) -> Result<(), ManagerError> {
    let root = manifests.get(MANIFEST_FILE_NAME).ok_or_else(|| {
        ManagerError::new(
            "XS3331",
            Phase::Manage,
            "workspace root manifest is missing",
        )
    })?;
    if manifests
        .iter()
        .any(|(path, manifest)| path != MANIFEST_FILE_NAME && !manifest.skills.is_empty())
    {
        return Err(ManagerError::new(
            "XS3331",
            Phase::Manage,
            "skills may only be declared in the workspace root manifest",
        ));
    }
    let empty = [];
    let locked = lock.map_or(empty.as_slice(), |lock| lock.skills.as_slice());
    if root.skills.len() != locked.len()
        || root.skills.iter().any(|(name, spec)| {
            !locked
                .iter()
                .any(|item| &item.name == name && lock_matches_spec(item, spec))
        })
    {
        return Err(ManagerError::new(
            "XS3331",
            Phase::Manage,
            "[skills] and xmlsquish.lock disagree; use add-skill/remove-skill or restore the manifest and lock together",
        ));
    }
    Ok(())
}

/// Package resolution must preserve exact skill pins; only skill commands may change them.
pub(crate) fn validate_preserved_skill_lock(
    prior: Option<&Lockfile>,
    resolved: &Lockfile,
) -> Result<(), ManagerError> {
    let empty = [];
    let expected = prior.map_or(empty.as_slice(), |lock| lock.skills.as_slice());
    if resolved.skills != expected {
        return Err(ManagerError::new(
            "XS3331",
            Phase::Resolve,
            "package resolver changed exact skill pins; use add-skill to refresh a skill",
        ));
    }
    Ok(())
}

fn source_from_spec(spec: &SkillSpec) -> Result<SkillSource, ManagerError> {
    fn value<T>(result: Result<T, impl std::fmt::Display>) -> Result<T, ManagerError> {
        result.map_err(|e| error("XS3331", Phase::Manage, "invalid locked skill source", e))
    }
    if let Some(path) = &spec.path {
        return Ok(SkillSource::Path {
            path: value(squish_protocol::ProjectPath::new(
                path.to_string_lossy().into_owned(),
            ))?,
        });
    }
    let repository = value(squish_protocol::RepositoryUrl::new(
        spec.git.clone().unwrap_or_default(),
    ))?;
    let reference = if let Some(rev) = &spec.git_reference.rev {
        GitReference::Revision(value(squish_protocol::GitRevision::new(rev.clone()))?)
    } else if let Some(tag) = &spec.git_reference.tag {
        GitReference::Tag(value(squish_protocol::GitTag::new(tag.clone()))?)
    } else if let Some(branch) = &spec.git_reference.branch {
        GitReference::Branch(value(squish_protocol::GitBranch::new(branch.clone()))?)
    } else {
        GitReference::Head
    };
    let subdir = spec
        .subdir
        .as_ref()
        .map(|path| {
            value(squish_protocol::ProjectPath::new(
                path.to_string_lossy().into_owned(),
            ))
        })
        .transpose()?;
    Ok(SkillSource::Git {
        repository,
        reference,
        subdir,
    })
}

fn sync_identity(snapshot: &ProjectSnapshot, work: &SyncWork) -> Digest {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish\0sync-skills\0v1\0");
    hash.update(snapshot.manifest_digest().as_bytes());
    hash.update(snapshot.lock_digest().as_bytes());
    for (name, digest, _) in &work.pending {
        hash.update(name.as_bytes());
        hash.update(digest.as_bytes());
    }
    for (name, digest) in &work.stale {
        hash.update(name.as_bytes());
        hash.update(digest.as_bytes());
    }
    Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec()).unwrap()
}

struct SyncExecutor {
    repo: ProjectRepository,
    work: SyncWork,
    dry_run: bool,
    expected_manifest_digest: String,
    progress: Mutex<(u64, u64)>,
}

impl WorkExecutor<mutation::MutationWork> for SyncExecutor {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }
    fn execute(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &PreparedPlan<mutation::MutationWork>,
        _: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("manager_cancelled", "sync-skills was cancelled").into();
        }
        if self.dry_run {
            return ActionResult::success().into();
        }
        for (name, digest, files) in &self.work.pending {
            match self
                .repo
                .sync_skill(name, digest, &self.expected_manifest_digest, files)
            {
                Ok(()) => self.progress.lock().unwrap().0 += 1,
                Err(RepositoryError::Contended(_)) => {
                    return WorkDisposition::Superseded {
                        reason: SupersedeReason::AuthoritativeRevisionChanged,
                        events: Vec::new(),
                    };
                }
                Err(e) => {
                    return ActionResult::failure("XS3333", format!("skill sync failed: {e}"))
                        .into();
                }
            }
        }
        for (name, digest) in &self.work.stale {
            match self.repo.prune_skill(name, digest) {
                Ok(()) => self.progress.lock().unwrap().1 += 1,
                Err(RepositoryError::Contended(_)) => {
                    return WorkDisposition::Superseded {
                        reason: SupersedeReason::AuthoritativeRevisionChanged,
                        events: Vec::new(),
                    };
                }
                Err(e) => {
                    return ActionResult::failure(
                        "XS3333",
                        format!("stale skill prune failed: {e}"),
                    )
                    .into();
                }
            }
        }
        ActionResult::success().into()
    }
    fn record(
        &self,
        _: &Dispatch,
        _: &mutation::MutationWork,
        _: &ActionResult,
    ) -> Result<(), ManagerError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::validate_frontmatter;

    #[test]
    fn yaml_frontmatter_accepts_comments_and_block_scalars() {
        validate_frontmatter("review", b"---\nname: review # comment\ndescription: >\n  Review XML\n  safely.\nlicense: MIT\n---\n# Guide\n").unwrap();
    }

    #[test]
    fn yaml_frontmatter_rejects_wrong_types_duplicates_and_large_values() {
        for source in [
            "---\nname: review\ndescription: [not, a, string]\n---\n",
            "---\nname: review\nname: review\ndescription: valid\n---\n",
            "---\nname: review\ndescription: valid\ndescription: duplicate\n---\n",
            "---\nname: other\ndescription: valid\n---\n",
        ] {
            assert!(
                validate_frontmatter("review", source.as_bytes()).is_err(),
                "{source}"
            );
        }
        let large = format!(
            "---\nname: review\ndescription: {}\n---\n",
            "x".repeat(1025)
        );
        assert!(validate_frontmatter("review", large.as_bytes()).is_err());
    }
}
