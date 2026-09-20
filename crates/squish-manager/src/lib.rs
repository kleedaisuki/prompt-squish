//! prompt-squish 的统一项目管理能力。 / Unified project-management capability for prompt-squish.
//!
//! 本 crate 把七种用户操作规划为同一种动作图；外部 I/O 仅通过 [`Services`] 进入。
//! This crate plans all seven user operations into one action graph; external I/O enters only
//! through [`Services`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod build;
pub mod clean;
mod error;
pub mod fmt;
pub mod inspect;
mod model;
pub mod mutation;
pub mod new;
pub mod orchestrator;
mod runtime;
mod services;

pub use error::{ManagerError, ServiceError};
pub use model::{
    ArtifactLocator, Effect, InspectSubject, InvalidArtifactLocator, PlannedWork, PreparedPlan,
    PreparedPlanError,
};
pub use runtime::{
    BuildRuntime, BuildRuntimeDescriptor, BuildRuntimeError, BuildRuntimeErrorKind,
    BuildRuntimeProvider, GenerationSpace,
};
pub use services::{
    ProjectBuildLayout, ProjectBuildLayoutError, ProjectCleanStatus, ProjectCreationLocation,
    ProjectCreationStatus, ProvenanceNonApplicability, ProvenanceRelation, ResolveRequest,
    ResolvedDependencies, Services,
};

use squish_kernel::{Capability, CapabilityDescriptor, InvocationContext, OperationOutcome};
use squish_protocol::{OperationKind, OperationRequest, PackageName, VcsChoice};
use squish_repository::{FaultInjector, NoFault};
use std::sync::Arc;

/// 持久化边界端口，用于宿主级故障观测与确定性恢复测试。 /
/// Durability-boundary ports for host-level observation and deterministic recovery tests.
///
/// 发布 observer 是具体发布器的构造输入，因而属于 host/root 组合根，不经过
/// manager。 / Publication observers are concrete-publisher construction inputs and therefore
/// belong to the host/root composition boundary rather than the manager.
#[derive(Clone)]
pub struct DurabilityPorts {
    repository: Arc<dyn FaultInjector>,
}

impl DurabilityPorts {
    /// 由仓库事务故障端口构造。 / Constructs from the repository-transaction fault port.
    pub fn new(repository: Arc<dyn FaultInjector>) -> Self {
        Self { repository }
    }

    pub(crate) fn repository(&self) -> Arc<dyn FaultInjector> {
        self.repository.clone()
    }
}

impl Default for DurabilityPorts {
    fn default() -> Self {
        Self::new(Arc::new(NoFault))
    }
}

/// 一次调用的组合根设置；不读取 CLI 或全局状态。 / Composition-root settings for one invocation; reads neither CLI nor global state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationSettings {
    /// 配置解析后的新项目默认 VCS；CLI 请求仍可覆盖。 / Config-resolved default VCS for new projects; an explicit CLI request still overrides it.
    pub new_vcs: VcsChoice,
    /// 不参与本次操作的包。 / Packages excluded from this operation.
    pub excluded_packages: Vec<PackageName>,
    /// 最大并行作业数；零按一处理。 / Maximum parallel jobs; zero is treated as one.
    pub jobs: usize,
    /// 独立动作失败后是否继续。 / Whether independent work continues after failure.
    pub keep_going: bool,
    /// 协议尚未类型化的查询对象（当前仅缓存键）。 / Inspection subject not yet represented by the typed protocol (currently only a cache key).
    pub inspect_subject: Option<InspectSubject>,
}

impl Default for InvocationSettings {
    fn default() -> Self {
        Self {
            new_vcs: VcsChoice::Git,
            excluded_packages: Vec::new(),
            jobs: 1,
            keep_going: true,
            inspect_subject: None,
        }
    }
}

/// 同时拥有七种项目操作的唯一静态能力。 / Sole static capability owning all seven project operations.
pub struct ManagerCapability<S> {
    services: S,
    settings: InvocationSettings,
    durability: Option<DurabilityPorts>,
}

impl<S> ManagerCapability<S> {
    /// 以显式外部服务构造管理器。 / Constructs the manager with explicit external services.
    pub const fn new(services: S, settings: InvocationSettings) -> Self {
        Self {
            services,
            settings,
            durability: None,
        }
    }

    /// 以显式持久化端口构造管理器。 / Constructs the manager with explicit durability ports.
    ///
    /// 生产组合根通常使用 [`Self::new`]；只有需要观察已完成持久化边界的
    /// host adapter 才应使用此构造器。 / Production composition normally uses
    /// [`Self::new`]; host adapters that must observe completed durability boundaries use this
    /// constructor.
    pub fn with_durability(
        services: S,
        settings: InvocationSettings,
        durability: DurabilityPorts,
    ) -> Self {
        Self {
            services,
            settings,
            durability: Some(durability),
        }
    }

    /// 使用单作业、继续执行的默认设置构造管理器。 / Constructs the manager with single-job, keep-going defaults.
    pub fn with_default_settings(services: S) -> Self {
        Self::new(services, InvocationSettings::default())
    }

    /// 返回外部服务，供组合根检查配置。 / Returns external services for composition-root inspection.
    pub const fn services(&self) -> &S {
        &self.services
    }

    /// 返回本次调用设置。 / Returns settings for this invocation.
    pub const fn settings(&self) -> &InvocationSettings {
        &self.settings
    }
}

static OPERATIONS: &[OperationKind] = &[
    OperationKind::New,
    OperationKind::Build,
    OperationKind::Format,
    OperationKind::Add,
    OperationKind::Remove,
    OperationKind::Inspect,
    OperationKind::Clean,
];

static DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "project-manager",
    operations: OPERATIONS,
    summary: "Create, build, format, mutate, inspect, and clean prompt-squish projects",
};

impl<S: Services> Capability for ManagerCapability<S> {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &DESCRIPTOR
    }

    fn execute(
        &self,
        operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome {
        let default_durability;
        let durability = match self.durability.as_ref() {
            Some(durability) => durability,
            None => {
                default_durability = DurabilityPorts::default();
                &default_durability
            }
        };
        match operation {
            OperationRequest::New(request) => {
                new::execute(request, &self.services, &self.settings, durability, context)
            }
            OperationRequest::Build(request) => build::execute_with_durability(
                request,
                &self.services,
                &self.settings,
                durability,
                context,
            ),
            OperationRequest::Format(request) => fmt::execute_with_durability(
                request,
                &self.services,
                &self.settings,
                durability,
                context,
            ),
            OperationRequest::Add(request) => mutation::add_with_durability(
                request,
                &self.services,
                &self.settings,
                durability,
                context,
            ),
            OperationRequest::Remove(request) => mutation::remove_with_durability(
                request,
                &self.services,
                &self.settings,
                durability,
                context,
            ),
            OperationRequest::Inspect(request) => {
                inspect::execute(request, &self.services, &self.settings, context)
            }
            OperationRequest::Clean(request) => {
                clean::execute(request, &self.services, &self.settings, context)
            }
        }
    }
}
