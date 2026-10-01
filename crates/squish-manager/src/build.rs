//! 构建用例的封闭计划与执行。 / Closed planning and execution for the build use case.

#[path = "archive_build.rs"]
mod archive_build;
#[path = "archive_reachability.rs"]
mod archive_reachability;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

use squish_backend::{BackendCacheIdentity, SquishOptions};
use squish_build::{
    Action, ActionEvent, ActionKind, ActionRecord, ActionResult, ArtifactDescriptor, ArtifactRead,
    BuildPlan, CommittedGeneration, Dispatch, GenerationArtifact, GenerationId, GenerationRef,
    InputRef, KeyRecipe, LogicalArtifactName, Output, OutputName, OutputRef, ProducedOutput,
    Publication, PublicationPath, PublicationTargetId, ResourceClass, Resources, ResultSource,
    VerifiedAction, VerifiedBlob,
};
use squish_ir::{
    ArtifactDigest, ArtifactIdentity, BundledSourceBlob, DebugBundle, DebugDigest, DocumentDigest,
    EntityKind, ImportBinding, ImportSpec, LinkImportRecord, LinkTrace, LinkedImageDigest,
    ObjectDigest, OriginId, QualifiedOriginRef, RelocatableUnitIr, ResolutionSnapshot,
    SourceArchiveReference, SourceKey, UnitKind, UnitRevision, ValidatedUnit,
    decode_expansion_trace, decode_linked_document, decode_linked_image, decode_static_link_map,
    decode_unit_container, encode_debug_bundle, encode_expansion_trace, encode_link_trace,
    encode_linked_document, encode_linked_image, encode_static_link_map,
};
use squish_link::{
    Budgets, InstantiateOutput, LinkKeyProjection, LinkOutput, PreparedUnit, PreparedUnitClosure,
};
use squish_project::{
    Lockfile, Manifest, MutationFile, MutationKind, MutationPlanner, ResolutionMode, TransactionId,
};
use squish_protocol::{
    ActionId, Artifact, ArtifactId, ArtifactKind, BuildRequest, BuildResult, DiagnosticId,
    DigestAlgorithm, EmitKind, FinalizationId, FinalizationKind, JobId, OperationKind,
    OperationResult, Phase, PlanMode, PlanningAttemptId, PlanningStepId, PlanningStepKind,
    PublishedArtifact, PublishedTarget, TargetName, WorkspaceScope,
};
use squish_repository::{Discovery, ProjectRepository, ProjectSnapshot, ResolvedTarget};
use squish_source::{FileSourceProvider, LogicalPath, SnapshotBuilder, SourceBlob};
use squish_xml_front::FrontendSourceContext;

use crate::{
    AcquiredSopack, BuildRuntime, BuildRuntimeDescriptor, BuildRuntimeError, BuildRuntimeErrorKind,
    DurabilityPorts, Effect, GenerationSpace, InvocationSettings, ManagerError, PlannedWork,
    PreparedPlan, ResolveRequest, Services,
    orchestrator::{
        self, ActionExecutionFact, CachedResult, ExecutionReport, ExecutionState, PlanningFailure,
        PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
    },
    skill::{validate_preserved_skill_lock, validate_skill_lock},
};
use serde::{Deserialize, Serialize};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};

/// 当前构建目录所使用的稳定 generation 身份。 / Stable generation identity used by the current-build catalog.
pub const BUILD_CATALOG_TARGET: &str = "xmlsquish-build-record";

/// 可持久、可公开解码且不含物理发布布局的构建记录。 / Persistable, publicly decodable build record without physical publication layout.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRecordV3 {
    /// 线格式版本。 / Wire-format version.
    pub schema: u32,
    /// Kernel 作业身份。 / Kernel job identity.
    pub job: JobId,
    /// 被执行的封闭计划及其规范身份。 / Closed plan and its canonical identity.
    pub plan: squish_protocol::PlanInspection,
    /// 按动作 ID 排序的真实终态。 / Actual terminal facts sorted by action ID.
    pub actions: Vec<BuildActionFact>,
    /// catalog 当前全部已发布 target generations；本次失败或未触及的 target 保留上一良好 generation。
    /// All target generations currently published in the catalog; failed or untouched targets
    /// retain their previous good generation.
    pub targets: Vec<RecordedGeneration>,
    /// 查找当前记录所需的显式 catalog target 身份。 / Explicit catalog target identity used to find the current record.
    pub catalog_target: PublicationTargetId,
}

/// 一次目标 generation 的不可变 catalog 投影。 / Immutable catalog projection of one target generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedGeneration {
    /// `package:target` 完整身份。 / Complete `package:target` identity.
    pub target_id: PublicationTargetId,
    /// 内容导出的不可变 generation 身份。 / Content-derived immutable generation identity.
    pub generation_id: GenerationId,
    /// 含逻辑目标路径的完整产物集合。 / Complete artifact set with logical destinations.
    pub artifacts: Vec<CatalogArtifact>,
}

/// catalog 中的一个产物及其用户逻辑目标。 / One catalog artifact and its user-facing logical destination.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogArtifact {
    /// 与物理位置无关的不可变产物描述符。 / Physical-location-independent immutable artifact descriptor.
    pub descriptor: ArtifactDescriptor,
    /// 规范、稳定的用户逻辑定位器。 / Canonical stable user logical locator.
    pub destination: PublicationPath,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRecordV1 {
    schema: u32,
    target: String,
    snapshot: String,
    artifacts: Vec<TargetRecordArtifactV1>,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct TargetRecordArtifactV1 {
    destination: String,
    kind: String,
    digest: String,
    size: u64,
}

/// 构建记录中的动作事实。 / Action fact stored in a build record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildActionFact {
    /// 动作身份。 / Action identity.
    pub action: ActionId,
    /// 动作类别。 / Action kind.
    pub kind: ActionKind,
    /// 规范直接前驱。 / Canonical direct prerequisites.
    pub dependencies: Vec<ActionId>,
    /// 真实终态。 / Actual terminal state.
    pub state: BuildTerminalState,
    /// 仅在物化成功后存在的动作键。 / Action key present only after materialization.
    pub key: Option<squish_protocol::ActionKeyId>,
    /// 成功动作的具名输出。 / Named outputs of a successful action.
    pub outputs: Vec<BuildOutputFact>,
    /// 成功输出来源。 / Source of successful outputs.
    pub source: Option<BuildResultSource>,
}

/// 构建记录中的具名输出。 / Named output stored in a build record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildOutputFact {
    /// 动作内输出名。 / Action-local output name.
    pub name: String,
    /// 产物类别。 / Artifact kind.
    pub kind: ArtifactKind,
    /// 内容摘要。 / Content digest.
    pub digest: squish_protocol::Digest,
    /// 内容字节数。 / Content size in bytes.
    pub size: u64,
}

/// 构建动作的真实终态。 / Actual terminal state of a build action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BuildTerminalState {
    /// 已声明但未执行。 / Declared but not executed.
    Declared,
    /// 成功。 / Succeeded.
    Succeeded,
    /// Worker 失败。 / Worker failed.
    Failed {
        /// 稳定机器错误码。 / Stable machine-readable error code.
        code: String,
        /// 面向用户的失败说明。 / User-facing failure message.
        message: String,
    },
    /// 被前驱阻塞。 / Blocked by a prerequisite.
    Blocked {
        /// 阻塞本动作的直接前驱。 / Direct prerequisite that blocked this action.
        dependency: ActionId,
    },
    /// 已取消。 / Cancelled.
    Cancelled,
    /// 因权威状态竞争而废弃。 / Superseded by an authoritative-state race.
    Superseded,
}

/// 成功动作结果的来源。 / Source of a successful action result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildResultSource {
    /// Worker 实际执行。 / Executed by a worker.
    Worker,
    /// 同次运行的 single-flight leader。 / Reused from a same-run single-flight leader.
    SingleFlight,
    /// 持久动作缓存。 / Persistent action cache.
    Cache,
}

/// Build record v2 编解码错误。 / Build-record v3 codec error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRecordCodecError(String);

impl fmt::Display for BuildRecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for BuildRecordCodecError {}

/// 规范编码 BuildRecord v3。 / Canonically encodes a BuildRecord v3.
pub fn encode_build_record(record: &BuildRecordV3) -> Result<Vec<u8>, BuildRecordCodecError> {
    validate_build_record(record)?;
    serde_json::to_vec(record).map_err(|error| BuildRecordCodecError(error.to_string()))
}

/// 解码并验证 BuildRecord v3。 / Decodes and validates a BuildRecord v3.
pub fn decode_build_record(bytes: &[u8]) -> Result<BuildRecordV3, BuildRecordCodecError> {
    let record: BuildRecordV3 =
        serde_json::from_slice(bytes).map_err(|error| BuildRecordCodecError(error.to_string()))?;
    validate_build_record(&record)?;
    Ok(record)
}

fn validate_build_record(record: &BuildRecordV3) -> Result<(), BuildRecordCodecError> {
    if record.schema != 3 {
        return Err(BuildRecordCodecError(format!(
            "unsupported build-record schema {}",
            record.schema
        )));
    }
    if record.job != record.plan.job {
        return Err(BuildRecordCodecError(
            "build-record job differs from its plan job".into(),
        ));
    }
    if record.catalog_target.as_str() != BUILD_CATALOG_TARGET {
        return Err(BuildRecordCodecError(
            "build-record catalog target is not canonical".into(),
        ));
    }
    let planned: BTreeSet<_> = record
        .plan
        .actions
        .iter()
        .map(|action| action.action.clone())
        .collect();
    let actual: BTreeSet<_> = record
        .actions
        .iter()
        .map(|action| action.action.clone())
        .collect();
    if planned != actual || actual.len() != record.actions.len() {
        return Err(BuildRecordCodecError(
            "build-record action facts do not exactly cover the sealed plan".into(),
        ));
    }
    for fact in &record.actions {
        let Some(planned) = record
            .plan
            .actions
            .iter()
            .find(|action| action.action == fact.action)
        else {
            return Err(BuildRecordCodecError(format!(
                "action fact `{}` is absent from the sealed plan",
                fact.action
            )));
        };
        if planned.kind != fact.kind
            || planned.dependencies != fact.dependencies
            || planned.action_key != fact.key
        {
            return Err(BuildRecordCodecError(format!(
                "action fact `{}` differs from its sealed-plan projection",
                fact.action
            )));
        }
        if !fact.dependencies.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(BuildRecordCodecError(format!(
                "action `{}` dependencies are not canonical",
                fact.action
            )));
        }
        match &fact.state {
            BuildTerminalState::Succeeded if fact.key.is_some() && fact.source.is_some() => {}
            BuildTerminalState::Succeeded => {
                return Err(BuildRecordCodecError(format!(
                    "successful action `{}` lacks a key or result source",
                    fact.action
                )));
            }
            _ if fact.source.is_none() && fact.outputs.is_empty() => {}
            _ => {
                return Err(BuildRecordCodecError(format!(
                    "non-successful action `{}` claims successful outputs",
                    fact.action
                )));
            }
        }
    }
    if !record
        .actions
        .windows(2)
        .all(|pair| pair[0].action < pair[1].action)
    {
        return Err(BuildRecordCodecError(
            "build-record actions are not canonically ordered".into(),
        ));
    }
    let mut artifact_ids = BTreeSet::new();
    let mut all_destinations = BTreeSet::new();
    let mut prior_target: Option<&str> = None;
    for target in &record.targets {
        if prior_target.is_some_and(|prior| prior >= target.target_id.as_str()) {
            return Err(BuildRecordCodecError(
                "target generations are not uniquely and canonically ordered".into(),
            ));
        }
        prior_target = Some(target.target_id.as_str());
        let maps = target
            .artifacts
            .iter()
            .filter(|item| {
                matches!(&item.descriptor.kind, ArtifactKind::Other(name) if name == "static-link-map")
            })
            .count();
        if maps != 1 {
            return Err(BuildRecordCodecError(format!(
                "target `{}` does not have exactly one static link map",
                target.target_id
            )));
        }
        if !target.artifacts.windows(2).all(|pair| {
            (&pair[0].destination, &pair[0].descriptor.id)
                < (&pair[1].destination, &pair[1].descriptor.id)
        }) {
            return Err(BuildRecordCodecError(format!(
                "artifacts for target `{}` are not canonically ordered",
                target.target_id
            )));
        }
        let mut destinations = BTreeSet::new();
        for item in &target.artifacts {
            if !artifact_ids.insert(item.descriptor.id.clone()) {
                return Err(BuildRecordCodecError(format!(
                    "artifact id `{}` is ambiguous across target generations",
                    item.descriptor.id
                )));
            }
            if !destinations.insert(item.destination.clone()) {
                return Err(BuildRecordCodecError(format!(
                    "logical destination `{}` is duplicated in target `{}`",
                    item.destination, target.target_id
                )));
            }
            if !all_destinations.insert(item.destination.clone()) {
                return Err(BuildRecordCodecError(format!(
                    "logical destination `{}` is ambiguous across targets",
                    item.destination
                )));
            }
        }
    }
    Ok(())
}

/// 持久 catalog 的读取或完整性错误。 / Persistent-catalog read or integrity error.
#[derive(Debug)]
pub enum BuildCatalogError {
    /// 存储适配器失败。 / Storage-adapter failure.
    Storage(String),
    /// current generation 存在但内容损坏。 / A current generation exists but is corrupt.
    Corrupt(String),
    /// 记录声明的 target current generation 已缺失。 / A target current generation declared by the record is missing.
    MissingCurrent {
        /// 完整 target 身份。 / Complete target identity.
        target: String,
        /// catalog 所记录的 generation。 / Generation recorded by the catalog.
        generation: String,
    },
    /// target current 指针已前移，catalog 记录成为历史快照。 / The target current pointer advanced, making the catalog record historical.
    Historical {
        /// 完整 target 身份。 / Complete target identity.
        target: String,
        /// catalog 的历史 generation。 / Historical catalog generation.
        recorded: String,
        /// publication 当前 generation。 / Current publication generation.
        current: String,
    },
}

impl fmt::Display for BuildCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(message) => write!(formatter, "build catalog storage error: {message}"),
            Self::Corrupt(message) => write!(formatter, "corrupt build catalog: {message}"),
            Self::MissingCurrent { target, generation } => write!(
                formatter,
                "target `{target}` current generation `{generation}` is missing"
            ),
            Self::Historical {
                target,
                recorded,
                current,
            } => write!(
                formatter,
                "target `{target}` catalog generation `{recorded}` is historical; current is `{current}`"
            ),
        }
    }
}

impl std::error::Error for BuildCatalogError {}

fn runtime_catalog_error(error: BuildRuntimeError) -> BuildCatalogError {
    match error.kind() {
        BuildRuntimeErrorKind::Corrupt => BuildCatalogError::Corrupt(error.to_string()),
        BuildRuntimeErrorKind::Storage | BuildRuntimeErrorKind::Tool => {
            BuildCatalogError::Storage(error.to_string())
        }
    }
}

/// Invocation-owned published snapshots; never a global or bounded-session cache.
struct VerifiedGeneration {
    /// Canonical generation metadata verified against all actual committed members.
    record: RecordedGeneration,
    /// Exact sealed published snapshots, ordered identically to record.artifacts.
    blobs: Vec<VerifiedBlob>,
}

fn read_base_build_catalog(
    runtime: &dyn BuildRuntime,
) -> Result<Option<BuildCatalogSnapshot>, BuildCatalogError> {
    let target = publication_target(BUILD_CATALOG_TARGET)?;
    let Some(generation) = runtime
        .current_generation(GenerationSpace::BuildCatalog, &target)
        .map_err(runtime_catalog_error)?
    else {
        return Ok(None);
    };
    if generation.artifacts.len() != 1 {
        return Err(BuildCatalogError::Corrupt(
            "build catalog generation membership is not canonical".into(),
        ));
    }
    let artifact = generation.artifacts.first().expect("one catalog artifact");
    if artifact.descriptor.id.as_str() != "build-record-v3"
        || artifact.path.as_str() != "build-record-v3.json"
    {
        return Err(BuildCatalogError::Corrupt(
            "current generation has no canonical build-record-v3 artifact".into(),
        ));
    }
    let bytes = read_published_bytes(
        runtime,
        GenerationSpace::BuildCatalog,
        &generation.identity,
        artifact,
    )?;
    let record = decode_build_record(bytes.bytes())
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    // Action outputs are disposable derived cache, not committed catalog authority.
    // Their own verified lookup boundary decides reuse or recomputation.
    Ok(Some(BuildCatalogSnapshot {
        record,
        record_artifact: protocol_artifact(artifact),
    }))
}

fn publication_target(value: &str) -> Result<PublicationTargetId, BuildCatalogError> {
    PublicationTargetId::new(value)
        .map_err(|_| BuildCatalogError::Corrupt(format!("invalid publication target `{value}`")))
}

