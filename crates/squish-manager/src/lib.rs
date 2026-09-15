//! prompt-squish 的统一项目管理能力。 / Unified project-management capability for prompt-squish.
//!
//! 本 crate 把五种用户操作规划为同一种动作图；外部 I/O 仅通过 [`Services`] 进入。
//! This crate plans all five user operations into one action graph; external I/O enters only
//! through [`Services`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod build;
mod error;
pub mod fmt;
pub mod inspect;
mod model;
pub mod mutation;
pub mod orchestrator;
mod services;

pub use error::{ManagerError, ServiceError};
pub use model::{
    ArtifactLocator, Effect, InspectSubject, InvalidArtifactLocator, PlannedWork, PreparedPlan,
    PreparedPlanError,
};
pub use services::{
    ProvenanceNonApplicability, ProvenanceRelation, ResolveRequest, ResolvedDependencies, Services,
    StorageLayout, StorageLayoutError,
};

use squish_kernel::{Capability, CapabilityDescriptor, InvocationContext, OperationOutcome};
use squish_protocol::{OperationKind, OperationRequest, PackageName};
use squish_publish::{NoopObserver as NoopPublishObserver, PublishObserver};
use squish_repository::{FaultInjector, NoFault};
use std::sync::Arc;

/// 持久化边界端口，用于宿主级故障观测与确定性恢复测试。 /
/// Durability-boundary ports for host-level observation and deterministic recovery tests.
///
/// 三个端口按持久化责任分区：仓库事务、目标产物 generation 与 build
/// catalog generation 不会依赖“第几次事件”这种脆弱的全局计数。生产默认值不注入
/// 故障且不观测发布事件。 / The three ports are partitioned by durability
/// responsibility: repository transactions, target artifact generations, and build-catalog
/// generations never depend on a brittle global occurrence counter. Production defaults inject
/// no faults and observe no publication events.
#[derive(Clone)]
pub struct DurabilityPorts {
    repository: Arc<dyn FaultInjector>,
    artifact_generation: Arc<dyn PublishObserver>,
    build_catalog: Arc<dyn PublishObserver>,
}

impl DurabilityPorts {
    /// 由三个独立的持久化端口构造。 / Constructs independent durability ports for each persistence domain.
    pub fn new(
        repository: Arc<dyn FaultInjector>,
        artifact_generation: Arc<dyn PublishObserver>,
        build_catalog: Arc<dyn PublishObserver>,
    ) -> Self {
        Self {
            repository,
            artifact_generation,
            build_catalog,
        }
    }

    pub(crate) fn repository(&self) -> Arc<dyn FaultInjector> {
        self.repository.clone()
    }

    pub(crate) fn artifact_generation(&self) -> Arc<dyn PublishObserver> {
        self.artifact_generation.clone()
    }

    pub(crate) fn build_catalog(&self) -> Arc<dyn PublishObserver> {
        self.build_catalog.clone()
    }
}

impl Default for DurabilityPorts {
    fn default() -> Self {
        Self::new(
            Arc::new(NoFault),
            Arc::new(NoopPublishObserver),
            Arc::new(NoopPublishObserver),
        )
    }
}

/// 一次调用的组合根设置；不读取 CLI 或全局状态。 / Composition-root settings for one invocation; reads neither CLI nor global state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationSettings {
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
            excluded_packages: Vec::new(),
            jobs: 1,
            keep_going: true,
            inspect_subject: None,
        }
    }
}

/// 同时拥有五种项目操作的唯一静态能力。 / Sole static capability owning all five project operations.
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
    OperationKind::Build,
    OperationKind::Format,
    OperationKind::Add,
    OperationKind::Remove,
    OperationKind::Inspect,
];

static DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "project-manager",
    operations: OPERATIONS,
    summary: "Build, format, mutate, and inspect prompt-squish projects",
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
        }
    }
}
