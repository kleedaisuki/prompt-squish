//! prompt-squish 组件间的稳定机器协议。 / Stable machine protocol between prompt-squish components.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    ops::Range,
    path::{Path, PathBuf},
};

/// 当前协议版本。 / Current protocol version.
pub const CURRENT_VERSION: ProtocolVersion = ProtocolVersion::new(3, 1);

/// 在过渡期内仍可解码的旧事件协议主版本。 / Legacy event-protocol major decoded during the compatibility window.
pub const LEGACY_EVENT_MAJOR: u16 = 1;

/// 遵循主/次兼容规则的协议版本。 / Major/minor compatible protocol version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtocolVersion {
    /// 不兼容变更递增的主版本。 / Major version for incompatible changes.
    pub major: u16,
    /// 仅附加变更递增的次版本。 / Minor version for additive changes.
    pub minor: u16,
}

impl ProtocolVersion {
    /// 创建版本。 / Creates a version.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

/// 空标识符错误。 / Empty identifier error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyId;
impl fmt::Display for EmptyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("identifier must not be empty")
    }
}
impl std::error::Error for EmptyId {}

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            /// 从非空文本创建。 / Creates from non-empty text.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
                let value = value.into();
                if value.is_empty() {
                    Err(EmptyId)
                } else {
                    Ok(Self(value))
                }
            }
            /// 返回协议文本。 / Returns protocol text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl TryFrom<String> for $name {
            type Error = EmptyId;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

string_id!(InvocationId, "一次管理器调用。 / One manager invocation.");
string_id!(CapabilityId, "静态内核能力。 / Static kernel capability.");
string_id!(JobId, "计划执行作业。 / Planned execution job.");
string_id!(
    PlanningAttemptId,
    "作业内唯一的计划尝试。 / Job-scoped unique planning attempt."
);
string_id!(
    PlanningStepId,
    "计划尝试内唯一的真实工作步骤。 / Attempt-scoped unique real planning step."
);
string_id!(
    PlanningIssueId,
    "计划尝试内唯一的问题。 / Attempt-scoped unique planning issue."
);
string_id!(
    FinalizationId,
    "作业内唯一的计划后收尾协调。 / Job-scoped unique post-plan finalization coordination."
);
string_id!(
    PlanScopeId,
    "计划问题影响的稳定目标或源范围。 / Stable target or source scope affected by a planning issue."
);
string_id!(
    PlanId,
    "作业内唯一的不可变计划。 / Job-scoped unique immutable plan."
);
string_id!(ActionId, "作业动作节点。 / Job action node.");
string_id!(
    ActionKeyId,
    "动作全部语义输入的稳定缓存键。 / Stable cache key for every semantic action input."
);
string_id!(ArtifactId, "构建产物。 / Build artifact.");
string_id!(DiagnosticId, "诊断实例。 / Diagnostic instance.");
string_id!(
    ProjectPath,
    "用户选择的项目路径。 / User-selected project path."
);
string_id!(TargetName, "项目目标名。 / Project target name.");
string_id!(DependencyName, "依赖别名。 / Dependency alias.");
string_id!(PackageName, "工作区包名。 / Workspace package name.");
string_id!(ProfileName, "构建配置名。 / Build profile name.");
string_id!(ArgumentName, "入口参数名。 / Entry argument name.");
string_id!(FeatureName, "依赖特性名。 / Dependency feature name.");
string_id!(RegistryName, "包注册表名。 / Package registry name.");
string_id!(
    VersionRequirement,
    "语义版本约束。 / Semantic version requirement."
);
string_id!(RepositoryUrl, "Git 仓库 URL。 / Git repository URL.");
string_id!(GitRevision, "Git 修订。 / Git revision.");
string_id!(GitBranch, "Git 分支名。 / Git branch name.");
string_id!(GitTag, "Git 标签名。 / Git tag name.");
string_id!(StyleEdition, "格式样式版本。 / Formatting style edition.");

/// 非法新项目包名。 / Invalid new-project package name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidNewPackageName;

impl fmt::Display for InvalidNewPackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "package name must be a non-empty sequence of ASCII letters, digits, '-' or '_'",
        )
    }
}

impl std::error::Error for InvalidNewPackageName {}

/// 项目创建与清单共享的严格包名。 / Strict package name shared by project creation and manifests.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct NewPackageName(String);

impl NewPackageName {
    /// 按唯一的清单包名语法验证文本。 / Validates text using the single manifest package-name grammar.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidNewPackageName> {
        let value = value.into();
        validate_new_package_name(&value)?;
        Ok(Self(value))
    }

    /// 判断文本是否符合共享文法。 / Tests text against the shared grammar.
    pub fn is_valid(value: &str) -> bool {
        validate_new_package_name(value).is_ok()
    }

    /// 返回已验证的包名。 / Returns the validated package name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 取得已验证的文本。 / Consumes the value and returns the validated text.
    pub fn into_string(self) -> String {
        self.0
    }
}

/// 按清单与协议共享的唯一文法验证包名。 / Validates a package name using the single grammar shared by manifests and protocol requests.
pub fn validate_new_package_name(value: &str) -> Result<(), InvalidNewPackageName> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(InvalidNewPackageName)
    }
}

impl fmt::Display for NewPackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for NewPackageName {
    type Error = InvalidNewPackageName;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for NewPackageName {
    type Error = InvalidNewPackageName;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<NewPackageName> for String {
    fn from(value: NewPackageName) -> Self {
        value.0
    }
}

/// 尚未存在的项目目标路径。 / Filesystem destination at which a project does not yet exist.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProjectDestination(PathBuf);

impl ProjectDestination {
    /// 创建不访问文件系统的目标路径。 / Creates a destination path without accessing the filesystem.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// 返回用户提供的路径。 / Returns the user-provided path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// 取得用户提供的路径。 / Consumes the value and returns the user-provided path.
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "encoding", content = "units")]
enum ProjectDestinationWire {
    UnixBytes(Vec<u8>),
    WindowsUtf16(Vec<u16>),
    Utf8(String),
}

impl Serialize for ProjectDestination {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            ProjectDestinationWire::UnixBytes(self.0.as_os_str().as_bytes().to_vec())
                .serialize(serializer)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            ProjectDestinationWire::WindowsUtf16(self.0.as_os_str().encode_wide().collect())
                .serialize(serializer)
        }
        #[cfg(not(any(unix, windows)))]
        ProjectDestinationWire::Utf8(self.0.to_string_lossy().into_owned()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProjectDestination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ProjectDestinationWire::deserialize(deserializer)?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            match wire {
                ProjectDestinationWire::UnixBytes(units) => {
                    Ok(Self(std::ffi::OsString::from_vec(units).into()))
                }
                ProjectDestinationWire::Utf8(value) => Ok(Self(value.into())),
                ProjectDestinationWire::WindowsUtf16(_) => Err(serde::de::Error::custom(
                    "a Windows UTF-16 path cannot be decoded on Unix",
                )),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            match wire {
                ProjectDestinationWire::WindowsUtf16(units) => {
                    Ok(Self(std::ffi::OsString::from_wide(&units).into()))
                }
                ProjectDestinationWire::Utf8(value) => Ok(Self(value.into())),
                ProjectDestinationWire::UnixBytes(_) => Err(serde::de::Error::custom(
                    "a Unix byte path cannot be decoded on Windows",
                )),
            }
        }
        #[cfg(not(any(unix, windows)))]
        match wire {
            ProjectDestinationWire::Utf8(value) => Ok(Self(value.into())),
            _ => Err(serde::de::Error::custom(
                "a native path encoding cannot be decoded on this platform",
            )),
        }
    }
}

/// 非规范项目内文件路径。 / Non-canonical project-relative file path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidProjectFilePath;

impl fmt::Display for InvalidProjectFilePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("project file path must be a normalized non-empty relative path")
    }
}

impl std::error::Error for InvalidProjectFilePath {}

/// 使用 `/` 分隔的规范项目内文件路径。 / Normalized `/`-separated project-relative file path.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectFilePath(String);

impl ProjectFilePath {
    /// 验证规范的相对文件路径。 / Validates a normalized relative file path.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidProjectFilePath> {
        let value = value.into();
        let bytes = value.as_bytes();
        let windows_drive_prefixed =
            bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        let normalized = !value.is_empty()
            && !windows_drive_prefixed
            && !value.contains('\\')
            && !value.contains('\0')
            && value
                .split('/')
                .all(|component| !component.is_empty() && !matches!(component, "." | ".."));
        if !normalized {
            return Err(InvalidProjectFilePath);
        }
        Ok(Self(value))
    }

    /// 返回规范路径文本。 / Returns the normalized path text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectFilePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for ProjectFilePath {
    type Error = InvalidProjectFilePath;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ProjectFilePath> for String {
    fn from(value: ProjectFilePath) -> Self {
        value.0
    }
}

/// 未经领域验证的不透明源码身份。 / Opaque source identity not yet domain-validated.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct OpaqueSourceId(String);
impl OpaqueSourceId {
    /// 创建非空不透明身份。 / Creates a non-empty opaque identity.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
        let value = value.into();
        if value.is_empty() {
            Err(EmptyId)
        } else {
            Ok(Self(value))
        }
    }
    /// 返回尚未验证的文本。 / Returns the not-yet-validated text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for OpaqueSourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl TryFrom<String> for OpaqueSourceId {
    type Error = EmptyId;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<OpaqueSourceId> for String {
    fn from(value: OpaqueSourceId) -> Self {
        value.0
    }
}

/// 可静态路由的操作类别。 / Statically routable operation kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// 创建项目。 / Create project.
    New,
    /// 构建。 / Build.
    Build,
    /// 格式化。 / Format.
    Format,
    /// 添加依赖。 / Add dependency.
    Add,
    /// 删除依赖。 / Remove dependency.
    Remove,
    /// 查询。 / Inspect.
    Inspect,
    /// 清理项目构建产物和已确认失效的依赖缓存。 / Clean project build outputs and provably invalid dependency-cache entries.
    Clean,
}

/// 由内核分派的类型化操作。 / Typed operation dispatched by the kernel.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "request")]
pub enum OperationRequest {
    /// 创建项目。 / Create project.
    New(NewRequest),
    /// 构建。 / Build.
    Build(BuildRequest),
    /// 格式化。 / Format.
    Format(FormatRequest),
    /// 添加依赖。 / Add dependency.
    Add(AddRequest),
    /// 删除依赖。 / Remove dependency.
    Remove(RemoveRequest),
    /// 查询状态。 / Inspect state.
    Inspect(InspectRequest),
    /// 清理项目状态。 / Clean project state.
    Clean(CleanRequest),
}
impl OperationRequest {
    /// 返回静态路由类别。 / Returns the static routing kind.
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::New(_) => OperationKind::New,
            Self::Build(_) => OperationKind::Build,
            Self::Format(_) => OperationKind::Format,
            Self::Add(_) => OperationKind::Add,
            Self::Remove(_) => OperationKind::Remove,
            Self::Inspect(_) => OperationKind::Inspect,
            Self::Clean(_) => OperationKind::Clean,
        }
    }

    /// 返回操作面向现有项目还是尚未存在的目标。 / Returns whether the operation addresses an existing project or a prospective destination.
    pub const fn location(&self) -> OperationLocation<'_> {
        match self {
            Self::New(request) => OperationLocation::Prospective(&request.destination),
            Self::Build(request) => OperationLocation::Existing(&request.project),
            Self::Format(request) => OperationLocation::Existing(&request.project),
            Self::Add(request) => OperationLocation::Existing(&request.project),
            Self::Remove(request) => OperationLocation::Existing(&request.project),
            Self::Inspect(request) => OperationLocation::Existing(&request.project),
            Self::Clean(request) => OperationLocation::Existing(&request.project),
        }
    }
}