fn read_published_bytes(
    runtime: &dyn BuildRuntime,
    space: GenerationSpace,
    generation: &GenerationRef,
    artifact: &GenerationArtifact,
) -> Result<VerifiedBlob, BuildCatalogError> {
    let (read, bytes) = runtime
        .read_generation_artifact(space, generation, &artifact.path)
        .map_err(runtime_catalog_error)?;
    match read {
        ArtifactRead::NotFound => Err(BuildCatalogError::Corrupt(format!(
            "generation member `{}` is missing",
            artifact.path
        ))),
        ArtifactRead::Verified(descriptor) if descriptor == artifact.descriptor => {
            let blob = VerifiedBlob::from_owned(bytes);
            if blob.digest() != &artifact.descriptor.digest
                || blob.bytes().len() as u64 != artifact.descriptor.size
            {
                return Err(BuildCatalogError::Corrupt(format!(
                    "generation member `{}` bytes differ from its catalog identity",
                    artifact.path
                )));
            }
            Ok(blob)
        }
        ArtifactRead::Verified(_) => Err(BuildCatalogError::Corrupt(format!(
            "generation member `{}` descriptor differs",
            artifact.path
        ))),
    }
}

fn protocol_artifact(artifact: &GenerationArtifact) -> Artifact {
    Artifact {
        id: artifact.descriptor.id.clone(),
        kind: artifact.descriptor.kind.clone(),
        uri: artifact.path.as_str().into(),
        size: artifact.descriptor.size,
        digest: artifact.descriptor.digest.clone(),
    }
}

fn validate_target_artifact_schema(
    artifact: &ArtifactDescriptor,
    bytes: &[u8],
) -> Result<(), BuildCatalogError> {
    let valid = match &artifact.kind {
        ArtifactKind::BinaryIr => decode_unit_container(bytes).is_ok(),
        ArtifactKind::DebugInfo => squish_ir::decode_debug_bundle(bytes).is_ok(),
        ArtifactKind::Other(name) if name == "static-link-map" => {
            decode_static_link_map(bytes).is_ok()
        }
        ArtifactKind::Metadata if artifact.id.as_str().ends_with(":target-record") => {
            serde_json::from_slice::<TargetRecordV1>(bytes)
                .is_ok_and(|record| record.schema == 1 && !record.snapshot.is_empty())
        }
        ArtifactKind::Prompt | ArtifactKind::Metadata | ArtifactKind::Other(_) => true,
    };
    if valid {
        Ok(())
    } else {
        Err(BuildCatalogError::Corrupt(format!(
            "artifact `{}` does not match its declared schema",
            artifact.id
        )))
    }
}

/// 通过注入运行时读取并严格验证当前 BuildRecord v3。 / Reads and strictly validates the current BuildRecord v3 through an injected runtime.
pub fn read_current_build_catalog(
    runtime: &dyn BuildRuntime,
) -> Result<Option<BuildCatalogSnapshot>, BuildCatalogError> {
    let Some(snapshot) = read_base_build_catalog(runtime)? else {
        return Ok(None);
    };
    for target in &snapshot.record.targets {
        // Read-only verification drops this generation's buffers before the next target.
        verify_published_generation(runtime, target)?;
        let Some(current) = runtime
            .current_generation(GenerationSpace::TargetArtifacts, &target.target_id)
            .map_err(runtime_catalog_error)?
        else {
            return Err(BuildCatalogError::MissingCurrent {
                target: target.target_id.to_string(),
                generation: target.generation_id.to_hex(),
            });
        };
        if current.identity.generation != target.generation_id {
            return Err(BuildCatalogError::Historical {
                target: target.target_id.to_string(),
                recorded: target.generation_id.to_hex(),
                current: current.identity.generation.to_hex(),
            });
        }
        if recorded_generation(&current) != *target {
            return Err(BuildCatalogError::Corrupt(format!(
                "target `{}` current generation differs from catalog semantics",
                target.target_id
            )));
        }
    }
    Ok(Some(snapshot))
}

/// 读取当前记录的便利投影。 / Convenience projection that reads the current record.
pub fn read_current_build_record(
    runtime: &dyn BuildRuntime,
) -> Result<Option<BuildRecordV3>, BuildCatalogError> {
    read_current_build_catalog(runtime).map(|snapshot| snapshot.map(|snapshot| snapshot.record))
}

fn recover_build_catalog(
    runtime: &dyn BuildRuntime,
) -> Result<Option<BuildRecordV3>, BuildCatalogError> {
    let Some(snapshot) = read_base_build_catalog(runtime)? else {
        return Ok(None);
    };
    let mut recovered = Vec::with_capacity(snapshot.record.targets.len());
    for target in &snapshot.record.targets {
        // Retain at most one target's committed buffers, not the sum across targets.
        let verified = verify_published_generation(runtime, target)?;
        let recorded = &verified.record;
        match runtime
            .current_generation(GenerationSpace::TargetArtifacts, &recorded.target_id)
            .map_err(runtime_catalog_error)?
        {
            Some(current) if recorded_generation(&current) == *recorded => {
                // This exact generation's bytes/schema were verified above. Reuse the receipt.
                recovered.push(verified.record);
            }
            Some(current) => recovered
                .push(verify_published_generation(runtime, &recorded_generation(&current))?.record),
            None => recovered.push(restore_recorded_generation(runtime, &verified)?),
        }
    }
    let mut record = snapshot.record;
    recovered.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    record.targets = recovered;
    validate_build_record(&record)
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    Ok(Some(record))
}

/// Verifies each committed member once and retains the exact bytes only for recovery.
fn verify_published_generation(
    runtime: &dyn BuildRuntime,
    recorded: &RecordedGeneration,
) -> Result<VerifiedGeneration, BuildCatalogError> {
    validate_recorded_generation(recorded)?;
    let reference = GenerationRef {
        target: recorded.target_id.clone(),
        generation: recorded.generation_id,
    };
    let mut blobs = Vec::with_capacity(recorded.artifacts.len());
    for item in &recorded.artifacts {
        let published = read_published_bytes(
            runtime,
            GenerationSpace::TargetArtifacts,
            &reference,
            &GenerationArtifact {
                descriptor: item.descriptor.clone(),
                path: item.destination.clone(),
            },
        )?;
        validate_target_artifact_schema(&item.descriptor, published.bytes())?;
        blobs.push(published);
    }
    validate_generation_target_record(recorded, &blobs)?;
    Ok(VerifiedGeneration {
        record: recorded.clone(),
        blobs,
    })
}

fn recorded_generation(current: &CommittedGeneration) -> RecordedGeneration {
    RecordedGeneration {
        target_id: current.identity.target.clone(),
        generation_id: current.identity.generation,
        artifacts: current
            .artifacts
            .iter()
            .map(|item| CatalogArtifact {
                descriptor: item.descriptor.clone(),
                destination: item.path.clone(),
            })
            .collect(),
    }
}

fn validate_recorded_generation(generation: &RecordedGeneration) -> Result<(), BuildCatalogError> {
    if generation.artifacts.is_empty()
        || !generation.artifacts.windows(2).all(|pair| {
            (&pair[0].destination, &pair[0].descriptor.id)
                < (&pair[1].destination, &pair[1].descriptor.id)
        })
    {
        return Err(BuildCatalogError::Corrupt(format!(
            "target `{}` artifact membership is not canonical",
            generation.target_id
        )));
    }
    let maps = generation
        .artifacts
        .iter()
        .filter(|item| {
            matches!(&item.descriptor.kind, ArtifactKind::Other(name) if name == "static-link-map")
        })
        .count();
    if maps != 1 {
        return Err(BuildCatalogError::Corrupt(format!(
            "target `{}` does not contain exactly one static link map",
            generation.target_id
        )));
    }
    let records = generation
        .artifacts
        .iter()
        .filter(|item| {
            matches!(item.descriptor.kind, ArtifactKind::Metadata)
                && item.descriptor.id.as_str().ends_with(":target-record")
        })
        .count();
    let unique_destinations: BTreeSet<_> = generation
        .artifacts
        .iter()
        .map(|item| item.destination.as_str())
        .collect();
    if records != 1 || unique_destinations.len() != generation.artifacts.len() {
        return Err(BuildCatalogError::Corrupt(format!(
            "target `{}` has invalid target-record or destination membership",
            generation.target_id
        )));
    }
    Ok(())
}

fn validate_generation_target_record(
    generation: &RecordedGeneration,
    blobs: &[VerifiedBlob],
) -> Result<(), BuildCatalogError> {
    let index = generation
        .artifacts
        .iter()
        .position(|item| {
            matches!(item.descriptor.kind, ArtifactKind::Metadata)
                && item.descriptor.id.as_str().ends_with(":target-record")
        })
        .ok_or_else(|| {
            BuildCatalogError::Corrupt(format!(
                "target `{}` has no target record",
                generation.target_id
            ))
        })?;
    let item = &generation.artifacts[index];
    let record: TargetRecordV1 = serde_json::from_slice(blobs[index].bytes())
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    if record.schema != 1
        || record.target != generation.target_id.as_str()
        || record.snapshot.is_empty()
    {
        return Err(BuildCatalogError::Corrupt(format!(
            "target record `{}` has the wrong identity",
            item.descriptor.id
        )));
    }
    let expected: BTreeMap<_, _> = generation
        .artifacts
        .iter()
        .filter(|candidate| candidate.descriptor.id != item.descriptor.id)
        .map(|candidate| {
            (
                candidate.destination.as_str().to_owned(),
                TargetRecordArtifactV1 {
                    destination: candidate.destination.as_str().to_owned(),
                    kind: format!("{:?}", candidate.descriptor.kind),
                    digest: candidate.descriptor.digest.hex(),
                    size: candidate.descriptor.size,
                },
            )
        })
        .collect();
    let actual_len = record.artifacts.len();
    let actual: BTreeMap<_, _> = record
        .artifacts
        .into_iter()
        .map(|artifact| (artifact.destination.clone(), artifact))
        .collect();
    if actual_len != actual.len() || expected != actual {
        return Err(BuildCatalogError::Corrupt(format!(
            "target record `{}` does not cover its generation",
            item.descriptor.id
        )));
    }
    Ok(())
}

fn restore_recorded_generation(
    runtime: &dyn BuildRuntime,
    verified: &VerifiedGeneration,
) -> Result<RecordedGeneration, BuildCatalogError> {
    let recorded = &verified.record;
    let publications: Vec<_> = recorded
        .artifacts
        .iter()
        .map(|item| {
            let name = OutputName::new(item.descriptor.id.as_str()).map_err(|_| {
                BuildCatalogError::Corrupt(format!(
                    "artifact `{}` cannot be restored as a named output",
                    item.descriptor.id
                ))
            })?;
            Ok(Publication {
                output: ProducedOutput {
                    name,
                    kind: item.descriptor.kind.clone(),
                    digest: item.descriptor.digest.clone(),
                    size: item.descriptor.size,
                },
                name: item.descriptor.name.clone(),
                destination: item.destination.clone(),
            })
        })
        .collect::<Result<_, BuildCatalogError>>()?;
    let restored = runtime
        .publish_generation_verified(
            GenerationSpace::TargetArtifacts,
            &recorded.target_id,
            &publications,
            &verified.blobs,
        )
        .map_err(runtime_catalog_error)?;
    if recorded_generation(&restored) != *recorded {
        return Err(BuildCatalogError::Corrupt(format!(
            "target `{}` could not restore its recorded generation",
            recorded.target_id
        )));
    }
    Ok(recorded.clone())
}

/// 一个经过完整验证的 current catalog 快照。 / A fully validated current-catalog snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildCatalogSnapshot {
    /// 类型化 BuildRecord v3。 / Typed BuildRecord v3.
    pub record: BuildRecordV3,
    /// catalog generation 中的记录产物自身。 / Record artifact in the catalog generation itself.
    pub record_artifact: Artifact,
}

pub(crate) fn catalog_protocol_artifact(item: &CatalogArtifact) -> Artifact {
    Artifact {
        id: item.descriptor.id.clone(),
        kind: item.descriptor.kind.clone(),
        uri: item.destination.as_str().into(),
        size: item.descriptor.size,
        digest: item.descriptor.digest.clone(),
    }
}

impl BuildCatalogSnapshot {
    /// 按全局 ID 查询 target 或 build-record 产物。 / Looks up a target or build-record artifact by global ID.
    pub fn artifact(&self, id: &ArtifactId) -> Option<Artifact> {
        if &self.record_artifact.id == id {
            Some(self.record_artifact.clone())
        } else {
            self.record.artifact(id)
        }
    }

    /// 按类型化逻辑路径查询 target 或 catalog 产物。 / Looks up a target or catalog artifact by typed logical path.
    pub fn artifact_at(&self, locator: &crate::ArtifactLocator) -> Option<Artifact> {
        let path = locator.as_path().to_string_lossy().replace('\\', "/");
        if self.record_artifact.uri == path {
            Some(self.record_artifact.clone())
        } else {
            self.record.artifact_at(locator)
        }
    }

    /// 查询唯一 target 的静态链接图。 / Looks up the static link map of a unique target.
    pub fn link_map(&self, target: &TargetName) -> Result<Option<Artifact>, BuildRecordQueryError> {
        self.record.link_map(target)
    }

    /// 返回宿主可直接使用的封闭类型化来源关系。 / Returns the closed typed provenance relation consumed directly by hosts.
    pub fn provenance_relation(&self, artifact: &ArtifactId) -> crate::ProvenanceRelation {
        if artifact == &self.record_artifact.id {
            return crate::ProvenanceRelation::NotApplicable(
                crate::ProvenanceNonApplicability::SelfDescribingBuildRecord,
            );
        }
        let Some(generation) = self.record.targets.iter().find(|target| {
            target
                .artifacts
                .iter()
                .any(|item| &item.descriptor.id == artifact)
        }) else {
            return crate::ProvenanceRelation::NotApplicable(
                crate::ProvenanceNonApplicability::UnsupportedKind,
            );
        };
        let Some(subject) = generation
            .artifacts
            .iter()
            .find(|item| &item.descriptor.id == artifact)
        else {
            return crate::ProvenanceRelation::NotApplicable(
                crate::ProvenanceNonApplicability::UnsupportedKind,
            );
        };
        match &subject.descriptor.kind {
            ArtifactKind::Prompt => {
                let mut evidence: Vec<_> = generation
                    .artifacts
                    .iter()
                    .filter(|item| matches!(item.descriptor.kind, ArtifactKind::DebugInfo))
                    .map(catalog_protocol_artifact)
                    .collect();
                evidence.push(self.record_artifact.clone());
                crate::ProvenanceRelation::Evidence(evidence)
            }
            ArtifactKind::Metadata
                if subject.descriptor.id.as_str().ends_with(":target-record") =>
            {
                crate::ProvenanceRelation::Evidence(vec![self.record_artifact.clone()])
            }
            ArtifactKind::BinaryIr | ArtifactKind::DebugInfo => {
                crate::ProvenanceRelation::NotApplicable(
                    crate::ProvenanceNonApplicability::SelfDescribingEvidence,
                )
            }
            ArtifactKind::Other(name) if name == "static-link-map" => {
                crate::ProvenanceRelation::NotApplicable(
                    crate::ProvenanceNonApplicability::SelfDescribingEvidence,
                )
            }
            ArtifactKind::Metadata | ArtifactKind::Other(_) => {
                crate::ProvenanceRelation::NotApplicable(
                    crate::ProvenanceNonApplicability::UnsupportedKind,
                )
            }
        }
    }
}

/// 类型化 catalog 查询错误。 / Typed catalog query error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildRecordQueryError {
    /// 仅按 target 名查询时命中多个 workspace owner。 / A target-name lookup matched multiple workspace owners.
    AmbiguousTarget(String),
}

impl fmt::Display for BuildRecordQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousTarget(target) => {
                write!(formatter, "target `{target}` is ambiguous across packages")
            }
        }
    }
}

impl std::error::Error for BuildRecordQueryError {}

impl BuildRecordV3 {
    /// 返回封闭计划投影。 / Returns the closed plan projection.
    pub fn planned_actions(&self) -> &squish_protocol::PlanInspection {
        &self.plan
    }

    /// 按全局唯一 ID 查询产物。 / Looks up an artifact by its globally unique ID.
    pub fn artifact(&self, id: &ArtifactId) -> Option<Artifact> {
        self.targets
            .iter()
            .flat_map(|target| &target.artifacts)
            .find(|item| &item.descriptor.id == id)
            .map(catalog_protocol_artifact)
    }

    /// 按 generation 相对 URI 查询产物。 / Looks up an artifact by generation-relative URI.
    pub fn artifact_at(&self, locator: &crate::ArtifactLocator) -> Option<Artifact> {
        let destination = locator.as_path().to_string_lossy().replace('\\', "/");
        self.targets
            .iter()
            .flat_map(|target| &target.artifacts)
            .find(|item| item.destination.as_str() == destination)
            .map(catalog_protocol_artifact)
    }

