use std::{
    collections::BTreeMap,
    fmt,
    path::{Component, Path, PathBuf},
};

use squish_kernel::CancellationToken;
use squish_project::{Lockfile, Manifest, ResolutionMode};
use squish_protocol::{
    Artifact, ArtifactId, CachedAction, CleanResult, Digest, PlanInspection, TargetName, VcsChoice,
};
use squish_repository::{
    CreateProjectRequest, CreatedProject, FaultInjector, PackageLocation, ProjectVcs,
    WorkspaceMembership,
};
use std::sync::Arc;

use crate::{ArtifactLocator, ServiceError};
use crate::{BuildRuntime, BuildRuntimeProvider};

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

/// 项目清理端口的封闭完成状态。 / Closed completion status of the project-clean port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectCleanStatus {
    /// 清理已完成，并返回实际删除统计。 / Cleaning completed with statistics for actual removals.
    Cleaned(CleanResult),
    /// 删除已经开始，但后续工作失败；统计仅描述已完成删除。 / Removal started but later work failed; statistics describe only completed removals.
    CommittedFailure {
        /// 失败前已经完成的删除统计。 / Removal statistics completed before the failure.
        result: CleanResult,
        /// 清理失败。 / Cleaning failure.
        error: ServiceError,
    },
    /// 在删除任何状态前响应取消。 / Cancellation was honored before any state was removed.
    Cancelled,
}

/// 项目构建布局无效。 / Invalid project build layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectBuildLayoutError {
    /// 必需路径为空。 / A required path is empty.
    Empty(&'static str),
    /// 路径不是锚定绝对路径。 / A path is not anchored and absolute.
    Relative(&'static str),
    /// 路径含 `.` 或 `..`，未结构化规范。 / A path contains `.` or `..` and is not structurally normalized.
    NonNormalized(&'static str),
    /// 项目根不存在，无法验证其规范身份。 / The project root does not exist, so its canonical identity cannot be verified.
    MissingProjectRoot,
    /// 调用者传入的项目根不是文件系统规范路径。 / The supplied project root is not the canonical filesystem path.
    NonCanonicalProjectRoot,
    /// 构建根没有可用于协调文件的末级名称。 / The build root has no final name for coordination files.
    MissingBuildRootName,
    /// `target-dir` 经现有符号链接解析后逃逸项目根。 / The `target-dir` escapes the project root through an existing symbolic link.
    TargetDirEscapesProject,
    /// `target-dir` 的现有祖先无法解析（例如悬空链接）。 / An existing `target-dir` ancestor cannot be resolved (for example, a dangling link).
    UnresolvableTargetDir,
}

impl fmt::Display for ProjectBuildLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(name) => write!(formatter, "storage path `{name}` is empty"),
            Self::Relative(name) => write!(formatter, "storage path `{name}` is not absolute"),
            Self::NonNormalized(name) => {
                write!(formatter, "storage path `{name}` is not normalized")
            }
            Self::MissingProjectRoot => write!(formatter, "project root does not exist"),
            Self::NonCanonicalProjectRoot => write!(formatter, "project root is not canonical"),
            Self::MissingBuildRootName => {
                write!(formatter, "build root has no final path component")
            }
            Self::TargetDirEscapesProject => write!(formatter, "target-dir escapes project root"),
            Self::UnresolvableTargetDir => write!(formatter, "target-dir cannot be resolved"),
        }
    }
}

impl std::error::Error for ProjectBuildLayoutError {}

/// 项目私有构建状态的唯一类型化布局。 / Sole typed layout for project-private build state.
///
/// 调用者只提供规范项目根与清单中的相对 `workspace.target-dir`。所有缓存、工作文件、恢复
/// 元数据和产物均由同一拥有根派生，机器共享存储因而无法被表达。 / Callers provide only
/// the canonical project root and manifest-relative `workspace.target-dir`. Products, caches,
/// work files, and recovery metadata derive from one ownership root, making machine-shared state
/// unrepresentable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectBuildLayout {
    ownership_root: PathBuf,
    artifacts_root: PathBuf,
    source_cache_root: PathBuf,
    cas_root: PathBuf,
    action_index: PathBuf,
    metadata_root: PathBuf,
    publications_root: PathBuf,
    catalog_root: PathBuf,
    layout_marker: PathBuf,
    work_root: PathBuf,
    publication_prefix: PathBuf,
    coordination_lock: PathBuf,
    clean_journal: PathBuf,
    trash_prefix: PathBuf,
}