/// 操作的中心位置代数。 / Central location algebra for manager operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationLocation<'a> {
    /// 必须通过项目发现解析的现有项目。 / Existing project resolved through project discovery.
    Existing(&'a ProjectPath),
    /// 尚未存在且不得通过项目发现解释的目标。 / Prospective destination that must not be interpreted through project discovery.
    Prospective(&'a ProjectDestination),
}

/// 创建操作请求。 / Project-creation request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NewRequest {
    /// 用户提供且尚未存在的目标。 / User-provided destination that does not yet exist.
    pub destination: ProjectDestination,
    /// 显式、已验证的包名；空值要求管理器从目标叶名推断。 / Explicit validated package name; absence asks the manager to infer from the destination leaf.
    pub name: Option<NewPackageName>,
    /// 显式 VCS 策略；空值让配置决定，并最终默认 Git。 / Explicit VCS policy; absence defers to configuration and ultimately defaults to Git.
    pub vcs: Option<VcsChoice>,
}

/// 项目创建的版本控制策略。 / Version-control policy for project creation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VcsChoice {
    /// 使用或创建 Git 仓库。 / Reuse or create a Git repository.
    Git,
    /// 不创建版本控制文件。 / Create no version-control files.
    None,
}

/// 构建请求。 / Build request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BuildRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 工作区选择。 / Workspace selection.
    pub scope: WorkspaceScope,
    /// 空集合选择默认目标。 / Empty selects default targets.
    #[serde(default)]
    pub targets: Vec<TargetName>,
    /// 构建配置。 / Build profile.
    pub profile: ProfileName,
    /// 入口参数。 / Entry arguments.
    #[serde(default)]
    pub arguments: BTreeMap<ArgumentName, String>,
    /// 请求输出。 / Requested outputs.
    pub emit: Vec<EmitKind>,
    /// 解析/网络策略。 / Resolution/network policy.
    pub lock: LockMode,
}
/// 工作区/包选择。 / Workspace or package selection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "packages")]
pub enum WorkspaceScope {
    /// 当前包。 / Current package.
    Current,
    /// 全工作区。 / Entire workspace.
    Workspace,
    /// 明确的包集合。 / Explicit package set.
    Packages(Vec<PackageName>),
}
/// 构建解析与联网模式。 / Build resolution and network mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    /// 必要时更新解析。 / Update resolution when needed.
    Update,
    /// 锁文件不可改变。 / Lockfile must not change.
    Locked,
    /// 禁止网络。 / Forbid network.
    Offline,
    /// 同时锁定并离线。 / Both locked and offline.
    Frozen,
}
/// 构建输出选择。 / Build output selection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitKind {
    /// 二进制 IR。 / Binary IR.
    BinaryIr,
    /// 最终 prompt。 / Final prompt.
    Prompt,
    /// 调试映射。 / Debug mapping.
    DebugInfo,
}
/// 格式化请求。 / Format request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormatRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 工作区选择。 / Workspace selection.
    pub scope: WorkspaceScope,
    /// 源码选择。 / Source selection.
    pub selection: FormatSelection,
    /// 样式版本；空值读取清单。 / Style edition; absent reads the manifest.
    pub style: Option<StyleEdition>,
    /// 只检查而不写回。 / Checks without writing.
    pub check: bool,
    /// 输出机器可读差异。 / Emit a machine-readable diff.
    pub diff: bool,
}
/// 格式化源码选择。 / Formatting source selection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "sources")]
pub enum FormatSelection {
    /// 所选包全部自有源码。 / All owned sources in selected packages.
    All,
    /// 明确的源码集合。 / Explicit source set.
    Sources(Vec<OpaqueSourceId>),
}
/// 依赖来源。 / Dependency source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DependencySource {
    /// 本地路径依赖。 / Local path dependency.
    Path {
        /// 依赖项目路径。 / Dependency project path.
        path: ProjectPath,
    },
    /// 注册表版本。 / Registry version.
    Registry {
        /// 可选非默认注册表。 / Optional non-default registry.
        registry: Option<RegistryName>,
        /// 语义版本约束。 / Semantic version requirement.
        version: VersionRequirement,
    },
    /// Git 来源。 / Git source.
    Git {
        /// 仓库 URL。 / Repository URL.
        repository: RepositoryUrl,
        /// 引用。 / Reference.
        reference: GitReference,
    },
    /// 同工作区包。 / Same-workspace package.
    Workspace {
        /// 包名。 / Package name.
        package: PackageName,
    },
}
/// Git 依赖引用。 / Git dependency reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum GitReference {
    /// 不可变提交。 / Immutable revision.
    Revision(GitRevision),
    /// 分支。 / Branch.
    Branch(GitBranch),
    /// 标签。 / Tag.
    Tag(GitTag),
}
/// 依赖作用域。 / Dependency scope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// 普通构建依赖。 / Normal build dependency.
    Normal,
    /// 开发/测试依赖。 / Development/test dependency.
    Development,
    /// 构建工具依赖。 / Build-tool dependency.
    Build,
}
/// 添加依赖请求。 / Add dependency request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 要编辑的工作区包。 / Workspace package to edit.
    pub package: Option<PackageName>,
    /// 依赖别名。 / Dependency alias.
    pub dependency: DependencyName,
    /// 重命名前的实际包名。 / Actual package name when renamed.
    pub rename: Option<PackageName>,
    /// 依赖来源。 / Dependency source.
    pub source: DependencySource,
    /// 依赖作用域。 / Dependency scope.
    pub kind: DependencyKind,
    /// 启用特性。 / Enabled features.
    #[serde(default)]
    pub features: Vec<FeatureName>,
    /// 关闭默认特性。 / Disable default features.
    pub no_default_features: bool,
    /// 可选依赖。 / Optional dependency.
    pub optional: bool,
    /// 解析与网络策略。 / Resolution and network policy.
    pub lock: LockMode,
    /// 不提交事务。 / Does not commit the transaction.
    pub dry_run: bool,
}
/// 删除依赖请求。 / Remove dependency request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoveRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 要编辑的工作区包。 / Workspace package to edit.
    pub package: Option<PackageName>,
    /// 依赖别名。 / Dependency alias.
    pub dependency: DependencyName,
    /// 依赖作用域。 / Dependency scope.
    pub kind: DependencyKind,
    /// 解析与网络策略。 / Resolution and network policy.
    pub lock: LockMode,
    /// 不提交事务。 / Does not commit the transaction.
    pub dry_run: bool,
}
/// 查询视图。 / Inspection view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectView {
    /// 项目元数据。 / Project metadata.
    Project,
    /// 冻结计划。 / Frozen plan.
    Plan,
    /// 缓存状态。 / Cache status.
    Cache,
    /// IR 产物。 / IR artifact.
    Ir(ArtifactId),
    /// 链接图。 / Link graph.
    Link(TargetName),
    /// 来源与调试位置。 / Source and debug locations.
    Source(OpaqueSourceId),
    /// 产物来源证明。 / Artifact provenance.
    Provenance(ArtifactId),
    /// 按项目内路径定位产物并查询其来源证明。 / Locates an artifact by its project path and inspects its provenance.
    ///
    /// 路径是用户选择器（selector），不是 [`ArtifactId`]。管理器必须通过
    /// 权威产物目录将它解析为真实产物身份。 / The path is a user selector,
    /// not an [`ArtifactId`]; the manager must resolve it through the authoritative
    /// artifact catalog to the real artifact identity.
    Artifact(ProjectPath),
}
/// 查询请求。 / Inspection request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InspectRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 查询视图。 / Inspection view.
    pub view: InspectView,
}

/// 项目清理请求。 / Project-clean request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CleanRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
}
/// 带版本的操作信封。 / Versioned operation envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationEnvelope {
    /// 协议版本。 / Protocol version.
    pub version: ProtocolVersion,
    /// 类型化操作。 / Typed operation.
    pub operation: OperationRequest,
}
impl OperationEnvelope {
    /// 使用当前版本封装。 / Wraps using the current version.
    pub const fn current(operation: OperationRequest) -> Self {
        Self {
            version: CURRENT_VERSION,
            operation,
        }
    }
    /// 解码并验证主版本。 / Decodes and validates the major version.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(DecodeError::Malformed)?;
        let version: ProtocolVersion = serde_json::from_value(
            value
                .get("version")
                .cloned()
                .ok_or(DecodeError::MissingVersion)?,
        )
        .map_err(DecodeError::Malformed)?;
        validate_major(version)?;
        let envelope: Self = serde_json::from_value(value).map_err(DecodeError::Malformed)?;
        if matches!(envelope.operation, OperationRequest::Clean(_)) && version.minor < 1 {
            return Err(DecodeError::OperationRequiresProtocolMinor {
                required: 1,
                received: version.minor,
            });
        }
        Ok(envelope)
    }
}

/// 诊断严重级别。 / Diagnostic severity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 追踪细节。 / Trace detail.
    Trace,
    /// 普通信息。 / Information.
    Info,
    /// 非致命警告。 / Non-fatal warning.
    Warning,
    /// 致命错误。 / Fatal error.
    Error,
}
/// 流水线阶段。 / Pipeline phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 项目发现。 / Project discovery.
    Discover,
    /// 依赖解析。 / Dependency resolution.
    Resolve,
    /// 输入快照。 / Input snapshot.
    Snapshot,
    /// 源码扫描。 / Source scan.
    Scan,
    /// 语法解析。 / Syntax parsing.
    Parse,
    /// 语义分析。 / Semantic analysis.
    Analyze,
    /// IR 实例化。 / IR instantiation.
    Instantiate,
    /// 缓存访问。 / Cache access.
    Cache,
    /// 链接。 / Linking.
    Link,
    /// 后端输出。 / Backend emission.
    Emit,
    /// 格式化。 / Formatting.
    Format,
    /// 项目管理。 / Project management.
    Manage,
    /// 产物发布。 / Artifact publication.
    Publish,
    /// 内核编排。 / Kernel orchestration.
    Orchestrate,
}
/// 非法源码范围。 / Invalid source span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSpan;
impl fmt::Display for InvalidSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("span start exceeds end")
    }
}
impl std::error::Error for InvalidSpan {}
/// 半开源码字节范围。 / Half-open source byte range.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "SpanWire", into = "SpanWire")]
pub struct Span {
    source: OpaqueSourceId,
    bytes: Range<u64>,
}
impl Span {
    /// 创建满足 `start <= end` 的范围。 / Creates a range satisfying `start <= end`.
    pub fn new(source: OpaqueSourceId, start: u64, end: u64) -> Result<Self, InvalidSpan> {
        if start > end {
            Err(InvalidSpan)
        } else {
            Ok(Self {
                source,
                bytes: start..end,
            })
        }
    }
    /// 返回源码身份。 / Returns the source identity.
    pub fn source(&self) -> &OpaqueSourceId {
        &self.source
    }
    /// 返回字节范围。 / Returns the byte range.
    pub fn bytes(&self) -> Range<u64> {
        self.bytes.clone()
    }
}
#[derive(Deserialize, Serialize)]
struct SpanWire {
    source: OpaqueSourceId,
    start: u64,
    end: u64,
}
impl TryFrom<SpanWire> for Span {
    type Error = InvalidSpan;
    fn try_from(value: SpanWire) -> Result<Self, Self::Error> {
        Self::new(value.source, value.start, value.end)
    }
}
impl From<Span> for SpanWire {
    fn from(value: Span) -> Self {
        Self {
            source: value.source,
            start: value.bytes.start,
            end: value.bytes.end,
        }
    }
}
/// 相关源码位置。 / Related source location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelatedSpan {
    /// 范围。 / Span.
    pub span: Span,
    /// 关系说明。 / Relationship label.
    pub label: String,
}
/// 结构化诊断。 / Structured diagnostic.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    /// 诊断 ID。 / Diagnostic ID.
    pub id: DiagnosticId,
    /// 稳定代码。 / Stable code.
    pub code: String,
    /// 严重级别。 / Severity.
    pub severity: Severity,
    /// 阶段。 / Phase.
    pub phase: Phase,
    /// 消息。 / Message.
    pub message: String,
    /// 主要位置。 / Primary span.
    pub primary: Option<Span>,
    /// 相关位置。 / Related spans.
    #[serde(default)]
    pub related: Vec<RelatedSpan>,
    /// 修复建议。 / Remediation hint.
    pub help: Option<String>,
}

