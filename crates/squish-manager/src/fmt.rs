//! 格式化操作的冻结选择、语义预检与事务提交。 / Frozen selection, semantic preflight, and transactional commit for formatting.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use squish_build::{
    Action, ActionEvent, ActionResult, BuildPlan, Dispatch, InputRef, KeyRecipe, ResourceClass,
    Resources,
};
use squish_format::{StyleEdition as FormatStyle, format};
use squish_ir::RelocatableUnitIr;
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_protocol::{
    ActionId, ActionKind, ActionTotals, Artifact, ArtifactId, ArtifactKind, Digest,
    DigestAlgorithm, FormatRequest, FormatResult, FormatSelection, JobId, OpaqueSourceId,
    OperationKind, OperationResult, Phase, PlanMode, PlanningAttemptId, PlanningStepId,
    PlanningStepKind, WorkspaceScope,
};
use squish_repository::{
    Discovery, FormatUpdate, PackageLocation, ProjectRepository, ProjectSnapshot, ProjectSource,
};
use squish_source::{
    FileSourceProvider, SnapshotBuilder, SourceBlob, SourceIdentity, SourceProvider,
};
use squish_xml_front::{FrontendSourceContext, compile};

use crate::{
    DurabilityPorts, Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan, Services,
    StorageLayout,
    orchestrator::{
        self, CachedResult, PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
    },
};

/// 格式化动作的类型化领域工作。 / Typed domain work for a formatting action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatWork {
    /// 纯格式化并执行语义等价预检。 / Pure formatting with a semantic-equivalence preflight.
    Format {
        /// 待格式化的稳定逻辑源码身份。 / Stable logical identity of the source to format.
        source: OpaqueSourceId,
    },
    /// 汇总结果，并可选地以一个可恢复事务提交全部候选。 / Aggregate results and optionally commit every candidate in one recoverable transaction.
    CommitTransaction {
        /// 是否写回源码；check 模式为 false。 / Whether source files are written; false in check mode.
        write: bool,
    },
}

impl PlannedWork for FormatWork {
    fn kind(&self) -> ActionKind {
        match self {
            Self::Format { .. } => ActionKind::Format,
            Self::CommitTransaction { .. } => ActionKind::CommitTransaction,
        }
    }

    fn effect(&self) -> Effect {
        match self {
            Self::Format { .. } => Effect::Transform,
            Self::CommitTransaction { write: true } => Effect::WriteEffect,
            Self::CommitTransaction { write: false } => Effect::Coordination,
        }
    }
}

#[derive(Clone, Debug)]
struct FrozenFormatSource {
    source: ProjectSource,
    blob: Arc<SourceBlob>,
}

#[derive(Clone, Debug, Default)]
struct BatchState {
    changed: BTreeSet<OpaqueSourceId>,
    updates: BTreeMap<std::path::PathBuf, FormatUpdate>,
    diffs: BTreeMap<OpaqueSourceId, Vec<u8>>,
    artifacts: Vec<Artifact>,
}

/// 仅冻结选择与源码字节的格式化批次；候选由 Format worker 产生。 / Formatting batch containing only frozen selection and bytes; Format workers produce candidates.
#[derive(Clone, Debug)]
pub struct FormatBatch {
    selected: Vec<OpaqueSourceId>,
    sources: BTreeMap<OpaqueSourceId, FrozenFormatSource>,
    state: Arc<Mutex<BatchState>>,
    cas_root: PathBuf,
    style: FormatStyle,
    check: bool,
    diff: bool,
}

impl FormatBatch {
    /// 返回最终聚合动作形成的稳定协议结果。 / Returns the stable protocol result formed by the final aggregation action.
    pub fn result(&self) -> FormatResult {
        let state = self.state.lock().expect("format state poisoned");
        FormatResult {
            selected: self.selected.clone(),
            changed: state.changed.iter().cloned().collect(),
            check: self.check,
            diffs: state.artifacts.clone(),
        }
    }

