//! prompt-squish 的构建规划与调度领域。 / Build-planning and scheduling domain for prompt-squish.
//!
//! 本 crate 只负责纯内存规划与同步调度。它不启动线程或外部进程，也不打印；宿主从
//! [`Scheduler::next_dispatch`] 取得工作，并以 [`ActionResult`] 归还结构化结果。
//! This crate owns pure in-memory planning and synchronous scheduling. It starts
//! neither threads nor external processes and never prints; a host obtains work
//! from [`Scheduler::next_dispatch`] and returns structured [`ActionResult`] values.

#![forbid(unsafe_code)]

mod model;
mod plan;
mod ports;
mod scheduler;

pub use model::{
    Action, ActionEvent, ActionKey, ActionKind, ActionResult, ContentDigest, InputRef, KeyRecipe,
    Output, OutputName, OutputRef, ProducedOutput, ResourceClass, Resources, WorkerFailure,
};
pub use plan::{BuildPlan, PlanError, SemanticGraphDigest};
pub use ports::{ActionIndex, ActionRecord, ArtifactPublisher, BlobStore, Publication};
pub use scheduler::{
    ActionState, CompletionError, Dispatch, ResultSource, ScheduleEvent, Scheduler, SchedulerError,
};
pub use squish_protocol::ActionId;

#[cfg(test)]
mod tests;
