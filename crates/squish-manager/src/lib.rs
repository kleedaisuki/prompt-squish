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

/// 一次调用的组合根设置；不读取 CLI 或全局状态。 / Composition-root settings for one invocation; reads neither CLI nor global state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationSettings {
    /// 不参与本次操作的包。 / Packages excluded from this operation.
    pub excluded_packages: Vec<PackageName>,
    /// 最大并行作业数；零按一处理。 / Maximum parallel jobs; zero is treated as one.
    pub jobs: usize,
    /// 独立动作失败后是否继续。 / Whether independent work continues after failure.
    pub keep_going: bool,
    /// 协议尚未类型化的查询对象。 / Inspection subject not yet represented by the typed protocol.
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
}

impl<S> ManagerCapability<S> {
    /// 以显式外部服务构造管理器。 / Constructs the manager with explicit external services.
    pub const fn new(services: S, settings: InvocationSettings) -> Self {
        Self { services, settings }
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
        match operation {
            OperationRequest::Build(request) => {
                build::execute(request, &self.services, &self.settings, context)
            }
            OperationRequest::Format(request) => {
                fmt::execute(request, &self.services, &self.settings, context)
            }
            OperationRequest::Add(request) => {
                mutation::add(request, &self.services, &self.settings, context)
            }
            OperationRequest::Remove(request) => {
                mutation::remove(request, &self.services, &self.settings, context)
            }
            OperationRequest::Inspect(request) => {
                inspect::execute(request, &self.services, &self.settings, context)
            }
        }
    }
}