    fn format_source(
        &self,
        id: &OpaqueSourceId,
        repository: &ProjectRepository,
    ) -> Result<(), ManagerError> {
        let frozen = self
            .sources
            .get(id)
            .expect("plan only names frozen sources");
        let bytes = candidate(&frozen.blob, &frozen.source, self.style)?;
        if bytes == frozen.blob.bytes() {
            return Ok(());
        }
        let path = frozen
            .source
            .locator
            .as_path()
            .strip_prefix(repository.root())
            .map_err(|_| {
                ManagerError::new(
                    "XS3106",
                    Phase::Manage,
                    format!(
                        "selected source `{}` is outside the project root",
                        frozen.source.id
                    ),
                )
            })?
            .to_path_buf();
        let expected_digest = ProjectRepository::candidate_digest(&path, frozen.blob.bytes())
            .map_err(|error| {
                manager_error(
                    "XS3106",
                    Phase::Manage,
                    "invalid format transaction path",
                    error,
                )
            })?;
        let diff = self
            .diff
            .then(|| machine_diff(id, frozen.blob.bytes(), &bytes))
            .transpose()?;
        let mut state = self.state.lock().expect("format state poisoned");
        state.changed.insert(id.clone());
        state.updates.insert(
            path.clone(),
            FormatUpdate {
                path,
                expected_digest,
                bytes,
            },
        );
        if let Some(diff) = diff {
            state.diffs.insert(id.clone(), diff);
        }
        Ok(())
    }

    fn finalize(&self, repository: &ProjectRepository) -> Result<(), ManagerError> {
        let (diffs, updates) = {
            let state = self.state.lock().expect("format state poisoned");
            (
                state.diffs.clone(),
                state.updates.values().cloned().collect::<Vec<_>>(),
            )
        };
        let artifacts = store_diffs(&self.cas_root, &diffs)?;
        if !self.check && !updates.is_empty() {
            repository
                .write_formatted_files(&updates)
                .map_err(|error| {
                    manager_error("XS3108", Phase::Publish, "format transaction failed", error)
                })?;
        }
        self.state.lock().expect("format state poisoned").artifacts = artifacts;
        Ok(())
    }

    /// 构造 Format* → CommitTransaction 的类型化封闭计划。 / Builds a typed closed Format* → CommitTransaction plan.
    pub fn plan(&self) -> Result<PreparedPlan<FormatWork>, ManagerError> {
        let mut actions = Vec::new();
        let mut work = BTreeMap::new();
        let mut formats = Vec::new();
        for (index, source) in self.selected.iter().enumerate() {
            let id = action_id(&format!("format.10.source.{index:08}"))?;
            let frozen = &self.sources[source];
            let digest = frozen.blob.digest().to_protocol();
            actions.push(action(
                id.clone(),
                ActionKind::Format,
                Vec::new(),
                vec![InputRef::Blob(digest)],
                BTreeMap::from([
                    ("diff".into(), self.diff.to_string()),
                    ("source".into(), source.as_str().to_owned()),
                    ("style".into(), self.style.to_string()),
                ]),
            )?);
            work.insert(
                id.clone(),
                FormatWork::Format {
                    source: source.clone(),
                },
            );
            formats.push(id);
        }
        let commit = action_id("format.20.commit")?;
        let dependencies = formats;
        actions.push(action(
            commit.clone(),
            ActionKind::CommitTransaction,
            dependencies,
            Vec::new(),
            BTreeMap::from([
                ("check".into(), self.check.to_string()),
                ("diff".into(), self.diff.to_string()),
                ("write".into(), (!self.check).to_string()),
            ]),
        )?);
        work.insert(commit, FormatWork::CommitTransaction { write: !self.check });
        let graph = BuildPlan::new(actions).map_err(|error| {
            manager_error("XS3109", Phase::Manage, "invalid format plan", error)
        })?;
        PreparedPlan::new(graph, work).map_err(|error| {
            manager_error(
                "XS3110",
                Phase::Manage,
                "invalid format work mapping",
                error,
            )
        })
    }
}