    /// 查询唯一 target 的静态链接图；同名跨包 target 返回歧义错误。 / Looks up a unique target's static link map; same-named targets across packages are ambiguous.
    pub fn link_map(&self, target: &TargetName) -> Result<Option<Artifact>, BuildRecordQueryError> {
        let generation = self.unique_target(target)?;
        Ok(generation.and_then(|generation| {
            generation
                .artifacts
                .iter()
                .map(catalog_protocol_artifact)
                .find(|artifact| {
                    matches!(&artifact.kind, ArtifactKind::Other(name) if name == "static-link-map")
                })
        }))
    }

    fn unique_target(
        &self,
        target: &TargetName,
    ) -> Result<Option<&RecordedGeneration>, BuildRecordQueryError> {
        let suffix = format!(":{}", target.as_str());
        let mut matching = self
            .targets
            .iter()
            .filter(|generation| generation.target_id.as_str().ends_with(&suffix));
        let first = matching.next();
        if matching.next().is_some() {
            Err(BuildRecordQueryError::AmbiguousTarget(
                target.as_str().into(),
            ))
        } else {
            Ok(first)
        }
    }
}

/// 构建图中一个动作的领域工作。 / Domain work represented by one build-graph action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildWork {
    /// 编译一个已冻结的逻辑源码。 / Compile one frozen logical source.
    Compile {
        /// 物理检出位置无关的源码身份。 / Checkout-independent source identity.
        source: SourceKey,
    },
    /// 从 target entry 链接闭包。 / Link a closure rooted at a target entry.
    Link {
        /// `package:target` 内部唯一身份。 / Internally unique `package:target` identity.
        target: String,
        /// 必须为 entry unit 的链接根。 / Link root, which must be an entry unit.
        entry: SourceKey,
    },
    /// 使用 target 参数与预算实例化。 / Instantiate with target arguments and budgets.
    Instantiate {
        /// `package:target` 内部唯一身份。 / Internally unique `package:target` identity.
        target: String,
    },
    /// 运行纯 Squish 后端。 / Run the pure Squish backend.
    Backend {
        /// `package:target` 内部唯一身份。 / Internally unique `package:target` identity.
        target: String,
    },
    /// 原子发布一个 target generation。 / Atomically publish one target generation.
    Publish {
        /// `package:target` 内部唯一身份。 / Internally unique `package:target` identity.
        target: String,
    },
}

impl PlannedWork for BuildWork {
    fn kind(&self) -> ActionKind {
        match self {
            Self::Compile { .. } => ActionKind::Compile,
            Self::Link { .. } => ActionKind::Link,
            Self::Instantiate { .. } => ActionKind::Instantiate,
            Self::Backend { .. } => ActionKind::Backend,
            Self::Publish { .. } => ActionKind::Publish,
        }
    }

    fn effect(&self) -> Effect {
        match self {
            Self::Publish { .. } => Effect::WriteEffect,
            _ => Effect::Transform,
        }
    }
}

/// 分析阶段封闭的构建输入、计划与一次读取的源码。 / Analysis-closed build inputs, plan, and once-read sources.
pub struct PreparedBuild {
    plan: PreparedPlan<BuildWork>,
    repository: ProjectRepository,
    snapshot: ProjectSnapshot,
    sources: Vec<FrozenSource>,
    source_indices: BTreeMap<SourceKey, usize>,
    targets: Vec<TargetBuild>,
    emit: Vec<EmitKind>,
    locations: Vec<squish_repository::PackageLocation>,
    runtime: Arc<dyn BuildRuntime>,
    runtime_descriptor: BuildRuntimeDescriptor,
    backend_identities: BTreeMap<String, BackendCacheIdentity>,
    prior_record: Option<BuildRecordV3>,
}

impl PreparedBuild {
    /// 返回经图/工作一一对应校验的计划。 / Returns the graph/work-validated plan.
    pub const fn plan(&self) -> &PreparedPlan<BuildWork> {
        &self.plan
    }
}

#[derive(Clone)]
struct FrozenSource {
    blob: Arc<SourceBlob>,
    package: squish_ir::PackageInstanceId,
    /// Imported archives preserve their relocated identity and immutable compiled object.
    precompiled: Option<Arc<RelocatableUnitIr>>,
    /// Cached once after archive reachability; recipe/execution share exact relocated object bytes.
    precompiled_blob: Option<VerifiedBlob>,
    /// IR-owned proof binds relocated arenas to actual canonical identities without caller labels.
    precompiled_validated: Option<ValidatedUnit>,
    /// Frozen resource bytes are explicit action inputs, never worker filesystem reads.
    assets: BTreeMap<String, Arc<[u8]>>,
    /// Archive-local resolved imports are immutable and source-neutral to the linker.
    archive: Option<Arc<squish_backend::archive::SharedSopackPayload>>,
}

struct TargetBuild {
    package: String,
    name: String,
    entry: SourceKey,
    resolved: ResolvedTarget,
    arguments: BTreeMap<String, String>,
}

#[derive(Clone)]
struct CompiledUnit {
    unit: Arc<RelocatableUnitIr>,
    /// Validated immutable specialization reused by every independently bound target/include.
    prepared: PreparedUnit,
    bytes: VerifiedBlob,
    object: ObjectDigest,
    semantic: squish_ir::SemanticUnitDigest,
    debug: DebugDigest,
}

#[derive(Clone)]
struct BackendStage {
    /// Only immutable publication bytes survive emission; provenance was sealed in debug.
    product: VerifiedBlob,
    debug: VerifiedBlob,
    kind: ArtifactKind,
}

/// 完成 resolve、权威 repository snapshot 与一次性 source sealing，并生成封闭执行图。
/// Resolves dependencies, freezes the authoritative repository snapshot, seals every source once,
/// and produces the closed execution graph.
pub fn prepare(
    request: &BuildRequest,
    services: &dyn Services,
) -> Result<PreparedBuild, ManagerError> {
    prepare_excluding(
        request,
        services,
        &BTreeSet::new(),
        &DurabilityPorts::default(),
    )
}

fn prepare_excluding(
    request: &BuildRequest,
    services: &dyn Services,
    excluded: &BTreeSet<String>,
    durability: &DurabilityPorts,
) -> Result<PreparedBuild, ManagerError> {
    let repository = ProjectRepository::discover_with_faults(
        Discovery::Explicit(request.project.as_str().into()),
        durability.repository(),
    )
    .map_err(|e| error("MGB001", Phase::Discover, e))?;
    let runtime = services
        .open_build_runtime(repository.root())
        .map_err(|e| ManagerError::new(e.code(), Phase::Cache, e.message()))?;
    let runtime_descriptor = runtime.descriptor();
    let prior_record = recover_build_catalog(runtime.as_ref())
        .map_err(|e| ManagerError::new("MGB124", Phase::Cache, e.to_string()))?;
    let mode = resolution_mode(request);
    let bare = repository.snapshot();
    let initial_locations = match &bare {
        Ok(snapshot) => snapshot
            .lockfile()
            .map(|lock| services.materialize_locked(repository.root(), lock, mode))
            .transpose(),
        Err(_) => {
            let bytes = std::fs::read(repository.root().join(squish_project::LOCK_FILE_NAME))
                .map_err(|e| error("MGB002", Phase::Snapshot, e))?;
            let lock = Lockfile::parse(
                std::str::from_utf8(&bytes).map_err(|e| error("MGB002", Phase::Snapshot, e))?,
            )
            .map_err(|e| error("MGB002", Phase::Snapshot, e))?;
            services
                .materialize_locked(repository.root(), &lock, mode)
                .map(Some)
        }
    };
    let initial_locations = initial_locations
        .map_err(|e| ManagerError::new(e.code(), Phase::Resolve, e.message()))?
        .unwrap_or_default();
    let initial = repository
        .snapshot_with_locations(&initial_locations)
        .map_err(|e| error("MGB002", Phase::Snapshot, e))?;
    let manifests = manifest_map(&initial);
    // XML resolution owns package locks only; reject stale skill intent before it can rewrite the lock.
    validate_skill_lock(&manifests, initial.lockfile())?;
    let resolved = services
        .resolve(ResolveRequest {
            manifests: &manifests,
            manifest_digest: initial.manifest_digest(),
            prior_lock: initial.lockfile(),
            mode,
        })
        .map_err(|e| ManagerError::new(e.code(), Phase::Resolve, e.message()))?;
    validate_skill_lock(&manifests, Some(&resolved.lockfile))?;
    validate_preserved_skill_lock(initial.lockfile(), &resolved.lockfile)?;
    ensure_authoritative_lock(&repository, &initial, &resolved.lockfile, mode)?;
    let snapshot = repository
        .snapshot_with_locations(&resolved.packages)
        .map_err(|e| error("MGB004", Phase::Discover, e))?;
    let sources = freeze_sources(&snapshot, &resolved.packages, &resolved.sopacks)?;
    let targets = select_targets(request, &snapshot, &sources, excluded, &resolved.packages)?;
    let backend_identities = freeze_backend_identities(&targets, runtime.as_ref())?;
    let plan = build_plan(
        &snapshot,
        &sources,
        &targets,
        &request.emit,
        &backend_identities,
        &runtime_descriptor,
    )?;
    Ok(PreparedBuild {
        plan,
        repository,
        snapshot,
        source_indices: sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source_key(source), index))
            .collect(),
        sources,
        targets,
        emit: request.emit.clone(),
        locations: resolved.packages,
        runtime,
        runtime_descriptor,
        backend_identities,
        prior_record,
    })
}

fn prepare_recorded(
    request: &BuildRequest,
    services: &dyn Services,
    excluded: &BTreeSet<String>,
    durability: &DurabilityPorts,
    planning: &mut PlanningRecorder<'_>,
    context: &InvocationContext,
) -> Result<PreparedBuild, PlanningFailure> {
    let repository = planning.step(step("locate"), PlanningStepKind::Locate, || {
        planning_not_cancelled(context)?;
        ProjectRepository::discover_with_faults(
            Discovery::Explicit(request.project.as_str().into()),
            durability.repository(),
        )
        .map_err(|e| error("MGB001", Phase::Discover, e))
    })?;
    planning.step(step("recover"), PlanningStepKind::Recover, || {
        planning_not_cancelled(context)?;
        repository
            .recover()
            .map_err(|e| error("MGB001", Phase::Discover, e))
    })?;
    let runtime = planning.step(
        step("open-build-runtime"),
        PlanningStepKind::Recover,
        || {
            services
                .open_build_runtime(repository.root())
                .map_err(|e| ManagerError::new(e.code(), Phase::Cache, e.message()))
        },
    )?;
    let runtime_descriptor = runtime.descriptor();
    let prior_record = planning.step(
        step("recover-build-catalog"),
        PlanningStepKind::Recover,
        || {
            planning_not_cancelled(context)?;
            recover_build_catalog(runtime.as_ref())
                .map_err(|e| ManagerError::new("MGB124", Phase::Cache, e.to_string()))
        },
    )?;
    let mode = resolution_mode(request);
    let workspace = planning.step(step("snapshot-initial"), PlanningStepKind::Snapshot, || {
        planning_not_cancelled(context)?;
        repository
            .snapshot()
            .map_err(|e| error("MGB002", Phase::Snapshot, e))
    })?;
    let initial_locations = planning.step(step("fetch-locked"), PlanningStepKind::Fetch, || {
        planning_not_cancelled(context)?;
        workspace
            .lockfile()
            .map(|lock| services.materialize_locked(repository.root(), lock, mode))
            .transpose()
            .map_err(|e| ManagerError::new(e.code(), Phase::Resolve, e.message()))
            .map(Option::unwrap_or_default)
    })?;
    let initial = planning.step(
        step("snapshot-materialized"),
        PlanningStepKind::Snapshot,
        || {
            planning_not_cancelled(context)?;
            repository
                .snapshot_with_locations(&initial_locations)
                .map_err(|e| error("MGB002", Phase::Snapshot, e))
        },
    )?;
    let manifests = manifest_map(&initial);
    planning.step(
        step("validate-skill-lock"),
        PlanningStepKind::Resolve,
        || validate_skill_lock(&manifests, initial.lockfile()),
    )?;
    let resolved = planning.step(step("resolve"), PlanningStepKind::Resolve, || {
        planning_not_cancelled(context)?;
        services
            .resolve(ResolveRequest {
                manifests: &manifests,
                manifest_digest: initial.manifest_digest(),
                prior_lock: initial.lockfile(),
                mode,
            })
            .map_err(|e| ManagerError::new(e.code(), Phase::Resolve, e.message()))
    })?;
    planning.step(
        step("validate-resolved-skill-lock"),
        PlanningStepKind::Resolve,
        || {
            validate_skill_lock(&manifests, Some(&resolved.lockfile))?;
            validate_preserved_skill_lock(initial.lockfile(), &resolved.lockfile)
        },
    )?;
    planning.step(
        step("reconcile-lock"),
        PlanningStepKind::ReconcileLock,
        || {
            planning_not_cancelled(context)?;
            ensure_authoritative_lock(&repository, &initial, &resolved.lockfile, mode)
        },
    )?;
    let snapshot = planning.step(
        step("snapshot-authoritative"),
        PlanningStepKind::Snapshot,
        || {
            planning_not_cancelled(context)?;
            repository
                .snapshot_with_locations(&resolved.packages)
                .map_err(|e| error("MGB004", Phase::Snapshot, e))
        },
    )?;
    let (sources, targets) = planning.step(step("scan"), PlanningStepKind::Scan, || {
        planning_not_cancelled(context)?;
        let sources = freeze_sources(&snapshot, &resolved.packages, &resolved.sopacks)?;
        let targets = select_targets(request, &snapshot, &sources, excluded, &resolved.packages)?;
        Ok((sources, targets))
    })?;
    let (backend_identities, plan) = planning.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || {
            planning_not_cancelled(context)?;
            let backend_identities = freeze_backend_identities(&targets, runtime.as_ref())?;
            let plan = build_plan(
                &snapshot,
                &sources,
                &targets,
                &request.emit,
                &backend_identities,
                &runtime_descriptor,
            )?;
            Ok((backend_identities, plan))
        },
    )?;
    Ok(PreparedBuild {
        plan,
        repository,
        snapshot,
        source_indices: sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source_key(source), index))
            .collect(),
        sources,
        targets,
        emit: request.emit.clone(),
        locations: resolved.packages,
        runtime,
        runtime_descriptor,
        backend_identities,
        prior_record,
    })
}

fn step(value: &str) -> PlanningStepId {
    PlanningStepId::new(value).expect("static planning step IDs are non-empty")
}

fn planning_not_cancelled(context: &InvocationContext) -> Result<(), ManagerError> {
    if context.is_cancelled() {
        Err(ManagerError::new(
            "MGB000",
            Phase::Manage,
            "build planning was cancelled",
        ))
    } else {
        Ok(())
    }
}

/// 通过统一 scheduler/orchestrator 生命周期执行构建。 / Executes a build through the unified scheduler/orchestrator lifecycle.
pub fn execute(
    request: &BuildRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    context: &InvocationContext,
) -> OperationOutcome {
    execute_with_durability(
        request,
        services,
        settings,
        &DurabilityPorts::default(),
        context,
    )
}

pub(crate) fn execute_with_durability(
    request: &BuildRequest,
    services: &dyn Services,
    settings: &InvocationSettings,
    durability: &DurabilityPorts,
    context: &InvocationContext,
) -> OperationOutcome {
    let job = JobId::new(format!("build-{}", context.id())).expect("invocation IDs are non-empty");
    let excluded = settings
        .excluded_packages
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    let mut planning = match PlanningRecorder::start(
        job.clone(),
        PlanningAttemptId::new("attempt-1").expect("static planning attempt"),
        context,
    ) {
        Ok(planning) => planning,
        Err(_) => return unavailable(job, context),
    };
    let prepared = match prepare_recorded(
        request,
        services,
        &excluded,
        durability,
        &mut planning,
        context,
    ) {
        Ok(prepared) => prepared,
        Err(failure) => return planning.unavailable(OperationKind::Build, &failure),
    };
    let plan = prepared.plan.clone();
    let snapshot = planning_snapshot_digest(&prepared);
    let executor = match planning.step(step("open-build-state"), PlanningStepKind::Recover, || {
        Ok(BuildExecutor::new(prepared))
    }) {
        Ok(executor) => executor,
        Err(failure) => return planning.unavailable(OperationKind::Build, &failure),
    };
    let sealed = match planning.seal(plan.clone(), &snapshot, PlanMode::Execute) {
        Ok(sealed) => sealed,
        Err(failure) => return unavailable_after_seal(job, &failure),
    };
    let inspection = sealed.inspection();
    match orchestrator::run(sealed, &executor, context, settings) {
        Ok(report) => {
            let build_record = match orchestrator::finalize(
                &job,
                FinalizationId::new("persist-build-catalog").expect("static finalization ID"),
                FinalizationKind::PersistBuildCatalog,
                context,
                || executor.persist_terminal_record(&inspection, &report),
            ) {
                Ok(record) => record,
                Err(failure) => {
                    return OperationOutcome {
                        job,
                        result: OperationResult::Unavailable {
                            kind: OperationKind::Build,
                        },
                        totals: report.totals,
                        root_failures: report.root_failures + failure.root_failures(),
                        cancelled: report.cancelled,
                    };
                }
            };
            OperationOutcome {
                job,
                result: OperationResult::Build(BuildResult {
                    published: executor.published(),
                    build_record: Some(build_record),
                }),
                totals: report.totals,
                root_failures: report.root_failures,
                cancelled: report.cancelled,
            }
        }
        Err(_) => OperationOutcome {
            job,
            result: OperationResult::Unavailable {
                kind: OperationKind::Build,
            },
            totals: Default::default(),
            root_failures: 0,
            cancelled: context.is_cancelled(),
        },
    }
}

