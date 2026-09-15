//! 构建用例的封闭计划与执行。 / Closed planning and execution for the build use case.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::Cursor,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

use squish_backend::{Backend, BackendOutput, BackendRequest, SquishBackend, SquishOptions};
use squish_build::{
    Action, ActionEvent, ActionIndex, ActionKind, ActionRecord, ActionResult, BlobStore, BuildPlan,
    Dispatch, InputRef, KeyRecipe, Output, OutputName, OutputRef, ProducedOutput, Publication,
    ResourceClass, Resources, ResultSource,
};
use squish_ir::{
    ArtifactDigest, ArtifactIdentity, BundledSourceBlob, DebugBundle, DebugDigest, DocumentDigest,
    EntityKind, ImportBinding, ImportSpec, LinkImportRecord, LinkTrace, LinkedImageDigest,
    ObjectDigest, OriginId, QualifiedOriginRef, RelocatableUnitIr, ResolutionSnapshot,
    SourceArchiveReference, SourceKey, UnitKind, UnitRevision, decode_container,
    decode_expansion_trace, decode_linked_document, decode_linked_image, decode_static_link_map,
    decode_unit_container, encode_debug_bundle, encode_expansion_trace, encode_link_trace,
    encode_linked_document, encode_linked_image, encode_static_link_map, encode_unit_container,
};
use squish_link::{
    Budgets, InstantiateOutput, Instantiator, LinkKeyProjection, LinkOutput, StaticLinker,
    UnitClosure,
};
use squish_project::{
    Lockfile, Manifest, MutationFile, MutationKind, MutationPlanner, ResolutionMode, TransactionId,
};
use squish_protocol::{
    ActionId, Artifact, ArtifactId, ArtifactKind, BuildRequest, BuildResult, DigestAlgorithm,
    EmitKind, FinalizationId, FinalizationKind, JobId, OperationKind, OperationResult, Phase,
    PlanMode, PlanningAttemptId, PlanningStepId, PlanningStepKind, PublishedTarget, TargetName,
    WorkspaceScope,
};
use squish_publish::FileArtifactPublisher;
use squish_publish::{PublishError, PublishedGeneration};
use squish_repository::{Discovery, ProjectRepository, ProjectSnapshot, ResolvedTarget};
use squish_source::{FileSourceProvider, LogicalPath, SnapshotBuilder, SourceBlob};
use squish_store::{BlobDigest, Cas, VerifiedActionIndex};
use squish_xml_front::{FrontendSourceContext, compile};

use crate::{
    Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan, ResolveRequest, Services,
    StorageLayout,
    orchestrator::{
        self, ActionExecutionFact, CachedResult, ExecutionReport, ExecutionState, PlanningFailure,
        PlanningRecorder, ResolvedInputs, WorkDisposition, WorkExecutor,
    },
};
use serde::{Deserialize, Serialize};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};

const XML_FRONTEND_ABI: &str = "xmlsquish.xml/1";
/// 当前构建目录所使用的稳定 generation 身份。 / Stable generation identity used by the current-build catalog.
pub const BUILD_CATALOG_TARGET: &str = "xmlsquish-build-record";

/// 可持久、可公开解码的构建记录。 / Persistable, publicly decodable build record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRecordV2 {
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
    /// 查找当前记录所需的显式 catalog generation 身份。 / Explicit catalog generation identity used to find the current record.
    pub catalog_target: String,
}

/// 一次目标 generation 的不可变 catalog 投影。 / Immutable catalog projection of one target generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedGeneration {
    /// `package:target` 完整身份。 / Complete `package:target` identity.
    pub target_id: String,
    /// 内容导出的不可变 generation 身份。 / Content-derived immutable generation identity.
    pub generation_id: String,
    /// 发布器 current manifest 定位器。 / Publisher current-manifest locator.
    pub manifest: String,
    /// 含逻辑目标路径的完整产物集合。 / Complete artifact set with logical destinations.
    pub artifacts: Vec<CatalogArtifact>,
}

/// catalog 中的一个产物及其用户逻辑目标。 / One catalog artifact and its user-facing logical destination.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogArtifact {
    /// 不可变 generation 产物。 / Immutable generation artifact.
    pub artifact: Artifact,
    /// 相对 publication root 的规范逻辑目标。 / Canonical logical destination relative to the publication root.
    pub destination: String,
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

/// Build record v2 编解码错误。 / Build-record v2 codec error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRecordCodecError(String);

impl fmt::Display for BuildRecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for BuildRecordCodecError {}

/// 规范编码 BuildRecord v2。 / Canonically encodes a BuildRecord v2.
pub fn encode_build_record(record: &BuildRecordV2) -> Result<Vec<u8>, BuildRecordCodecError> {
    validate_build_record(record)?;
    serde_json::to_vec(record).map_err(|error| BuildRecordCodecError(error.to_string()))
}

/// 解码并验证 BuildRecord v2。 / Decodes and validates a BuildRecord v2.
pub fn decode_build_record(bytes: &[u8]) -> Result<BuildRecordV2, BuildRecordCodecError> {
    let record: BuildRecordV2 =
        serde_json::from_slice(bytes).map_err(|error| BuildRecordCodecError(error.to_string()))?;
    validate_build_record(&record)?;
    Ok(record)
}