/// 摘要算法。 / Digest algorithm.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum DigestAlgorithm {
    /// SHA-256。 / SHA-256.
    Sha256,
    /// BLAKE3。
    Blake3,
    /// 保留名称的扩展算法。 / Named extension algorithm.
    Other(String),
}
/// 内容摘要，线上为小写十六进制。 / Content digest, lowercase hexadecimal on wire.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(try_from = "DigestWire", into = "DigestWire")]
pub struct Digest {
    algorithm: DigestAlgorithm,
    bytes: Vec<u8>,
}
/// 空或非规范摘要。 / Empty or non-canonical digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidDigest;
impl fmt::Display for InvalidDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("digest must have the algorithm's canonical lowercase hexadecimal length")
    }
}
impl std::error::Error for InvalidDigest {}
impl Digest {
    /// 从非空字节创建。 / Creates from non-empty bytes.
    pub fn new(algorithm: DigestAlgorithm, bytes: Vec<u8>) -> Result<Self, InvalidDigest> {
        let valid_length = match &algorithm {
            DigestAlgorithm::Sha256 | DigestAlgorithm::Blake3 => bytes.len() == 32,
            DigestAlgorithm::Other(_) => !bytes.is_empty(),
        };
        if !valid_length {
            Err(InvalidDigest)
        } else {
            Ok(Self { algorithm, bytes })
        }
    }
    /// 返回算法。 / Returns the algorithm.
    pub fn algorithm(&self) -> &DigestAlgorithm {
        &self.algorithm
    }
    /// 返回字节。 / Returns bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// 返回规范十六进制。 / Returns canonical hexadecimal.
    pub fn hex(&self) -> String {
        const D: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(self.bytes.len() * 2);
        for byte in &self.bytes {
            out.push(D[(byte >> 4) as usize] as char);
            out.push(D[(byte & 15) as usize] as char);
        }
        out
    }
}

/// 快照、问题与封闭 DAG 的规范语义摘要。 / Canonical semantic digest of the snapshot, issues, and closed DAG.
///
/// 独立新类型防止将产物或源内容摘要误用为计划身份。 /
/// The distinct newtype prevents artifact or source-content digests from being
/// accidentally substituted for a plan identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PlanDigest(Digest);
impl PlanDigest {
    /// 从已验证的规范摘要创建计划身份。 / Creates a plan identity from an already validated canonical digest.
    pub const fn new(digest: Digest) -> Self {
        Self(digest)
    }

    /// 返回底层规范摘要。 / Returns the underlying canonical digest.
    pub const fn digest(&self) -> &Digest {
        &self.0
    }
}

#[derive(Deserialize, Serialize)]
struct DigestWire {
    algorithm: DigestAlgorithm,
    hex: String,
}
impl TryFrom<DigestWire> for Digest {
    type Error = InvalidDigest;
    fn try_from(value: DigestWire) -> Result<Self, Self::Error> {
        if value.hex.is_empty() || value.hex.len() & 1 == 1 {
            return Err(InvalidDigest);
        }
        let source = value.hex.as_bytes();
        let bytes = (0..source.len())
            .step_by(2)
            .map(|index| Ok((nibble(source[index])? << 4) | nibble(source[index + 1])?))
            .collect::<Result<Vec<_>, InvalidDigest>>()?;
        Self::new(value.algorithm, bytes)
    }
}
impl From<Digest> for DigestWire {
    fn from(value: Digest) -> Self {
        let hex = value.hex();
        Self {
            algorithm: value.algorithm,
            hex,
        }
    }
}
fn nibble(value: u8) -> Result<u8, InvalidDigest> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(InvalidDigest),
    }
}

/// 产物语义类别。 / Semantic artifact kind.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum ArtifactKind {
    /// 二进制 IR。 / Binary IR.
    BinaryIr,
    /// 最终 `.prompt`。 / Final `.prompt`.
    Prompt,
    /// 调试信息。 / Debug information.
    DebugInfo,
    /// 机器元数据。 / Machine metadata.
    Metadata,
    /// 命名扩展类别。 / Named extension kind.
    Other(String),
}
/// 已发布或 CAS 产物；摘要强制存在。 / Published or CAS artifact with mandatory digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Artifact {
    /// 产物 ID。 / Artifact ID.
    pub id: ArtifactId,
    /// 类别。 / Kind.
    pub kind: ArtifactKind,
    /// 宿主解释的 URI。 / Host-interpreted URI.
    pub uri: String,
    /// 字节数。 / Byte size.
    pub size: u64,
    /// 内容寻址摘要。 / Content-addressed digest.
    pub digest: Digest,
}
/// 动作耗时，不参与产物身份。 / Action timing excluded from artifact identity.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Timing {
    /// 墙钟毫秒。 / Wall-clock milliseconds.
    pub elapsed_ms: u64,
}
/// 缓存命中来源。 / Cache hit source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    /// 本机 CAS。 / Local CAS.
    Local,
    /// 远程 CAS。 / Remote CAS.
    Remote,
}
/// 动作终态计数。 / Action terminal counts.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionTotals {
    /// 成功。 / Succeeded.
    pub succeeded: u64,
    /// 失败。 / Failed.
    pub failed: u64,
    /// 阻塞。 / Blocked.
    pub blocked: u64,
    /// 取消。 / Cancelled.
    pub cancelled: u64,
}
impl ActionTotals {
    /// 返回终态总数。 / Returns terminal total.
    pub const fn total(self) -> u64 {
        self.succeeded + self.failed + self.blocked + self.cancelled
    }
}
/// 归约后的退出状态。 / Reduced exit status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitStatus {
    /// 成功。 / Success.
    Success,
    /// 失败。 / Failure.
    Failed,
    /// 取消。 / Cancellation.
    Cancelled,
}
impl ExitStatus {
    /// 返回进程退出码。 / Returns process exit code.
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failed => 1,
            Self::Cancelled => 130,
        }
    }
}
/// 作业摘要。 / Job summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JobSummary {
    /// 作业 ID。 / Job ID.
    pub job: JobId,
    /// 终态计数。 / Terminal counts.
    pub totals: ActionTotals,
    /// 根失败数。 / Root failures.
    pub root_failures: u64,
    /// 缓存命中数。 / Cache hits.
    pub cache_hits: u64,
    /// 总耗时。 / Total timing.
    pub timing: Timing,
    /// 退出状态。 / Exit status.
    pub status: ExitStatus,
}

/// 动作的封闭核心类别。 / Closed set of core action kinds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// 原子创建完整项目及可选工作区成员关系。 / Atomically create a complete project and optional workspace membership.
    CreateProject,
    /// 仅解码 v1 的旧依赖解析占位动作；v2 计划必须使用 [`PlanningStepKind::Resolve`]。 / Legacy v1 dependency-resolution placeholder for decoding only; v2 plans must use [`PlanningStepKind::Resolve`].
    Resolve,
    /// 仅解码 v1 的旧快照占位动作；v2 计划必须使用 [`PlanningStepKind::Snapshot`]。 / Legacy v1 snapshot placeholder for decoding only; v2 plans must use [`PlanningStepKind::Snapshot`].
    Snapshot,
    /// 仅解码 v1 的旧扫描占位动作；v2 计划必须使用 [`PlanningStepKind::Scan`]。 / Legacy v1 scan placeholder for decoding only; v2 plans must use [`PlanningStepKind::Scan`].
    Scan,
    /// 前端编译。 / Frontend compilation.
    Compile,
    /// 链接。 / Link.
    Link,
    /// IR 实例化。 / IR instantiation.
    Instantiate,
    /// 后端生成。 / Backend generation.
    Backend,
    /// 产物发布。 / Artifact publication.
    Publish,
    /// 源码格式化。 / Source formatting.
    Format,
    /// 执行非缓存的查询读取，包括 CAS/文件读取、摘要计算及 IR 或 psdbg 验证。 / Perform a non-cacheable inspection read, including CAS/file reads, hashing, and IR or psdbg validation.
    Inspect,
    /// 删除项目构建产物并裁剪已确认失效的依赖缓存。 / Remove project build outputs and prune provably invalid dependency-cache entries.
    Clean,
    /// 仅解码 v1 的旧候选求值占位动作；v2 计划必须使用 [`PlanningStepKind::PrepareCandidate`]。 / Legacy v1 candidate-evaluation placeholder for decoding only; v2 plans must use [`PlanningStepKind::PrepareCandidate`].
    ResolveCandidate,
    /// 项目事务提交。 / Project transaction commit.
    CommitTransaction,
}

/// 创建项目时产生的可验证公共文件。 / Verifiable public file created with a project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedProjectFile {
    /// 项目内规范相对路径。 / Normalized project-relative path.
    pub path: ProjectFilePath,
    /// 精确文件字节摘要。 / Digest of the exact file bytes.
    pub digest: Digest,
    /// 精确文件字节数。 / Exact file byte size.
    pub size: u64,
}

/// 新包在外围工作区中的落位。 / Placement of a new package in an enclosing workspace.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspacePlacement {
    /// 被复用或修改的绝对工作区清单路径。 / Absolute workspace manifest path reused or modified.
    pub manifest: ProjectDestination,
    /// 写入或确认的规范工作区相对成员路径。 / Normalized workspace-relative member path written or confirmed.
    pub member: ProjectFilePath,
}

/// VCS 操作的实际结果。 / Actual disposition of the VCS operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VcsDisposition {
    /// 创建了新仓库。 / A new repository was created.
    Created,
    /// 复用了外围仓库。 / An enclosing repository was reused.
    Reused,
    /// VCS 已明确禁用。 / Version control was disabled.
    Disabled,
}

/// 创建项目最终采用的 VCS 策略与处置。 / Effective VCS policy and disposition for project creation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VcsResult {
    /// 最终采用的 VCS 类别。 / Effective VCS kind.
    pub kind: VcsChoice,
    /// 仓库创建、复用或禁用结果。 / Repository creation, reuse, or disabled disposition.
    pub disposition: VcsDisposition,
}

/// 创建项目的完整领域结果。 / Complete domain result of project creation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NewResult {
    /// 最终包身份。 / Final package identity.
    pub package: NewPackageName,
    /// 创建后的绝对目标路径。 / Absolute destination path after creation.
    pub path: ProjectDestination,
    /// 创建后的绝对包清单路径。 / Absolute package manifest path after creation.
    pub manifest: ProjectDestination,
    /// 规范脚手架的默认目标。 / Default target in the canonical scaffold.
    pub target: TargetName,
    /// 按规范路径词法序排列的公共文件。 / Public files in lexical order by normalized path.
    pub created: Vec<CreatedProjectFile>,
    /// 外围工作区落位；独立项目为空。 / Enclosing workspace placement, absent for a standalone project.
    pub workspace: Option<WorkspacePlacement>,
    /// 实际采用的版本控制行为。 / Effective version-control behavior.
    pub vcs: VcsResult,
}