fn unavailable(job: JobId, context: &InvocationContext) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::Build,
        },
        totals: Default::default(),
        root_failures: 1,
        cancelled: context.is_cancelled(),
    }
}

fn unavailable_after_seal(job: JobId, failure: &PlanningFailure) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable {
            kind: OperationKind::Build,
        },
        totals: Default::default(),
        root_failures: u64::from(matches!(failure, PlanningFailure::Failed(_))),
        cancelled: matches!(failure, PlanningFailure::Cancelled),
    }
}

fn planning_snapshot_digest(prepared: &PreparedBuild) -> squish_protocol::Digest {
    let mut bytes = b"xmlsquish-planning-snapshot\0v1".to_vec();
    encode_field(&mut bytes, prepared.snapshot.manifest_digest().as_bytes());
    encode_field(&mut bytes, prepared.snapshot.lock_digest().as_bytes());
    let mut lock_ids: Vec<_> = prepared
        .locations
        .iter()
        .map(|location| location.lock_id.as_str())
        .collect();
    lock_ids.sort_unstable();
    for lock_id in lock_ids {
        encode_field(&mut bytes, lock_id.as_bytes());
    }
    let mut sources: Vec<_> = prepared.sources.iter().collect();
    sources.sort_by_key(|source| source_key(source));
    for source in sources {
        encode_source_key(&mut bytes, &source_key(source));
        encode_field(&mut bytes, source.blob.digest().as_bytes());
        for (path, asset) in &source.assets {
            encode_field(&mut bytes, path.as_bytes());
            encode_field(&mut bytes, blake3::hash(asset).as_bytes());
        }
    }
    protocol_blake3(&bytes)
}

struct BuildExecutor {
    runtime: Arc<dyn BuildRuntime>,
    prepared: PreparedBuild,
    state: Mutex<BuildExecutionState>,
}

/// All companion evidence participates in alias identity; equal documents can have different assets.
type InstantiationIdentity = (
    squish_protocol::Digest,
    squish_protocol::Digest,
    Option<squish_protocol::Digest>,
);

#[derive(Default)]
struct BuildExecutionState {
    compiled: BTreeMap<SourceKey, Arc<CompiledUnit>>,
    linked: BTreeMap<String, (Arc<LinkOutput>, Arc<ResolutionSnapshot>)>,
    /// Encoded maps are sealed once and shared with publication, including aliases.
    link_maps: BTreeMap<String, VerifiedBlob>,
    link_maps_by_digest: HashMap<squish_protocol::Digest, VerifiedBlob>,
    instantiated: BTreeMap<String, Arc<InstantiateOutput>>,
    backend: BTreeMap<String, Arc<BackendStage>>,
    /// Content aliases resolve without serializing or hashing existing stage payloads.
    linked_by_digest: HashMap<squish_protocol::Digest, (Arc<LinkOutput>, Arc<ResolutionSnapshot>)>,
    instantiated_by_digest: HashMap<InstantiationIdentity, Arc<InstantiateOutput>>,
    backend_by_digest: HashMap<(squish_protocol::Digest, squish_protocol::Digest), String>,
    published: Vec<PublishedTarget>,
    generations: Vec<RecordedGeneration>,
}

impl BuildExecutionState {
    /// Record both stable target identity and exact verified image identity atomically.
    fn insert_linked(
        &mut self,
        target: String,
        digest: squish_protocol::Digest,
        linked: Arc<LinkOutput>,
        resolution: Arc<ResolutionSnapshot>,
        map: VerifiedBlob,
    ) {
        self.link_maps_by_digest.insert(digest.clone(), map.clone());
        self.link_maps.insert(target.clone(), map);
        self.linked_by_digest
            .insert(digest, (Arc::clone(&linked), Arc::clone(&resolution)));
        self.linked.insert(target, (linked, resolution));
    }

    /// Register a normalized document once; consumers clone only the immutable handle.
    fn insert_instantiated(
        &mut self,
        target: String,
        digest: InstantiationIdentity,
        output: Arc<InstantiateOutput>,
    ) {
        self.instantiated_by_digest
            .insert(digest, Arc::clone(&output));
        self.instantiated.insert(target, output);
    }

    /// Index the complete product/debug pair without rehashing publication bytes.
    fn insert_backend(&mut self, target: String, stage: BackendStage) {
        self.backend_by_digest.insert(
            (stage.product.digest().clone(), stage.debug.digest().clone()),
            target.clone(),
        );
        self.backend.insert(target, Arc::new(stage));
    }
}

impl BuildExecutor {
    fn new(prepared: PreparedBuild) -> Self {
        let runtime = Arc::clone(&prepared.runtime);
        Self {
            runtime,
            prepared,
            state: Mutex::new(BuildExecutionState::default()),
        }
    }

    fn published(&self) -> Vec<PublishedTarget> {
        let state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        state.published.clone()
    }

