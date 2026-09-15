use std::{
    collections::BTreeMap,
    fmt,
    path::{Component, Path, PathBuf},
};

use squish_kernel::CancellationToken;
use squish_project::{Lockfile, Manifest, ResolutionMode};
use squish_protocol::{
    Artifact, ArtifactId, CachedAction, Digest, PlanInspection, TargetName, VcsChoice,
};
use squish_repository::{
    CreateProjectRequest, CreatedProject, FaultInjector, PackageLocation, ProjectVcs,
    WorkspaceMembership,
};
use std::sync::Arc;

use crate::{ArtifactLocator, ServiceError};

/// 创建目标经宿主定位后冻结的外部环境。 / Host-located external environment frozen for project creation.
///
/// 该值只描述权威输入，不创建目录；真正写入必须延后到唯一的 `CreateProject`
/// 动作。 / This value describes authoritative inputs without creating directories; all writes
/// remain deferred to the sole `CreateProject` action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectCreationLocation {
    /// 规范绝对目标路径。 / Normalized absolute destination path.
    pub destination: PathBuf,
    /// 实际 Git 落位。 / Effective Git placement.
    pub vcs: ProjectVcs,
    /// 可选外围工作区成员关系。 / Optional enclosing-workspace membership.
    pub workspace: Option<WorkspaceMembership>,
}

/// 创建动作的封闭执行结果。 / Closed execution result of a creation action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectCreationStatus {
    /// 已跨过提交决定并完成或可恢复地发布。 / Publication crossed its commit decision and completed or is recoverable.
    Created(CreatedProject),
    /// 已越过提交决定，但完成或恢复报告失败。 / The commit decision was crossed, but completion or recovery reported a failure.
    CommittedFailure(ServiceError),
    /// 在提交决定前响应取消，未发布项目。 / Cancellation was honored before the commit decision; no project was published.
    Cancelled,
}

/// 存储职责路径无效。 / Invalid storage-responsibility paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageLayoutError {
    /// 必需路径为空。 / A required path is empty.
    Empty(&'static str),
    /// 路径不是锚定绝对路径。 / A path is not anchored and absolute.
    Relative(&'static str),
    /// 路径含 `.` 或 `..`，未结构化规范。 / A path contains `.` or `..` and is not structurally normalized.
    NonNormalized(&'static str),
    /// 两种不同职责错误地共享同一路径。 / Two distinct responsibilities incorrectly share one path.
    Aliased {
        /// 第一项职责。 / First responsibility.
        left: &'static str,
        /// 第二项职责。 / Second responsibility.
        right: &'static str,
    },
}

impl fmt::Display for StorageLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(name) => write!(formatter, "storage path `{name}` is empty"),
            Self::Relative(name) => write!(formatter, "storage path `{name}` is not absolute"),
            Self::NonNormalized(name) => {
                write!(formatter, "storage path `{name}` is not normalized")
            }
            Self::Aliased { left, right } => {
                write!(formatter, "storage paths `{left}` and `{right}` alias")
            }
        }
    }
}

impl std::error::Error for StorageLayoutError {}

/// 明确分离持久存储职责的布局。 / Layout explicitly separating persistent-storage responsibilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLayout {
    cas_root: PathBuf,
    action_index: PathBuf,
    publication_root: PathBuf,
    catalog_root: PathBuf,
}

impl StorageLayout {
    /// 验证四条非空、互不别名的职责路径后创建布局。 / Creates a layout after validating four non-empty, non-aliasing responsibility paths.
    pub fn new(
        cas_root: impl Into<PathBuf>,
        action_index: impl Into<PathBuf>,
        publication_root: impl Into<PathBuf>,
        catalog_root: impl Into<PathBuf>,
    ) -> Result<Self, StorageLayoutError> {
        let values = [
            (
                "cas_root",
                normalize_layout_path("cas_root", cas_root.into())?,
            ),
            (
                "action_index",
                normalize_layout_path("action_index", action_index.into())?,
            ),
            (
                "publication_root",
                normalize_layout_path("publication_root", publication_root.into())?,
            ),
            (
                "catalog_root",
                normalize_layout_path("catalog_root", catalog_root.into())?,
            ),
        ];
        validate_layout(&values)?;
        let [
            (_, cas_root),
            (_, action_index),
            (_, publication_root),
            (_, catalog_root),
        ] = values;
        Ok(Self {
            cas_root,
            action_index,
            publication_root,
            catalog_root,
        })
    }