/// 一个目标已成功提交的完整产物集合。 / Complete artifact set committed for one target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublishedTarget {
    /// 目标名。 / Target name.
    pub target: TargetName,
    /// 工作区内无歧义的完整发布目标身份。 / Unambiguous complete publication target identity in the workspace.
    pub target_id: String,
    /// 发布器签发的不透明不可变 generation 身份。 / Opaque immutable generation identity issued by the publisher.
    pub generation_id: String,
    /// 同一提交世代中的全部产物。 / Every artifact in the same committed generation.
    pub artifacts: Vec<PublishedArtifact>,
}

/// 成功发布的产物及其稳定逻辑定位器。 / Successfully published artifact and its stable logical locator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublishedArtifact {
    /// 不依赖发布布局的产物 ID。 / Publication-layout-independent artifact ID.
    pub id: ArtifactId,
    /// 类别。 / Kind.
    pub kind: ArtifactKind,
    /// 可原样传给 `inspect artifact` 的规范逻辑定位器。 / Canonical logical locator accepted verbatim by `inspect artifact`.
    pub locator: ProjectFilePath,
    /// 字节数。 / Byte size.
    pub size: u64,
    /// 内容寻址摘要。 / Content-addressed digest.
    pub digest: Digest,
}

/// 构建操作结果；动作失败仍由统一作业摘要计数。 / Build-operation result; action failures remain counted by the common job summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BuildResult {
    /// 成功提交的目标及其完整产物集合。 / Successfully committed targets and their complete artifact sets.
    pub published: Vec<PublishedTarget>,
    /// 记录输入、动作键与最终结果的构建记录；早期失败时可不存在。 / Build record naming inputs, action keys, and final results; absent after an early failure.
    pub build_record: Option<Artifact>,
}

/// 格式化操作结果。 / Format-operation result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormatResult {
    /// 实际检查的稳定源码集合。 / Stable source set actually inspected.
    pub selected: Vec<OpaqueSourceId>,
    /// 会变化或已写回的源码集合。 / Sources that would change or were rewritten.
    pub changed: Vec<OpaqueSourceId>,
    /// 是否为只读检查。 / Whether this was a read-only check.
    pub check: bool,
    /// 请求差异时产生的完整机器差异产物。 / Complete machine diff artifacts produced when requested.
    pub diffs: Vec<Artifact>,
}

/// 一份项目权威状态的内容身份。 / Content identity of authoritative project state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectStateDigests {
    /// 清单内容摘要。 / Manifest content digest.
    pub manifest: Digest,
    /// 锁文件摘要；尚无锁文件时为空。 / Lockfile digest, absent when no lockfile exists yet.
    pub lock: Option<Digest>,
}

/// 添加依赖操作结果。 / Add-dependency operation result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddResult {
    /// 被添加或更新的依赖别名。 / Dependency alias added or updated.
    pub dependency: DependencyName,
    /// 操作前的权威状态。 / Authoritative state before the operation.
    pub before: ProjectStateDigests,
    /// 候选或已提交的权威状态。 / Candidate or committed authoritative state.
    pub after: ProjectStateDigests,
    /// 是否只计算而未提交。 / Whether the change was computed but not committed.
    pub dry_run: bool,
}

/// 删除依赖操作结果。 / Remove-dependency operation result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoveResult {
    /// 被删除的依赖别名。 / Dependency alias removed.
    pub dependency: DependencyName,
    /// 操作前的权威状态。 / Authoritative state before the operation.
    pub before: ProjectStateDigests,
    /// 候选或已提交的权威状态。 / Candidate or committed authoritative state.
    pub after: ProjectStateDigests,
    /// 是否只计算而未提交。 / Whether the change was computed but not committed.
    pub dry_run: bool,
    /// 仍静态引用该别名的源码。 / Sources still statically importing the alias.
    pub affected_sources: Vec<OpaqueSourceId>,
}

/// 项目查询结果。 / Project inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectInspection {
    /// 查询到的工作区包。 / Workspace packages found.
    pub packages: Vec<PackageName>,
    /// 查询到的构建目标。 / Build targets found.
    pub targets: Vec<TargetName>,
}

/// 冻结计划中的一个动作。 / One action in an inspected frozen plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlannedAction {
    /// 动作 ID。 / Action ID.
    pub action: ActionId,
    /// 核心动作类别。 / Core action kind.
    pub kind: ActionKind,
    /// 已物化时的动作键。 / Materialized action key when available.
    pub action_key: Option<ActionKeyId>,
    /// 稳定顺序的完整前置动作。 / Complete prerequisites in stable order.
    pub dependencies: Vec<ActionId>,
}

/// 计划查询结果。 / Plan inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanInspection {
    /// 计划作业。 / Planned job.
    pub job: JobId,
    /// 不可变计划实例。 / Immutable plan instance.
    pub plan: PlanId,
    /// 计划语义身份。 / Semantic plan identity.
    pub digest: PlanDigest,
    /// 执行或仅报告模式。 / Execute or report-only mode.
    pub mode: PlanMode,
    /// 稳定拓扑顺序的动作。 / Actions in stable topological order.
    pub actions: Vec<PlannedAction>,
}

/// 一项可复用的缓存动作结果。 / One reusable cached action result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CachedAction {
    /// 完整动作键。 / Complete action key.
    pub action_key: ActionKeyId,
    /// 动作结果记录摘要。 / Action-result record digest.
    pub result_digest: Digest,
    /// 结果记录引用的完整输出集合。 / Complete output set referenced by the result record.
    pub outputs: Vec<Artifact>,
}

/// 缓存查询结果。 / Cache inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CacheInspection {
    /// 稳定键顺序的缓存结果。 / Cached results in stable key order.
    pub actions: Vec<CachedAction>,
}

/// IR 查询结果。 / IR inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IrInspection {
    /// 被查询的完整 IR 产物。 / Complete inspected IR artifact.
    pub artifact: Artifact,
}

/// 链接图查询结果。 / Link-graph inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LinkInspection {
    /// 查询目标。 / Inspected target.
    pub target: TargetName,
    /// 持久静态链接证据产物。 / Persistent static-link evidence artifact.
    pub link_map: Artifact,
}

/// 源码查询结果。 / Source inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceInspection {
    /// 源码身份。 / Source identity.
    pub source: OpaqueSourceId,
    /// 精确源码字节摘要。 / Exact source-byte digest.
    pub digest: Digest,
    /// 精确源码字节数。 / Exact source byte size.
    pub size: u64,
}

/// 已按 catalog 描述符验证的产物内容。 / Artifact content verified against its catalog descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactInspection {
    /// 被读取的权威产物描述。 / Authoritative artifact descriptor that was read.
    pub artifact: Artifact,
    /// 完整、摘要验证后的原始字节。 / Complete digest-verified raw bytes.
    pub bytes: Vec<u8>,
}

/// 产物来源证明查询结果。 / Artifact-provenance inspection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProvenanceInspection {
    /// 被解释的产物。 / Artifact being explained.
    pub artifact: Artifact,
    /// 完整、可移植的调试或来源证明伴随产物。 / Complete portable debug or provenance companions.
    pub evidence: Vec<Artifact>,
}

/// 与 [`InspectView`] 一一对应的类型化查询负载。 / Typed inspection payload corresponding one-to-one with [`InspectView`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "view", content = "value")]
pub enum InspectResult {
    /// 项目元数据。 / Project metadata.
    Project(ProjectInspection),
    /// 冻结计划。 / Frozen plan.
    Plan(PlanInspection),
    /// 缓存状态。 / Cache state.
    Cache(CacheInspection),
    /// IR 产物。 / IR artifact.
    Ir(IrInspection),
    /// 链接图。 / Link graph.
    Link(LinkInspection),
    /// 源码摘要。 / Source summary.
    Source(SourceInspection),
    /// 逻辑定位器解析出的已验证产物内容。 / Verified artifact content resolved from a logical locator.
    Artifact(ArtifactInspection),
    /// 产物来源证明。 / Artifact provenance.
    Provenance(ProvenanceInspection),
}

/// 项目清理的类型化统计。 / Typed statistics for a project clean.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CleanResult {
    /// 删除的项目构建文件数。 / Number of project build files removed.
    pub build_files: u64,
    /// 删除的项目构建文件字节数。 / Bytes removed from project build files.
    pub build_bytes: u64,
    /// 裁剪的已确认失效或废弃依赖缓存条目数。 / Number of provably invalid or abandoned dependency-cache entries pruned.
    pub invalid_dependency_entries: u64,
    /// 裁剪的已确认失效或废弃依赖缓存字节数。 / Bytes pruned from provably invalid or abandoned dependency-cache entries.
    pub invalid_dependency_bytes: u64,
    /// 因并发使用而安全跳过的依赖缓存条目数。 / Dependency-cache entries safely skipped because they were in concurrent use.
    pub busy_dependency_entries: u64,
}

/// 一次操作的领域结果；退出状态不属于能力结果。 / Domain result of one operation; process exit status is not a capability result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "result")]
pub enum OperationResult {
    /// 操作在产生领域数据前失败或取消。 / Operation failed or was cancelled before producing domain data.
    Unavailable {
        /// 未能产生结果的操作类别。 / Operation kind whose result could not be produced.
        kind: OperationKind,
    },
    /// 创建项目结果。 / Project-creation result.
    New(NewResult),
    /// 构建结果。 / Build result.
    Build(BuildResult),
    /// 格式化结果。 / Format result.
    Format(FormatResult),
    /// 添加依赖结果。 / Add-dependency result.
    Add(AddResult),
    /// 删除依赖结果。 / Remove-dependency result.
    Remove(RemoveResult),
    /// 查询结果。 / Inspection result.
    Inspect(InspectResult),
    /// 清理结果。 / Clean result.
    Clean(CleanResult),
}
impl OperationResult {
    /// 返回与请求路由一致的操作类别。 / Returns the operation kind matching request routing.
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::Unavailable { kind } => *kind,
            Self::New(_) => OperationKind::New,
            Self::Build(_) => OperationKind::Build,
            Self::Format(_) => OperationKind::Format,
            Self::Add(_) => OperationKind::Add,
            Self::Remove(_) => OperationKind::Remove,
            Self::Inspect(_) => OperationKind::Inspect,
            Self::Clean(_) => OperationKind::Clean,
        }
    }

    /// 验证结果类别、查询视图及请求携带的对象身份。 / Validates the result kind, inspection view, and request-carried object identity.
    pub fn matches_request(&self, request: &OperationRequest) -> bool {
        match (self, request) {
            (Self::Unavailable { kind }, request) => *kind == request.kind(),
            (Self::New(result), OperationRequest::New(request)) => {
                request
                    .name
                    .as_ref()
                    .is_none_or(|name| name == &result.package)
                    && request.vcs.is_none_or(|vcs| vcs == result.vcs.kind)
            }
            (Self::Build(_), OperationRequest::Build(_)) => true,
            (Self::Format(result), OperationRequest::Format(request)) => {
                result.check == request.check
            }
            (Self::Add(result), OperationRequest::Add(request)) => {
                result.dependency == request.dependency && result.dry_run == request.dry_run
            }
            (Self::Remove(result), OperationRequest::Remove(request)) => {
                result.dependency == request.dependency && result.dry_run == request.dry_run
            }
            (Self::Inspect(result), OperationRequest::Inspect(request)) => {
                result.matches_view(&request.view)
            }
            (Self::Clean(_), OperationRequest::Clean(_)) => true,
            _ => false,
        }
    }

    /// 返回结果是否因领域数据尚不可用而为空。 / Returns whether domain data is unavailable.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }

    /// 返回可用的领域结果是否满足命令的成功策略。 / Returns whether an available domain result satisfies command success policy.
    ///
    /// 该判定不是诊断或根失败（root failure）：`fmt --check` 可以成功执行所有工作，
    /// 却因发现需要改写的源而不满足命令。这种“否定检查结果”应导致非零进程状态，
    /// 但不应伪造动作失败或诊断。 /
    /// This predicate is neither a diagnostic nor a root failure: `fmt --check`
    /// may execute all work successfully yet fail to satisfy the command because
    /// sources would change. That negative check result should select a non-zero
    /// process status without manufacturing an action failure or diagnostic.
    pub fn command_succeeded(&self) -> bool {
        match self {
            Self::Unavailable { .. } => false,
            Self::Format(result) if result.check => result.changed.is_empty(),
            Self::New(_)
            | Self::Build(_)
            | Self::Format(_)
            | Self::Add(_)
            | Self::Remove(_)
            | Self::Inspect(_)
            | Self::Clean(_) => true,
        }
    }
}