    fn persist_terminal_record(
        &self,
        inspection: &squish_protocol::PlanInspection,
        report: &ExecutionReport,
    ) -> Result<Artifact, ManagerError> {
        let state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        let current = state.generations.clone();
        drop(state);
        let mut targets: BTreeMap<_, _> = self
            .prepared
            .prior_record
            .clone()
            .into_iter()
            .flat_map(|record| record.targets)
            .map(|generation| (generation.target_id.clone(), generation))
            .collect();
        for generation in current {
            targets.insert(generation.target_id.clone(), generation);
        }
        let targets = targets.into_values().collect();
        let actions: Vec<_> = report.actions.iter().map(build_action_fact).collect();
        let mut plan = inspection.clone();
        for action in &mut plan.actions {
            action.action_key = actions
                .iter()
                .find(|fact| fact.action == action.action)
                .and_then(|fact| fact.key.clone());
        }
        let record = BuildRecordV3 {
            schema: 3,
            job: inspection.job.clone(),
            plan,
            actions,
            targets,
            catalog_target: PublicationTargetId::new(BUILD_CATALOG_TARGET)
                .expect("static catalog target"),
        };
        let bytes = encode_build_record(&record)
            .map_err(|error| ManagerError::new("MGB121", Phase::Publish, error.to_string()))?;
        let bytes = VerifiedBlob::from_owned(bytes);
        let digest = self.store_verified(&bytes, "MGB122", Phase::Cache)?;
        let publication = Publication {
            output: ProducedOutput {
                name: output_name("build-record-v3"),
                kind: ArtifactKind::Metadata,
                digest,
                size: bytes.bytes().len() as u64,
            },
            name: LogicalArtifactName::new("build-record-v3").expect("static logical name"),
            destination: PublicationPath::new("build-record-v3.json").expect("static catalog path"),
        };
        let catalog_target =
            PublicationTargetId::new(BUILD_CATALOG_TARGET).expect("static catalog target");
        let generation = self
            .runtime
            .publish_generation_verified(
                GenerationSpace::BuildCatalog,
                &catalog_target,
                &[publication],
                &[bytes],
            )
            .map_err(|cause| ManagerError::new(cause.code(), Phase::Publish, cause.message()))?;
        self.runtime
            .flush_advisory()
            .map_err(|cause| ManagerError::new(cause.code(), Phase::Cache, cause.message()))?;
        generation
            .artifacts
            .into_iter()
            .next()
            .map(|item| protocol_artifact(&item))
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB123",
                    Phase::Publish,
                    "build catalog committed an empty generation",
                )
            })
    }

    /// Preserve the runtime identity check without rehashing a sealed immutable buffer.
    fn store_verified(
        &self,
        blob: &VerifiedBlob,
        code: &'static str,
        phase: Phase,
    ) -> Result<squish_protocol::Digest, ManagerError> {
        let actual = self
            .runtime
            .write_verified_blob(blob)
            .map_err(|cause| ManagerError::new(cause.code(), phase, cause.message()))?;
        if &actual != blob.digest() {
            return Err(ManagerError::new(
                code,
                phase,
                "runtime returned a mismatched blob digest",
            ));
        }
        Ok(actual)
    }

    fn target(&self, name: &str) -> Result<&TargetBuild, ManagerError> {
        self.prepared
            .targets
            .iter()
            .find(|target| format!("{}:{}", target.package, target.name) == name)
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB100",
                    Phase::Manage,
                    format!("unknown planned target `{name}`"),
                )
            })
    }

    /// Resolve invocation aliases by exact image digest, never by serialized payload scans.
    fn linked_for(
        &self,
        target: &str,
        inputs: &ResolvedInputs,
    ) -> Result<Arc<LinkOutput>, ManagerError> {
        let mut state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        if let Some((linked, _)) = state.linked.get(target) {
            return Ok(Arc::clone(linked));
        }
        let expected = expected_output(inputs, "image").ok_or_else(|| {
            ManagerError::new("MGB102", Phase::Instantiate, "linked image input is absent")
        })?;
        let value = state
            .linked_by_digest
            .get(&expected)
            .cloned()
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB102",
                    Phase::Instantiate,
                    "linked program cannot be restored",
                )
            })?;
        let linked = Arc::clone(&value.0);
        let map = state
            .link_maps_by_digest
            .get(&expected)
            .cloned()
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB102",
                    Phase::Instantiate,
                    "linked map cannot be restored",
                )
            })?;
        state.link_maps.insert(target.into(), map);
        state.linked.insert(target.into(), value);
        Ok(linked)
    }

    /// Normalize once and share the exact document/trace/directive result between stages.
    fn instantiated_for(
        &self,
        target: &str,
        inputs: &ResolvedInputs,
    ) -> Result<Arc<InstantiateOutput>, ManagerError> {
        let mut state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        if let Some(value) = state.instantiated.get(target) {
            return Ok(Arc::clone(value));
        }
        let expected = instantiation_identity(inputs)?;
        let value = state
            .instantiated_by_digest
            .get(&expected)
            .cloned()
            .ok_or_else(|| {
                ManagerError::new("MGB103", Phase::Emit, "document cannot be restored")
            })?;
        state.instantiated.insert(target.into(), Arc::clone(&value));
        Ok(value)
    }

    fn execute_work(
        &self,
        _dispatch: &Dispatch,
        work: &BuildWork,
        inputs: &ResolvedInputs,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        match work {
            BuildWork::Compile { source } => self.compile(source),
            BuildWork::Link { target, entry } => self.link(target, entry),
            BuildWork::Instantiate { target } => self.instantiate(target, inputs),
            BuildWork::Backend { target } => self.backend(target, inputs),
            BuildWork::Publish { target } => self.publish_target(target, inputs),
        }
    }

    fn compile(
        &self,
        key: &SourceKey,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let source = self
            .prepared
            .source_indices
            .get(key)
            .and_then(|index| self.prepared.sources.get(*index))
            .ok_or_else(|| {
                ManagerError::new("MGB101", Phase::Analyze, "planned source is absent")
            })?;
        let unit = match &source.precompiled {
            Some(unit) => unit.clone(),
            None => Arc::new(
                self.runtime
                    .compile(
                        &source.blob,
                        &FrontendSourceContext::new_with_source_package(
                            source.package.clone(),
                            source.blob.id().package().clone(),
                        ),
                    )
                    .map_err(|cause| runtime_error(cause.code().to_owned(), Phase::Analyze, cause))?
                    .unit,
            ),
        };
        if &header(&unit).source != key
            || header(&unit).frontend_abi.0 != self.prepared.runtime_descriptor.frontend_abi
        {
            return Err(ManagerError::new(
                "MGB040",
                Phase::Analyze,
                "frontend returned an output with a different frozen ABI",
            ));
        }
        let (validated, bytes) = match (&source.precompiled_validated, &source.precompiled_blob) {
            (Some(validated), Some(bytes)) => (validated.clone(), bytes.clone()),
            (None, None) => {
                let (validated, bytes) = ValidatedUnit::encode_new(Arc::clone(&unit))
                    .map_err(|e| error("MGB041", Phase::Analyze, e))?;
                (validated, VerifiedBlob::from_owned(bytes))
            }
            _ => {
                return Err(ManagerError::new(
                    "MGB041",
                    Phase::Analyze,
                    "compiled archive proof and encoded bytes must be frozen together",
                ));
            }
        };
        let revision = validated.revision().clone();
        let debug = validated.debug_digest();
        self.store_verified(&bytes, "MGB042", Phase::Cache)?;
        let prepared = PreparedUnit::from_validated(validated);
        let output = produced_verified("xsir", ArtifactKind::BinaryIr, &bytes);
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .compiled
            .insert(
                key.clone(),
                Arc::new(CompiledUnit {
                    unit,
                    prepared,
                    bytes,
                    object: revision.object,
                    semantic: revision.semantic,
                    debug,
                }),
            );
        Ok((vec![output], Vec::new()))
    }

    fn link(
        &self,
        target_name: &str,
        entry: &SourceKey,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        let resolution = Arc::new(bind_imports(
            &self.prepared.snapshot,
            &self.prepared.locations,
            &state.compiled,
            &self.prepared.sources,
        )?);
        let payloads = state
            .compiled
            .iter()
            .map(|(key, value)| (key.clone(), value.prepared.clone()))
            .collect();
        drop(state);
        let linked = self
            .runtime
            .link_prepared(
                entry,
                PreparedUnitClosure {
                    snapshot: resolution.clone(),
                    units: payloads,
                },
            )
            .map_err(|e| runtime_error("MGB070", Phase::Link, e))?;
        let image = VerifiedBlob::from_owned(encode_linked_image(&linked.image));
        let map = VerifiedBlob::from_owned(encode_static_link_map(&linked.map));
        self.store_verified(&image, "MGB071", Phase::Cache)?;
        self.store_verified(&map, "MGB072", Phase::Cache)?;
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .insert_linked(
                target_name.into(),
                image.digest().clone(),
                Arc::new(linked),
                resolution,
                map.clone(),
            );
        Ok((
            vec![
                produced_verified("image", ArtifactKind::Metadata, &image),
                produced_verified("map", ArtifactKind::Metadata, &map),
            ],
            Vec::new(),
        ))
    }

    fn instantiate(
        &self,
        target_name: &str,
        inputs: &ResolvedInputs,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let target = self.target(target_name)?;
        let linked = self.linked_for(target_name, inputs)?;
        let mut instantiated = self
            .runtime
            .instantiate(
                &linked.program,
                target.arguments.clone(),
                budgets(&target.resolved),
            )
            .map_err(|e| runtime_error("MGB073", Phase::Instantiate, e))?;
        let archive = is_archive_backend(&target.resolved.backend);
        if !archive && !instantiated.directives.is_empty() {
            return Err(ManagerError::new(
                "MGB150",
                Phase::Emit,
                "prompt targets cannot emit asset/include members; select the pack backend",
            ));
        }
        instantiated.document.document_abi =
            squish_ir::AbiId(self.prepared.runtime_descriptor.document_abi.clone());
        let document_bytes =
            VerifiedBlob::from_owned(encode_linked_document(&instantiated.document));
        let trace_bytes = VerifiedBlob::from_owned(encode_expansion_trace(&instantiated.trace));
        self.store_verified(&document_bytes, "MGB074", Phase::Cache)?;
        self.store_verified(&trace_bytes, "MGB075", Phase::Cache)?;
        let mut outputs = vec![
            produced_verified("document", ArtifactKind::Metadata, &document_bytes),
            produced_verified("trace", ArtifactKind::DebugInfo, &trace_bytes),
        ];
        if archive {
            let directive_bytes = VerifiedBlob::from_owned(squish_link::encode_archive_directives(
                &instantiated.directives,
            ));
            self.store_verified(&directive_bytes, "MGB075", Phase::Cache)?;
            outputs.push(produced_verified(
                "directives",
                ArtifactKind::Metadata,
                &directive_bytes,
            ));
        }
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .insert_instantiated(
                target_name.into(),
                (
                    document_bytes.digest().clone(),
                    trace_bytes.digest().clone(),
                    outputs
                        .iter()
                        .find(|output| output.name.as_str() == "directives")
                        .map(|output| output.digest.clone()),
                ),
                Arc::new(instantiated),
            );
        Ok((outputs, Vec::new()))
    }

    fn backend(
        &self,
        target_name: &str,
        inputs: &ResolvedInputs,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let target = self.target(target_name)?;
        let instantiated = self.instantiated_for(target_name, inputs)?;
        let document = &instantiated.document;
        let linked = self.linked_for(target_name, inputs)?;
        if matches!(target.resolved.backend.as_str(), "pack" | "sopack") {
            return archive_build::emit(self, target_name, target, &linked, &instantiated);
        }
        if !instantiated.directives.is_empty() {
            return Err(ManagerError::new(
                "MGB150",
                Phase::Emit,
                "prompt targets cannot emit asset/include members; select the pack backend",
            ));
        }
        let mut output = self
            .runtime
            .render_shared(
                document,
                &instantiated.trace,
                SquishOptions {
                    max_output_bytes: budgets(&target.resolved).max_output_bytes,
                },
            )
            .map_err(|e| runtime_error("MGB076", Phase::Emit, e))?;
        let expected_identity = self
            .prepared
            .backend_identities
            .get(target_name)
            .expect("every planned target has a frozen backend identity");
        if output.cache_identity != *expected_identity {
            return Err(ManagerError::new(
                "MGB076",
                Phase::Emit,
                "backend returned an output with a different frozen cache identity",
            ));
        }
        let state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        let resolution = state
            .linked
            .get(target_name)
            .map(|value| value.1.clone())
            .ok_or_else(|| {
                ManagerError::new("MGB130", Phase::Emit, "link resolution evidence is absent")
            })?;
        let compiled = state.compiled.clone();
        drop(state);
        let link_trace = make_link_trace(&target.entry, &resolution, &compiled)?;
        let bundle = debug_bundle(
            document,
            &mut output,
            &linked.image,
            &link_trace,
            &compiled,
            &self.prepared.sources,
        );
        let debug = VerifiedBlob::from_owned(
            encode_debug_bundle(&bundle).map_err(|e| error("MGB078", Phase::Emit, e))?,
        );
        let product = VerifiedBlob::from_owned(output.bytes);
        self.store_verified(&product, "MGB077", Phase::Cache)?;
        self.store_verified(&debug, "MGB079", Phase::Cache)?;
        let kind = product_kind(&target.resolved.backend);
        let produced = vec![
            produced_verified("prompt", kind.clone(), &product),
            produced_verified("backend-result", ArtifactKind::DebugInfo, &debug),
        ];
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .insert_backend(
                target_name.into(),
                BackendStage {
                    product,
                    debug,
                    kind,
                },
            );
        Ok((produced, Vec::new()))
    }

    fn publish_target(
        &self,
        target_name: &str,
        inputs: &ResolvedInputs,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let target = self.target(target_name)?;
        let state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        let source_target = if state.backend.contains_key(target_name) {
            target_name.to_owned()
        } else {
            let expected = inputs
                .iter()
                .map(|(_, outputs)| outputs)
                .find(|outputs| {
                    outputs.len() == 2
                        && outputs
                            .iter()
                            .any(|output| output.name.as_str() == "prompt")
                        && outputs
                            .iter()
                            .any(|output| output.name.as_str() == "backend-result")
                })
                .ok_or_else(|| {
                    ManagerError::new(
                        "MGB104",
                        Phase::Publish,
                        "complete backend output manifest is absent",
                    )
                })?;
            let key = (
                expected
                    .iter()
                    .find(|output| output.name.as_str() == "prompt")
                    .unwrap()
                    .digest
                    .clone(),
                expected
                    .iter()
                    .find(|output| output.name.as_str() == "backend-result")
                    .unwrap()
                    .digest
                    .clone(),
            );
            let name = state.backend_by_digest.get(&key).ok_or_else(|| {
                ManagerError::new(
                    "MGB104",
                    Phase::Publish,
                    "backend product cannot be restored",
                )
            })?;
            let stage = &state.backend[name];
            let actual = [
                produced_verified("prompt", stage.kind.clone(), &stage.product),
                produced_verified("backend-result", ArtifactKind::DebugInfo, &stage.debug),
            ];
            if actual.as_slice() != expected {
                return Err(ManagerError::new(
                    "MGB104",
                    Phase::Publish,
                    "backend output schema disagrees with input evidence",
                ));
            }
            name.clone()
        };
        let output = state.backend[&source_target].clone();
        let compiled = state.compiled.clone();
        let link_map = state
            .link_maps
            .get(target_name)
            .or_else(|| state.link_maps.get(&source_target))
            .cloned()
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB120",
                    Phase::Publish,
                    "static link-map evidence is absent",
                )
            })?;
        drop(state);
        let prompt_digest = output.product.digest().clone();
        let mut publications = Vec::new();
        let mut publication_blobs = Vec::new();
        if has_emit(&self.prepared.emit, EmitKind::BinaryIr) {
            for (source, unit) in &compiled {
                let digest = unit.bytes.digest().clone();
                publication_blobs.push(unit.bytes.clone());
                publications.push(publication(
                    &format!("{target_name}:ir-{}", publications.len()),
                    ArtifactKind::BinaryIr,
                    &ir_destination(&target.resolved.output, source),
                    digest,
                    unit.bytes.bytes().len() as u64,
                    self.prepared.repository.root(),
                )?);
            }
        }
        if has_emit(&self.prepared.emit, EmitKind::Prompt)
            || has_emit(&self.prepared.emit, EmitKind::DebugInfo)
        {
            publication_blobs.push(output.product.clone());
            publications.push(publication(
                &format!("{target_name}:prompt"),
                product_kind(&target.resolved.backend),
                &target.resolved.output,
                prompt_digest,
                output.product.bytes().len() as u64,
                self.prepared.repository.root(),
            )?);
        }
        if has_emit(&self.prepared.emit, EmitKind::DebugInfo)
            && !matches!(target.resolved.backend.as_str(), "pack" | "sopack")
        {
            let bytes = &output.debug;
            let digest = bytes.digest().clone();
            publication_blobs.push(bytes.clone());
            publications.push(publication(
                &format!("{target_name}:debug"),
                ArtifactKind::DebugInfo,
                &target.resolved.output.with_extension("psdbg"),
                digest,
                bytes.bytes().len() as u64,
                self.prepared.repository.root(),
            )?);
        }
        let link_map_digest = link_map.digest().clone();
        publication_blobs.push(link_map.clone());
        publications.push(publication(
            &format!("{target_name}:static-link-map"),
            ArtifactKind::Other("static-link-map".into()),
            &target.resolved.output.with_extension("xsmap"),
            link_map_digest,
            link_map.bytes().len() as u64,
            self.prepared.repository.root(),
        )?);
        let target_record = target_record_bytes(
            target_name,
            self.prepared.snapshot.manifest_digest(),
            &publications,
        )?;
        let target_record = VerifiedBlob::from_owned(target_record);
        let record_digest = self.store_verified(&target_record, "MGB118", Phase::Cache)?;
        publication_blobs.push(target_record.clone());
        publications.push(publication(
            &format!("{target_name}:target-record"),
            ArtifactKind::Metadata,
            &target.resolved.output.with_extension("build.json"),
            record_digest,
            target_record.bytes().len() as u64,
            self.prepared.repository.root(),
        )?);
        publications.sort_by(|left, right| left.output.name.cmp(&right.output.name));
        let target_id = PublicationTargetId::new(target_name).map_err(|_| {
            ManagerError::new(
                "MGB119",
                Phase::Publish,
                "invalid target publication identity",
            )
        })?;
        let generation = self
            .runtime
            .publish_generation_verified(
                GenerationSpace::TargetArtifacts,
                &target_id,
                &publications,
                &publication_blobs,
            )
            .map_err(|cause| ManagerError::new(cause.code(), Phase::Publish, cause.message()))?;
        let published = PublishedTarget {
            target: TargetName::new(target.name.clone()).expect("validated target name"),
            target_id: generation.identity.target.to_string(),
            generation_id: generation.identity.generation.to_hex(),
            artifacts: generation
                .artifacts
                .iter()
                .map(|item| PublishedArtifact {
                    id: item.descriptor.id.clone(),
                    kind: item.descriptor.kind.clone(),
                    locator: squish_protocol::ProjectFilePath::new(item.path.as_str())
                        .expect("publication path is canonical"),
                    size: item.descriptor.size,
                    digest: item.descriptor.digest.clone(),
                })
                .collect(),
        };
        let recorded = recorded_generation(&generation);
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .generations
            .push(recorded);
        let events = generation
            .artifacts
            .iter()
            .map(protocol_artifact)
            .map(ActionEvent::Artifact)
            .collect();
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .published
            .push(published);
        Ok((Vec::new(), events))
    }

    fn hydrate_cached(
        &self,
        work: &BuildWork,
        _inputs: &ResolvedInputs,
        verified: &VerifiedAction,
    ) -> Result<(), ManagerError> {
        match work {
            BuildWork::Compile { source } => {
                let bytes = self.cached_bytes(verified, "xsir")?;
                let frozen = self
                    .prepared
                    .source_indices
                    .get(source)
                    .and_then(|index| self.prepared.sources.get(*index))
                    .ok_or_else(|| {
                        ManagerError::new(
                            "MGB111",
                            Phase::Cache,
                            "cached source is absent from the frozen snapshot",
                        )
                    })?;
                let validated = cached_unit_proof(frozen, &bytes)?;
                let unit = Arc::clone(validated.unit());
                if &header(&unit).source != source
                    || header(&unit).frontend_abi.0 != self.prepared.runtime_descriptor.frontend_abi
                {
                    return Err(ManagerError::new(
                        "MGB111",
                        Phase::Cache,
                        "cached unit identity or frontend ABI differs from the frozen snapshot",
                    ));
                }
                let revision = validated.revision().clone();
                let debug = validated.debug_digest();
                let prepared = PreparedUnit::from_validated(validated);
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .compiled
                    .insert(
                        source.clone(),
                        Arc::new(CompiledUnit {
                            unit,
                            prepared,
                            bytes,
                            object: revision.object,
                            semantic: revision.semantic,
                            debug,
                        }),
                    );
            }
            BuildWork::Link { target, .. } => {
                let image = decode_linked_image(self.cached_bytes(verified, "image")?.bytes())
                    .map_err(|e| error("MGB121", Phase::Cache, e))?;
                let map = decode_static_link_map(self.cached_bytes(verified, "map")?.bytes())
                    .map_err(|e| error("MGB122", Phase::Cache, e))?;
                if image.link_map != map {
                    return Err(ManagerError::new(
                        "MGB123",
                        Phase::Cache,
                        "cached link image and map disagree",
                    ));
                }
                let state = self
                    .state
                    .lock()
                    .expect("build state mutex is not poisoned");
                let units = state
                    .compiled
                    .iter()
                    .map(|(key, unit)| (key.clone(), unit.prepared.clone()))
                    .collect();
                let resolution = bind_imports(
                    &self.prepared.snapshot,
                    &self.prepared.locations,
                    &state.compiled,
                    &self.prepared.sources,
                )?;
                drop(state);
                let program =
                    squish_link::LinkedProgram::reconstruct_prepared(image.clone(), units)
                        .map_err(|e| error("MGB124", Phase::Cache, e))?;
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .insert_linked(
                        target.clone(),
                        self.cached_bytes(verified, "image")?.digest().clone(),
                        Arc::new(LinkOutput {
                            map,
                            image,
                            program,
                        }),
                        Arc::new(resolution),
                        self.cached_bytes(verified, "map")?,
                    );
            }
            BuildWork::Instantiate { target } => {
                let document =
                    decode_linked_document(self.cached_bytes(verified, "document")?.bytes())
                        .map_err(|e| error("MGB125", Phase::Cache, e))?;
                let trace = decode_expansion_trace(self.cached_bytes(verified, "trace")?.bytes())
                    .map_err(|e| error("MGB126", Phase::Cache, e))?;
                let directives = if is_archive_backend(&self.target(target)?.resolved.backend) {
                    let directives = squish_link::decode_archive_directives(
                        self.cached_bytes(verified, "directives")?.bytes(),
                    )
                    .map_err(|e| error("MGB126", Phase::Cache, e))?;
                    archive_build::validate_directives(&directives, &trace)?;
                    directives
                } else {
                    Vec::new()
                };
                if document.document_abi.0 != self.prepared.runtime_descriptor.document_abi {
                    return Err(ManagerError::new(
                        "MGB125",
                        Phase::Cache,
                        "cached document was produced for a different document ABI",
                    ));
                }
                let identity = (
                    self.cached_bytes(verified, "document")?.digest().clone(),
                    self.cached_bytes(verified, "trace")?.digest().clone(),
                    verified
                        .record
                        .outputs
                        .iter()
                        .find(|output| output.name.as_str() == "directives")
                        .map(|output| output.digest.clone()),
                );
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .insert_instantiated(
                        target.clone(),
                        identity,
                        Arc::new(InstantiateOutput {
                            document,
                            trace,
                            directives,
                        }),
                    );
            }
            BuildWork::Backend { target } => {
                let bytes = self.cached_bytes(verified, "prompt")?;
                let debug = self.cached_bytes(verified, "backend-result")?;
                if matches!(
                    self.target(target)?.resolved.backend.as_str(),
                    "pack" | "sopack"
                ) {
                    return archive_build::hydrate(self, target, bytes, debug);
                }
                let bundle = squish_ir::decode_debug_bundle(debug.bytes())
                    .map_err(|e| error("MGB127", Phase::Cache, e))?;
                if bundle.artifact.digest != ArtifactDigest::of(bytes.bytes())
                    || bundle.artifact.byte_len != bytes.bytes().len() as u64
                {
                    return Err(ManagerError::new(
                        "MGB128",
                        Phase::Cache,
                        "cached prompt differs from backend evidence",
                    ));
                }
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .insert_backend(
                        target.clone(),
                        BackendStage {
                            product: bytes,
                            debug,
                            kind: product_kind(&self.target(target)?.resolved.backend),
                        },
                    );
            }
            BuildWork::Publish { .. } => {
                return Err(ManagerError::new(
                    "MGB129",
                    Phase::Cache,
                    "effect actions cannot be cache hits",
                ));
            }
        }
        Ok(())
    }

    /// Use lookup-owned verified buffers directly, including outputs larger than session caches.
    fn cached_bytes(
        &self,
        verified: &VerifiedAction,
        name: &str,
    ) -> Result<VerifiedBlob, ManagerError> {
        let index = verified
            .record
            .outputs
            .iter()
            .position(|output| output.name.as_str() == name)
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB108",
                    Phase::Cache,
                    format!("cached action has no `{name}` output"),
                )
            })?;
        let output = &verified.record.outputs[index];
        let blob = verified.blobs.get(index).ok_or_else(|| {
            ManagerError::new(
                "MGB110",
                Phase::Cache,
                format!("cached `{name}` verified bytes are absent"),
            )
        })?;
        if blob.bytes().len() as u64 != output.size || blob.digest() != &output.digest {
            return Err(ManagerError::new(
                "MGB110",
                Phase::Cache,
                format!("cached `{name}` blob differs from its declared identity"),
            ));
        }
        Ok(blob.clone())
    }
}

