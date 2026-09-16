//! 构建运行时端口；管理器只编排领域操作。 / Build-runtime ports; the manager only orchestrates domain operations.

use std::{collections::BTreeMap, fmt, path::Path, sync::Arc};

use squish_backend::{BackendCacheIdentity, BackendOutput, BackendRequest, SquishOptions};
use squish_build::{
    ActionKey, ActionRecord, ArtifactRead, CommittedGeneration, GenerationRef, Publication,
    PublicationPath, PublicationTargetId,
};
use squish_ir::SourceKey;
use squish_link::{Budgets, InstantiateOutput, LinkOutput, LinkedProgram, UnitClosure};
use squish_protocol::Digest;
use squish_source::SourceBlob;
use squish_xml_front::{FrontendOutput, FrontendSourceContext};

/// 持久化 generation 的隔离空间。 / Isolated persistence space for generations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationSpace {
    /// 用户目标产物。 / User target artifacts.
    TargetArtifacts,
    /// 项目 build catalog。 / Project build catalog.
    BuildCatalog,
}

/// 封闭计划必须冻结的完整工具链身份。 / Complete toolchain identity frozen into a sealed plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRuntimeDescriptor {
    /// 前端语义 ABI。 / Frontend semantic ABI.
    pub frontend_abi: String,
    /// 静态链接器语义 ABI。 / Static-linker semantic ABI.
    pub linker_abi: String,
    /// 实例化器/求值器语义 ABI。 / Instantiator/evaluator semantic ABI.
    pub evaluator_abi: String,
    /// 传给后端的文档 ABI。 / Document ABI supplied to the backend.
    pub document_abi: String,
}

/// 宿主运行时的稳定结构化错误。 / Stable structured error returned by a host runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildRuntimeErrorKind {
    /// 可用性、I/O 或底层存储故障。 / Availability, I/O, or underlying storage failure.
    Storage,
    /// 已持久状态不满足完整性不变量。 / Persisted state violates an integrity invariant.
    Corrupt,
    /// 工具链或其他非存储运行时失败。 / Toolchain or other non-storage runtime failure.
    Tool,
}

/// 宿主运行时的稳定结构化错误。 / Stable structured error returned by a host runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRuntimeError {
    kind: BuildRuntimeErrorKind,
    code: &'static str,
    message: String,
}

impl BuildRuntimeError {
    /// 从稳定代码和非本地化详情构造工具链故障。 /
    /// Constructs a toolchain failure from a stable code and non-localized detail.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Tool,
            code,
            message: message.into(),
        }
    }

    /// 构造存储/可用性故障。 / Constructs a storage or availability failure.
    pub fn storage(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Storage,
            code,
            message: message.into(),
        }
    }

    /// 构造持久状态完整性故障。 / Constructs a persisted-state integrity failure.
    pub fn corrupt(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: BuildRuntimeErrorKind::Corrupt,
            code,
            message: message.into(),
        }
    }

    /// 返回失败类别。 / Returns the failure category.
    pub const fn kind(&self) -> BuildRuntimeErrorKind {
        self.kind
    }

    /// 返回稳定机器代码。 / Returns the stable machine code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// 返回非本地化详情。 / Returns the non-localized detail.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BuildRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for BuildRuntimeError {}

/// 一次调用共享的完整构建运行时。 / Complete invocation-scoped build runtime.
///
/// 实现拥有具体存储、发布和工具链选择；不得发射 kernel 事件。 /
/// Implementations own concrete storage, publication, and toolchain selection and must not emit
/// kernel events.
pub trait BuildRuntime: Send + Sync {
    /// 返回本对象的工具链身份。 / Returns this object's toolchain identity.
    fn descriptor(&self) -> BuildRuntimeDescriptor;
    /// 返回给定选项的后端缓存身份。 / Returns the backend cache identity for options.
    fn backend_identity(
        &self,
        options: SquishOptions,
    ) -> Result<BackendCacheIdentity, BuildRuntimeError>;
    /// 读取并由运行时校验 blob。 / Reads and runtime-verifies a blob.
    fn read_blob(&self, digest: &Digest) -> Result<Option<Vec<u8>>, BuildRuntimeError>;
    /// 幂等写入 blob，并返回已校验摘要。 / Idempotently writes a blob and returns its verified digest.
    fn write_blob(&self, bytes: &[u8]) -> Result<Digest, BuildRuntimeError>;
    /// 查找已验证动作记录。 / Looks up a verified action record.
    fn lookup_action(&self, key: &ActionKey) -> Result<Option<ActionRecord>, BuildRuntimeError>;
    /// 幂等记录成功动作。 / Idempotently records a successful action.
    fn record_action(&self, record: &ActionRecord) -> Result<(), BuildRuntimeError>;
    /// 查询某隔离空间的当前 generation。 / Queries the current generation in one isolated space.
    fn current_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, BuildRuntimeError>;
    /// 读取并验证 generation 成员。 / Reads and verifies a generation member.
    fn read_generation_artifact(
        &self,
        space: GenerationSpace,
        generation: &GenerationRef,
        destination: &PublicationPath,
    ) -> Result<(ArtifactRead, Vec<u8>), BuildRuntimeError>;
    /// 原子发布完整 generation。 / Atomically publishes a complete generation.
    fn publish_generation(
        &self,
        space: GenerationSpace,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, BuildRuntimeError>;
    /// 使用选定前端编译冻结源。 / Compiles a frozen source with the selected frontend.
    fn compile(
        &self,
        source: &SourceBlob,
        context: &FrontendSourceContext,
    ) -> Result<FrontendOutput, BuildRuntimeError>;
    /// 使用选定链接器链接封闭单元。 / Links a closed unit set with the selected linker.
    fn link(
        &self,
        entry: &SourceKey,
        closure: UnitClosure,
    ) -> Result<LinkOutput, BuildRuntimeError>;
    /// 使用选定求值器实例化程序。 / Instantiates a program with the selected evaluator.
    fn instantiate(
        &self,
        program: &LinkedProgram,
        arguments: BTreeMap<String, String>,
        budgets: Budgets,
    ) -> Result<InstantiateOutput, BuildRuntimeError>;
    /// 使用选定后端渲染产物。 / Renders output with the selected backend.
    fn render(&self, request: BackendRequest) -> Result<BackendOutput, BuildRuntimeError>;
}

/// 在计划恢复边界打开调用级运行时的端口。 / Port opening an invocation runtime at the planning recovery boundary.
pub trait BuildRuntimeProvider: Send + Sync {
    /// 为已规范化项目根打开一个共享运行时。 / Opens one shared runtime for a normalized project root.
    fn open_build_runtime(
        &self,
        project_root: &Path,
    ) -> Result<Arc<dyn BuildRuntime>, crate::ServiceError>;
}