impl InspectResult {
    fn matches_view(&self, view: &InspectView) -> bool {
        match (self, view) {
            (Self::Project(_), InspectView::Project)
            | (Self::Plan(_), InspectView::Plan)
            | (Self::Cache(_), InspectView::Cache) => true,
            (Self::Ir(result), InspectView::Ir(requested)) => &result.artifact.id == requested,
            (Self::Link(result), InspectView::Link(requested)) => &result.target == requested,
            (Self::Source(result), InspectView::Source(requested)) => &result.source == requested,
            (Self::Artifact(_), InspectView::Artifact(_)) => true,
            (Self::Provenance(result), InspectView::Provenance(requested)) => {
                &result.artifact.id == requested
            }
            // A path selects an artifact but does not prescribe its content identity. The
            // manager validates the catalog mapping before returning this typed result. /
            // 路径只选择产物，不预设其内容身份；管理器在返回类型化结果前验证目录映射。
            _ => false,
        }
    }
}

/// 计划的执行策略。 / Execution policy of a sealed plan.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    /// 声明完整图后执行动作。 / Execute actions after declaring the complete graph.
    Execute,
    /// 声明完整图但不启动动作。 / Declare the complete graph without starting actions.
    ReportOnly,
    /// 仅供 v1 解码器丢失性查看的旧投影；不是可由 v2 内核执行的模式。 /
    /// Lossy legacy projection for v1 decoder inspection only; not executable by a v2 kernel.
    Legacy,
}

/// 对用户有意义的真实计划工作类别。 / User-meaningful kind of real planning work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlanningStepKind {
    /// 恢复未完成的权威事务。 / Recover an incomplete authoritative transaction.
    Recover,
    /// 定位项目或工作区。 / Locate a project or workspace.
    Locate,
    /// 解析依赖图。 / Resolve the dependency graph.
    Resolve,
    /// 获取或物化依赖。 / Fetch or materialize dependencies.
    Fetch,
    /// 协调候选锁状态。 / Reconcile candidate lock state.
    ReconcileLock,
    /// 冻结权威输入快照。 / Freeze the authoritative input snapshot.
    Snapshot,
    /// 扫描源闭包和目标。 / Scan the source closure and targets.
    Scan,
    /// 验证并封闭动作图。 / Validate and close the action graph.
    ValidatePlan,
    /// 为项目变更准备精确候选。 / Prepare an exact project-mutation candidate.
    PrepareCandidate,
}

/// 乐观提交动作被废弃的封闭原因。 / Closed reason for superseding optimistic commit work.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupersedeReason {
    /// 提交决定前权威修订已变化。 / Authoritative revision changed before the commit decision.
    AuthoritativeRevisionChanged,
}

/// 取消必须延迟到权威动作提交完成的封闭原因。 / Closed reason why cancellation must be deferred until an authoritative action commit completes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationDeferralReason {
    /// 动作已越过不可逆的持久提交决定。 / The action crossed an irreversible durable commit decision.
    IrreversibleCommit,
}

/// 不可变计划关闭的封闭原因。 / Closed reason for closing an immutable plan.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanCloseReason {
    /// 可执行计划已到终态。 / Executable plan reached terminal state.
    Executed,
    /// 仅报告计划已完整声明。 / Report-only plan was fully declared.
    Reported,
    /// 权威提交决定前计划已被替代。 / Plan was superseded before an authoritative commit decision.
    Superseded,
}

/// 计划关闭后、操作结果发布前的真实收尾工作。 / Real finalization work after plan closure and before operation-result publication.
///
/// 收尾不是执行动作，也不是计划步骤；它用于在失败或取消的计划后仍保存必须的终态事实。 /
/// Finalization is neither an execution action nor a planning step; it records
/// required terminal facts even after a failed or cancelled plan.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FinalizationKind {
    /// 持久化包含失败/取消结果的构建目录记录；开始后必须达到终态而不发布取消事件。 / Persist the build-catalog record including failed/cancelled outcomes; once started it must reach a terminal event and has no cancellation event.
    PersistBuildCatalog,
}

/// 完整的计划与调度事件代数。 / Complete planning and scheduling event algebra.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
#[non_exhaustive]
pub enum EventPayload {
    /// 开始一次计划尝试。 / A planning attempt started.
    PlanningStarted {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
    },
    /// 开始一项真实计划工作。 / A real planning step started.
    PlanningStepStarted {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 计划步骤。 / Planning step.
        step: PlanningStepId,
        /// 类型化类别。 / Typed kind.
        kind: PlanningStepKind,
    },
    /// 计划步骤成功。 / A planning step succeeded.
    PlanningStepSucceeded {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 计划步骤。 / Planning step.
        step: PlanningStepId,
        /// 耗时。 / Timing.
        timing: Timing,
    },
    /// 计划步骤失败。 / A planning step failed.
    PlanningStepFailed {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 计划步骤。 / Planning step.
        step: PlanningStepId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 结构化诊断。 / Structured diagnostic.
        diagnostic: Diagnostic,
    },
    /// 计划步骤被取消。 / A planning step was cancelled.
    PlanningStepCancelled {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 计划步骤。 / Planning step.
        step: PlanningStepId,
        /// 耗时。 / Timing.
        timing: Timing,
    },
    /// 记录不阻止独立子图封闭的范围化问题。 / Records a scoped issue that does not prevent sealing independent subgraphs.
    PlanningIssue {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 计划问题。 / Planning issue.
        issue: PlanningIssueId,
        /// 按规范顺序排列的受影响范围。 / Affected scopes in canonical order.
        affected: Vec<PlanScopeId>,
        /// 结构化诊断。 / Structured diagnostic.
        diagnostic: Diagnostic,
    },
    /// 尝试在产生计划前失败。 / An attempt failed before producing a plan.
    PlanningFailed {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 结构化诊断。 / Structured diagnostic.
        diagnostic: Diagnostic,
    },
    /// 尝试在产生计划前被取消。 / An attempt was cancelled before producing a plan.
    PlanningCancelled {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
    },
    /// 完整不可变计划已原子封闭。 / A complete immutable plan was atomically sealed.
    PlanReady {
        /// 作业。 / Job.
        job: JobId,
        /// 计划尝试。 / Planning attempt.
        attempt: PlanningAttemptId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 语义摘要。 / Semantic digest.
        digest: PlanDigest,
        /// 计划模式。 / Plan mode.
        mode: PlanMode,
        /// 后续唯一动作声明数。 / Number of subsequent unique action declarations.
        actions: u64,
        /// 此前属于此计划的唯一问题数。 / Number of preceding unique issues belonging to this plan.
        issues: u64,
    },
    /// 声明封闭 DAG 的一个顶点。 / Declares one vertex of the sealed DAG.
    ActionDeclared {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 类型化类别。 / Typed kind.
        kind: ActionKind,
        /// 按规范顺序排列的完整前置动作。 / Complete prerequisites in canonical order.
        dependencies: Vec<ActionId>,
    },
    /// 动作开始。 / Action started.
    ActionStarted {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
    },
    /// 缓存命中。 / Cache hit.
    CacheHit {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 缓存来源。 / Cache source.
        cache: CacheKind,
        /// 语义摘要。 / Semantic digest.
        digest: Digest,
        /// 完整动作键；仅 v1 兼容投影可能缺失。 / Complete action key; only a v1 compatibility projection may omit it.
        /// 完整动作键。 / Complete action key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action_key: Option<ActionKeyId>,
        /// 结果记录引用的全部输出。 / Every output referenced by the result record.
        #[serde(default)]
        outputs: Vec<Artifact>,
    },
    /// 动作越过提交决定后，将已请求的取消延迟到动作成功终态。 / Defers requested cancellation to the successful action terminal state after crossing its commit decision.
    ///
    /// 该事件仅在动作处于运行态时有效，并且必须紧邻相同动作的
    /// [`EventPayload::ActionSucceeded`] 或 [`EventPayload::ActionFailed`] 之前发出。 / This
    /// event is valid only while the action is running and must be emitted immediately before
    /// [`EventPayload::ActionSucceeded`] or [`EventPayload::ActionFailed`] for the same action.
    CancellationDeferred {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 已越过持久提交决定的动作。 / Action that crossed its durable commit decision.
        action: ActionId,
        /// 延迟取消的封闭原因。 / Closed reason for deferring cancellation.
        reason: CancellationDeferralReason,
    },
    /// 动作成功。 / Action succeeded.
    ActionSucceeded {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 此动作产生的产物。 / Artifacts produced by this action.
        artifacts: Vec<Artifact>,
    },
    /// 动作失败。 / Action failed.
    ActionFailed {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 结构化诊断。 / Structured diagnostic.
        diagnostic: Diagnostic,
    },
    /// 动作被依赖阻塞。 / Action blocked by dependencies.
    ActionBlocked {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 按规范顺序排列的失败或阻塞前置动作。 / Failed or blocked prerequisites in canonical order.
        blocked_by: Vec<ActionId>,
    },
    /// 动作取消。 / Action cancelled.
    ActionCancelled {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
    },
    /// 动作因权威修订竞争而被废弃。 / Action superseded after an authoritative-revision race.
    ActionSuperseded {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 计划范围内的动作。 / Plan-scoped action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 封闭的状态转换原因。 / Closed state-transition reason.
        reason: SupersedeReason,
    },
    /// 关闭不可变计划。 / Closes an immutable plan.
    PlanClosed {
        /// 作业。 / Job.
        job: JobId,
        /// 不可变计划。 / Immutable plan.
        plan: PlanId,
        /// 封闭的状态转换原因。 / Closed state-transition reason.
        reason: PlanCloseReason,
    },
    /// 在最终计划关闭后开始一项必须达到终态的收尾工作。 / Starts must-terminate finalization after the final plan closes.
    FinalizationStarted {
        /// 作业。 / Job.
        job: JobId,
        /// 作业范围内的收尾身份。 / Job-scoped finalization identity.
        id: FinalizationId,
        /// 收尾工作类别。 / Finalization work kind.
        kind: FinalizationKind,
    },
    /// 收尾工作成功。 / Finalization succeeded.
    FinalizationSucceeded {
        /// 作业。 / Job.
        job: JobId,
        /// 作业范围内的收尾身份。 / Job-scoped finalization identity.
        id: FinalizationId,
        /// 耗时。 / Timing.
        timing: Timing,
    },
    /// 收尾工作失败；诊断描述持久化或恢复失败。 / Finalization failed; the diagnostic describes persistence or recovery failure.
    FinalizationFailed {
        /// 作业。 / Job.
        job: JobId,
        /// 作业范围内的收尾身份。 / Job-scoped finalization identity.
        id: FinalizationId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 结构化失败诊断。 / Structured failure diagnostic.
        diagnostic: Diagnostic,
    },
    /// 非动作诊断。 / Non-action diagnostic.
    Diagnostic(Diagnostic),
    /// 内核确认并发布的类型化操作结果。 / Typed operation result validated and published by the kernel.
    OperationCompleted {
        /// 作业。 / Job.
        job: JobId,
        /// 与请求匹配的领域结果。 / Domain result matching the request.
        result: OperationResult,
    },
    /// 内核生成的作业摘要。 / Kernel-produced job summary.
    JobFinished(JobSummary),
}