impl WorkExecutor<BuildWork> for BuildExecutor {
    fn lookup(
        &self,
        dispatch: &Dispatch,
        work: &BuildWork,
        _plan: &PreparedPlan<BuildWork>,
        inputs: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError> {
        if !matches!(
            work,
            BuildWork::Compile { .. }
                | BuildWork::Link { .. }
                | BuildWork::Instantiate { .. }
                | BuildWork::Backend { .. }
        ) {
            return Ok(None);
        }
        let Some(verified) = self
            .runtime
            .lookup_action_verified(&dispatch.key)
            .map_err(|cause| ManagerError::new(cause.code(), Phase::Cache, cause.message()))?
        else {
            return Ok(None);
        };
        if verified.record.key != dispatch.key
            || verified.blobs.len() != verified.record.outputs.len()
        {
            return Err(ManagerError::new(
                "MGB110",
                Phase::Cache,
                "cached action identity or verified output count disagrees with dispatch",
            ));
        }
        for (output, blob) in verified.record.outputs.iter().zip(&verified.blobs) {
            if output.digest != *blob.digest() || output.size != blob.bytes().len() as u64 {
                return Err(ManagerError::new(
                    "MGB110",
                    Phase::Cache,
                    "cached output differs from its verified bytes",
                ));
            }
        }
        self.hydrate_cached(work, inputs, &verified)?;
        let record = verified.record;
        Ok(Some(CachedResult {
            digest: cache_manifest_digest(&record.outputs),
            outputs: record.outputs,
            artifacts: Vec::new(),
            cache: squish_protocol::CacheKind::Local,
        }))
    }

    fn execute(
        &self,
        dispatch: &Dispatch,
        work: &BuildWork,
        _plan: &PreparedPlan<BuildWork>,
        inputs: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition {
        if cancellation.is_cancelled() {
            return ActionResult::failure("manager_cancelled", "build was cancelled").into();
        }
        let result = match self.execute_work(dispatch, work, inputs) {
            Ok((outputs, events)) => ActionResult {
                outcome: Ok(()),
                outputs,
                events,
            },
            Err(error) => {
                let mut result = ActionResult::failure(error.code(), error.message());
                if error.source_diagnostic().is_some() {
                    let id =
                        DiagnosticId::new(format!("manager-source-failure-{}", dispatch.action.id))
                            .expect("non-empty action diagnostic identity");
                    result
                        .events
                        .push(ActionEvent::Diagnostic(error.diagnostic(id)));
                }
                result
            }
        };
        result.into()
    }

    fn record(
        &self,
        dispatch: &Dispatch,
        work: &BuildWork,
        result: &ActionResult,
    ) -> Result<(), ManagerError> {
        if work.effect() == Effect::Transform && result.outcome.is_ok() {
            self.runtime
                .record_action(&ActionRecord {
                    key: dispatch.key.clone(),
                    outputs: result.outputs.clone(),
                })
                .map_err(|cause| ManagerError::new(cause.code(), Phase::Cache, cause.message()))?;
        }
        Ok(())
    }
}

fn build_action_fact(fact: &ActionExecutionFact) -> BuildActionFact {
    BuildActionFact {
        action: fact.action.clone(),
        kind: fact.kind,
        dependencies: fact.dependencies.clone(),
        state: match &fact.state {
            ExecutionState::Declared => BuildTerminalState::Declared,
            ExecutionState::Succeeded => BuildTerminalState::Succeeded,
            ExecutionState::Failed { code, message } => BuildTerminalState::Failed {
                code: code.clone(),
                message: message.clone(),
            },
            ExecutionState::Blocked { dependency } => BuildTerminalState::Blocked {
                dependency: dependency.clone(),
            },
            ExecutionState::Cancelled => BuildTerminalState::Cancelled,
            ExecutionState::Superseded => BuildTerminalState::Superseded,
        },
        key: fact.key.clone(),
        outputs: fact
            .outputs
            .iter()
            .map(|output| BuildOutputFact {
                name: output.name.as_str().into(),
                kind: output.kind.clone(),
                digest: output.digest.clone(),
                size: output.size,
            })
            .collect(),
        source: fact.source.map(|source| match source {
            ResultSource::Worker => BuildResultSource::Worker,
            ResultSource::SingleFlight => BuildResultSource::SingleFlight,
            ResultSource::Cache => BuildResultSource::Cache,
        }),
    }
}

fn expected_output(inputs: &ResolvedInputs, name: &str) -> Option<squish_protocol::Digest> {
    inputs
        .iter()
        .flat_map(|(_, outputs)| outputs)
        .find(|output| output.name.as_str() == name)
        .map(|output| output.digest.clone())
}

/// Companion provenance/directives are never interchangeable merely because document bytes match.
fn instantiation_identity(inputs: &ResolvedInputs) -> Result<InstantiationIdentity, ManagerError> {
    let document = expected_output(inputs, "document")
        .ok_or_else(|| ManagerError::new("MGB103", Phase::Emit, "document input is absent"))?;
    let trace = expected_output(inputs, "trace")
        .ok_or_else(|| ManagerError::new("MGB103", Phase::Emit, "trace input is absent"))?;
    Ok((document, trace, expected_output(inputs, "directives")))
}

fn manifest_map(snapshot: &ProjectSnapshot) -> BTreeMap<String, Manifest> {
    snapshot
        .manifests()
        .iter()
        .map(|item| {
            (
                item.path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                item.manifest.clone(),
            )
        })
        .collect()
}

fn resolution_mode(request: &BuildRequest) -> ResolutionMode {
    use squish_protocol::LockMode;
    match request.lock {
        LockMode::Update => ResolutionMode::Online,
        LockMode::Locked => ResolutionMode::Locked,
        LockMode::Offline => ResolutionMode::Offline,
        LockMode::Frozen => ResolutionMode::Frozen,
    }
}

fn ensure_authoritative_lock(
    repository: &ProjectRepository,
    snapshot: &ProjectSnapshot,
    resolved: &Lockfile,
    mode: ResolutionMode,
) -> Result<(), ManagerError> {
    if snapshot.lockfile() == Some(resolved) {
        return Ok(());
    }
    if matches!(mode, ResolutionMode::Locked | ResolutionMode::Frozen) {
        return Err(ManagerError::new(
            "MGB005",
            Phase::Resolve,
            "locked resolution returned a different lockfile",
        ));
    }
    let root = snapshot
        .manifests()
        .first()
        .expect("snapshot has a root manifest");
    let lock_path = Path::new(squish_project::LOCK_FILE_NAME).to_path_buf();
    let lock_bytes = resolved
        .to_toml()
        .map_err(|e| error("MGB006", Phase::Resolve, e))?
        .into_bytes();
    let files = vec![
        MutationFile {
            path: root.path.clone(),
            expected_digest: root.digest.clone(),
            candidate: root.bytes.to_vec(),
            candidate_digest: ProjectRepository::candidate_digest(&root.path, &root.bytes)
                .map_err(|e| error("MGB007", Phase::Manage, e))?,
        },
        MutationFile {
            path: lock_path.clone(),
            expected_digest: snapshot.lock_digest().to_owned(),
            candidate_digest: ProjectRepository::candidate_digest(&lock_path, &lock_bytes)
                .map_err(|e| error("MGB007", Phase::Manage, e))?,
            candidate: lock_bytes,
        },
    ];
    let plan = MutationPlanner::plan(
        TransactionId(format!(
            "build-lock-{}",
            blake3::hash(snapshot.manifest_digest().as_bytes()).to_hex()
        )),
        MutationKind::AddDependency {
            alias: "__lock_refresh__".into(),
        },
        snapshot.manifest_digest().to_owned(),
        resolved,
        snapshot.read_set().clone(),
        files,
    )
    .map_err(|e| error("MGB008", Phase::Manage, e))?;
    repository
        .commit(&plan)
        .map_err(|e| error("MGB009", Phase::Manage, e))
}

fn freeze_sources(
    snapshot: &ProjectSnapshot,
    locations: &[squish_repository::PackageLocation],
    sopacks: &BTreeMap<String, Arc<AcquiredSopack>>,
) -> Result<Vec<FrozenSource>, ManagerError> {
    let owned = snapshot
        .owned_sources(locations)
        .map_err(|e| error("MGB010", Phase::Analyze, e))?;
    let mut builder = SnapshotBuilder::new(FileSourceProvider);
    let mut loaded = Vec::with_capacity(owned.len());
    for source in owned {
        let blob = builder
            .load(source.id, source.locator)
            .map_err(|e| error("MGB011", Phase::Analyze, e))?;
        loaded.push((blob, source.package));
    }
    let sealed = builder
        .seal()
        .map_err(|e| error("MGB012", Phase::Analyze, e))?;
    let mut sources = loaded
        .into_iter()
        .map(|(blob, package)| {
            let _ = sealed
                .get(blob.id())
                .expect("every successful load is present after seal");
            FrozenSource {
                blob,
                package,
                precompiled: None,
                precompiled_blob: None,
                precompiled_validated: None,
                assets: BTreeMap::new(),
                archive: None,
            }
        })
        .collect::<Vec<_>>();
    archive_build::freeze_inputs(snapshot, &mut sources, sopacks)?;
    // Decode/diagnostic validation has covered the whole immutable library. Only
    // referenced compiled objects need actions/encoded CAS representations.
    archive_reachability::prune(snapshot, locations, &mut sources)?;
    for source in &mut sources {
        if let Some(unit) = &source.precompiled {
            let (validated, bytes) = ValidatedUnit::encode_new(Arc::clone(unit))
                .map_err(|e| error("MGB142", Phase::Analyze, e))?;
            source.precompiled_blob = Some(VerifiedBlob::from_owned(bytes));
            source.precompiled_validated = Some(validated);
        }
    }

    Ok(sources)
}

fn source_key(source: &FrozenSource) -> SourceKey {
    if let Some(unit) = &source.precompiled {
        return header(unit).source.clone();
    }
    SourceKey::Project {
        package: source.package.clone(),
        path: source
            .blob
            .id()
            .path()
            .as_str()
            .split('/')
            .map(str::to_owned)
            .collect(),
    }
}

fn select_targets(
    request: &BuildRequest,
    snapshot: &ProjectSnapshot,
    sources: &[FrozenSource],
    excluded: &BTreeSet<String>,
    locations: &[squish_repository::PackageLocation],
) -> Result<Vec<TargetBuild>, ManagerError> {
    let selected_packages: BTreeSet<_> = match &request.scope {
        WorkspaceScope::Current => snapshot
            .manifests()
            .first()
            .and_then(|m| m.manifest.package.as_ref())
            .map(|p| BTreeSet::from([p.name.clone()]))
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB013",
                    Phase::Analyze,
                    "current scope is ambiguous for a virtual workspace",
                )
            })?,
        WorkspaceScope::Workspace => snapshot
            .manifests()
            .iter()
            .filter_map(|m| m.manifest.package.as_ref().map(|p| p.name.clone()))
            .collect(),
        WorkspaceScope::Packages(names) => names.iter().map(|n| n.as_str().to_owned()).collect(),
    };
    let packages = snapshot
        .resolved_packages(locations)
        .map_err(|e| error("MGB014", Phase::Analyze, e))?;
    let requested: BTreeSet<_> = request.targets.iter().map(|n| n.as_str()).collect();
    let mut out = Vec::new();
    for manifest in snapshot.manifests() {
        let Some(package) = &manifest.manifest.package else {
            continue;
        };
        if !selected_packages.contains(&package.name) || excluded.contains(&package.name) {
            continue;
        }
        for name in manifest.manifest.targets.keys() {
            if !requested.is_empty() && !requested.contains(name.as_str()) {
                continue;
            }
            let profile = manifest
                .manifest
                .profiles
                .contains_key(request.profile.as_str())
                .then_some(request.profile.as_str());
            let resolved = snapshot
                .resolve_target(&package.name, name, profile)
                .map_err(|e| error("MGB014", Phase::Analyze, e))?;
            if !matches!(
                resolved.backend.as_str(),
                "squish" | "prompt" | "pack" | "sopack"
            ) {
                return Err(ManagerError::new(
                    "MGB015",
                    Phase::Emit,
                    format!("unsupported backend `{}`", resolved.backend),
                ));
            }
            let logical = LogicalPath::new(
                resolved
                    .entry
                    .strip_prefix(&resolved.package_dir)
                    .map_err(|_| {
                        ManagerError::new(
                            "MGB016",
                            Phase::Analyze,
                            "target entry escapes its package",
                        )
                    })?
                    .to_string_lossy(),
            )
            .map_err(|e| error("MGB016", Phase::Analyze, e))?;
            let owner = packages
                .iter()
                .find(|p| {
                    p.instance.package_name == package.name
                        && p.locator.as_path() == resolved.package_dir
                        && matches!(p.instance.source_kind, 3 | 4)
                })
                .ok_or_else(|| {
                    ManagerError::new(
                        "MGB017",
                        Phase::Analyze,
                        "target owner is absent from the exact lock mapping",
                    )
                })?;
            let entry = sources
                .iter()
                .find(|s| s.package == owner.instance && s.blob.id().path() == &logical)
                .map(source_key)
                .ok_or_else(|| {
                    ManagerError::new(
                        "MGB017",
                        Phase::Analyze,
                        format!(
                            "target `{name}` entry was not included in the sealed source snapshot"
                        ),
                    )
                })?;
            let mut arguments = resolved.args.clone();
            for (key, value) in &request.arguments {
                let raw = key.as_str();
                if let Some((scope, arg)) = raw.split_once('.') {
                    if scope == name {
                        arguments.insert(arg.into(), value.clone());
                    }
                } else if out.is_empty() && request.targets.len() <= 1 {
                    arguments.insert(raw.into(), value.clone());
                }
            }
            out.push(TargetBuild {
                package: package.name.clone(),
                name: name.clone(),
                entry,
                resolved,
                arguments,
            });
        }
    }
    if out.is_empty() {
        return Err(ManagerError::new(
            "MGB018",
            Phase::Analyze,
            "build selection contains no targets",
        ));
    }
    Ok(out)
}

fn build_plan(
    snapshot: &ProjectSnapshot,
    sources: &[FrozenSource],
    targets: &[TargetBuild],
    emit: &[EmitKind],
    backend_identities: &BTreeMap<String, BackendCacheIdentity>,
    descriptor: &BuildRuntimeDescriptor,
) -> Result<PreparedPlan<BuildWork>, ManagerError> {
    let mut actions = Vec::new();
    let mut work = BTreeMap::new();
    let mut compile_ids = Vec::new();
    for source in sources {
        let key = source_key(source);
        let id = action_id("compile", &canonical_source_key(&key));
        compile_ids.push(id.clone());
        add_action(
            &mut actions,
            &mut work,
            id,
            BuildWork::Compile {
                source: key.clone(),
            },
            Vec::new(),
            vec![Output {
                name: output_name("xsir"),
                kind: ArtifactKind::BinaryIr,
            }],
            (
                archive_build::compile_inputs(source)?,
                BTreeMap::from([
                    ("source-key".into(), canonical_source_key(&key)),
                    ("frontend-abi".into(), descriptor.frontend_abi.clone()),
                    ("manager-schema".into(), "v1.2.1".into()),
                ]),
            ),
        )?;
    }
    for target in targets {
        let identity = format!("{}:{}", target.package, target.name);
        let link = action_id("link", &identity);
        let link_options = BTreeMap::from([
            ("entry".into(), canonical_source_key(&target.entry)),
            ("linker-abi".into(), descriptor.linker_abi.clone()),
            (
                "resolution-fingerprint".into(),
                resolution_fingerprint(snapshot, sources),
            ),
        ]);
        let link_inputs: Vec<_> = compile_ids
            .iter()
            .cloned()
            .map(|action| {
                InputRef::Output(OutputRef {
                    action,
                    output: output_name("xsir"),
                })
            })
            .collect();
        add_action(
            &mut actions,
            &mut work,
            link.clone(),
            BuildWork::Link {
                target: identity.clone(),
                entry: target.entry.clone(),
            },
            compile_ids.clone(),
            vec![
                Output {
                    name: output_name("image"),
                    kind: ArtifactKind::Metadata,
                },
                Output {
                    name: output_name("map"),
                    kind: ArtifactKind::Metadata,
                },
            ],
            (link_inputs, link_options),
        )?;
        let instantiate = action_id("instantiate", &identity);
        let instantiate_inputs = vec![InputRef::Output(OutputRef {
            action: link.clone(),
            output: output_name("image"),
        })];
        let mut instantiate_outputs = vec![
            Output {
                name: output_name("document"),
                kind: ArtifactKind::Metadata,
            },
            Output {
                name: output_name("trace"),
                kind: ArtifactKind::DebugInfo,
            },
        ];
        if is_archive_backend(&target.resolved.backend) {
            instantiate_outputs.push(Output {
                name: output_name("directives"),
                kind: ArtifactKind::Metadata,
            });
        }
        add_action(
            &mut actions,
            &mut work,
            instantiate.clone(),
            BuildWork::Instantiate {
                target: identity.clone(),
            },
            vec![link.clone()],
            instantiate_outputs,
            (instantiate_inputs, instantiate_options(target, descriptor)),
        )?;
        let backend = action_id("backend", &identity);
        let mut backend_inputs = vec![
            InputRef::Output(OutputRef {
                action: instantiate.clone(),
                output: output_name("document"),
            }),
            InputRef::Output(OutputRef {
                action: instantiate.clone(),
                output: output_name("trace"),
            }),
        ];
        if is_archive_backend(&target.resolved.backend) {
            backend_inputs.push(InputRef::Output(OutputRef {
                action: instantiate.clone(),
                output: output_name("directives"),
            }));
        }
        backend_inputs.push(InputRef::Output(OutputRef {
            action: link.clone(),
            output: output_name("image"),
        }));
        backend_inputs.push(InputRef::Output(OutputRef {
            action: link.clone(),
            output: output_name("map"),
        }));
        backend_inputs.extend(
            sources
                .iter()
                .flat_map(|s| s.assets.values())
                .map(|bytes| InputRef::Blob(protocol_blake3(bytes))),
        );
        let mut backend_dependencies = vec![instantiate, link];
        if matches!(target.resolved.backend.as_str(), "pack" | "sopack") {
            backend_dependencies.extend(compile_ids.iter().cloned());
            backend_inputs.extend(compile_ids.iter().cloned().map(|action| {
                InputRef::Output(OutputRef {
                    action,
                    output: output_name("xsir"),
                })
            }));
        }
        add_action(
            &mut actions,
            &mut work,
            backend.clone(),
            BuildWork::Backend {
                target: identity.clone(),
            },
            backend_dependencies,
            vec![
                Output {
                    name: output_name("prompt"),
                    kind: product_kind(&target.resolved.backend),
                },
                Output {
                    name: output_name("backend-result"),
                    kind: ArtifactKind::DebugInfo,
                },
            ],
            (
                backend_inputs,
                backend_options(
                    backend_identities
                        .get(&identity)
                        .expect("every target identity is frozen"),
                ),
            ),
        )?;
        let publish = action_id("publish", &identity);
        let publish_inputs = vec![
            InputRef::Output(OutputRef {
                action: backend.clone(),
                output: output_name("prompt"),
            }),
            InputRef::Output(OutputRef {
                action: backend.clone(),
                output: output_name("backend-result"),
            }),
        ];
        add_action(
            &mut actions,
            &mut work,
            publish,
            BuildWork::Publish {
                target: identity.clone(),
            },
            vec![backend],
            vec![],
            (
                publish_inputs,
                BTreeMap::from([
                    (
                        "destination".into(),
                        target.resolved.output.to_string_lossy().into_owned(),
                    ),
                    ("emit".into(), canonical_emit_set(emit)),
                ]),
            ),
        )?;
    }
    let graph = BuildPlan::new(actions).map_err(|e| error("MGB020", Phase::Manage, e))?;
    PreparedPlan::new(graph, work).map_err(|e| error("MGB021", Phase::Manage, e))
}