impl ProjectBuildLayout {
    /// 从规范项目根与清单 `target-dir` 创建完整布局。 / Creates the complete layout from a canonical project root and manifest `target-dir`.
    ///
    /// `project_root` 必须存在且已规范化；`target_dir` 必须是非空且不含 `.`、`..` 的项目
    /// 相对路径。构造不会创建目录。 / `project_root` must exist and already be canonical.
    /// `target_dir` must be a non-empty project-relative path without `.` or `..`. Construction
    /// performs no filesystem writes.
    pub fn new(
        project_root: impl Into<PathBuf>,
        target_dir: impl Into<PathBuf>,
    ) -> Result<Self, ProjectBuildLayoutError> {
        let project_root = normalize_layout_path("project_root", project_root.into())?;
        let canonical = std::fs::canonicalize(&project_root)
            .map_err(|_| ProjectBuildLayoutError::MissingProjectRoot)?;
        if canonical != project_root {
            return Err(ProjectBuildLayoutError::NonCanonicalProjectRoot);
        }
        let target_dir = normalize_relative_path("target_dir", target_dir.into())?;
        let ownership_root = resolve_existing_ancestor(&project_root.join(&target_dir))?;
        if !ownership_root.starts_with(&project_root) {
            return Err(ProjectBuildLayoutError::TargetDirEscapesProject);
        }
        let artifacts_root = ownership_root.join("artifacts");
        let cache_root = ownership_root.join("cache");
        let metadata_root = ownership_root.join("metadata");
        let leaf = ownership_root
            .file_name()
            .ok_or(ProjectBuildLayoutError::MissingBuildRootName)?
            .to_os_string();
        let coordination_parent = ownership_root
            .parent()
            .ok_or(ProjectBuildLayoutError::MissingBuildRootName)?;
        Ok(Self {
            source_cache_root: cache_root.join("sources"),
            cas_root: cache_root.join("cas"),
            action_index: cache_root.join("actions.sqlite3"),
            publications_root: metadata_root.join("publications"),
            catalog_root: metadata_root.join("catalog"),
            layout_marker: metadata_root.join("layout.json"),
            work_root: ownership_root.join("work"),
            publication_prefix: target_dir.join("artifacts"),
            coordination_lock: coordination_parent
                .join(coordination_name(&leaf, ".xmlsquish.lock")),
            clean_journal: coordination_parent
                .join(coordination_name(&leaf, ".xmlsquish.clean.json")),
            trash_prefix: coordination_parent.join(coordination_name(&leaf, ".xmlsquish-trash-")),
            ownership_root,
            artifacts_root,
            metadata_root,
        })
    }

    /// 仅供测试适配器使用的默认项目布局。 / Default project layout intended only for test adapters.
    pub fn project_local_for_tests(project: &Path) -> Self {
        let project = std::fs::canonicalize(project)
            .expect("test project root must exist and be canonicalizable");
        Self::new(project, "target/xmlsquish").expect("default test target-dir is valid")
    }

    /// 返回完整项目私有拥有根。 / Returns the complete project-private ownership root.
    pub fn ownership_root(&self) -> &Path {
        &self.ownership_root
    }
    /// 返回用户产物 generation 根目录。 / Returns the user-artifact generation root.
    pub fn artifacts_root(&self) -> &Path {
        &self.artifacts_root
    }
    /// 返回 Registry/Git 依赖源码缓存根目录。 / Returns the registry/Git dependency-source cache root.
    pub fn source_cache_root(&self) -> &Path {
        &self.source_cache_root
    }
    /// 返回内容寻址存储根目录。 / Returns the content-addressed store root.
    pub fn cas_root(&self) -> &Path {
        &self.cas_root
    }
    /// 返回动作索引数据库路径。 / Returns the action-index database path.
    pub fn action_index(&self) -> &Path {
        &self.action_index
    }
    /// 返回项目相对的公开产物定位符前缀。 / Returns the project-relative public artifact-locator prefix.
    pub fn publication_prefix(&self) -> &Path {
        &self.publication_prefix
    }
    /// 返回恢复与目录元数据根。 / Returns the recovery-and-catalog metadata root.
    pub fn metadata_root(&self) -> &Path {
        &self.metadata_root
    }
    /// 返回 publication 恢复元数据根。 / Returns the publication-recovery metadata root.
    pub fn publications_root(&self) -> &Path {
        &self.publications_root
    }
    /// 返回查询目录根路径。 / Returns the inspection-catalog root.
    pub fn catalog_root(&self) -> &Path {
        &self.catalog_root
    }
    /// 返回布局版本标记。 / Returns the layout-version marker path.
    pub fn layout_marker(&self) -> &Path {
        &self.layout_marker
    }
    /// 返回临时工作根。 / Returns the temporary-work root.
    pub fn work_root(&self) -> &Path {
        &self.work_root
    }
    /// 返回拥有根旁的互斥锁路径。 / Returns the ownership-root sibling lock path.
    pub fn coordination_lock(&self) -> &Path {
        &self.coordination_lock
    }
    /// 返回拥有根旁的清理恢复 journal。 / Returns the ownership-root sibling clean-recovery journal.
    pub fn clean_journal(&self) -> &Path {
        &self.clean_journal
    }
    /// 返回唯一垃圾目录名称前缀。 / Returns the prefix used for unique trash-directory names.
    pub fn trash_prefix(&self) -> &Path {
        &self.trash_prefix
    }
}

