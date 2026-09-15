//! prompt-squish 组件间的稳定机器协议。 / Stable machine protocol between prompt-squish components.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, ops::Range};

/// 当前协议版本。 / Current protocol version.
pub const CURRENT_VERSION: ProtocolVersion = ProtocolVersion::new(1, 1);

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
}

/// 由内核分派的类型化操作。 / Typed operation dispatched by the kernel.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "request")]
pub enum OperationRequest {
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
}
impl OperationRequest {
    /// 返回静态路由类别。 / Returns the static routing kind.
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::Build(_) => OperationKind::Build,
            Self::Format(_) => OperationKind::Format,
            Self::Add(_) => OperationKind::Add,
            Self::Remove(_) => OperationKind::Remove,
            Self::Inspect(_) => OperationKind::Inspect,
        }
    }
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
}
/// 查询请求。 / Inspection request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InspectRequest {
    /// 项目路径。 / Project path.
    pub project: ProjectPath,
    /// 查询视图。 / Inspection view.
    pub view: InspectView,
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
        serde_json::from_value(value).map_err(DecodeError::Malformed)
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
    /// 依赖解析。 / Dependency resolution.
    Resolve,
    /// 输入快照。 / Input snapshot.
    Snapshot,
    /// 源码扫描。 / Source scan.
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
    /// 解析候选求值。 / Resolution candidate evaluation.
    ResolveCandidate,
    /// 项目事务提交。 / Project transaction commit.
    CommitTransaction,
}

/// 一个目标已成功提交的完整产物集合。 / Complete artifact set committed for one target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublishedTarget {
    /// 目标名。 / Target name.
    pub target: TargetName,
    /// 同一提交世代中的全部产物。 / Every artifact in the same committed generation.
    pub artifacts: Vec<Artifact>,
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
    /// 产物来源证明。 / Artifact provenance.
    Provenance(ProvenanceInspection),
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
}
impl OperationResult {
    /// 返回与请求路由一致的操作类别。 / Returns the operation kind matching request routing.
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::Unavailable { kind } => *kind,
            Self::Build(_) => OperationKind::Build,
            Self::Format(_) => OperationKind::Format,
            Self::Add(_) => OperationKind::Add,
            Self::Remove(_) => OperationKind::Remove,
            Self::Inspect(_) => OperationKind::Inspect,
        }
    }

    /// 验证结果类别、查询视图及请求携带的对象身份。 / Validates the result kind, inspection view, and request-carried object identity.
    pub fn matches_request(&self, request: &OperationRequest) -> bool {
        match (self, request) {
            (Self::Unavailable { kind }, request) => *kind == request.kind(),
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
            _ => false,
        }
    }

    /// 返回结果是否因领域数据尚不可用而为空。 / Returns whether domain data is unavailable.
    pub const fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
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
            (Self::Provenance(result), InspectView::Provenance(requested)) => {
                &result.artifact.id == requested
            }
            _ => false,
        }
    }
}

/// 完整调度事件代数。 / Complete scheduling event algebra.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
#[non_exhaustive]
pub enum EventPayload {
    /// 计划已冻结。 / Plan is ready.
    PlanReady {
        /// 作业。 / Job.
        job: JobId,
        /// 计划动作总数。 / Planned action count.
        actions: u64,
    },
    /// 动作已排队。 / Action queued.
    ActionQueued {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 核心动作类别。 / Core action kind.
        kind: ActionKind,
        /// 稳定顺序的完整前置动作。 / Complete prerequisite actions in stable order.
        dependencies: Vec<ActionId>,
    },
    /// 动作开始。 / Action started.
    ActionStarted {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
    },
    /// 缓存命中。 / Cache hit.
    CacheHit {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 缓存来源。 / Cache source.
        cache: CacheKind,
        /// 命中内容摘要。 / Hit content digest.
        digest: Digest,
        /// 完整动作键；旧的 v1.0 发送方可能省略。 / Complete action key; legacy v1.0 senders may omit it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action_key: Option<ActionKeyId>,
        /// 命中结果记录引用的全部输出/产物。 / Every output/artifact referenced by the hit result record.
        #[serde(default)]
        outputs: Vec<Artifact>,
    },
    /// 动作成功。 / Action succeeded.
    ActionSucceeded {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 此动作发布的产物。 / Artifacts published by this action.
        artifacts: Vec<Artifact>,
    },
    /// 动作失败。 / Action failed.
    ActionFailed {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 耗时。 / Timing.
        timing: Timing,
        /// 根因诊断。 / Root-cause diagnostic.
        diagnostic: Diagnostic,
    },
    /// 动作被依赖阻塞。 / Action blocked by dependencies.
    ActionBlocked {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 已失败或阻塞的前置动作。 / Failed or blocked prerequisite actions.
        blocked_by: Vec<ActionId>,
    },
    /// 动作取消。 / Action cancelled.
    ActionCancelled {
        /// 作业。 / Job.
        job: JobId,
        /// 动作。 / Action.
        action: ActionId,
        /// 取消前耗时。 / Timing before cancellation.
        timing: Timing,
    },
    /// 非动作诊断。 / Non-action diagnostic.
    Diagnostic(Diagnostic),
    /// 内核确认并发布的类型化操作结果。 / Typed operation result validated and published by the kernel.
    OperationCompleted {
        /// 作业。 / Job.
        job: JobId,
        /// 与请求类别匹配且不含退出码的领域结果。 / Domain result matching the request kind and containing no exit code.
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
    /// 解码已知事件，或跳过同主版本未知事件。 / Decodes a known event or skips an unknown same-major event.
    pub fn decode_json(bytes: &[u8]) -> Result<DecodedEvent, DecodeError> {
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
        let event_type = value
            .pointer("/payload/type")
            .and_then(serde_json::Value::as_str)
            .ok_or(DecodeError::MissingEventType)?;
        if !known_event(event_type) {
            return Ok(DecodedEvent::SkippedUnknown {
                version,
                event_type: event_type.to_owned(),
            });
        }
        serde_json::from_value(value)
            .map(Box::new)
            .map(DecodedEvent::Known)
            .map_err(DecodeError::Malformed)
    }
}
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
fn known_event(value: &str) -> bool {
    matches!(
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn other_major_is_rejected() {
        let json = br#"{"version":{"major":2,"minor":0},"operation":{"type":"inspect","request":{"project":".","view":"project"}}}"#;
        assert!(matches!(
            OperationEnvelope::decode_json(json),
            Err(DecodeError::UnsupportedMajor { .. })
        ));
    }
    #[test]
    fn unknown_additive_event_is_skipped() {
        let json = br#"{"version":{"major":1,"minor":9},"invocation":"i","sequence":7,"payload":{"type":"future_metric","data":{}}}"#;
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
                actions: 2,
            },
        );
        assert_eq!(
            Event::decode_json(&serde_json::to_vec(&event).unwrap()).unwrap(),
            DecodedEvent::Known(Box::new(event))
        );
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
}