fn add_action(
    actions: &mut Vec<Action>,
    work: &mut BTreeMap<ActionId, BuildWork>,
    id: ActionId,
    item: BuildWork,
    dependencies: Vec<ActionId>,
    outputs: Vec<Output>,
    key_parts: (Vec<InputRef>, BTreeMap<String, String>),
) -> Result<(), ManagerError> {
    let kind = item.kind();
    let resources = match kind {
        ActionKind::Link => Resources::new(1, 0, 64 * 1024 * 1024),
        ActionKind::Publish => Resources::new(0, 1, 0),
        _ => Resources::new(1, 0, 8 * 1024 * 1024),
    };
    let class = if kind == ActionKind::Publish {
        ResourceClass::Io
    } else if kind == ActionKind::Link {
        ResourceClass::Memory
    } else {
        ResourceClass::Cpu
    };
    let key = KeyRecipe::new(format!("manager/{kind:?}/1.2.1"), key_parts.0, key_parts.1)
        .map_err(|e| error("MGB022", Phase::Manage, e))?;
    actions.push(Action {
        id: id.clone(),
        key,
        kind,
        class,
        resources,
        dependencies,
        outputs,
    });
    work.insert(id, item);
    Ok(())
}

fn instantiate_options(
    target: &TargetBuild,
    descriptor: &BuildRuntimeDescriptor,
) -> BTreeMap<String, String> {
    let limits = budgets(&target.resolved);
    let mut options = BTreeMap::from([
        ("evaluator-abi".into(), descriptor.evaluator_abi.clone()),
        ("document-abi".into(), descriptor.document_abi.clone()),
        ("max-depth".into(), limits.max_depth.to_string()),
        ("max-expansions".into(), limits.max_expansions.to_string()),
        (
            "max-output-bytes".into(),
            limits.max_output_bytes.to_string(),
        ),
    ]);
    for (name, value) in &target.arguments {
        options.insert(format!("arg.{name}"), value.clone());
    }
    options
}

fn backend_options(identity: &BackendCacheIdentity) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("backend-id".into(), identity.backend_id.into()),
        ("backend-version".into(), identity.backend_version.into()),
        ("canonical-options".into(), hex(&identity.canonical_options)),
    ])
}

fn freeze_backend_identities(
    targets: &[TargetBuild],
    runtime: &dyn BuildRuntime,
) -> Result<BTreeMap<String, BackendCacheIdentity>, ManagerError> {
    targets
        .iter()
        .map(|target| {
            let target_name = format!("{}:{}", target.package, target.name);
            if matches!(target.resolved.backend.as_str(), "pack" | "sopack") {
                return Ok((target_name, archive_build::identity(target)));
            }
            let identity = runtime
                .backend_identity(SquishOptions {
                    max_output_bytes: budgets(&target.resolved).max_output_bytes,
                })
                .map_err(|error| ManagerError::new(error.code(), Phase::Manage, error.message()))?;
            Ok((target_name, identity))
        })
        .collect()
}

fn resolution_fingerprint(snapshot: &ProjectSnapshot, sources: &[FrozenSource]) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish-manager-resolution\0v1");
    hash_field(&mut hash, snapshot.manifest_digest().as_bytes());
    hash_field(&mut hash, snapshot.lock_digest().as_bytes());
    let mut keys: Vec<_> = sources.iter().map(source_key).collect();
    keys.sort();
    for source in keys {
        let mut encoded = Vec::new();
        encode_source_key(&mut encoded, &source);
        hash_field(&mut hash, &encoded);
    }
    format!("blake3:{}", hash.finalize().to_hex())
}

fn canonical_source_key(source: &SourceKey) -> String {
    let mut bytes = Vec::new();
    encode_source_key(&mut bytes, source);
    hex(&bytes)
}

fn encode_source_key(bytes: &mut Vec<u8>, source: &SourceKey) {
    match source {
        SourceKey::AdHoc { uri } => {
            bytes.push(0);
            encode_field(bytes, uri.as_bytes());
        }
        SourceKey::Project { package, path } => {
            bytes.push(1);
            bytes.extend_from_slice(&package.source_kind.to_le_bytes());
            encode_field(bytes, package.canonical_source.as_bytes());
            encode_field(bytes, package.package_name.as_bytes());
            encode_field(bytes, package.exact_revision.as_bytes());
            bytes.extend_from_slice(&(path.len() as u64).to_le_bytes());
            for component in path {
                encode_field(bytes, component.as_bytes());
            }
        }
    }
}

fn encode_field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

fn hash_field(hash: &mut blake3::Hasher, value: &[u8]) {
    hash.update(&(value.len() as u64).to_le_bytes());
    hash.update(value);
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

fn bind_imports(
    snapshot: &ProjectSnapshot,
    locations: &[squish_repository::PackageLocation],
    units: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
    sources: &[FrozenSource],
) -> Result<ResolutionSnapshot, ManagerError> {
    let lock = snapshot
        .lockfile()
        .expect("prepare requires authoritative lock");
    let locked_manifests: BTreeMap<_, _> = snapshot
        .locked_manifests()
        .iter()
        .map(|m| (m.lock_id.as_str(), &m.manifest))
        .collect();
    let package_by_instance: BTreeMap<_, _> = snapshot
        .resolved_packages(locations)
        .map_err(|e| error("MGB049", Phase::Link, e))?
        .into_iter()
        .map(|p| (p.instance, p.lock_id))
        .collect();
    let archives = sources
        .iter()
        .filter_map(|s| s.archive.as_ref().map(|a| (s.package.clone(), a)))
        .collect::<BTreeMap<_, _>>();
    let archived_imports = archives
        .values()
        .flat_map(|a| &a.imports)
        .map(|b| ((b.importer.clone(), b.import.0), b))
        .collect::<BTreeMap<_, _>>();
    let context = FrozenImportContext {
        lock,
        manifests: &locked_manifests,
        instances: &package_by_instance,
        units,
        archives: &archives,
    };
    let mut revisions = Vec::new();
    let mut imports = Vec::new();
    for (source, compiled) in units {
        let header = header(&compiled.unit);
        revisions.push((
            source.clone(),
            UnitRevision {
                kind: unit_kind(&compiled.unit),
                semantic: compiled.semantic,
                object: compiled.object,
            },
        ));
        for import in &header.imports {
            if let Some(binding) = archived_imports.get(&(source.clone(), import.local_id.0)) {
                imports.push((*binding).clone());
                continue;
            }
            let target = resolve_import(source, &import.spec, import.expected_kind, &context)?;
            imports.push(ImportBinding {
                importer: source.clone(),
                import: import.local_id,
                target,
            });
        }
    }
    revisions.sort_by(|a, b| a.0.cmp(&b.0));
    imports.sort_by(|a, b| (&a.importer, a.import.0).cmp(&(&b.importer, b.import.0)));
    Ok(ResolutionSnapshot {
        units: revisions,
        imports,
    })
}

/// Exact, frozen provider indexes shared by every import-resolution decision.
struct FrozenImportContext<'a> {
    /// Authoritative dependency edges and immutable source identities.
    lock: &'a Lockfile,
    /// Parsed package export declarations indexed by exact lock node.
    manifests: &'a BTreeMap<&'a str, &'a Manifest>,
    /// Compiler package instances mapped to exact dependency lock nodes.
    instances: &'a BTreeMap<squish_ir::PackageInstanceId, String>,
    /// Uniform compiled objects from local sources and reusable archives.
    units: &'a BTreeMap<SourceKey, Arc<CompiledUnit>>,
    /// Immutable archive export tables indexed by their provider identity.
    archives: &'a BTreeMap<
        squish_ir::PackageInstanceId,
        &'a Arc<squish_backend::archive::SharedSopackPayload>,
    >,
}

/// Binds a declaration using one coherent frozen provider context.
fn resolve_import(
    source: &SourceKey,
    spec: &ImportSpec,
    expected: UnitKind,
    context: &FrozenImportContext<'_>,
) -> Result<SourceKey, ManagerError> {
    let FrozenImportContext {
        lock,
        manifests,
        instances,
        units,
        archives,
    } = context;
    let SourceKey::Project { package, path } = source else {
        return Err(ManagerError::new(
            "MGB050",
            Phase::Link,
            "ad-hoc source cannot participate in project import binding",
        ));
    };
    let target = match spec {
        ImportSpec::RelativeUri(relative) => {
            let mut joined = path[..path.len().saturating_sub(1)].join("/");
            if !joined.is_empty() {
                joined.push('/');
            }
            joined.push_str(relative);
            let logical = LogicalPath::new(joined).map_err(|e| error("MGB051", Phase::Link, e))?;
            SourceKey::Project {
                package: package.clone(),
                path: logical.as_str().split('/').map(str::to_owned).collect(),
            }
        }
        ImportSpec::PackageExport {
            dependency_alias,
            export,
        } => {
            let importer_id = instances.get(package).ok_or_else(|| {
                ManagerError::new(
                    "MGB052",
                    Phase::Link,
                    "importer package is absent from lock mapping",
                )
            })?;
            let importer = lock
                .packages
                .iter()
                .find(|p| &p.id == importer_id)
                .expect("mapped lock node exists");
            let dependency_id = importer.dependencies.get(dependency_alias).ok_or_else(|| {
                ManagerError::new(
                    "MGB053",
                    Phase::Link,
                    format!(
                        "dependency alias `{dependency_alias}` is not a direct locked dependency"
                    ),
                )
            })?;
            let dependency = lock
                .packages
                .iter()
                .find(|p| &p.id == dependency_id)
                .expect("lock validation guarantees dependency");
            let manifest = manifests.get(dependency_id.as_str()).ok_or_else(|| {
                ManagerError::new(
                    "MGB054",
                    Phase::Link,
                    "dependency manifest is absent from authoritative snapshot",
                )
            })?;
            if let Some(key) = instances.iter().find_map(|(instance, id)| {
                (id == dependency_id)
                    .then(|| archives.get(instance))
                    .flatten()
                    .and_then(|archive| archive.exports.get(export))
            }) {
                return validate_import_kind(key.clone(), expected, units);
            }
            let path = manifest.exports.get(export).ok_or_else(|| {
                ManagerError::new(
                    "MGB055",
                    Phase::Link,
                    format!("package `{}` does not export `{export}`", dependency.name),
                )
            })?;
            let package = instances
                .iter()
                .find_map(|(instance, id)| (id == dependency_id).then(|| instance.clone()))
                .ok_or_else(|| {
                    ManagerError::new(
                        "MGB056",
                        Phase::Link,
                        "dependency package identity is absent",
                    )
                })?;
            let logical = LogicalPath::new(path.to_string_lossy())
                .map_err(|e| error("MGB057", Phase::Link, e))?;
            SourceKey::Project {
                package,
                path: logical.as_str().split('/').map(str::to_owned).collect(),
            }
        }
        ImportSpec::AbsoluteFileUri(uri) => {
            return Err(ManagerError::new(
                "MGB058",
                Phase::Link,
                format!("absolute import `{uri}` is not portable in project mode"),
            ));
        }
    };
    validate_import_kind(target, expected, units)
}

/// Matches the declaration's semantic kind, independent of the source provider.
fn validate_import_kind(
    target: SourceKey,
    expected: UnitKind,
    units: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
) -> Result<SourceKey, ManagerError> {
    match units.get(&target).map(|u| unit_kind(&u.unit)) {
        Some(kind)
            if kind == expected || (expected == UnitKind::Module && kind == UnitKind::Sopack) =>
        {
            Ok(target)
        }
        Some(kind) => Err(ManagerError::new(
            "MGB059",
            Phase::Link,
            format!("reference requires {expected:?}, but resolves to {kind:?}"),
        )),
        None => Err(ManagerError::new(
            "MGB060",
            Phase::Link,
            format!("import target `{target:?}` is absent from the sealed snapshot"),
        )),
    }
}

fn make_link_trace(
    entry: &SourceKey,
    snapshot: &ResolutionSnapshot,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
) -> Result<LinkTrace, ManagerError> {
    let entry_object = compiled[entry].object;
    let mut imports = Vec::new();
    for binding in &snapshot.imports {
        let importer = &compiled[&binding.importer];
        let declaration = header(&importer.unit)
            .imports
            .iter()
            .find(|d| d.local_id == binding.import)
            .expect("binding was made from declaration");
        let local = origins(&importer.unit)
            .entries
            .iter()
            .enumerate()
            .find_map(|(index, item)| {
                (item.entity_kind == EntityKind::Import && item.local_id == binding.import.0)
                    .then_some(OriginId(index as u32))
            })
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB080",
                    Phase::Link,
                    "import declaration has no static origin",
                )
            })?;
        imports.push(LinkImportRecord {
            importer: binding.importer.clone(),
            import_id: binding.import,
            spec: declaration.spec.clone(),
            resolved_source: binding.target.clone(),
            resolved_object: compiled[&binding.target].object,
            origin: QualifiedOriginRef {
                object: importer.object,
                local,
            },
        });
    }
    let snapshot_digest = LinkKeyProjection {
        linker_abi: "xmlsquish.link/1".into(),
        entry: entry.clone(),
        resolution: snapshot.clone(),
    }
    .digest();
    Ok(LinkTrace {
        entry_object,
        resolution_snapshot: snapshot_digest,
        imports,
        symbols: Vec::new(),
        diagnostics: Vec::new(),
    })
}

fn debug_bundle(
    document: &squish_ir::LinkedDocumentIr,
    output: &mut squish_backend::BackendOutput,
    image: &squish_ir::LinkedImage,
    link_trace: &LinkTrace,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
    sources: &[FrozenSource],
) -> DebugBundle {
    // These backend-owned results are not used after debug serialization. Move them,
    // preserving the shared instantiation trace without another full provenance copy.
    let mut trace = std::mem::take(&mut output.trace);
    let mut artifact_map = std::mem::take(&mut output.byte_map);
    retain_backend_origin_dag(&mut trace, &mut artifact_map, image, compiled);
    let mut archives = Vec::new();
    let mut blobs = BTreeMap::new();
    let mut source_bytes: BTreeMap<SourceKey, &[u8]> = sources
        .iter()
        .map(|source| (source_key(source), source.blob.bytes()))
        .collect();
    let archive_index: BTreeMap<_, _> = sources
        .iter()
        .filter_map(|source| {
            source
                .archive
                .as_ref()
                .map(|archive| (&source.package, archive))
        })
        .collect();
    for archive in archive_index.values() {
        for (key, bytes) in &archive.sources {
            source_bytes.entry(key.clone()).or_insert(bytes.as_ref());
        }
    }
    for unit in compiled.values() {
        let (archive, origins) = archive_and_origins(&unit.unit);
        archives.push(SourceArchiveReference {
            object: unit.object,
            debug_digest: unit.debug,
            archive: archive.clone(),
            origins: origins.clone(),
        });
        for record in &archive.records {
            if let Some(bytes) = source_bytes.get(&record.key).map(|bytes| bytes.to_vec()) {
                blobs.insert(
                    (record.exact_bytes.digest, record.exact_bytes.byte_len),
                    BundledSourceBlob {
                        reference: record.exact_bytes.clone(),
                        bytes,
                    },
                );
            }
        }
    }
    archives.sort_by_key(|a| a.object);
    let source_blobs = blobs.into_values().collect();
    let image_bytes = encode_linked_image(image);
    let document_bytes = encode_linked_document(document);
    DebugBundle {
        schema: squish_ir::Version { major: 1, minor: 0 },
        feature_bits: squish_ir::FeatureBits(0),
        artifact: ArtifactIdentity {
            digest: ArtifactDigest::of(&output.bytes),
            byte_len: output.bytes.len() as u64,
        },
        document_digest: DocumentDigest::of(&document_bytes),
        linked_image: LinkedImageDigest::of(&image_bytes),
        expansion_trace_digest: DebugDigest::of(&encode_expansion_trace(&trace)),
        link_trace_digest: DebugDigest::of(&encode_link_trace(link_trace)),
        document: document.clone(),
        expansion_trace: trace,
        link_trace: link_trace.clone(),
        artifact_map,
        source_archives: archives,
        source_blobs,
    }
}