/// 只冻结选择及其源码字节；不提前执行 Format worker。 / Freezes only the selection and source bytes without executing Format workers early.
pub fn prepare(
    request: &FormatRequest,
    snapshot: &ProjectSnapshot,
    locations: &[PackageLocation],
    storage: &StorageLayout,
) -> Result<FormatBatch, ManagerError> {
    let style = style(request)?;
    let _ = locations;
    let owned = snapshot.owned_workspace_sources().map_err(|error| {
        manager_error(
            "XS3100",
            Phase::Snapshot,
            "could not enumerate workspace-owned sources",
            error,
        )
    })?;
    let eligible = eligible_sources(request, snapshot, owned, &BTreeSet::new())?;
    let selected = select_sources(&request.selection, snapshot.root(), eligible)?;
    prepare_selected(
        request.check,
        request.diff,
        style,
        snapshot.root(),
        selected,
        storage.cas_root().to_path_buf(),
    )
}

pub(crate) fn execute_with_durability(
    request: &FormatRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = JobId::new(format!("format-{}", context.id())).expect("invocation IDs are non-empty");
    let attempt = PlanningAttemptId::new("attempt-1").expect("static planning attempt ID");
    let mut planning = match PlanningRecorder::start(job.clone(), attempt, context) {
        Ok(planning) => planning,
        Err(_) => return unavailable(job, context, true),
    };
    let repository = match planning.step(step("locate"), PlanningStepKind::Locate, || {
        ProjectRepository::discover_with_faults(
            Discovery::Explicit(request.project.as_str().into()),
            durability.repository(),
        )
        .map_err(|error| {
            manager_error(
                "XS3000",
                Phase::Discover,
                "could not discover project",
                error,
            )
        })
    }) {
        Ok(repository) => repository,
        Err(failure) => return planning.unavailable(OperationKind::Format, &failure),
    };
    let layout = match planning.step(step("storage-layout"), PlanningStepKind::Locate, || {
        services
            .storage_layout(repository.root())
            .map_err(|error| ManagerError::new(error.code(), Phase::Cache, error.message()))
    }) {
        Ok(layout) => layout,
        Err(failure) => return planning.unavailable(OperationKind::Format, &failure),
    };
    let snapshot = match planning.step(
        step("snapshot-workspace"),
        PlanningStepKind::Snapshot,
        || {
            repository.snapshot_workspace().map_err(|error| {
                manager_error(
                    "XS3001",
                    Phase::Snapshot,
                    "could not freeze workspace snapshot",
                    error,
                )
            })
        },
    ) {
        Ok(snapshot) => snapshot,
        Err(failure) => return planning.unavailable(OperationKind::Format, &failure),
    };
    let excluded: BTreeSet<_> = settings
        .excluded_packages
        .iter()
        .map(|package| package.as_str())
        .collect();
    let batch = match planning.step(step("select-and-freeze"), PlanningStepKind::Scan, || {
        let style = style(request)?;
        let owned = snapshot.owned_workspace_sources().map_err(|error| {
            manager_error(
                "XS3100",
                Phase::Snapshot,
                "could not enumerate workspace-owned sources",
                error,
            )
        })?;
        let eligible = eligible_sources(request, &snapshot, owned, &excluded)?;
        let selected = select_sources(&request.selection, snapshot.root(), eligible)?;
        prepare_selected(
            request.check,
            request.diff,
            style,
            snapshot.root(),
            selected,
            layout.cas_root().to_path_buf(),
        )
    }) {
        Ok(batch) => batch,
        Err(failure) => return planning.unavailable(OperationKind::Format, &failure),
    };
    let plan = match planning.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || batch.plan(),
    ) {
        Ok(plan) => plan,
        Err(failure) => return planning.unavailable(OperationKind::Format, &failure),
    };
    let snapshot_digest = workspace_snapshot_digest(&snapshot);
    let sealed = match planning.seal(plan, &snapshot_digest, PlanMode::Execute) {
        Ok(sealed) => sealed,
        Err(failure) => {
            return unavailable(
                job,
                context,
                matches!(failure, orchestrator::PlanningFailure::Failed(_)),
            );
        }
    };
    let executor = FormatExecutor {
        repository: Some(&repository),
        batch: Some(&batch),
    };
    match orchestrator::run(sealed, &executor, context, settings) {
        Ok(report) => OperationOutcome {
            job,
            result: if report.totals.failed == 0 && report.superseded.is_none() {
                OperationResult::Format(batch.result())
            } else {
                OperationResult::Unavailable {
                    kind: OperationKind::Format,
                }
            },
            totals: report.totals,
            root_failures: report.root_failures,
            cancelled: report.cancelled,
        },
        Err(_) => OperationOutcome {
            job,
            result: OperationResult::Unavailable {
                kind: OperationKind::Format,
            },
            totals: ActionTotals::default(),
            root_failures: 0,
            cancelled: context.is_cancelled(),
        },
    }
}