/// 版本化事件信封。 / Versioned event envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Event {
    /// 协议版本。 / Protocol version.
    pub version: ProtocolVersion,
    /// 调用 ID。 / Invocation ID.
    pub invocation: InvocationId,
    /// 调用内有序序号；最终事件的值等于此前事件数。 / Ordered invocation sequence; the final event's value equals the preceding event count.
    pub sequence: u64,
    /// 负载。 / Payload.
    pub payload: EventPayload,
}
impl Event {
    /// 创建当前版本事件。 / Creates a current-version event.
    pub fn new(invocation: InvocationId, sequence: u64, payload: EventPayload) -> Self {
        Self {
            version: CURRENT_VERSION,
            invocation,
            sequence,
            payload,
        }
    }
    /// 以稳定字段顺序生成规范 JSON。 / Encodes canonical JSON with stable field ordering.
    ///
    /// 该方法会先拒绝单事件层面的非规范集合；跨事件状态机由内核归约器验证。 /
    /// This rejects non-canonical collections visible within one event first;
    /// cross-event state-machine validation belongs to the kernel reducer.
    pub fn encode_json(&self) -> Result<Vec<u8>, EncodeError> {
        self.validate().map_err(EncodeError::Invalid)?;
        serde_json::to_vec(self).map_err(EncodeError::Serialize)
    }

    /// 验证单事件的局部规范不变式。 / Validates local canonical invariants of one event.
    pub fn validate(&self) -> Result<(), EventValidationError> {
        if matches!(self.payload, EventPayload::CancellationDeferred { .. })
            && (self.version.major < 2 || (self.version.major == 2 && self.version.minor < 1))
        {
            return Err(EventValidationError::EventRequiresProtocolMinor {
                required: 1,
                received: self.version.minor,
            });
        }
        if self.version.major == CURRENT_VERSION.major
            && self.version.minor < 1
            && matches!(
                &self.payload,
                EventPayload::ActionDeclared {
                    kind: ActionKind::Clean,
                    ..
                } | EventPayload::OperationCompleted {
                    result: OperationResult::Clean(_),
                    ..
                } | EventPayload::OperationCompleted {
                    result: OperationResult::Unavailable {
                        kind: OperationKind::Clean
                    },
                    ..
                }
            )
        {
            return Err(EventValidationError::EventRequiresProtocolMinor {
                required: 1,
                received: self.version.minor,
            });
        }
        validate_payload(&self.payload)
    }

    /// 解码 v2 事件，或将 v1 的唯一计划流投影为一个仅供查看的旧计划。 / Decodes v2 or projects v1's sole plan stream into one inspection-only legacy plan.
    ///
    /// 投影不伪造缺失的计划开始/关闭事件，且 [`PlanMode::Legacy`] 必须被严格 v2 归约器拒绝。 /
    /// The projection does not invent missing planning start/close events, and
    /// strict v2 reducers must reject [`PlanMode::Legacy`].
    pub fn decode_json(bytes: &[u8]) -> Result<DecodedEvent, DecodeError> {
        let mut value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(DecodeError::Malformed)?;
        let version: ProtocolVersion = serde_json::from_value(
            value
                .get("version")
                .cloned()
                .ok_or(DecodeError::MissingVersion)?,
        )
        .map_err(DecodeError::Malformed)?;
        if version.major != CURRENT_VERSION.major && version.major != LEGACY_EVENT_MAJOR {
            return Err(DecodeError::UnsupportedMajor {
                received: version.major,
                supported: CURRENT_VERSION.major,
            });
        }
        let event_type = value
            .pointer("/payload/type")
            .and_then(serde_json::Value::as_str)
            .ok_or(DecodeError::MissingEventType)?
            .to_owned();
        if !known_event(&event_type, version.major) {
            return Ok(DecodedEvent::SkippedUnknown {
                version,
                event_type,
            });
        }
        if version.major == LEGACY_EVENT_MAJOR {
            project_v1_event(&mut value, &event_type)?;
        }
        let event: Event = serde_json::from_value(value).map_err(DecodeError::Malformed)?;
        if version.major == CURRENT_VERSION.major {
            event.validate().map_err(DecodeError::InvalidEvent)?;
        }
        Ok(DecodedEvent::Known(Box::new(event)))
    }
}

/// 事件规范编码失败。 / Canonical event encoding failure.
#[derive(Debug)]
pub enum EncodeError {
    /// 事件违反局部不变式。 / Event violates a local invariant.
    Invalid(EventValidationError),
    /// JSON 序列化失败。 / JSON serialization failed.
    Serialize(serde_json::Error),
}
impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(f),
            Self::Serialize(error) => write!(f, "cannot serialize protocol event: {error}"),
        }
    }
}
impl std::error::Error for EncodeError {}

/// 单事件层面的规范验证错误。 / Canonical validation error visible within one event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventValidationError {
    /// 事件类型晚于信封声明的协议次版本。 / Event kind was introduced after the envelope's declared protocol minor.
    EventRequiresProtocolMinor {
        /// 该事件所需的最小次版本。 / Minimum minor required by the event.
        required: u16,
        /// 信封声明的次版本。 / Minor declared by the envelope.
        received: u16,
    },
    /// 列表不是严格递增的唯一集合。 / A list is not a strictly increasing unique set.
    NonCanonicalSet,
    /// 动作将自身声明为依赖或阻塞者。 / An action names itself as a dependency or blocker.
    SelfReference,
    /// 旧 v1 查看投影不能重新编码为原生 v2 事件。 / A legacy v1 inspection projection cannot be re-encoded as a native v2 event.
    LegacyProjection,
    /// 原生 v2 缓存命中缺少完整动作键。 / A native v2 cache hit lacks its complete action key.
    MissingActionKey,
    /// v2 计划将解析、快照或扫描伪造为执行动作。 / A v2 plan represents resolve, snapshot, or scan work as a fake execution action.
    PlanningWorkDeclaredAsAction,
}
impl fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventRequiresProtocolMinor { required, received } => write!(
                f,
                "event requires protocol minor {required}, but envelope declares {received}"
            ),
            Self::NonCanonicalSet => f.write_str("event set must be unique and in canonical order"),
            Self::SelfReference => f.write_str("action must not refer to itself"),
            Self::LegacyProjection => {
                f.write_str("legacy v1 inspection projection is not a native v2 event")
            }
            Self::MissingActionKey => f.write_str("native v2 cache hit requires an action key"),
            Self::PlanningWorkDeclaredAsAction => {
                f.write_str("planning work must not be declared as a v2 execution action")
            }
        }
    }
}
impl std::error::Error for EventValidationError {}
/// 兼容解码结果。 / Compatible decode result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedEvent {
    /// 已知事件。 / Known event.
    Known(Box<Event>),
    /// 安全跳过的附加事件。 / Safely skipped additive event.
    SkippedUnknown {
        /// 发送方协议版本。 / Sender protocol version.
        version: ProtocolVersion,
        /// 未知判别符。 / Unknown discriminator.
        event_type: String,
    },
}
/// 协议解码错误。 / Protocol decode error.
#[derive(Debug)]
pub enum DecodeError {
    /// JSON/负载错误。 / JSON or payload error.
    Malformed(serde_json::Error),
    /// 缺少版本。 / Missing version.
    MissingVersion,
    /// 缺少事件类型。 / Missing event type.
    MissingEventType,
    /// JSON 值缺少投影所需的信封字段。 / JSON value lacks an envelope field required for projection.
    MalformedJsonShape,
    /// 事件违反局部规范不变式。 / Event violates a local canonical invariant.
    InvalidEvent(EventValidationError),
    /// 操作类型晚于信封声明的协议次版本。 / Operation kind was introduced after the envelope's declared protocol minor.
    OperationRequiresProtocolMinor {
        /// 该操作所需的最小次版本。 / Minimum minor required by the operation.
        required: u16,
        /// 信封声明的次版本。 / Minor declared by the envelope.
        received: u16,
    },
    /// 不支持的主版本。 / Unsupported major version.
    UnsupportedMajor {
        /// 收到的主版本。 / Received major version.
        received: u16,
        /// 当前支持的主版本。 / Currently supported major version.
        supported: u16,
    },
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(f, "malformed protocol envelope: {error}"),
            Self::MissingVersion => f.write_str("protocol envelope is missing version"),
            Self::MissingEventType => f.write_str("event envelope is missing payload type"),
            Self::MalformedJsonShape => f.write_str("malformed protocol envelope shape"),
            Self::InvalidEvent(error) => write!(f, "invalid protocol event: {error}"),
            Self::OperationRequiresProtocolMinor { required, received } => write!(
                f,
                "operation requires protocol minor {required}, but envelope declares {received}"
            ),
            Self::UnsupportedMajor {
                received,
                supported,
            } => write!(
                f,
                "unsupported protocol major {received}; supported {supported}"
            ),
        }
    }
}
impl std::error::Error for DecodeError {}
fn validate_major(version: ProtocolVersion) -> Result<(), DecodeError> {
    if version.major == CURRENT_VERSION.major {
        Ok(())
    } else {
        Err(DecodeError::UnsupportedMajor {
            received: version.major,
            supported: CURRENT_VERSION.major,
        })
    }
}
fn known_event(value: &str, major: u16) -> bool {
    if major == LEGACY_EVENT_MAJOR {
        return matches!(
            value,
            "plan_ready"
                | "action_queued"
                | "action_started"
                | "cache_hit"
                | "action_succeeded"
                | "action_failed"
                | "action_blocked"
                | "action_cancelled"
                | "diagnostic"
                | "operation_completed"
                | "job_finished"
        );
    }
    matches!(
        value,
        "planning_started"
            | "planning_step_started"
            | "planning_step_succeeded"
            | "planning_step_failed"
            | "planning_step_cancelled"
            | "planning_issue"
            | "planning_failed"
            | "planning_cancelled"
            | "plan_ready"
            | "action_declared"
            | "action_started"
            | "cache_hit"
            | "cancellation_deferred"
            | "action_succeeded"
            | "action_failed"
            | "action_blocked"
            | "action_cancelled"
            | "action_superseded"
            | "plan_closed"
            | "finalization_started"
            | "finalization_succeeded"
            | "finalization_failed"
            | "diagnostic"
            | "operation_completed"
            | "job_finished"
    )
}

fn validate_payload(payload: &EventPayload) -> Result<(), EventValidationError> {
    match payload {
        EventPayload::PlanReady {
            mode: PlanMode::Legacy,
            ..
        } => Err(EventValidationError::LegacyProjection),
        EventPayload::CacheHit {
            action_key: None, ..
        } => Err(EventValidationError::MissingActionKey),
        EventPayload::ActionDeclared { kind, .. } if !is_v2_executable_action(*kind) => {
            Err(EventValidationError::PlanningWorkDeclaredAsAction)
        }
        EventPayload::ActionDeclared {
            action,
            dependencies,
            ..
        } => validate_action_set(action, dependencies),
        EventPayload::ActionBlocked {
            action, blocked_by, ..
        } => validate_action_set(action, blocked_by),
        EventPayload::PlanningIssue { affected, .. } => validate_ordered(affected),
        _ => Ok(()),
    }
}