/// 保留非 Unicode 路径字节并生成同级协调名称。 / Builds a sibling coordination name while preserving non-Unicode path units.
fn coordination_name(leaf: &std::ffi::OsStr, suffix: &str) -> std::ffi::OsString {
    let mut name = std::ffi::OsString::from(".");
    name.push(leaf);
    name.push(suffix);
    name
}

/// 验证并重建纯普通分量组成的相对路径。 / Validates and rebuilds a relative path made solely of normal components.
fn normalize_relative_path(
    name: &'static str,
    path: PathBuf,
) -> Result<PathBuf, ProjectBuildLayoutError> {
    if path.as_os_str().is_empty() {
        return Err(ProjectBuildLayoutError::Empty(name));
    }
    if path.is_absolute() {
        return Err(ProjectBuildLayoutError::NonNormalized(name));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            _ => return Err(ProjectBuildLayoutError::NonNormalized(name)),
        }
    }
    Ok(normalized)
}

/// 对绝对路径执行不访问文件系统的结构规范化。 / Structurally normalizes an absolute path without filesystem access.
fn normalize_layout_path(
    name: &'static str,
    path: PathBuf,
) -> Result<PathBuf, ProjectBuildLayoutError> {
    if path.as_os_str().is_empty() {
        return Err(ProjectBuildLayoutError::Empty(name));
    }
    if !path.is_absolute() {
        return Err(ProjectBuildLayoutError::Relative(name));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(ProjectBuildLayoutError::NonNormalized(name));
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// 规范化最近的现有祖先，再接回尚不存在的后缀。 / Canonicalizes the nearest existing ancestor and reattaches the nonexistent suffix.
fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf, ProjectBuildLayoutError> {
    let mut existing = path;
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing
                    .file_name()
                    .ok_or(ProjectBuildLayoutError::MissingBuildRootName)?;
                suffix.push(name.to_os_string());
                existing = existing
                    .parent()
                    .ok_or(ProjectBuildLayoutError::MissingBuildRootName)?;
            }
            Err(_) => return Err(ProjectBuildLayoutError::UnresolvableTargetDir),
        }
    }
    let mut resolved = std::fs::canonicalize(existing)
        .map_err(|_| ProjectBuildLayoutError::UnresolvableTargetDir)?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
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
    /// 在规划恢复阶段打开唯一调用级构建运行时。 / Opens the sole invocation-scoped build runtime during planning recovery.
    fn open_build_runtime(
        &self,
        _project_root: &Path,
    ) -> Result<Arc<dyn BuildRuntime>, ServiceError> {
        unavailable("build runtime")
    }
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

    /// 删除完整的项目私有构建拥有根。 / Removes the complete project-private build ownership root.
    ///
    /// 依赖源码、内容寻址存储（CAS）、动作索引、产物与恢复元数据同属此边界。
    /// [`ProjectCleanStatus::Cancelled`] 仅允许在尚未删除任何状态时返回；一旦开始删除，
    /// 适配器必须完成可恢复清理。 / Dependency sources, the content-addressed store (CAS),
    /// action index, products, and recovery metadata all belong to this boundary.
    /// [`ProjectCleanStatus::Cancelled`] is valid only before any state is removed; after removal
    /// begins the adapter must finish recoverably and return [`ProjectCleanStatus::Cleaned`] or
    /// [`ProjectCleanStatus::CommittedFailure`].
    fn clean_project(
        &self,
        _project_root: &Path,
        _cancellation: CancellationToken,
    ) -> Result<ProjectCleanStatus, ServiceError> {
        unavailable("project cleaner")
    }

    /// 返回由项目 `target-dir` 唯一派生的私有构建布局。 / Returns the private build layout uniquely derived from the project's `target-dir`.
    fn storage_layout(&self, project_root: &Path) -> Result<ProjectBuildLayout, ServiceError>;

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

impl<T: Services + ?Sized> BuildRuntimeProvider for T {
    fn open_build_runtime(
        &self,
        project_root: &Path,
    ) -> Result<Arc<dyn BuildRuntime>, ServiceError> {
        Services::open_build_runtime(self, project_root)
    }
}

fn unavailable<T>(service: &str) -> Result<T, ServiceError> {
    Err(ServiceError::new(
        "service_unavailable",
        format!("{service} service is unavailable"),
    ))
}