fn step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("static planning step IDs are non-empty")
}

fn workspace_snapshot_digest(snapshot: &ProjectSnapshot) -> Digest {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish-format-workspace-snapshot\0v1");
    for (path, digest) in snapshot.read_set() {
        let path = path.to_string_lossy().replace('\\', "/");
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(&(digest.len() as u64).to_le_bytes());
        hash.update(digest.as_bytes());
    }
    Digest::new(DigestAlgorithm::Blake3, hash.finalize().as_bytes().to_vec())
        .expect("BLAKE3 digest length is canonical")
}

fn unavailable(job: JobId, context: &InvocationContext, failed: bool) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::Format,
        },
        totals: ActionTotals::default(),
        root_failures: u64::from(failed),
        cancelled: context.is_cancelled(),
    }
}

struct FormatExecutor<'a> {
    repository: Option<&'a ProjectRepository>,
    batch: Option<&'a FormatBatch>,
}

impl WorkExecutor<FormatWork> for FormatExecutor<'_> {
    fn lookup(
        &self,
        _: &Dispatch,
        _: &FormatWork,
        _: &PreparedPlan<FormatWork>,
        _inputs: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        Ok(None)
    }
    fn execute(
        &self,
        _: &Dispatch,
        work: &FormatWork,
        _: &PreparedPlan<FormatWork>,
        _inputs: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("cancelled", "format operation was cancelled").into();
        }
        let result = match work {
            FormatWork::Format { source } => self
                .batch
                .expect("format executor has a batch")
                .format_source(
                    source,
                    self.repository.expect("format executor has a repository"),
                )
                .map_or_else(
                    |error| ActionResult::failure(error.code(), error.message()),
                    |_| ActionResult::success(),
                ),
            FormatWork::CommitTransaction { .. } => self
                .batch
                .expect("commit executor has a batch")
                .finalize(self.repository.expect("commit executor has a repository"))
                .map_or_else(
                    |error| ActionResult::failure(error.code(), error.message()),
                    |_| {
                        let batch = self.batch.expect("commit executor has a batch");
                        let mut result = ActionResult::success();
                        result.events = batch
                            .state
                            .lock()
                            .expect("format state poisoned")
                            .artifacts
                            .iter()
                            .cloned()
                            .map(ActionEvent::Artifact)
                            .collect();
                        result
                    },
                ),
        };
        result.into()
    }
    fn record(&self, _: &Dispatch, _: &FormatWork, _: &ActionResult) -> Result<(), ManagerError> {
        Ok(())
    }
}

fn prepare_selected(
    check: bool,
    diff: bool,
    style: FormatStyle,
    _root: &Path,
    selected: Vec<ProjectSource>,
    cas_root: PathBuf,
) -> Result<FormatBatch, ManagerError> {
    let mut builder = SnapshotBuilder::new(FileSourceProvider);
    let mut loaded = BTreeMap::new();
    for source in &selected {
        let blob = builder
            .load(source.id.clone(), source.locator.clone())
            .map_err(|error| {
                manager_error(
                    "XS3103",
                    Phase::Snapshot,
                    "could not freeze format source",
                    error,
                )
            })?;
        loaded.insert(source.id.to_protocol(), blob);
    }
    let frozen = builder.seal().map_err(|error| {
        manager_error(
            "XS3103",
            Phase::Snapshot,
            "could not seal format snapshot",
            error,
        )
    })?;
    let mut ids = Vec::with_capacity(selected.len());
    let mut sources = BTreeMap::new();
    for source in selected {
        let id = source.id.to_protocol();
        ids.push(id.clone());
        let blob = loaded
            .remove(&id)
            .expect("all selected sources were loaded");
        debug_assert!(frozen.get(&source.id).is_some());
        sources.insert(id, FrozenFormatSource { source, blob });
    }
    Ok(FormatBatch {
        selected: ids,
        sources,
        state: Arc::new(Mutex::new(BatchState::default())),
        cas_root,
        style,
        check,
        diff,
    })
}