fn retain_backend_origin_dag(
    trace: &mut squish_ir::ExpansionTrace,
    artifact_map: &mut squish_ir::ArtifactByteMap,
    image: &squish_ir::LinkedImage,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
) {
    let mut closed = BTreeMap::new();
    for entry in &mut artifact_map.entries {
        if origin_reaches_static_node(entry.origin, trace, &mut BTreeSet::new()) {
            continue;
        }
        if let Some(origin) = closed.get(&entry.origin) {
            entry.origin = *origin;
            continue;
        }
        let mut origins = Vec::new();
        collect_static_origins_for_dynamic_node(
            entry.origin,
            trace,
            image,
            compiled,
            &mut BTreeSet::new(),
            &mut origins,
        );
        if origins.is_empty() {
            continue;
        }
        let Some(backend_step) =
            backend_step_for_dynamic_node(entry.origin, trace, &mut BTreeSet::new())
        else {
            continue;
        };
        let mut inputs = vec![squish_ir::OriginEdge {
            role: squish_ir::OriginRole::Input,
            parent: entry.origin,
        }];
        for origin in origins {
            let source = squish_ir::OriginNodeId(trace.origins.len() as u32);
            trace
                .origins
                .push(squish_ir::OriginNode::SourceSpan { origin });
            inputs.push(squish_ir::OriginEdge {
                role: squish_ir::OriginRole::Transform,
                parent: source,
            });
        }
        let wrapper = squish_ir::OriginNodeId(trace.origins.len() as u32);
        trace.origins.push(squish_ir::OriginNode::BackendTransform {
            backend_step,
            inputs,
        });
        closed.insert(entry.origin, wrapper);
        entry.origin = wrapper;
    }
}

fn backend_step_for_dynamic_node(
    id: squish_ir::OriginNodeId,
    trace: &squish_ir::ExpansionTrace,
    seen: &mut BTreeSet<u32>,
) -> Option<squish_ir::DebugStringId> {
    if !seen.insert(id.0) {
        return None;
    }
    use squish_ir::OriginNode;
    match trace.origins.get(id.0 as usize)? {
        OriginNode::BackendTransform { backend_step, .. } => Some(*backend_step),
        OriginNode::Import { child, .. } | OriginNode::RegexCapture { input: child, .. } => {
            backend_step_for_dynamic_node(*child, trace, seen)
        }
        OriginNode::Concat { ordered_inputs } => ordered_inputs
            .iter()
            .find_map(|parent| backend_step_for_dynamic_node(*parent, trace, seen)),
        OriginNode::Fused { inputs, .. } => inputs
            .iter()
            .find_map(|edge| backend_step_for_dynamic_node(edge.parent, trace, seen)),
        OriginNode::Synthetic {
            nearest: Some(parent),
            ..
        } => backend_step_for_dynamic_node(*parent, trace, seen),
        _ => None,
    }
}

fn collect_static_origins_for_dynamic_node(
    id: squish_ir::OriginNodeId,
    trace: &squish_ir::ExpansionTrace,
    image: &squish_ir::LinkedImage,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
    seen: &mut BTreeSet<u32>,
    output: &mut Vec<QualifiedOriginRef>,
) {
    if !seen.insert(id.0) {
        return;
    }
    use squish_ir::OriginNode;
    let Some(node) = trace.origins.get(id.0 as usize) else {
        return;
    };
    match node {
        OriginNode::SourceSpan { origin } => push_unique_origin(output, origin.clone()),
        OriginNode::Expansion { producer, .. } => {
            if let Some(origin) = producer_static_origin(*producer, image, compiled) {
                push_unique_origin(output, origin);
            }
        }
        OriginNode::Import { child, .. } | OriginNode::RegexCapture { input: child, .. } => {
            collect_static_origins_for_dynamic_node(*child, trace, image, compiled, seen, output);
        }
        OriginNode::Concat { ordered_inputs } => {
            for parent in ordered_inputs {
                collect_static_origins_for_dynamic_node(
                    *parent, trace, image, compiled, seen, output,
                );
            }
        }
        OriginNode::BackendTransform { inputs, .. } | OriginNode::Fused { inputs, .. } => {
            for edge in inputs {
                collect_static_origins_for_dynamic_node(
                    edge.parent,
                    trace,
                    image,
                    compiled,
                    seen,
                    output,
                );
            }
        }
        OriginNode::Synthetic {
            nearest: Some(parent),
            ..
        } => collect_static_origins_for_dynamic_node(*parent, trace, image, compiled, seen, output),
        _ => {}
    }
}

fn push_unique_origin(output: &mut Vec<QualifiedOriginRef>, origin: QualifiedOriginRef) {
    if !output.contains(&origin) {
        output.push(origin);
    }
}

fn origin_reaches_static_node(
    id: squish_ir::OriginNodeId,
    trace: &squish_ir::ExpansionTrace,
    seen: &mut BTreeSet<u32>,
) -> bool {
    if !seen.insert(id.0) {
        return false;
    }
    use squish_ir::OriginNode;
    match trace.origins.get(id.0 as usize) {
        Some(OriginNode::SourceSpan { .. } | OriginNode::DecodedSegment { .. }) => true,
        Some(OriginNode::Import { child, .. } | OriginNode::RegexCapture { input: child, .. }) => {
            origin_reaches_static_node(*child, trace, seen)
        }
        Some(OriginNode::Concat { ordered_inputs }) => ordered_inputs
            .iter()
            .any(|parent| origin_reaches_static_node(*parent, trace, seen)),
        Some(OriginNode::BackendTransform { inputs, .. } | OriginNode::Fused { inputs, .. }) => {
            inputs
                .iter()
                .any(|edge| origin_reaches_static_node(edge.parent, trace, seen))
        }
        Some(OriginNode::Synthetic {
            nearest: Some(parent),
            ..
        }) => origin_reaches_static_node(*parent, trace, seen),
        _ => false,
    }
}

fn producer_static_origin(
    producer: squish_ir::LinkedOpRef,
    image: &squish_ir::LinkedImage,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
) -> Option<QualifiedOriginRef> {
    let source = &image.units.get(producer.unit_slot as usize)?.source;
    let unit = compiled.get(source)?;
    let origins = unit.unit.origins();
    let local = origins.entries.iter().position(|entry| {
        entry.entity_kind == EntityKind::Operation && entry.local_id == producer.op.0
    })?;
    Some(QualifiedOriginRef {
        object: unit.object,
        local: OriginId(local as u32),
    })
}

fn publication(
    name: &str,
    kind: ArtifactKind,
    destination: &Path,
    digest: squish_protocol::Digest,
    size: u64,
    root: &Path,
) -> Result<Publication, ManagerError> {
    let relative = destination.strip_prefix(root).map_err(|_| {
        ManagerError::new(
            "MGB090",
            Phase::Publish,
            "artifact destination escapes repository root",
        )
    })?;
    Ok(Publication {
        output: ProducedOutput {
            name: output_name(name),
            kind,
            digest,
            size,
        },
        name: LogicalArtifactName::new(name.rsplit(':').next().unwrap_or(name)).map_err(|_| {
            ManagerError::new("MGB090", Phase::Publish, "invalid logical artifact name")
        })?,
        destination: PublicationPath::new(portable_path(relative)?).map_err(|_| {
            ManagerError::new("MGB090", Phase::Publish, "invalid publication destination")
        })?,
    })
}

fn target_record_bytes(
    target: &str,
    snapshot: &str,
    publications: &[Publication],
) -> Result<Vec<u8>, ManagerError> {
    let artifacts: Vec<_> = publications
        .iter()
        .map(|item| {
            serde_json::json!({
                "destination": item.destination.as_str(),
                "kind": format!("{:?}", item.output.kind),
                "digest": item.output.digest.hex(),
                "size": item.output.size,
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "schema": 1,
        "target": target,
        "snapshot": snapshot,
        "artifacts": artifacts,
    }))
    .map_err(|e| error("MGB120", Phase::Publish, e))
}

fn portable_path(path: &Path) -> Result<String, ManagerError> {
    let mut parts = Vec::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        } else {
            return Err(ManagerError::new(
                "MGB092",
                Phase::Publish,
                "artifact destination is not portable",
            ));
        }
    }
    Ok(parts.join("/"))
}

fn ir_destination(output: &Path, source: &SourceKey) -> std::path::PathBuf {
    let (package, path) = match source {
        SourceKey::Project { package, path } => (package.package_name.as_str(), path.as_slice()),
        SourceKey::AdHoc { .. } => ("adhoc", &[][..]),
    };
    let mut destination = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ir")
        .join(
            output
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("target"),
        )
        .join(package);
    for component in path {
        destination.push(component);
    }
    destination.set_extension("xsir");
    destination
}

fn budgets(target: &ResolvedTarget) -> Budgets {
    let defaults = Budgets::default();
    Budgets {
        max_depth: target.limits.max_depth.unwrap_or(defaults.max_depth),
        max_expansions: target
            .limits
            .max_expansions
            .unwrap_or(defaults.max_expansions),
        max_output_bytes: target
            .limits
            .max_output_bytes
            .unwrap_or(defaults.max_output_bytes),
    }
}

fn header(unit: &RelocatableUnitIr) -> &squish_ir::UnitHeader {
    unit.header()
}
fn origins(unit: &RelocatableUnitIr) -> &squish_ir::OriginTable {
    unit.origins()
}
fn archive_and_origins(
    unit: &RelocatableUnitIr,
) -> (&squish_ir::SourceArchive, &squish_ir::OriginTable) {
    (unit.sources(), unit.origins())
}
fn unit_kind(unit: &RelocatableUnitIr) -> UnitKind {
    unit.kind()
}
/// Archive distribution requires persistent directives; prompt distribution forbids them.
fn is_archive_backend(backend: &str) -> bool {
    matches!(backend, "pack" | "sopack")
}

/// Reports backend distribution truthfully while keeping internal output names stable.
fn product_kind(backend: &str) -> ArtifactKind {
    match backend {
        "pack" | "xmlsquish.pack" => ArtifactKind::Other("pack".into()),
        "sopack" | "xmlsquish.sopack" => ArtifactKind::Other("sopack".into()),
        _ => ArtifactKind::Prompt,
    }
}

fn action_id(stage: &str, identity: &str) -> ActionId {
    ActionId::new(format!(
        "build:{stage}:{}",
        blake3::hash(identity.as_bytes()).to_hex()
    ))
    .expect("generated action id")
}
fn output_name(name: &str) -> OutputName {
    OutputName::new(name).expect("static output name")
}
fn has_emit(values: &[EmitKind], requested: EmitKind) -> bool {
    values.contains(&requested)
}
fn canonical_emit_set(values: &[EmitKind]) -> String {
    let mut tags: Vec<_> = values
        .iter()
        .map(|value| match value {
            EmitKind::BinaryIr => "binary-ir",
            EmitKind::Prompt => "prompt",
            EmitKind::DebugInfo => "debug-info",
        })
        .collect();
    tags.sort_unstable();
    tags.dedup();
    tags.join(",")
}
fn protocol_blake3(bytes: &[u8]) -> squish_protocol::Digest {
    squish_protocol::Digest::new(
        DigestAlgorithm::Blake3,
        blake3::hash(bytes).as_bytes().to_vec(),
    )
    .expect("BLAKE3 digest length is canonical")
}

fn cache_manifest_digest(outputs: &[ProducedOutput]) -> squish_protocol::Digest {
    let mut bytes = b"xmlsquish-cache-result\0v1".to_vec();
    bytes.extend_from_slice(&(outputs.len() as u64).to_le_bytes());
    for output in outputs {
        encode_field(&mut bytes, output.name.as_str().as_bytes());
        match &output.kind {
            ArtifactKind::BinaryIr => bytes.push(0),
            ArtifactKind::Prompt => bytes.push(1),
            ArtifactKind::DebugInfo => bytes.push(2),
            ArtifactKind::Metadata => bytes.push(3),
            ArtifactKind::Other(name) => {
                bytes.push(4);
                encode_field(&mut bytes, name.as_bytes());
            }
        }
        match output.digest.algorithm() {
            DigestAlgorithm::Sha256 => bytes.push(0),
            DigestAlgorithm::Blake3 => bytes.push(1),
            DigestAlgorithm::Other(name) => {
                bytes.push(2);
                encode_field(&mut bytes, name.as_bytes());
            }
        }
        encode_field(&mut bytes, output.digest.bytes());
        bytes.extend_from_slice(&output.size.to_le_bytes());
    }
    protocol_blake3(&bytes)
}

/// Reuse identity established at a sealed ownership boundary, without another hash pass.
fn produced_verified(name: &str, kind: ArtifactKind, blob: &VerifiedBlob) -> ProducedOutput {
    ProducedOutput {
        name: output_name(name),
        kind,
        digest: blob.digest().clone(),
        size: blob.bytes().len() as u64,
    }
}
/// Reuses an immutable archive proof only for its exact privately sealed encoding.
/// Local cached objects still pass the public checked decoder; unrelated valid IR is
/// never allowed to replace an archive unit already acquired from the locked provider.
fn cached_unit_proof(
    source: &FrozenSource,
    bytes: &VerifiedBlob,
) -> Result<ValidatedUnit, ManagerError> {
    match (
        &source.precompiled,
        &source.precompiled_validated,
        &source.precompiled_blob,
    ) {
        (Some(_), Some(validated), Some(frozen)) => {
            if bytes.digest() != frozen.digest() || bytes.bytes().len() != frozen.bytes().len() {
                return Err(ManagerError::new(
                    "MGB111",
                    Phase::Cache,
                    "cached archive unit differs from its exact frozen encoded identity",
                ));
            }
            Ok(validated.clone())
        }
        (None, None, None) => {
            ValidatedUnit::decode(bytes.bytes()).map_err(|e| error("MGB111", Phase::Cache, e))
        }
        _ => Err(ManagerError::new(
            "MGB111",
            Phase::Cache,
            "compiled archive proof and encoded bytes must be frozen together",
        )),
    }
}

/// Preserves compiler source/frame evidence across the manager runtime boundary.
fn runtime_error(code: impl Into<String>, phase: Phase, value: BuildRuntimeError) -> ManagerError {
    let error = ManagerError::new(code, phase, value.to_string());
    match value.diagnostic() {
        Some(diagnostic) => error.with_diagnostic(diagnostic.clone()),
        None => error,
    }
}

fn error(code: &'static str, phase: Phase, value: impl std::fmt::Display) -> ManagerError {
    ManagerError::new(code, phase, value.to_string())
}

#[cfg(test)]
mod catalog_error_tests {
    use super::*;
    use squish_build::{GenerationId, PublicationPath, PublicationTargetId};

    #[test]
    fn typed_publication_identities_reject_layout_like_invalid_inputs() {
        assert!(PublicationTargetId::new("").is_err());
        for invalid in ["../x", "A", "abc"] {
            assert!(GenerationId::from_hex(invalid).is_err());
        }
        for invalid in ["../x", ".squish-publish/current.json", "NUL", "a\\b"] {
            assert!(PublicationPath::new(invalid).is_err());
        }
    }

    #[test]
    fn immutable_instantiation_aliases_include_all_companion_evidence() {
        let document = squish_ir::LinkedDocumentIr {
            schema: squish_ir::Version { major: 1, minor: 0 },
            document_abi: squish_ir::AbiId(squish_ir::DOCUMENT_ABI.into()),
            root: squish_ir::DocumentRegionId(0),
            regions: vec![squish_ir::DocumentRegion {
                id: squish_ir::DocumentRegionId(0),
                start: 0,
                end: 0,
            }],
            items: vec![],
            strings: vec![],
            qnames: vec![],
            feature_bits: squish_ir::FeatureBits(0),
        };
        let digest = VerifiedBlob::from_owned(encode_linked_document(&document))
            .digest()
            .clone();
        let make = |label: &str| {
            Arc::new(InstantiateOutput {
                document: document.clone(),
                trace: squish_ir::ExpansionTrace {
                    debug_strings: vec![label.into()],
                    ..Default::default()
                },
                directives: Vec::new(),
            })
        };
        let a = make("a");
        let b = make("b");
        let identity = |output: &InstantiateOutput| {
            (
                digest.clone(),
                VerifiedBlob::from_owned(encode_expansion_trace(&output.trace))
                    .digest()
                    .clone(),
                None,
            )
        };
        let a_identity = identity(&a);
        let b_identity = identity(&b);
        let mut state = BuildExecutionState::default();
        state.insert_instantiated("a1".into(), a_identity.clone(), Arc::clone(&a));
        state.insert_instantiated("b".into(), b_identity.clone(), Arc::clone(&b));
        assert_ne!(
            a_identity, b_identity,
            "equal document bytes do not imply equal provenance"
        );
        assert!(Arc::ptr_eq(&state.instantiated_by_digest[&a_identity], &a));
        assert!(Arc::ptr_eq(&state.instantiated_by_digest[&b_identity], &b));
        let mut archive_identity = a_identity.clone();
        archive_identity.2 = Some(
            VerifiedBlob::from_owned(b"archive-directives".to_vec())
                .digest()
                .clone(),
        );
        state.insert_instantiated("archive".into(), archive_identity.clone(), Arc::clone(&b));
        assert!(Arc::ptr_eq(&state.instantiated_by_digest[&a_identity], &a));
        assert!(Arc::ptr_eq(
            &state.instantiated_by_digest[&archive_identity],
            &b
        ));
    }
}