fn is_v2_executable_action(kind: ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::CreateProject
            | ActionKind::Compile
            | ActionKind::Link
            | ActionKind::Instantiate
            | ActionKind::Backend
            | ActionKind::Publish
            | ActionKind::Format
            | ActionKind::Inspect
            | ActionKind::Clean
            | ActionKind::CommitTransaction
    )
}

fn validate_action_set(action: &ActionId, values: &[ActionId]) -> Result<(), EventValidationError> {
    if values.iter().any(|value| value == action) {
        return Err(EventValidationError::SelfReference);
    }
    validate_ordered(values)
}

fn validate_ordered<T: Ord>(values: &[T]) -> Result<(), EventValidationError> {
    if values.windows(2).all(|pair| pair[0] < pair[1]) {
        Ok(())
    } else {
        Err(EventValidationError::NonCanonicalSet)
    }
}

fn project_v1_event(value: &mut serde_json::Value, event_type: &str) -> Result<(), DecodeError> {
    let invocation = value
        .get("invocation")
        .and_then(serde_json::Value::as_str)
        .ok_or(DecodeError::MalformedJsonShape)?
        .to_owned();
    let data = value
        .pointer_mut("/payload/data")
        .and_then(serde_json::Value::as_object_mut);
    if let Some(data) = data {
        let job = data
            .get("job")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("legacy-job")
            .to_owned();
        let plan = format!("legacy-v1-plan:{invocation}:{job}");
        match event_type {
            "plan_ready" => {
                data.insert(
                    "attempt".into(),
                    serde_json::Value::String(format!("legacy-v1-attempt:{invocation}:{job}")),
                );
                data.insert("plan".into(), serde_json::Value::String(plan));
                data.insert(
                    "digest".into(),
                    serde_json::to_value(PlanDigest::new(
                        Digest::new(
                            DigestAlgorithm::Other("legacy-v1-projection".into()),
                            format!("{invocation}\0{job}").into_bytes(),
                        )
                        .expect("legacy projection identity is non-empty"),
                    ))
                    .expect("plan digest serialization cannot fail"),
                );
                data.insert("mode".into(), serde_json::Value::String("legacy".into()));
                data.insert("issues".into(), serde_json::Value::from(0));
            }
            "action_queued" => {
                data.insert("plan".into(), serde_json::Value::String(plan));
                value["payload"]["type"] = serde_json::Value::String("action_declared".into());
            }
            "action_started" | "cache_hit" | "action_succeeded" | "action_failed"
            | "action_blocked" | "action_cancelled" => {
                data.insert("plan".into(), serde_json::Value::String(plan));
            }
            _ => {}
        }
    }
    value["version"] =
        serde_json::json!({"major": CURRENT_VERSION.major, "minor": CURRENT_VERSION.minor});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn other_major_is_rejected() {
        let json = br#"{"version":{"major":4,"minor":0},"operation":{"type":"inspect","request":{"project":".","view":"project"}}}"#;
        assert!(matches!(
            OperationEnvelope::decode_json(json),
            Err(DecodeError::UnsupportedMajor { .. })
        ));
    }
    #[test]
    fn unknown_additive_event_is_skipped() {
        let json = br#"{"version":{"major":3,"minor":9},"invocation":"i","sequence":7,"payload":{"type":"future_metric","data":{}}}"#;
        assert!(matches!(
            Event::decode_json(json),
            Ok(DecodedEvent::SkippedUnknown { .. })
        ));
    }
    #[test]
    fn known_event_round_trips() {
        let event = Event::new(
            InvocationId::new("i").unwrap(),
            0,
            EventPayload::PlanReady {
                job: JobId::new("j").unwrap(),
                attempt: PlanningAttemptId::new("attempt-1").unwrap(),
                plan: PlanId::new("plan-1").unwrap(),
                digest: PlanDigest::new(Digest::new(DigestAlgorithm::Sha256, vec![1; 32]).unwrap()),
                mode: PlanMode::Execute,
                actions: 2,
                issues: 0,
            },
        );
        assert_eq!(
            Event::decode_json(&event.encode_json().unwrap()).unwrap(),
            DecodedEvent::Known(Box::new(event))
        );
    }
    #[test]
    fn cancellation_deferred_is_a_native_v2_1_typed_event() {
        let event = Event::new(
            InvocationId::new("i").unwrap(),
            3,
            EventPayload::CancellationDeferred {
                job: JobId::new("j").unwrap(),
                plan: PlanId::new("p").unwrap(),
                action: ActionId::new("create-project").unwrap(),
                reason: CancellationDeferralReason::IrreversibleCommit,
            },
        );
        let encoded = event.encode_json().unwrap();
        let json = std::str::from_utf8(&encoded).unwrap();
        assert!(json.contains(r#""type":"cancellation_deferred""#));
        assert!(json.contains(r#""reason":"irreversible_commit""#));
        assert_eq!(
            Event::decode_json(&encoded).unwrap(),
            DecodedEvent::Known(Box::new(event.clone()))
        );

        let mut mislabeled = event;
        mislabeled.version = ProtocolVersion::new(2, 0);
        assert_eq!(
            mislabeled.validate(),
            Err(EventValidationError::EventRequiresProtocolMinor {
                required: 1,
                received: 0,
            })
        );
        assert!(matches!(
            Event::decode_json(&serde_json::to_vec(&mislabeled).unwrap()),
            Err(DecodeError::UnsupportedMajor { .. })
        ));
    }
    #[test]
    fn legacy_v1_plan_and_queue_project_to_one_marked_inspection_plan() {
        let ready = br#"{"version":{"major":1,"minor":1},"invocation":"i","sequence":0,"payload":{"type":"plan_ready","data":{"job":"j","actions":1}}}"#;
        let queued = br#"{"version":{"major":1,"minor":1},"invocation":"i","sequence":1,"payload":{"type":"action_queued","data":{"job":"j","action":"a","kind":"compile","dependencies":[]}}}"#;
        let DecodedEvent::Known(ready) = Event::decode_json(ready).unwrap() else {
            panic!("legacy plan was skipped");
        };
        let DecodedEvent::Known(queued) = Event::decode_json(queued).unwrap() else {
            panic!("legacy declaration was skipped");
        };
        assert_eq!(
            ready.validate(),
            Err(EventValidationError::LegacyProjection)
        );
        let EventPayload::PlanReady {
            plan: ready_plan,
            mode,
            issues,
            ..
        } = ready.payload
        else {
            panic!("legacy plan was not projected");
        };
        let EventPayload::ActionDeclared {
            plan: queued_plan, ..
        } = queued.payload
        else {
            panic!("legacy queue was not projected");
        };
        assert_eq!(ready.version, CURRENT_VERSION);
        assert_eq!(queued.version, CURRENT_VERSION);
        assert_eq!(ready_plan, queued_plan);
        assert_eq!(mode, PlanMode::Legacy);
        assert_eq!(issues, 0);
    }
    #[test]
    fn legacy_v1_cache_hit_decodes_missing_additive_fields() {
        let json = br#"{"version":{"major":1,"minor":0},"invocation":"i","sequence":3,"payload":{"type":"cache_hit","data":{"job":"j","action":"a","cache":"local","digest":{"algorithm":{"type":"sha256"},"hex":"0000000000000000000000000000000000000000000000000000000000000000"}}}}"#;
        let DecodedEvent::Known(event) = Event::decode_json(json).unwrap() else {
            panic!("known v1.0 cache event was skipped");
        };
        let EventPayload::CacheHit {
            action_key,
            outputs,
            ..
        } = event.payload
        else {
            panic!("decoded the wrong event payload");
        };
        assert_eq!(action_key, None);
        assert!(outputs.is_empty());
    }
    #[test]
    fn declarations_require_canonical_dependencies_without_self_edges() {
        let base = |dependencies| {
            Event::new(
                InvocationId::new("i").unwrap(),
                1,
                EventPayload::ActionDeclared {
                    job: JobId::new("j").unwrap(),
                    plan: PlanId::new("p").unwrap(),
                    action: ActionId::new("b").unwrap(),
                    kind: ActionKind::Compile,
                    dependencies,
                },
            )
        };
        assert!(base(vec![ActionId::new("a").unwrap()]).validate().is_ok());
        assert_eq!(
            base(vec![ActionId::new("b").unwrap()]).validate(),
            Err(EventValidationError::SelfReference)
        );
        assert_eq!(
            base(vec![
                ActionId::new("a").unwrap(),
                ActionId::new("a").unwrap()
            ])
            .validate(),
            Err(EventValidationError::NonCanonicalSet)
        );
        let fake = Event::new(
            InvocationId::new("i").unwrap(),
            2,
            EventPayload::ActionDeclared {
                job: JobId::new("j").unwrap(),
                plan: PlanId::new("p").unwrap(),
                action: ActionId::new("resolve").unwrap(),
                kind: ActionKind::Resolve,
                dependencies: vec![],
            },
        );
        assert_eq!(
            fake.validate(),
            Err(EventValidationError::PlanningWorkDeclaredAsAction)
        );
    }
    #[test]
    fn report_only_plan_has_explicit_seal_declarations_and_reported_close() {
        let invocation = InvocationId::new("i").unwrap();
        let job = JobId::new("j").unwrap();
        let plan = PlanId::new("p").unwrap();
        let ready = Event::new(
            invocation.clone(),
            2,
            EventPayload::PlanReady {
                job: job.clone(),
                attempt: PlanningAttemptId::new("a").unwrap(),
                plan: plan.clone(),
                digest: PlanDigest::new(Digest::new(DigestAlgorithm::Blake3, vec![2; 32]).unwrap()),
                mode: PlanMode::ReportOnly,
                actions: 1,
                issues: 0,
            },
        );
        let declared = Event::new(
            invocation.clone(),
            3,
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan.clone(),
                action: ActionId::new("compile.main").unwrap(),
                kind: ActionKind::Compile,
                dependencies: vec![],
            },
        );
        let closed = Event::new(
            invocation,
            4,
            EventPayload::PlanClosed {
                job,
                plan,
                reason: PlanCloseReason::Reported,
            },
        );

        for event in [ready, declared, closed] {
            let bytes = event.encode_json().unwrap();
            assert_eq!(
                Event::decode_json(&bytes).unwrap(),
                DecodedEvent::Known(Box::new(event))
            );
        }
    }
    #[test]
    fn inspect_is_a_truthful_native_read_effect_action() {
        let event = Event::new(
            InvocationId::new("invocation").unwrap(),
            7,
            EventPayload::ActionDeclared {
                job: JobId::new("inspect-job").unwrap(),
                plan: PlanId::new("inspect-plan").unwrap(),
                action: ActionId::new("inspect.read-effect").unwrap(),
                kind: ActionKind::Inspect,
                dependencies: vec![],
            },
        );

        assert!(event.validate().is_ok());
        let bytes = event.encode_json().unwrap();
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .contains(r#""kind":"inspect""#)
        );
        assert_eq!(
            Event::decode_json(&bytes).unwrap(),
            DecodedEvent::Known(Box::new(event))
        );
    }
    #[test]
    fn post_plan_build_catalog_finalization_round_trips_terminal_outcomes() {
        let invocation = InvocationId::new("invocation").unwrap();
        let job = JobId::new("job").unwrap();
        let id = FinalizationId::new("persist-build-catalog").unwrap();
        let diagnostic = Diagnostic {
            id: DiagnosticId::new("catalog-write-failed").unwrap(),
            code: "store.build_catalog_write".into(),
            severity: Severity::Error,
            phase: Phase::Orchestrate,
            message: "could not persist terminal build facts".into(),
            primary: None,
            related: vec![],
            help: None,
        };
        let events = [
            Event::new(
                invocation.clone(),
                10,
                EventPayload::FinalizationStarted {
                    job: job.clone(),
                    id: id.clone(),
                    kind: FinalizationKind::PersistBuildCatalog,
                },
            ),
            Event::new(
                invocation.clone(),
                11,
                EventPayload::FinalizationSucceeded {
                    job: job.clone(),
                    id: id.clone(),
                    timing: Timing { elapsed_ms: 4 },
                },
            ),
            Event::new(
                invocation,
                11,
                EventPayload::FinalizationFailed {
                    job,
                    id,
                    timing: Timing { elapsed_ms: 4 },
                    diagnostic,
                },
            ),
        ];

        for event in events {
            let bytes = event.encode_json().unwrap();
            assert_eq!(
                Event::decode_json(&bytes).unwrap(),
                DecodedEvent::Known(Box::new(event))
            );
        }

        let empty_id = br#"{"version":{"major":3,"minor":0},"invocation":"i","sequence":1,"payload":{"type":"finalization_started","data":{"job":"j","id":"","kind":"persist_build_catalog"}}}"#;
        assert!(matches!(
            Event::decode_json(empty_id),
            Err(DecodeError::Malformed(_))
        ));
    }
    #[test]
    fn inspect_result_matching_checks_view_and_identity() {
        let source = OpaqueSourceId::new("src/main.squish").unwrap();
        let request = OperationRequest::Inspect(InspectRequest {
            project: ProjectPath::new(".").unwrap(),
            view: InspectView::Source(source.clone()),
        });
        let matching = OperationResult::Inspect(InspectResult::Source(SourceInspection {
            source: source.clone(),
            digest: Digest::new(DigestAlgorithm::Sha256, vec![0; 32]).unwrap(),
            size: 10,
        }));
        let wrong_identity = OperationResult::Inspect(InspectResult::Source(SourceInspection {
            source: OpaqueSourceId::new("src/other.squish").unwrap(),
            digest: Digest::new(DigestAlgorithm::Sha256, vec![0; 32]).unwrap(),
            size: 10,
        }));
        let wrong_view = OperationResult::Inspect(InspectResult::Project(ProjectInspection {
            packages: vec![],
            targets: vec![],
        }));
        assert!(matching.matches_request(&request));
        assert!(!wrong_identity.matches_request(&request));
        assert!(!wrong_view.matches_request(&request));
        assert!(
            OperationResult::Unavailable {
                kind: OperationKind::Inspect
            }
            .matches_request(&request)
        );
    }
    #[test]
    fn artifact_path_selector_matches_resolved_content_by_result_type() {
        let request = OperationRequest::Inspect(InspectRequest {
            project: ProjectPath::new(".").unwrap(),
            view: InspectView::Artifact(ProjectPath::new("target/main.prompt").unwrap()),
        });
        let artifact = Artifact {
            id: ArtifactId::new("prompt:sha256:content-identity").unwrap(),
            kind: ArtifactKind::Prompt,
            uri: "cas://sha256/content-identity".into(),
            digest: Digest::new(DigestAlgorithm::Sha256, vec![7; 32]).unwrap(),
            size: 42,
        };
        let result = OperationResult::Inspect(InspectResult::Artifact(ArtifactInspection {
            artifact,
            bytes: vec![7; 42],
        }));

        assert!(result.matches_request(&request));
        assert!(
            !OperationResult::Inspect(InspectResult::Project(ProjectInspection {
                packages: Vec::new(),
                targets: Vec::new(),
            }))
            .matches_request(&request)
        );
    }
    #[test]
    fn command_success_distinguishes_negative_format_check_from_execution_failure() {
        let source = OpaqueSourceId::new("src/main.xml").unwrap();
        let format = |check, changed| {
            OperationResult::Format(FormatResult {
                selected: vec![source.clone()],
                changed,
                check,
                diffs: vec![],
            })
        };

        assert!(format(true, vec![]).command_succeeded());
        assert!(!format(true, vec![source.clone()]).command_succeeded());
        assert!(format(false, vec![source.clone()]).command_succeeded());
        assert!(
            OperationResult::Build(BuildResult {
                published: vec![],
                build_record: None,
            })
            .command_succeeded()
        );
        assert!(
            !OperationResult::Unavailable {
                kind: OperationKind::Format,
            }
            .command_succeeded()
        );
    }
    #[test]
    fn digest_has_canonical_hex_wire_form() {
        let mut bytes = vec![0; 32];
        bytes[0] = 1;
        bytes[1] = 0xaf;
        let digest = Digest::new(DigestAlgorithm::Sha256, bytes).unwrap();
        let json = serde_json::to_string(&digest).unwrap();
        assert!(json.contains(r#""hex":"01af00"#));
        assert_eq!(serde_json::from_str::<Digest>(&json).unwrap(), digest);
        assert!(
            serde_json::from_str::<Digest>(r#"{"algorithm":{"type":"sha256"},"hex":"AF"}"#)
                .is_err()
        );
    }

    #[test]
    fn existing_package_name_retains_v2_0_nonempty_identifier_contract() {
        for accepted in ["a", "prompt.common", "has space", "café", "slash/name"] {
            let name = PackageName::new(accepted).unwrap();
            let json = serde_json::to_string(&name).unwrap();
            assert_eq!(serde_json::from_str::<PackageName>(&json).unwrap(), name);
        }
        assert!(PackageName::new("").is_err());
        assert!(serde_json::from_str::<PackageName>(r#"""#).is_err());
    }

    #[test]
    fn new_package_name_has_strict_wire_and_constructor_grammar() {
        for valid in ["a", "Prompt_2", "prompt-common"] {
            let name = NewPackageName::new(valid).unwrap();
            let json = serde_json::to_string(&name).unwrap();
            assert_eq!(serde_json::from_str::<NewPackageName>(&json).unwrap(), name);
        }
        for invalid in ["", "prompt.common", "has space", "café", "slash/name"] {
            assert!(NewPackageName::new(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_value::<NewPackageName>(serde_json::json!(invalid)).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn project_file_paths_reject_non_normalized_or_escaping_forms() {
        for valid in ["xmlsquish.toml", ".gitignore", "src/prompt.xml"] {
            assert_eq!(ProjectFilePath::new(valid).unwrap().as_str(), valid);
        }
        for invalid in [
            "",
            "/absolute",
            "../escape",
            "src/../escape",
            "./file",
            "src\\file",
            "src/",
            "C:/absolute",
            "C:drive-relative",
            "z:",
        ] {
            assert!(ProjectFilePath::new(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn new_request_and_result_round_trip_with_truthful_identity_matching() {
        let request = OperationRequest::New(NewRequest {
            destination: ProjectDestination::new("relative/support"),
            name: Some(NewPackageName::new("support").unwrap()),
            vcs: Some(VcsChoice::Git),
        });
        assert_eq!(request.kind(), OperationKind::New);
        assert!(matches!(
            request.location(),
            OperationLocation::Prospective(destination)
                if destination.as_path() == Path::new("relative/support")
        ));
        let request_json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<OperationRequest>(&request_json).unwrap(),
            request
        );

        let result = OperationResult::New(NewResult {
            package: NewPackageName::new("support").unwrap(),
            path: ProjectDestination::new("/absolute/support"),
            manifest: ProjectDestination::new("/absolute/support/xmlsquish.toml"),
            target: TargetName::new("prompt").unwrap(),
            created: vec![CreatedProjectFile {
                path: ProjectFilePath::new("xmlsquish.toml").unwrap(),
                digest: Digest::new(DigestAlgorithm::Sha256, vec![3; 32]).unwrap(),
                size: 42,
            }],
            workspace: None,
            vcs: VcsResult {
                kind: VcsChoice::Git,
                disposition: VcsDisposition::Created,
            },
        });
        assert_eq!(result.kind(), OperationKind::New);
        assert!(result.command_succeeded());
        assert!(result.matches_request(&request));
        let result_json = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serde_json::from_str::<OperationResult>(&result_json).unwrap(),
            result
        );

        let wrong_vcs = OperationRequest::New(NewRequest {
            destination: ProjectDestination::new("relative/support"),
            name: Some(NewPackageName::new("support").unwrap()),
            vcs: Some(VcsChoice::None),
        });
        assert!(!result.matches_request(&wrong_vcs));
    }

    #[test]
    fn clean_is_a_versioned_typed_request_and_result() {
        let v3_inspect = br#"{"version":{"major":3,"minor":0},"operation":{"type":"inspect","request":{"project":".","view":"project"}}}"#;
        assert!(OperationEnvelope::decode_json(v3_inspect).is_ok());

        let request = OperationRequest::Clean(CleanRequest {
            project: ProjectPath::new("workspace/xmlsquish.toml").unwrap(),
        });
        assert_eq!(request.kind(), OperationKind::Clean);
        assert!(matches!(
            request.location(),
            OperationLocation::Existing(project) if project.as_str() == "workspace/xmlsquish.toml"
        ));
        let envelope = OperationEnvelope::current(request.clone());
        let encoded = serde_json::to_vec(&envelope).unwrap();
        assert_eq!(OperationEnvelope::decode_json(&encoded).unwrap(), envelope);

        let result = OperationResult::Clean(CleanResult {
            build_files: 3,
            build_bytes: 1_024,
            invalid_dependency_entries: 2,
            invalid_dependency_bytes: 4_096,
            busy_dependency_entries: 1,
        });
        assert_eq!(result.kind(), OperationKind::Clean);
        assert!(result.matches_request(&request));
        assert!(result.command_succeeded());
        assert_eq!(
            serde_json::from_str::<OperationResult>(&serde_json::to_string(&result).unwrap())
                .unwrap(),
            result
        );

        let mut mislabeled = serde_json::to_value(envelope).unwrap();
        mislabeled["version"]["minor"] = serde_json::json!(0);
        assert!(matches!(
            OperationEnvelope::decode_json(&serde_json::to_vec(&mislabeled).unwrap()),
            Err(DecodeError::OperationRequiresProtocolMinor {
                required: 1,
                received: 0
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn project_destination_wire_round_trips_non_utf8_unix_bytes_losslessly() {
        use std::os::unix::ffi::OsStringExt;

        let destination = ProjectDestination::new(std::ffi::OsString::from_vec(vec![
            b'p', b'r', b'o', b'j', 0xff,
        ]));
        let json = serde_json::to_string(&destination).unwrap();
        assert!(json.contains(r#""encoding":"unix_bytes""#));
        assert_eq!(
            serde_json::from_str::<ProjectDestination>(&json).unwrap(),
            destination
        );
    }

    #[cfg(windows)]
    #[test]
    fn project_destination_wire_round_trips_unpaired_utf16_losslessly() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let original = std::ffi::OsString::from_wide(&[b'p' as u16, 0xd800, b'x' as u16]);
        let destination = ProjectDestination::new(original.clone());
        let json = serde_json::to_string(&destination).unwrap();
        assert!(json.contains(r#""encoding":"windows_utf16""#));
        let decoded = serde_json::from_str::<ProjectDestination>(&json).unwrap();
        assert_eq!(
            decoded
                .as_path()
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>(),
            original.encode_wide().collect::<Vec<_>>()
        );
    }
}