fn candidate(
    blob: &SourceBlob,
    source: &ProjectSource,
    style: FormatStyle,
) -> Result<Vec<u8>, ManagerError> {
    std::str::from_utf8(blob.bytes()).map_err(|error| {
        manager_error("XS3114", Phase::Format, "format source is not UTF-8", error)
    })?;
    let plan = format(blob.bytes(), style).map_err(|error| {
        manager_error(
            "XS3104",
            Phase::Analyze,
            "source is not lossless XML",
            error,
        )
    })?;
    let bytes = plan.apply(blob.bytes()).map_err(|error| {
        manager_error(
            "XS3105",
            Phase::Analyze,
            "format plan could not be applied",
            error,
        )
    })?;
    let context = FrontendSourceContext::new(source.package.clone());
    let before = compile(blob, &context).map_err(|error| {
        manager_error(
            "XS3107",
            Phase::Analyze,
            "source failed semantic preflight",
            error.message.clone(),
        )
    })?;
    let after_blob = candidate_blob(source, &bytes)?;
    let after = compile(&after_blob, &context).map_err(|error| {
        manager_error(
            "XS3107",
            Phase::Analyze,
            "formatted candidate failed semantic preflight",
            error.message.clone(),
        )
    })?;
    if !same_semantics(&before.unit, &after.unit) {
        return Err(ManagerError::new(
            "XS3111",
            Phase::Analyze,
            format!("formatting `{}` changed its semantic IR", source.id),
        ));
    }
    Ok(bytes)
}

struct BytesProvider<'a>(&'a [u8]);
impl SourceProvider for BytesProvider<'_> {
    fn read(&self, _locator: &squish_source::SourceLocator) -> io::Result<Vec<u8>> {
        Ok(self.0.to_vec())
    }
}

fn candidate_blob(
    source: &ProjectSource,
    bytes: &[u8],
) -> Result<std::sync::Arc<SourceBlob>, ManagerError> {
    let mut builder = SnapshotBuilder::new(BytesProvider(bytes));
    builder
        .load(source.id.clone(), source.locator.clone())
        .map_err(|error| {
            manager_error(
                "XS3107",
                Phase::Analyze,
                "could not freeze formatted candidate",
                error,
            )
        })
}

fn same_semantics(left: &RelocatableUnitIr, right: &RelocatableUnitIr) -> bool {
    match (left, right) {
        (RelocatableUnitIr::Module(a), RelocatableUnitIr::Module(b)) => {
            a.header == b.header
                && a.definitions == b.definitions
                && a.external_symbols == b.external_symbols
                && a.interface == b.interface
                && a.regions == b.regions
                && a.ops == b.ops
                && canonical_origins(&a.origins) == canonical_origins(&b.origins)
                && a.producer == b.producer
        }
        (RelocatableUnitIr::Entry(a), RelocatableUnitIr::Entry(b)) => {
            a.header == b.header
                && a.required_params == b.required_params
                && a.root_region == b.root_region
                && a.external_symbols == b.external_symbols
                && a.regions == b.regions
                && a.ops == b.ops
                && canonical_origins(&a.origins) == canonical_origins(&b.origins)
                && a.producer == b.producer
        }
        _ => false,
    }
}

fn canonical_origins(value: &squish_ir::OriginTable) -> squish_ir::OriginTable {
    let mut value = value.clone();
    for entry in &mut value.entries {
        entry.origin.span = squish_ir::Span { start: 0, end: 0 };
    }
    for map in &mut value.decoded_values {
        for segment in &mut map.segments {
            segment.source_span = squish_ir::Span { start: 0, end: 0 };
        }
    }
    value
}