    /// 仅供测试适配器使用的项目内布局。 / Project-local layout intended only for test adapters.
    pub fn project_local_for_tests(project: &Path) -> Self {
        let project = std::fs::canonicalize(project)
            .expect("test project root must exist and be canonicalizable");
        Self::new(
            project.join(".cache/xmlsquish/cas"),
            project.join(".cache/xmlsquish/actions.sqlite3"),
            project.join("target/xmlsquish"),
            project.join(".cache/xmlsquish/catalog"),
        )
        .expect("test layout paths are distinct")
    }

    /// 返回 CAS 根目录。 / Returns the CAS root directory.
    pub fn cas_root(&self) -> &Path {
        &self.cas_root
    }
    /// 返回动作索引文件路径。 / Returns the action-index file path.
    pub fn action_index(&self) -> &Path {
        &self.action_index
    }
    /// 返回 generation 发布根目录。 / Returns the generation-publication root.
    pub fn publication_root(&self) -> &Path {
        &self.publication_root
    }
    /// 返回查询目录根路径。 / Returns the inspection-catalog root.
    pub fn catalog_root(&self) -> &Path {
        &self.catalog_root
    }
}

fn validate_layout(values: &[(&'static str, PathBuf); 4]) -> Result<(), StorageLayoutError> {
    for left in 0..values.len() {
        for right in left + 1..values.len() {
            if paths_overlap(&values[left].1, &values[right].1) {
                return Err(StorageLayoutError::Aliased {
                    left: values[left].0,
                    right: values[right].0,
                });
            }
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        let left = left.to_string_lossy().to_lowercase();
        let right = right.to_string_lossy().to_lowercase();
        return left == right
            || left
                .strip_prefix(&right)
                .is_some_and(|tail| tail.starts_with(['\\', '/']))
            || right
                .strip_prefix(&left)
                .is_some_and(|tail| tail.starts_with(['\\', '/']));
    }
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn normalize_layout_path(name: &'static str, path: PathBuf) -> Result<PathBuf, StorageLayoutError> {
    if path.as_os_str().is_empty() {
        return Err(StorageLayoutError::Empty(name));
    }
    if !path.is_absolute() {
        return Err(StorageLayoutError::Relative(name));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => return Err(StorageLayoutError::NonNormalized(name)),
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// 一次完整且可复现的依赖解析输入。 / Complete reproducible dependency-resolution input.
pub struct ResolveRequest<'a> {
    /// 以稳定清单路径为键的候选清单快照。 / Candidate manifest snapshot keyed by stable manifest path.
    pub manifests: &'a BTreeMap<String, Manifest>,
    /// 候选清单集合的规范摘要。 / Canonical digest of the candidate manifest set.
    pub manifest_digest: &'a str,
    /// 可复用的先前精确锁。 / Prior exact lock that may be reused.
    pub prior_lock: Option<&'a Lockfile>,
    /// 联网和锁更新模式。 / Network and lock-update mode.
    pub mode: ResolutionMode,
}

/// 解析后的精确依赖及其物化位置。 / Exact resolved dependencies and their materialized locations.
#[derive(Clone, Debug)]
pub struct ResolvedDependencies {
    /// 完整精确锁。 / Complete exact lock.
    pub lockfile: Lockfile,
    /// Registry 归档、Git checkout 和本地包的位置。 / Locations of registry archives, Git checkouts, and local packages.
    pub packages: Vec<PackageLocation>,
}

/// 来源证明目录的类型化查询结果。 / Typed result of a provenance-catalog query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvenanceRelation {
    /// 可验证的来源证明产物。 / Verifiable provenance-evidence artifacts.
    Evidence(Vec<Artifact>),
    /// 该产物按定义无需外部证明。 / The artifact requires no external evidence by definition.
    NotApplicable(ProvenanceNonApplicability),
}

/// 来源证明不适用的封闭原因。 / Closed reason why provenance evidence is not applicable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProvenanceNonApplicability {
    /// Build record 自描述。 / The build record is self-describing.
    SelfDescribingBuildRecord,
    /// 查询对象自身就是完整证明。 / The inspected object is itself complete evidence.
    SelfDescribingEvidence,
    /// 该产物类别不定义来源证明关系。 / The artifact kind defines no provenance relation.
    UnsupportedKind,
}

/// 管理器所需的依赖来源服务端口。 / Dependency-source service port required by the manager.
///
/// 适配器必须在这里完成 registry 查询、Git 引用固定以及 checkout 物化；管理器不会使用
/// 隐式网络或进程全局状态。 / The adapter performs registry lookup, Git reference pinning,
/// and checkout materialization here; the manager uses no implicit network or process-global state.
///
/// 带 `service_unavailable` 的默认查询实现只供命令级测试替身省略无关端口。生产组合根必须
/// 实现它所注册的全部原始目录与 blob 端口，不得依赖默认实现。 / Default inspection methods
/// returning `service_unavailable` exist only so command-scoped test doubles can omit unrelated
/// ports. A production composition root must implement every raw catalog and blob port and must
/// never rely on these defaults.
pub trait Services: Send + Sync {
    /// 只读定位新项目的绝对目标、Git 与外围工作区语义。 / Read-only locates a new project's absolute destination, Git, and enclosing-workspace semantics.
    fn locate_project_creation(
        &self,
        _destination: &Path,
        _vcs: VcsChoice,
    ) -> Result<ProjectCreationLocation, ServiceError> {
        unavailable("project creation locator")
    }

    /// 以一个可恢复写边界发布完整项目。 / Publishes a complete project through one recoverable write boundary.
    ///
    /// 返回 [`ProjectCreationStatus::Cancelled`] 只允许发生在持久提交决定之前；决定之后即使
    /// token 随后被设置，也必须返回成功。 / [`ProjectCreationStatus::Cancelled`] is valid only
    /// before the durable commit decision; after that decision, the adapter must return either
    /// `Created` or `CommittedFailure`, even if the token is subsequently set. A created receipt
    /// must describe the request's normalized destination exactly.
    fn create_project(
        &self,
        _request: &CreateProjectRequest,
        _cancellation: CancellationToken,
        _faults: Arc<dyn FaultInjector>,
    ) -> Result<ProjectCreationStatus, ServiceError> {
        unavailable("project creation publisher")
    }

    /// 返回生产适配器使用的显式持久存储布局。 / Returns the explicit persistent-storage layout used by production adapters.
    fn storage_layout(&self, project_root: &Path) -> Result<StorageLayout, ServiceError>;

    /// 按已有锁物化 Registry/Git 包，供仓库冻结完整候选快照。 / Materializes Registry/Git packages from an existing lock so the repository can freeze a complete candidate snapshot.
    fn materialize_locked(
        &self,
        project: &Path,
        lock: &Lockfile,
        mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError>;

    /// 解析并确保所有精确包位置可用。 / Resolves and ensures every exact package location is available.
    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError>;

    /// 返回缓存记录的原始类型化目录；管理器仍会校验其 blob。 / Returns the raw typed cache catalog; the manager still validates its blobs.
    fn cache_records(&self, _project: &Path) -> Result<Vec<CachedAction>, ServiceError> {
        unavailable("cache catalog")
    }

    /// 读取摘要标识的原始 blob；管理器校验大小和摘要。 / Reads a raw digest-addressed blob; the manager validates size and digest.
    fn read_blob(
        &self,
        _project: &Path,
        _digest: &Digest,
    ) -> Result<Option<Vec<u8>>, ServiceError> {
        unavailable("blob reader")
    }

    /// 查询产物目录中的原始记录。 / Looks up a raw artifact-catalog record.
    fn artifact(
        &self,
        _project: &Path,
        _id: &ArtifactId,
    ) -> Result<Option<Artifact>, ServiceError> {
        unavailable("artifact catalog")
    }

    /// 按用户验证后的路径查询产物记录。 / Looks up an artifact record by a user-validated path.
    fn artifact_at(
        &self,
        _project: &Path,
        _locator: &ArtifactLocator,
    ) -> Result<Option<Artifact>, ServiceError> {
        unavailable("artifact path catalog")
    }

    /// 查询目标的静态链接图产物记录。 / Looks up the static-link-map artifact record for a target.
    fn link_map(
        &self,
        _project: &Path,
        _target: &TargetName,
    ) -> Result<Option<Artifact>, ServiceError> {
        unavailable("link-map catalog")
    }

    /// 查询产物的来源证明候选。 / Looks up provenance-evidence candidates for an artifact.
    fn provenance_evidence(
        &self,
        _project: &Path,
        _artifact: &ArtifactId,
    ) -> Result<ProvenanceRelation, ServiceError> {
        unavailable("provenance catalog")
    }

    /// 返回持久 build record 的计划投影；管理器仍会验证 ID 与依赖。 / Returns the plan projection from a persistent build record; the manager still validates IDs and dependencies.
    fn planned_actions(&self, _project: &Path) -> Result<Option<PlanInspection>, ServiceError> {
        unavailable("plan catalog")
    }
}

fn unavailable<T>(service: &str) -> Result<T, ServiceError> {
    Err(ServiceError::new(
        "service_unavailable",
        format!("{service} service is unavailable"),
    ))
}