fn validate_build_record(record: &BuildRecordV2) -> Result<(), BuildRecordCodecError> {
    if record.schema != 2 {
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
    if record.catalog_target != BUILD_CATALOG_TARGET {
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
        if target.target_id.is_empty() || target.generation_id.is_empty() {
            return Err(BuildRecordCodecError(
                "target or generation identity is empty".into(),
            ));
        }
        validate_catalog_path(&target.manifest)?;
        if prior_target.is_some_and(|prior| prior >= target.target_id.as_str()) {
            return Err(BuildRecordCodecError(
                "target generations are not uniquely and canonically ordered".into(),
            ));
        }
        prior_target = Some(&target.target_id);
        let maps = target
            .artifacts
            .iter()
            .filter(|item| {
                matches!(&item.artifact.kind, ArtifactKind::Other(name) if name == "static-link-map")
            })
            .count();
        if maps != 1 {
            return Err(BuildRecordCodecError(format!(
                "target `{}` does not have exactly one static link map",
                target.target_id
            )));
        }
        if !target
            .artifacts
            .windows(2)
            .all(|pair| pair[0].artifact.id < pair[1].artifact.id)
        {
            return Err(BuildRecordCodecError(format!(
                "artifacts for target `{}` are not canonically ordered",
                target.target_id
            )));
        }
        let mut destinations = BTreeSet::new();
        for item in &target.artifacts {
            if !artifact_ids.insert(item.artifact.id.clone()) {
                return Err(BuildRecordCodecError(format!(
                    "artifact id `{}` is ambiguous across target generations",
                    item.artifact.id
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
            validate_catalog_path(&item.destination)?;
            validate_catalog_path(&item.artifact.uri)?;
        }
    }
    Ok(())
}

fn validate_catalog_path(value: &str) -> Result<(), BuildRecordCodecError> {
    let path = Path::new(value);
    let segments_are_normal = value
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    if value.is_empty()
        || value
            .chars()
            .any(|character| character == '\\' || character == '\0')
        || value.starts_with('/')
        || value.as_bytes().get(1) == Some(&b':')
        || !segments_are_normal
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        Err(BuildRecordCodecError(format!(
            "catalog path `{value}` is not a normalized relative path"
        )))
    } else {
        Ok(())
    }
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

/// 从显式布局读取并验证当前 BuildRecord v2；仅目录不存在时返回 `None`。
/// Reads and validates the current BuildRecord v2 from an explicit layout; returns `None` only
/// when the catalog generation is absent.
pub fn read_current_build_catalog(
    layout: &StorageLayout,
) -> Result<Option<BuildCatalogSnapshot>, BuildCatalogError> {
    let publisher = FileArtifactPublisher::open(
        layout.catalog_root(),
        Cas::open(layout.cas_root())
            .map_err(|error| BuildCatalogError::Storage(error.to_string()))?,
    )
    .map_err(|error| BuildCatalogError::Storage(error.to_string()))?;
    let Some(generation) = publisher
        .current_generation(BUILD_CATALOG_TARGET)
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?
    else {
        return Ok(None);
    };
    let mut records = generation
        .artifacts
        .iter()
        .filter(|artifact| artifact.id.as_str() == "build-record-v2");
    let artifact = records.next().ok_or_else(|| {
        BuildCatalogError::Corrupt("current generation has no build-record-v2 artifact".into())
    })?;
    if records.next().is_some() {
        return Err(BuildCatalogError::Corrupt(
            "current generation has duplicate build-record-v2 artifacts".into(),
        ));
    }
    validate_catalog_path(&artifact.uri)
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    let bytes = std::fs::read(layout.catalog_root().join(&artifact.uri))
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    if bytes.len() as u64 != artifact.size || protocol_blake3(&bytes) != artifact.digest {
        return Err(BuildCatalogError::Corrupt(
            "build-record artifact size or digest mismatch".into(),
        ));
    }
    let record = decode_build_record(&bytes)
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    let cas = Cas::open(layout.cas_root())
        .map_err(|error| BuildCatalogError::Storage(error.to_string()))?;
    let publication_publisher = FileArtifactPublisher::open(
        layout.publication_root(),
        Cas::open(layout.cas_root())
            .map_err(|error| BuildCatalogError::Storage(error.to_string()))?,
    )
    .map_err(|error| BuildCatalogError::Storage(error.to_string()))?;
    let record_digest = blob_digest(&artifact.digest)
        .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    let stored_record = cas
        .get(record_digest)
        .map_err(|error| BuildCatalogError::Storage(error.to_string()))?
        .ok_or_else(|| BuildCatalogError::Corrupt("build record is absent from CAS".into()))?;
    if stored_record != bytes {
        return Err(BuildCatalogError::Corrupt(
            "published build record differs from CAS".into(),
        ));
    }
    for output in record.actions.iter().flat_map(|action| &action.outputs) {
        verify_catalog_blob(
            &cas,
            &output.digest,
            output.size,
            &format!("action output `{}`", output.name),
        )?;
    }
    for item in record.targets.iter().flat_map(|target| &target.artifacts) {
        let stored = verify_catalog_blob(
            &cas,
            &item.artifact.digest,
            item.artifact.size,
            &format!("artifact `{}`", item.artifact.id),
        )?;
        let published = std::fs::read(layout.publication_root().join(&item.artifact.uri))
            .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
        if published != stored {
            return Err(BuildCatalogError::Corrupt(format!(
                "immutable publication `{}` differs from CAS",
                item.artifact.uri
            )));
        }
    }
    for target in &record.targets {
        let Some(current) = publication_publisher
            .current_generation(&target.target_id)
            .map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?
        else {
            return Err(BuildCatalogError::MissingCurrent {
                target: target.target_id.clone(),
                generation: target.generation_id.clone(),
            });
        };
        if current.generation_id != target.generation_id {
            return Err(BuildCatalogError::Historical {
                target: target.target_id.clone(),
                recorded: target.generation_id.clone(),
                current: current.generation_id,
            });
        }
        let artifacts: Vec<_> = target
            .artifacts
            .iter()
            .map(|item| item.artifact.clone())
            .collect();
        if current.target_id != target.target_id
            || current.manifest != target.manifest
            || current.artifacts != artifacts
        {
            return Err(BuildCatalogError::Corrupt(format!(
                "target `{}` current manifest differs from the recorded generation",
                target.target_id
            )));
        }
        for item in &target.artifacts {
            let suffix = format!("/artifacts/{}", item.destination);
            if !item.artifact.uri.ends_with(&suffix)
                || !item
                    .artifact
                    .uri
                    .contains(&format!("/{}/", target.generation_id))
            {
                return Err(BuildCatalogError::Corrupt(format!(
                    "artifact `{}` URI does not encode its generation and logical destination",
                    item.artifact.id
                )));
            }
        }
    }
    Ok(Some(BuildCatalogSnapshot {
        record,
        record_artifact: artifact.clone(),
    }))
}

fn verify_catalog_blob(
    cas: &Cas,
    digest: &squish_protocol::Digest,
    size: u64,
    subject: &str,
) -> Result<Vec<u8>, BuildCatalogError> {
    let blob =
        blob_digest(digest).map_err(|error| BuildCatalogError::Corrupt(error.to_string()))?;
    let stored = cas
        .get(blob)
        .map_err(|error| BuildCatalogError::Storage(error.to_string()))?
        .ok_or_else(|| BuildCatalogError::Corrupt(format!("{subject} is absent from CAS")))?;
    if stored.len() as u64 != size || protocol_blake3(&stored) != *digest {
        Err(BuildCatalogError::Corrupt(format!(
            "{subject} differs from its catalog identity"
        )))
    } else {
        Ok(stored)
    }
}

/// 读取当前记录的便利投影。 / Convenience projection that reads the current record.
pub fn read_current_build_record(
    layout: &StorageLayout,
) -> Result<Option<BuildRecordV2>, BuildCatalogError> {
    read_current_build_catalog(layout).map(|snapshot| snapshot.map(|snapshot| snapshot.record))
}

/// 一个经过完整验证的 current catalog 快照。 / A fully validated current-catalog snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildCatalogSnapshot {
    /// 类型化 BuildRecord v2。 / Typed BuildRecord v2.
    pub record: BuildRecordV2,
    /// catalog generation 中的记录产物自身。 / Record artifact in the catalog generation itself.
    pub record_artifact: Artifact,
}

impl BuildCatalogSnapshot {
    /// 按全局 ID 查询 target 或 build-record 产物。 / Looks up a target or build-record artifact by global ID.
    pub fn artifact(&self, id: &ArtifactId) -> Option<&Artifact> {
        if &self.record_artifact.id == id {
            Some(&self.record_artifact)
        } else {
            self.record.artifact(id)
        }
    }

    /// 按类型化逻辑路径查询 target 或 catalog 产物。 / Looks up a target or catalog artifact by typed logical path.
    pub fn artifact_at(&self, locator: &crate::ArtifactLocator) -> Option<&Artifact> {
        let path = locator.as_path().to_string_lossy().replace('\\', "/");
        if self.record_artifact.uri == path {
            Some(&self.record_artifact)
        } else {
            self.record.artifact_at(locator)
        }
    }

    /// 查询唯一 target 的静态链接图。 / Looks up the static link map of a unique target.
    pub fn link_map(
        &self,
        target: &TargetName,
    ) -> Result<Option<&Artifact>, BuildRecordQueryError> {
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
                .any(|item| &item.artifact.id == artifact)
        }) else {
            return crate::ProvenanceRelation::NotApplicable(
                crate::ProvenanceNonApplicability::UnsupportedKind,
            );
        };
        let Some(subject) = generation
            .artifacts
            .iter()
            .find(|item| &item.artifact.id == artifact)
        else {
            return crate::ProvenanceRelation::NotApplicable(
                crate::ProvenanceNonApplicability::UnsupportedKind,
            );
        };
        match &subject.artifact.kind {
            ArtifactKind::Prompt => {
                let mut evidence: Vec<_> = generation
                    .artifacts
                    .iter()
                    .map(|item| &item.artifact)
                    .filter(|item| matches!(item.kind, ArtifactKind::DebugInfo))
                    .cloned()
                    .collect();
                evidence.push(self.record_artifact.clone());
                crate::ProvenanceRelation::Evidence(evidence)
            }
            ArtifactKind::Metadata if subject.artifact.id.as_str().ends_with(":target-record") => {
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

impl BuildRecordV2 {
    /// 返回封闭计划投影。 / Returns the closed plan projection.
    pub fn planned_actions(&self) -> &squish_protocol::PlanInspection {
        &self.plan
    }

    /// 按全局唯一 ID 查询产物。 / Looks up an artifact by its globally unique ID.
    pub fn artifact(&self, id: &ArtifactId) -> Option<&Artifact> {
        self.targets
            .iter()
            .flat_map(|target| &target.artifacts)
            .map(|item| &item.artifact)
            .find(|artifact| &artifact.id == id)
    }

    /// 按 generation 相对 URI 查询产物。 / Looks up an artifact by generation-relative URI.
    pub fn artifact_at(&self, locator: &crate::ArtifactLocator) -> Option<&Artifact> {
        let destination = locator.as_path().to_string_lossy().replace('\\', "/");
        self.targets
            .iter()
            .flat_map(|target| &target.artifacts)
            .find(|item| item.destination == destination)
            .map(|item| &item.artifact)
    }

    /// 查询唯一 target 的静态链接图；同名跨包 target 返回歧义错误。 / Looks up a unique target's static link map; same-named targets across packages are ambiguous.
    pub fn link_map(
        &self,
        target: &TargetName,
    ) -> Result<Option<&Artifact>, BuildRecordQueryError> {
        let generation = self.unique_target(target)?;
        Ok(generation.and_then(|generation| {
            generation
                .artifacts
                .iter()
                .map(|item| &item.artifact)
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
            .filter(|generation| generation.target_id.ends_with(&suffix));
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
    targets: Vec<TargetBuild>,
    emit: Vec<EmitKind>,
    locations: Vec<squish_repository::PackageLocation>,
    storage: StorageLayout,
    prior_record: Option<BuildRecordV2>,
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
    unit: RelocatableUnitIr,
    bytes: Vec<u8>,
    object: ObjectDigest,
    semantic: squish_ir::SemanticUnitDigest,
    debug: DebugDigest,
}

#[derive(Clone)]
struct BackendStage {
    output: BackendOutput,
    debug: Vec<u8>,
}

/// 完成 resolve、权威 repository snapshot 与一次性 source sealing，并生成封闭执行图。
/// Resolves dependencies, freezes the authoritative repository snapshot, seals every source once,
/// and produces the closed execution graph.
pub fn prepare(
    request: &BuildRequest,
    services: &dyn Services,
) -> Result<PreparedBuild, ManagerError> {
    prepare_excluding(request, services, &BTreeSet::new())
}

fn prepare_excluding(
    request: &BuildRequest,
    services: &dyn Services,
    excluded: &BTreeSet<String>,
) -> Result<PreparedBuild, ManagerError> {
    let repository =
        ProjectRepository::discover(Discovery::Explicit(request.project.as_str().into()))
            .map_err(|e| error("MGB001", Phase::Discover, e))?;
    let storage = services
        .storage_layout(repository.root())
        .map_err(|e| ManagerError::new(e.code(), Phase::Cache, e.message()))?;
    let prior_record = read_current_build_record(&storage)
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
    let resolved = services
        .resolve(ResolveRequest {
            manifests: &manifests,
            manifest_digest: initial.manifest_digest(),
            prior_lock: initial.lockfile(),
            mode,
        })
        .map_err(|e| ManagerError::new(e.code(), Phase::Resolve, e.message()))?;
    ensure_authoritative_lock(&repository, &initial, &resolved.lockfile, mode)?;
    let snapshot = repository
        .snapshot_with_locations(&resolved.packages)
        .map_err(|e| error("MGB004", Phase::Discover, e))?;
    let sources = freeze_sources(&snapshot, &resolved.packages)?;
    let targets = select_targets(request, &snapshot, &sources, excluded)?;
    let plan = build_plan(&snapshot, &sources, &targets, &request.emit)?;
    Ok(PreparedBuild {
        plan,
        repository,
        snapshot,
        sources,
        targets,
        emit: request.emit.clone(),
        locations: resolved.packages,
        storage,
        prior_record,
    })
}

fn prepare_recorded(
    request: &BuildRequest,
    services: &dyn Services,
    excluded: &BTreeSet<String>,
    planning: &mut PlanningRecorder<'_>,
    context: &InvocationContext,
) -> Result<PreparedBuild, PlanningFailure> {
    let repository = planning.step(step("locate"), PlanningStepKind::Locate, || {
        planning_not_cancelled(context)?;
        ProjectRepository::discover(Discovery::Explicit(request.project.as_str().into()))
            .map_err(|e| error("MGB001", Phase::Discover, e))
    })?;
    planning.step(step("recover"), PlanningStepKind::Recover, || {
        planning_not_cancelled(context)?;
        repository
            .recover()
            .map_err(|e| error("MGB001", Phase::Discover, e))
    })?;
    let storage = planning.step(step("locate-storage"), PlanningStepKind::Locate, || {
        services
            .storage_layout(repository.root())
            .map_err(|e| ManagerError::new(e.code(), Phase::Cache, e.message()))
    })?;
    let prior_record =
        planning.step(step("read-build-catalog"), PlanningStepKind::Fetch, || {
            read_current_build_record(&storage)
                .map_err(|e| ManagerError::new("MGB124", Phase::Cache, e.to_string()))
        })?;
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
        let sources = freeze_sources(&snapshot, &resolved.packages)?;
        let targets = select_targets(request, &snapshot, &sources, excluded)?;
        Ok((sources, targets))
    })?;
    let plan = planning.step(
        step("validate-plan"),
        PlanningStepKind::ValidatePlan,
        || {
            planning_not_cancelled(context)?;
            build_plan(&snapshot, &sources, &targets, &request.emit)
        },
    )?;
    Ok(PreparedBuild {
        plan,
        repository,
        snapshot,
        sources,
        targets,
        emit: request.emit.clone(),
        locations: resolved.packages,
        storage,
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
    let prepared = match prepare_recorded(request, services, &excluded, &mut planning, context) {
        Ok(prepared) => prepared,
        Err(failure) => return planning.unavailable(OperationKind::Build, &failure),
    };
    let plan = prepared.plan.clone();
    let snapshot = planning_snapshot_digest(&prepared);
    let executor = match planning.step(step("open-build-state"), PlanningStepKind::Recover, || {
        BuildExecutor::new(prepared)
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
    }
    protocol_blake3(&bytes)
}

struct BuildExecutor {
    prepared: PreparedBuild,
    cas: Cas,
    publisher: Mutex<FileArtifactPublisher<Cas>>,
    catalog: FileArtifactPublisher<Cas>,
    index: VerifiedActionIndex,
    state: Mutex<BuildExecutionState>,
}

#[derive(Default)]
struct BuildExecutionState {
    compiled: BTreeMap<SourceKey, CompiledUnit>,
    linked: BTreeMap<String, (LinkOutput, ResolutionSnapshot)>,
    instantiated: BTreeMap<String, (squish_ir::LinkedDocumentIr, InstantiateOutput)>,
    backend: BTreeMap<String, BackendStage>,
    published: Vec<PublishedTarget>,
    generations: Vec<RecordedGeneration>,
}

impl BuildExecutor {
    fn new(prepared: PreparedBuild) -> Result<Self, ManagerError> {
        let cas =
            Cas::open(prepared.storage.cas_root()).map_err(|e| error("MGB030", Phase::Cache, e))?;
        let publisher = FileArtifactPublisher::open(
            prepared.storage.publication_root(),
            Cas::open(cas.root()).map_err(|e| error("MGB031", Phase::Publish, e))?,
        )
        .map_err(|e| error("MGB032", Phase::Publish, e))?;
        let catalog = FileArtifactPublisher::open(
            prepared.storage.catalog_root(),
            Cas::open(cas.root()).map_err(|e| error("MGB031", Phase::Publish, e))?,
        )
        .map_err(|e| error("MGB032", Phase::Publish, e))?;
        let index_cas =
            Arc::new(Cas::open(cas.root()).map_err(|e| error("MGB035", Phase::Cache, e))?);
        let index = VerifiedActionIndex::open(prepared.storage.action_index(), index_cas)
            .map_err(|e| error("MGB036", Phase::Cache, e))?;
        Ok(Self {
            prepared,
            cas,
            publisher: Mutex::new(publisher),
            catalog,
            index,
            state: Mutex::new(BuildExecutionState::default()),
        })
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
        let record = BuildRecordV2 {
            schema: 2,
            job: inspection.job.clone(),
            plan,
            actions,
            targets,
            catalog_target: BUILD_CATALOG_TARGET.into(),
        };
        let bytes = encode_build_record(&record)
            .map_err(|error| ManagerError::new("MGB121", Phase::Publish, error.to_string()))?;
        let digest = self
            .cas
            .write_from(&mut Cursor::new(&bytes))
            .map_err(|cause| error("MGB122", Phase::Cache, cause))?;
        let publication = publication(
            "build-record-v2",
            ArtifactKind::Metadata,
            &self
                .prepared
                .storage
                .catalog_root()
                .join("build-record-v2.json"),
            digest,
            bytes.len() as u64,
            self.prepared.storage.catalog_root(),
        )?;
        let generation = publish_generation(&self.catalog, BUILD_CATALOG_TARGET, &[publication])?;
        generation.artifacts.into_iter().next().ok_or_else(|| {
            ManagerError::new(
                "MGB123",
                Phase::Publish,
                "build catalog committed an empty generation",
            )
        })
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

    fn linked_for(
        &self,
        target: &str,
        inputs: &ResolvedInputs,
    ) -> Result<LinkOutput, ManagerError> {
        let mut state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        if let Some((linked, _)) = state.linked.get(target) {
            return Ok(linked.clone());
        }
        let expected = expected_output(inputs, "image").ok_or_else(|| {
            ManagerError::new("MGB102", Phase::Instantiate, "linked image input is absent")
        })?;
        let value = state
            .linked
            .values()
            .find(|(linked, _)| protocol_blake3(&encode_linked_image(&linked.image)) == expected)
            .cloned()
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB102",
                    Phase::Instantiate,
                    "linked program cannot be restored",
                )
            })?;
        let linked = value.0.clone();
        state.linked.insert(target.into(), value);
        Ok(linked)
    }

    fn instantiated_for(
        &self,
        target: &str,
        inputs: &ResolvedInputs,
    ) -> Result<(squish_ir::LinkedDocumentIr, InstantiateOutput), ManagerError> {
        let mut state = self
            .state
            .lock()
            .expect("build state mutex is not poisoned");
        if let Some(value) = state.instantiated.get(target) {
            return Ok(value.clone());
        }
        let expected = expected_output(inputs, "document")
            .ok_or_else(|| ManagerError::new("MGB103", Phase::Emit, "document input is absent"))?;
        let value = state
            .instantiated
            .values()
            .find(|(document, _)| protocol_blake3(&encode_linked_document(document)) == expected)
            .cloned()
            .ok_or_else(|| {
                ManagerError::new("MGB103", Phase::Emit, "document cannot be restored")
            })?;
        state.instantiated.insert(target.into(), value.clone());
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
            .sources
            .iter()
            .find(|source| source_key(source) == *key)
            .ok_or_else(|| {
                ManagerError::new("MGB101", Phase::Analyze, "planned source is absent")
            })?;
        let unit = compile(
            &source.blob,
            &FrontendSourceContext::new(source.package.clone()),
        )
        .map_err(|diagnostic| {
            ManagerError::new("MGB040", Phase::Analyze, diagnostic.message.clone())
        })?
        .unit;
        let bytes = encode_unit_container(&unit).map_err(|e| error("MGB041", Phase::Analyze, e))?;
        self.cas
            .put(&bytes)
            .map_err(|e| error("MGB042", Phase::Cache, e))?;
        let container = decode_container(&bytes).map_err(|e| error("MGB043", Phase::Analyze, e))?;
        let output = produced("xsir", ArtifactKind::BinaryIr, &bytes);
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .compiled
            .insert(
                key.clone(),
                CompiledUnit {
                    unit,
                    bytes,
                    object: container.object_digest(),
                    semantic: container.semantic_digest(),
                    debug: container.debug_digest(),
                },
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
        let resolution = bind_imports(
            &self.prepared.snapshot,
            &self.prepared.locations,
            &state.compiled,
        )?;
        let payloads = state
            .compiled
            .iter()
            .map(|(key, value)| (key.clone(), value.unit.clone()))
            .collect();
        drop(state);
        let linked = StaticLinker
            .link(
                entry,
                UnitClosure {
                    snapshot: resolution.clone(),
                    units: payloads,
                },
            )
            .map_err(|e| error("MGB070", Phase::Link, e))?;
        let image = encode_linked_image(&linked.image);
        let map = encode_static_link_map(&linked.map);
        self.cas
            .put(&image)
            .map_err(|e| error("MGB071", Phase::Cache, e))?;
        self.cas
            .put(&map)
            .map_err(|e| error("MGB072", Phase::Cache, e))?;
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .linked
            .insert(target_name.into(), (linked, resolution));
        Ok((
            vec![
                produced("image", ArtifactKind::Metadata, &image),
                produced("map", ArtifactKind::Metadata, &map),
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
        let instantiated = Instantiator
            .instantiate(
                &linked.program,
                target.arguments.clone(),
                budgets(&target.resolved),
            )
            .map_err(|e| error("MGB073", Phase::Instantiate, e))?;
        let mut document = instantiated.document.clone();
        document.document_abi = squish_ir::AbiId(squish_backend::DOCUMENT_ABI.to_owned());
        let document_bytes = encode_linked_document(&document);
        let trace_bytes = encode_expansion_trace(&instantiated.trace);
        self.cas
            .put(&document_bytes)
            .map_err(|e| error("MGB074", Phase::Cache, e))?;
        self.cas
            .put(&trace_bytes)
            .map_err(|e| error("MGB075", Phase::Cache, e))?;
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .instantiated
            .insert(target_name.into(), (document, instantiated));
        Ok((
            vec![
                produced("document", ArtifactKind::Metadata, &document_bytes),
                produced("trace", ArtifactKind::DebugInfo, &trace_bytes),
            ],
            Vec::new(),
        ))
    }

    fn backend(
        &self,
        target_name: &str,
        inputs: &ResolvedInputs,
    ) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
        let target = self.target(target_name)?;
        let (document, instantiated) = self.instantiated_for(target_name, inputs)?;
        let linked = self.linked_for(target_name, inputs)?;
        let output = SquishBackend
            .emit(BackendRequest {
                document: document.clone(),
                trace: instantiated.trace,
                options: SquishOptions {
                    max_output_bytes: budgets(&target.resolved).max_output_bytes,
                },
            })
            .map_err(|e| error("MGB076", Phase::Emit, e))?;
        self.cas
            .write_from(&mut Cursor::new(&output.bytes))
            .map_err(|e| error("MGB077", Phase::Cache, e))?;
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
            &document,
            &output,
            &linked.image,
            &link_trace,
            &compiled,
            &self.prepared.sources,
        );
        let debug = encode_debug_bundle(&bundle).map_err(|e| error("MGB078", Phase::Emit, e))?;
        self.cas
            .write_from(&mut Cursor::new(&debug))
            .map_err(|e| error("MGB079", Phase::Cache, e))?;
        let produced = vec![
            produced("prompt", ArtifactKind::Prompt, &output.bytes),
            produced("backend-result", ArtifactKind::DebugInfo, &debug),
        ];
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .backend
            .insert(target_name.into(), BackendStage { output, debug });
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
            state
                .backend
                .iter()
                .find_map(|(name, value)| {
                    let actual = [
                        produced("prompt", ArtifactKind::Prompt, &value.output.bytes),
                        produced("backend-result", ArtifactKind::DebugInfo, &value.debug),
                    ];
                    (actual.as_slice() == expected).then(|| name.clone())
                })
                .ok_or_else(|| {
                    ManagerError::new(
                        "MGB104",
                        Phase::Publish,
                        "backend product cannot be restored",
                    )
                })?
        };
        let output = state.backend[&source_target].clone();
        let compiled = state.compiled.clone();
        let link_map = state
            .linked
            .get(target_name)
            .or_else(|| state.linked.get(&source_target))
            .map(|(linked, _)| encode_static_link_map(&linked.map))
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB120",
                    Phase::Publish,
                    "static link-map evidence is absent",
                )
            })?;
        drop(state);
        let prompt_digest = self
            .cas
            .write_from(&mut Cursor::new(&output.output.bytes))
            .map_err(|e| error("MGB077", Phase::Cache, e))?;
        let mut publications = Vec::new();
        if has_emit(&self.prepared.emit, EmitKind::BinaryIr) {
            for (source, unit) in &compiled {
                let digest = self
                    .cas
                    .write_from(&mut Cursor::new(&unit.bytes))
                    .map_err(|e| error("MGB077", Phase::Cache, e))?;
                publications.push(publication(
                    &format!("{target_name}:ir-{}", publications.len()),
                    ArtifactKind::BinaryIr,
                    &ir_destination(&target.resolved.output, source),
                    digest,
                    unit.bytes.len() as u64,
                    self.prepared.repository.root(),
                )?);
            }
        }
        if has_emit(&self.prepared.emit, EmitKind::Prompt)
            || has_emit(&self.prepared.emit, EmitKind::DebugInfo)
        {
            publications.push(publication(
                &format!("{target_name}:prompt"),
                ArtifactKind::Prompt,
                &target.resolved.output,
                prompt_digest,
                output.output.bytes.len() as u64,
                self.prepared.repository.root(),
            )?);
        }
        if has_emit(&self.prepared.emit, EmitKind::DebugInfo) {
            let bytes = output.debug;
            let digest = self
                .cas
                .write_from(&mut Cursor::new(&bytes))
                .map_err(|e| error("MGB079", Phase::Cache, e))?;
            publications.push(publication(
                &format!("{target_name}:debug"),
                ArtifactKind::DebugInfo,
                &target.resolved.output.with_extension("psdbg"),
                digest,
                bytes.len() as u64,
                self.prepared.repository.root(),
            )?);
        }
        let link_map_digest = self
            .cas
            .write_from(&mut Cursor::new(&link_map))
            .map_err(|e| error("MGB120", Phase::Cache, e))?;
        publications.push(publication(
            &format!("{target_name}:static-link-map"),
            ArtifactKind::Other("static-link-map".into()),
            &target.resolved.output.with_extension("xsmap"),
            link_map_digest,
            link_map.len() as u64,
            self.prepared.repository.root(),
        )?);
        let target_record = target_record_bytes(
            target_name,
            self.prepared.snapshot.manifest_digest(),
            &publications,
        )?;
        let record_digest = self
            .cas
            .write_from(&mut Cursor::new(&target_record))
            .map_err(|e| error("MGB118", Phase::Cache, e))?;
        publications.push(publication(
            &format!("{target_name}:target-record"),
            ArtifactKind::Metadata,
            &target.resolved.output.with_extension("build.json"),
            record_digest,
            target_record.len() as u64,
            self.prepared.repository.root(),
        )?);
        publications.sort_by(|left, right| left.output.name.cmp(&right.output.name));
        let publisher = self.publisher.lock().map_err(|_| {
            ManagerError::new("MGB119", Phase::Publish, "publisher coordination failed")
        })?;
        let generation = publish_generation(&publisher, target_name, &publications)?;
        let published = PublishedTarget {
            target: TargetName::new(target.name.clone()).expect("validated target name"),
            artifacts: generation.artifacts.clone(),
        };
        let recorded = RecordedGeneration {
            target_id: generation.target_id.clone(),
            generation_id: generation.generation_id.clone(),
            manifest: generation.manifest.clone(),
            artifacts: generation
                .artifacts
                .iter()
                .zip(&publications)
                .map(|(artifact, publication)| CatalogArtifact {
                    artifact: artifact.clone(),
                    destination: publication.destination.replace('\\', "/"),
                })
                .collect(),
        };
        self.state
            .lock()
            .expect("build state mutex is not poisoned")
            .generations
            .push(recorded);
        let events = published
            .artifacts
            .iter()
            .cloned()
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
        record: &ActionRecord,
    ) -> Result<(), ManagerError> {
        match work {
            BuildWork::Compile { source } => {
                let bytes = self.cached_bytes(record, "xsir")?;
                let unit =
                    decode_unit_container(&bytes).map_err(|e| error("MGB111", Phase::Cache, e))?;
                let container =
                    decode_container(&bytes).map_err(|e| error("MGB112", Phase::Cache, e))?;
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .compiled
                    .insert(
                        source.clone(),
                        CompiledUnit {
                            unit,
                            bytes,
                            object: container.object_digest(),
                            semantic: container.semantic_digest(),
                            debug: container.debug_digest(),
                        },
                    );
            }
            BuildWork::Link { target, .. } => {
                let image = decode_linked_image(&self.cached_bytes(record, "image")?)
                    .map_err(|e| error("MGB121", Phase::Cache, e))?;
                let map = decode_static_link_map(&self.cached_bytes(record, "map")?)
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
                    .map(|(key, unit)| (key.clone(), unit.unit.clone()))
                    .collect();
                let objects = state
                    .compiled
                    .iter()
                    .map(|(key, unit)| (key.clone(), unit.object))
                    .collect();
                let resolution = bind_imports(
                    &self.prepared.snapshot,
                    &self.prepared.locations,
                    &state.compiled,
                )?;
                drop(state);
                let program =
                    squish_link::LinkedProgram::reconstruct(image.clone(), units, objects)
                        .map_err(|e| error("MGB124", Phase::Cache, e))?;
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .linked
                    .insert(
                        target.clone(),
                        (
                            LinkOutput {
                                map,
                                image,
                                program,
                            },
                            resolution,
                        ),
                    );
            }
            BuildWork::Instantiate { target } => {
                let document = decode_linked_document(&self.cached_bytes(record, "document")?)
                    .map_err(|e| error("MGB125", Phase::Cache, e))?;
                let trace = decode_expansion_trace(&self.cached_bytes(record, "trace")?)
                    .map_err(|e| error("MGB126", Phase::Cache, e))?;
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .instantiated
                    .insert(
                        target.clone(),
                        (document.clone(), InstantiateOutput { document, trace }),
                    );
            }
            BuildWork::Backend { target } => {
                let bytes = self.cached_bytes(record, "prompt")?;
                let debug = self.cached_bytes(record, "backend-result")?;
                let target_build = self.target(target)?;
                let bundle = squish_ir::decode_debug_bundle(&debug)
                    .map_err(|e| error("MGB127", Phase::Cache, e))?;
                if bundle.artifact.digest != ArtifactDigest::of(&bytes)
                    || bundle.artifact.byte_len != bytes.len() as u64
                {
                    return Err(ManagerError::new(
                        "MGB128",
                        Phase::Cache,
                        "cached prompt differs from backend evidence",
                    ));
                }
                let output = BackendOutput {
                    bytes,
                    byte_map: bundle.artifact_map,
                    trace: bundle.expansion_trace,
                    cache_identity: SquishBackend.cache_identity(SquishOptions {
                        max_output_bytes: budgets(&target_build.resolved).max_output_bytes,
                    }),
                    metrics: Default::default(),
                };
                self.state
                    .lock()
                    .expect("build state mutex is not poisoned")
                    .backend
                    .insert(target.clone(), BackendStage { output, debug });
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

    fn cached_bytes(&self, record: &ActionRecord, name: &str) -> Result<Vec<u8>, ManagerError> {
        let output = record
            .outputs
            .iter()
            .find(|output| output.name.as_str() == name)
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB108",
                    Phase::Cache,
                    format!("cached action has no `{name}` output"),
                )
            })?;
        self.cas
            .get(blob_digest(&output.digest)?)
            .map_err(|e| error("MGB109", Phase::Cache, e))?
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB110",
                    Phase::Cache,
                    format!("cached `{name}` blob disappeared"),
                )
            })
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
        let Some(record) = self
            .index
            .lookup(&dispatch.key)
            .map_err(|e| error("MGB107", Phase::Cache, e))?
        else {
            return Ok(None);
        };
        self.hydrate_cached(work, inputs, &record)?;
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
            Err(error) => ActionResult::failure(error.code(), error.message()),
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
            self.index
                .record(&ActionRecord {
                    key: dispatch.key.clone(),
                    outputs: result.outputs.clone(),
                })
                .map_err(|e| error("MGB113", Phase::Cache, e))?;
        }
        Ok(())
    }
}

fn publish_generation(
    publisher: &FileArtifactPublisher<Cas>,
    target: &str,
    publications: &[Publication],
) -> Result<PublishedGeneration, ManagerError> {
    for _ in 0..2 {
        match publisher.publish_generation(target, publications) {
            Ok(generation) => return Ok(generation),
            Err(PublishError::Io(cause))
                if cause.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(cause) => return Err(error("MGB119", Phase::Publish, cause)),
        }
    }
    publisher
        .publish_generation(target, publications)
        .map_err(|cause| error("MGB119", Phase::Publish, cause))
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

fn blob_digest(digest: &squish_protocol::Digest) -> Result<BlobDigest, ManagerError> {
    if digest.algorithm() != &DigestAlgorithm::Blake3 {
        return Err(ManagerError::new(
            "MGB114",
            Phase::Cache,
            "cached blob does not use BLAKE3",
        ));
    }
    let bytes: [u8; 32] = digest.bytes().try_into().map_err(|_| {
        ManagerError::new("MGB114", Phase::Cache, "cached digest has invalid length")
    })?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn expected_output(inputs: &ResolvedInputs, name: &str) -> Option<squish_protocol::Digest> {
    inputs
        .iter()
        .flat_map(|(_, outputs)| outputs)
        .find(|output| output.name.as_str() == name)
        .map(|output| output.digest.clone())
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
    let sources = loaded
        .into_iter()
        .map(|(blob, package)| {
            let _ = sealed
                .get(blob.id())
                .expect("every successful load is present after seal");
            FrozenSource { blob, package }
        })
        .collect();
    Ok(sources)
}

fn source_key(source: &FrozenSource) -> SourceKey {
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
            if resolved.backend != "squish" {
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
            let entry = sources
                .iter()
                .find(|s| s.package.package_name == package.name && s.blob.id().path() == &logical)
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
                vec![InputRef::Blob(source.blob.digest().to_protocol())],
                BTreeMap::from([
                    ("source-key".into(), canonical_source_key(&key)),
                    ("frontend-abi".into(), XML_FRONTEND_ABI.into()),
                ]),
            ),
        )?;
    }
    for target in targets {
        let identity = format!("{}:{}", target.package, target.name);
        let link = action_id("link", &identity);
        let link_options = BTreeMap::from([
            ("entry".into(), canonical_source_key(&target.entry)),
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
        add_action(
            &mut actions,
            &mut work,
            instantiate.clone(),
            BuildWork::Instantiate {
                target: identity.clone(),
            },
            vec![link],
            vec![
                Output {
                    name: output_name("document"),
                    kind: ArtifactKind::Metadata,
                },
                Output {
                    name: output_name("trace"),
                    kind: ArtifactKind::DebugInfo,
                },
            ],
            (instantiate_inputs, instantiate_options(target)),
        )?;
        let backend = action_id("backend", &identity);
        let backend_inputs = vec![
            InputRef::Output(OutputRef {
                action: instantiate.clone(),
                output: output_name("document"),
            }),
            InputRef::Output(OutputRef {
                action: instantiate.clone(),
                output: output_name("trace"),
            }),
        ];
        add_action(
            &mut actions,
            &mut work,
            backend.clone(),
            BuildWork::Backend {
                target: identity.clone(),
            },
            vec![instantiate],
            vec![
                Output {
                    name: output_name("prompt"),
                    kind: ArtifactKind::Prompt,
                },
                Output {
                    name: output_name("backend-result"),
                    kind: ArtifactKind::DebugInfo,
                },
            ],
            (backend_inputs, backend_options(target)),
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
    let key = KeyRecipe::new(format!("manager/{kind:?}/1"), key_parts.0, key_parts.1)
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

fn instantiate_options(target: &TargetBuild) -> BTreeMap<String, String> {
    let limits = budgets(&target.resolved);
    let mut options = BTreeMap::from([
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

fn backend_options(target: &TargetBuild) -> BTreeMap<String, String> {
    let identity = SquishBackend.cache_identity(SquishOptions {
        max_output_bytes: budgets(&target.resolved).max_output_bytes,
    });
    BTreeMap::from([
        ("backend-id".into(), identity.backend_id.into()),
        ("backend-version".into(), identity.backend_version.into()),
        ("canonical-options".into(), hex(&identity.canonical_options)),
    ])
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
    units: &BTreeMap<SourceKey, CompiledUnit>,
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
            let target = resolve_import(
                source,
                &import.spec,
                lock,
                &locked_manifests,
                &package_by_instance,
                units,
            )?;
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

fn resolve_import(
    source: &SourceKey,
    spec: &ImportSpec,
    lock: &Lockfile,
    manifests: &BTreeMap<&str, &Manifest>,
    instances: &BTreeMap<squish_ir::PackageInstanceId, String>,
    units: &BTreeMap<SourceKey, CompiledUnit>,
) -> Result<SourceKey, ManagerError> {
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
    match units.get(&target).map(|u| unit_kind(&u.unit)) {
        Some(UnitKind::Module) => Ok(target),
        Some(UnitKind::Entry) => Err(ManagerError::new(
            "MGB059",
            Phase::Link,
            "imports may target modules only",
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
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
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
    output: &squish_backend::BackendOutput,
    image: &squish_ir::LinkedImage,
    link_trace: &LinkTrace,
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
    sources: &[FrozenSource],
) -> DebugBundle {
    let mut trace = output.trace.clone();
    retain_backend_origin_dag(&mut trace);
    let mut archives = Vec::new();
    let mut blobs = BTreeMap::new();
    for unit in compiled.values() {
        let (archive, origins) = archive_and_origins(&unit.unit);
        archives.push(SourceArchiveReference {
            object: unit.object,
            debug_digest: unit.debug,
            archive: archive.clone(),
            origins: origins.clone(),
        });
        for record in &archive.records {
            if let Some(bytes) = sources
                .iter()
                .find(|source| source_key(source) == record.key)
                .map(|source| source.blob.bytes().to_vec())
            {
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
        artifact_map: output.byte_map.clone(),
        source_archives: archives,
        source_blobs,
    }
}

fn retain_backend_origin_dag(trace: &mut squish_ir::ExpansionTrace) {
    for index in 0..trace.origins.len() {
        let source = source_origin(
            squish_ir::OriginNodeId(index as u32),
            trace,
            &mut BTreeSet::new(),
        );
        let Some(source) = source else { continue };
        if let squish_ir::OriginNode::BackendTransform { inputs, .. } = &mut trace.origins[index]
            && !inputs.iter().any(|edge| edge.parent == source)
        {
            inputs.push(squish_ir::OriginEdge {
                role: squish_ir::OriginRole::Transform,
                parent: source,
            });
        }
    }
}

fn source_origin(
    id: squish_ir::OriginNodeId,
    trace: &squish_ir::ExpansionTrace,
    seen: &mut BTreeSet<u32>,
) -> Option<squish_ir::OriginNodeId> {
    if !seen.insert(id.0) {
        return None;
    }
    use squish_ir::OriginNode;
    match trace.origins.get(id.0 as usize)? {
        OriginNode::SourceSpan { .. } | OriginNode::DecodedSegment { .. } => Some(id),
        OriginNode::Expansion { producer, .. } => {
            let origin = trace
                .document_items
                .iter()
                .find(|item| item.producer_op == *producer)?
                .definition_origin
                .clone();
            trace
                .origins
                .iter()
                .enumerate()
                .find_map(|(index, node)| match node {
                    OriginNode::SourceSpan { origin: candidate } if *candidate == origin => {
                        Some(squish_ir::OriginNodeId(index as u32))
                    }
                    _ => None,
                })
        }
        OriginNode::Import { child, .. } | OriginNode::RegexCapture { input: child, .. } => {
            source_origin(*child, trace, seen)
        }
        OriginNode::Concat { ordered_inputs } => ordered_inputs
            .iter()
            .find_map(|parent| source_origin(*parent, trace, seen)),
        OriginNode::BackendTransform { inputs, .. } | OriginNode::Fused { inputs, .. } => inputs
            .iter()
            .find_map(|edge| source_origin(edge.parent, trace, seen)),
        OriginNode::Synthetic {
            nearest: Some(parent),
            ..
        } => source_origin(*parent, trace, seen),
        _ => None,
    }
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
        destination: portable_path(relative)?,
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
                "destination": item.destination,
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
    match unit {
        RelocatableUnitIr::Module(v) => &v.header,
        RelocatableUnitIr::Entry(v) => &v.header,
    }
}
fn origins(unit: &RelocatableUnitIr) -> &squish_ir::OriginTable {
    match unit {
        RelocatableUnitIr::Module(v) => &v.origins,
        RelocatableUnitIr::Entry(v) => &v.origins,
    }
}
fn archive_and_origins(
    unit: &RelocatableUnitIr,
) -> (&squish_ir::SourceArchive, &squish_ir::OriginTable) {
    match unit {
        RelocatableUnitIr::Module(v) => (&v.sources, &v.origins),
        RelocatableUnitIr::Entry(v) => (&v.sources, &v.origins),
    }
}
fn unit_kind(unit: &RelocatableUnitIr) -> UnitKind {
    match unit {
        RelocatableUnitIr::Module(_) => UnitKind::Module,
        RelocatableUnitIr::Entry(_) => UnitKind::Entry,
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

fn produced(name: &str, kind: ArtifactKind, bytes: &[u8]) -> ProducedOutput {
    ProducedOutput {
        name: output_name(name),
        kind,
        digest: protocol_blake3(bytes),
        size: bytes.len() as u64,
    }
}
fn error(code: &'static str, phase: Phase, value: impl std::fmt::Display) -> ManagerError {
    ManagerError::new(code, phase, value.to_string())
}