fn machine_diff(
    source: &OpaqueSourceId,
    original: &[u8],
    formatted: &[u8],
) -> Result<Vec<u8>, ManagerError> {
    if original == formatted {
        return Ok(Vec::new());
    }
    let original = std::str::from_utf8(original).map_err(|error| {
        manager_error("XS3114", Phase::Format, "format source is not UTF-8", error)
    })?;
    let formatted = std::str::from_utf8(formatted).map_err(|error| {
        manager_error(
            "XS3114",
            Phase::Format,
            "formatted source is not UTF-8",
            error,
        )
    })?;
    let old = diff_lines(original);
    let new = diff_lines(formatted);
    let mut output = format!(
        "--- a/{0}\n+++ b/{0}\n@@ -{1},{2} +{3},{4} @@\n",
        source.as_str(),
        range_start(old.len()),
        old.len(),
        range_start(new.len()),
        new.len(),
    )
    .into_bytes();
    append_diff_lines(&mut output, b'-', &old);
    append_diff_lines(&mut output, b'+', &new);
    Ok(output)
}

#[derive(Clone, Copy)]
struct DiffLine<'a> {
    text: &'a str,
    terminated: bool,
}

fn diff_lines(source: &str) -> Vec<DiffLine<'_>> {
    if source.is_empty() {
        return Vec::new();
    }
    source
        .split_inclusive('\n')
        .map(|line| {
            let terminated = line.ends_with('\n');
            let line = line.strip_suffix('\n').unwrap_or(line);
            let text = if terminated {
                line.strip_suffix('\r').unwrap_or(line)
            } else {
                line
            };
            DiffLine { text, terminated }
        })
        .collect()
}

fn range_start(lines: usize) -> usize {
    usize::from(lines != 0)
}

fn append_diff_lines(output: &mut Vec<u8>, prefix: u8, lines: &[DiffLine<'_>]) {
    for line in lines {
        output.push(prefix);
        output.extend_from_slice(line.text.as_bytes());
        output.push(b'\n');
        if !line.terminated {
            output.extend_from_slice(b"\\ No newline at end of file\n");
        }
    }
}

fn store_diffs(
    cas_root: &Path,
    diffs: &BTreeMap<OpaqueSourceId, Vec<u8>>,
) -> Result<Vec<Artifact>, ManagerError> {
    if diffs.is_empty() {
        return Ok(Vec::new());
    }
    let cas = squish_store::Cas::open(cas_root).map_err(|error| {
        manager_error("XS3113", Phase::Cache, "could not open diff store", error)
    })?;
    diffs
        .iter()
        .map(|(source, bytes)| {
            let digest = cas.put(bytes).map_err(|error| {
                manager_error("XS3113", Phase::Cache, "could not store format diff", error)
            })?;
            Ok(Artifact {
                id: ArtifactId::new(format!("format-diff:{source}"))
                    .expect("source IDs are non-empty"),
                kind: ArtifactKind::Other("text/x-diff".into()),
                uri: format!("cas://blake3/{}", digest.to_hex()),
                size: bytes.len() as u64,
                digest: Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
                    .expect("BLAKE3 is non-empty"),
            })
        })
        .collect()
}

fn eligible_sources(
    request: &FormatRequest,
    snapshot: &ProjectSnapshot,
    owned: Vec<ProjectSource>,
    excluded: &BTreeSet<&str>,
) -> Result<Vec<ProjectSource>, ManagerError> {
    let available: BTreeSet<_> = snapshot
        .manifests()
        .iter()
        .filter_map(|item| {
            item.manifest
                .package
                .as_ref()
                .map(|package| package.name.as_str())
        })
        .collect();
    let selected: BTreeSet<&str> = match &request.scope {
        WorkspaceScope::Current => snapshot
            .manifests()
            .first()
            .and_then(|item| item.manifest.package.as_ref())
            .map(|package| package.name.as_str())
            .into_iter()
            .collect(),
        WorkspaceScope::Workspace => available.clone(),
        WorkspaceScope::Packages(packages) => {
            for package in packages {
                if !available.contains(package.as_str()) {
                    return Err(ManagerError::new(
                        "XS3101",
                        Phase::Manage,
                        format!("unknown workspace package `{package}`"),
                    ));
                }
            }
            packages.iter().map(|package| package.as_str()).collect()
        }
    };
    if matches!(request.scope, WorkspaceScope::Current) && selected.is_empty() {
        return Err(ManagerError::new(
            "XS3101",
            Phase::Manage,
            "current project is a virtual workspace without a package",
        ));
    }
    Ok(owned
        .into_iter()
        .filter(|source| {
            selected.contains(source.id.package().as_str())
                && !excluded.contains(source.id.package().as_str())
        })
        .collect())
}

fn select_sources(
    selection: &FormatSelection,
    root: &Path,
    mut eligible: Vec<ProjectSource>,
) -> Result<Vec<ProjectSource>, ManagerError> {
    eligible.sort_by(|a, b| a.id.cmp(&b.id));
    if matches!(selection, FormatSelection::All) {
        return Ok(eligible);
    }
    let FormatSelection::Sources(requested) = selection else {
        unreachable!()
    };
    let mut selected = BTreeSet::new();
    for opaque in requested {
        let matches: Vec<_> = eligible
            .iter()
            .enumerate()
            .filter(|(_, source)| matches_source(opaque, root, source))
            .map(|(index, _)| index)
            .collect();
        match matches.as_slice() {
            [index] => {
                selected.insert(*index);
            }
            [] => {
                return Err(ManagerError::new(
                    "XS3102",
                    Phase::Manage,
                    format!("selected source `{opaque}` is not owned by the selected packages"),
                ));
            }
            _ => {
                return Err(ManagerError::new(
                    "XS3102",
                    Phase::Manage,
                    format!("selected source `{opaque}` is ambiguous; use its xmlsquish URI"),
                ));
            }
        }
    }
    Ok(selected
        .into_iter()
        .map(|index| eligible[index].clone())
        .collect())
}

fn matches_source(opaque: &OpaqueSourceId, root: &Path, source: &ProjectSource) -> bool {
    if SourceIdentity::try_from_protocol(opaque).as_ref() == Ok(&source.id) {
        return true;
    }
    let requested = opaque.as_str().replace('\\', "/");
    if requested == source.id.path().as_str() {
        return true;
    }
    source
        .locator
        .as_path()
        .strip_prefix(root)
        .ok()
        .is_some_and(|path| path.to_string_lossy().replace('\\', "/") == requested)
}

fn style(request: &FormatRequest) -> Result<FormatStyle, ManagerError> {
    request
        .style
        .as_ref()
        .map_or(Ok(FormatStyle::default()), |style| {
            style.as_str().parse().map_err(|error| {
                manager_error(
                    "XS3112",
                    Phase::Manage,
                    "unsupported formatting style",
                    error,
                )
            })
        })
}

fn action_id(value: &str) -> Result<ActionId, ManagerError> {
    ActionId::new(value)
        .map_err(|error| manager_error("XS3109", Phase::Manage, "invalid format action ID", error))
}

fn action(
    id: ActionId,
    kind: ActionKind,
    dependencies: Vec<ActionId>,
    inputs: Vec<InputRef>,
    options: BTreeMap<String, String>,
) -> Result<Action, ManagerError> {
    let key = KeyRecipe::new("xmlsquish-format-v1", inputs, options).map_err(|error| {
        manager_error("XS3109", Phase::Manage, "invalid format action key", error)
    })?;
    Ok(Action {
        id,
        key,
        kind,
        class: if kind == ActionKind::Format {
            ResourceClass::Cpu
        } else {
            ResourceClass::Io
        },
        resources: if kind == ActionKind::Format {
            Resources::new(1, 0, 0)
        } else {
            Resources::new(0, 1, 0)
        },
        dependencies,
        outputs: Vec::new(),
    })
}

fn manager_error(
    code: &str,
    phase: Phase,
    context: &str,
    error: impl std::fmt::Display,
) -> ManagerError {
    ManagerError::new(code, phase, format!("{context}: {error}"))
}
