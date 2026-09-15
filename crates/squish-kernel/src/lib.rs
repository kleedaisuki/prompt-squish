//! prompt-squish 的领域无关静态微内核。 / Domain-neutral static microkernel for prompt-squish.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use squish_protocol::{
    ActionId, ActionTotals, CapabilityId, Event, EventPayload, ExitStatus, InvocationId, JobId,
    JobSummary, OperationKind, OperationRequest, OperationResult, Timing,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

/// 静态能力的可展示元数据。 / Display metadata for a static capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDescriptor {
    /// 跨版本稳定的能力 ID。 / Capability ID stable across versions.
    pub id: &'static str,
    /// 此能力处理的类型化操作。 / Typed operations handled by this capability.
    pub operations: &'static [OperationKind],
    /// 面向用户的一行说明。 / One-line user-facing summary.
    pub summary: &'static str,
}

/// 能力对一次完整作业的声明结果。 / Capability-declared result for one complete job.
///
/// 内核会与实际事件归约结果逐项核对，因此能力不能自行选择退出码。
/// The kernel compares every field with reduced events, so capabilities cannot
/// choose their own exit code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationOutcome {
    /// 作业 ID。 / Job ID.
    pub job: JobId,
    /// 与请求类别匹配且不含退出码的领域结果。 / Domain result matching the request kind and containing no exit code.
    pub result: OperationResult,
    /// 所有动作的终态计数。 / Terminal counts for every action.
    pub totals: ActionTotals,
    /// 自身执行失败（非阻塞）的根失败数。 / Root execution failures, excluding blocked actions.
    pub root_failures: u64,
    /// 调用是否被取消。 / Whether the invocation was cancelled.
    pub cancelled: bool,
}

/// 编译期链接且启动时显式注册的能力。 / Capability linked at compile time and registered explicitly at startup.
pub trait Capability: Sync {
    /// 返回静态描述。 / Returns the static descriptor.
    fn descriptor(&self) -> &'static CapabilityDescriptor;
    /// 执行类型化操作并返回可验证结果。 / Executes a typed operation and returns a verifiable outcome.
    fn execute(
        &self,
        operation: &OperationRequest,
        context: &InvocationContext,
    ) -> OperationOutcome;
}

/// 线程安全事件目的地。 / Thread-safe event destination.
pub trait EventSink: Send + Sync {
    /// 保存或展示事件。 / Stores or presents an event.
    fn emit(&self, event: Event) -> Result<(), SinkError>;
}

/// 事件目的地拒绝事件。 / Event destination rejected an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SinkError(String);
impl SinkError {
    /// 保留底层错误说明。 / Preserves the underlying error message.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for SinkError {}

/// 可克隆的协作式取消句柄。 / Cloneable cooperative cancellation handle.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    /// 幂等地请求取消。 / Idempotently requests cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// 返回是否已请求取消。 / Returns whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// 非法动作生命周期。 / Invalid action lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleError(String);
impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for LifecycleError {}

/// 发布事件失败。 / Event publication failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmitError {
    /// 生命周期转换无效。 / Lifecycle transition was invalid.
    Lifecycle(LifecycleError),
    /// 下游目的地失败。 / Downstream sink failed.
    Sink(SinkError),
}
impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lifecycle(e) => write!(f, "invalid lifecycle: {e}"),
            Self::Sink(e) => write!(f, "event sink failed: {e}"),
        }
    }
}
impl std::error::Error for EmitError {}

/// 单次操作的所有横切依赖。 / All cross-cutting dependencies for one operation.
#[derive(Clone)]
pub struct InvocationContext {
    id: InvocationId,
    cancellation: CancellationToken,
    events: Arc<dyn EventSink>,
    sequence: Arc<AtomicU64>,
    delivery: Arc<Mutex<()>>,
    lifecycle: Arc<Mutex<Lifecycle>>,
    emission_failure: Arc<Mutex<Option<EmitError>>>,
}
impl InvocationContext {
    /// 创建无全局状态的调用上下文。 / Creates an invocation context without global state.
    pub fn new(
        id: InvocationId,
        cancellation: CancellationToken,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            id,
            cancellation,
            events,
            sequence: Arc::new(AtomicU64::new(0)),
            delivery: Arc::new(Mutex::new(())),
            lifecycle: Arc::new(Mutex::new(Lifecycle::default())),
            emission_failure: Arc::new(Mutex::new(None)),
        }
    }
    /// 返回调用 ID。 / Returns the invocation ID.
    pub fn id(&self) -> &InvocationId {
        &self.id
    }
    /// 返回是否取消。 / Returns whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    /// 返回可传播的取消句柄。 / Returns a propagatable cancellation handle.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    /// 发布动作事件并验证状态转换。 / Publishes an action event after validating its transition.
    ///
    /// 一个独立交付门保证 sink 按序收到事件；生命周期锁与序号操作均在调用可能阻塞的
    /// sink 前完成且不被持有。`JobFinished.sequence` 同时给出此前事件的准确数量。
    /// A separate delivery gate guarantees ordered sink calls; neither the lifecycle
    /// lock nor sequence operation is held while calling the potentially blocking sink.
    /// `JobFinished.sequence` also gives the exact count of preceding events.
    pub fn emit(&self, payload: EventPayload) -> Result<(), EmitError> {
        let _delivery = lock(&self.delivery);
        let sequence = {
            let mut lifecycle = lock(&self.lifecycle);
            if let Err(error) = lifecycle.observe(&payload).map_err(EmitError::Lifecycle) {
                drop(lifecycle);
                self.remember(error.clone());
                return Err(error);
            }
            self.sequence.fetch_add(1, Ordering::Relaxed)
        };
        self.deliver(sequence, payload)
    }
    fn complete(&self, job: JobId, result: OperationResult) -> Result<(), EmitError> {
        let _delivery = lock(&self.delivery);
        lock(&self.lifecycle)
            .complete(&job)
            .map_err(EmitError::Lifecycle)?;
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.deliver(sequence, EventPayload::OperationCompleted { job, result })
    }
    fn finish_job(&self, summary: JobSummary) -> Result<(), EmitError> {
        let _delivery = lock(&self.delivery);
        lock(&self.lifecycle)
            .finish_job(&summary.job)
            .map_err(EmitError::Lifecycle)?;
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        self.deliver(sequence, EventPayload::JobFinished(summary))
    }
    fn deliver(&self, sequence: u64, payload: EventPayload) -> Result<(), EmitError> {
        let result = self
            .events
            .emit(Event::new(self.id.clone(), sequence, payload))
            .map_err(EmitError::Sink);
        if let Err(error) = &result {
            self.remember(error.clone());
        }
        result
    }
    fn remember(&self, error: EmitError) {
        let mut slot = lock(&self.emission_failure);
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

/// 注册或调度错误。 / Registration or dispatch error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelError {
    /// 能力 ID 重复。 / Duplicate capability ID.
    DuplicateCapability(String),
    /// 操作处理器重复。 / Duplicate operation handler.
    DuplicateOperation(OperationKind),
    /// 描述含空 ID 或零操作。 / Descriptor has an empty ID or no operations.
    InvalidDescriptor,
    /// 没有处理器。 / No handler exists.
    UnsupportedOperation(OperationKind),
    /// 能力返回的结果类别与请求不匹配。 / Capability result kind did not match the request.
    ResultKindMismatch {
        /// 请求类别。 / Requested kind.
        requested: OperationKind,
        /// 返回类别。 / Returned kind.
        returned: OperationKind,
    },
    /// 结果的查询视图或对象身份与请求不匹配。 / Result inspection view or object identity did not match the request.
    ResultRequestMismatch,
    /// 成功操作错误地声明领域结果不可用。 / Successful operation incorrectly declared its domain result unavailable.
    UnavailableResultOnSuccess,
    /// 能力产生无效/未闭合生命周期。 / Capability produced invalid or unclosed lifecycle.
    Lifecycle(LifecycleError),
    /// 事件发布失败。 / Event publication failed.
    Emit(EmitError),
}
impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCapability(id) => write!(f, "duplicate capability `{id}`"),
            Self::DuplicateOperation(kind) => write!(f, "duplicate handler for `{kind:?}`"),
            Self::InvalidDescriptor => {
                f.write_str("capability id and operations must not be empty")
            }
            Self::UnsupportedOperation(kind) => write!(f, "unsupported operation `{kind:?}`"),
            Self::ResultKindMismatch {
                requested,
                returned,
            } => write!(
                f,
                "operation result kind `{returned:?}` differs from request `{requested:?}`"
            ),
            Self::ResultRequestMismatch => {
                f.write_str("operation result view or identity differs from request")
            }
            Self::UnavailableResultOnSuccess => {
                f.write_str("successful operation must provide domain result data")
            }
            Self::Lifecycle(error) => write!(f, "invalid lifecycle: {error}"),
            Self::Emit(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for KernelError {}

/// 已完成调度的结构化结果。 / Structured completed-dispatch result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    /// 处理能力。 / Handling capability.
    pub capability: CapabilityId,
    /// 已验证且已发布的领域结果。 / Validated and published domain result.
    pub result: OperationResult,
    /// 内核归约的作业摘要。 / Kernel-reduced job summary.
    pub summary: JobSummary,
}

/// 显式静态能力注册表。 / Explicit static capability registry.
pub struct Kernel<'a> {
    capabilities: &'a [&'a dyn Capability],
}
impl<'a> Kernel<'a> {
    /// 验证静态注册表。 / Validates the static registry.
    pub fn new(capabilities: &'a [&'a dyn Capability]) -> Result<Self, KernelError> {
        validate(capabilities)?;
        Ok(Self { capabilities })
    }
    /// 返回描述，用于帮助和补全。 / Returns descriptors for help and completion.
    pub fn capabilities(
        &self,
    ) -> impl ExactSizeIterator<Item = &'static CapabilityDescriptor> + '_ {
        self.capabilities.iter().map(|c| c.descriptor())
    }
    /// 执行类型化操作，验证事件闭合并统一归约退出状态。 / Executes a typed operation, validates lifecycle closure, and centrally reduces exit status.
    pub fn dispatch(
        &self,
        operation: &OperationRequest,
        context: &InvocationContext,
    ) -> Result<DispatchOutcome, KernelError> {
        let capability = self.find(operation.kind())?;
        let started = Instant::now();
        let outcome = capability.execute(operation, context);
        if let Some(error) = lock(&context.emission_failure).clone() {
            return Err(KernelError::Emit(error));
        }
        if outcome.result.kind() != operation.kind() {
            return Err(KernelError::ResultKindMismatch {
                requested: operation.kind(),
                returned: outcome.result.kind(),
            });
        }
        if !outcome.result.matches_request(operation) {
            return Err(KernelError::ResultRequestMismatch);
        }
        let reduction = lock(&context.lifecycle)
            .finish(&outcome, context.is_cancelled())
            .map_err(KernelError::Lifecycle)?;
        if outcome.result.is_unavailable() && reduction.totals.failed == 0 && !outcome.cancelled {
            return Err(KernelError::UnavailableResultOnSuccess);
        }
        let status = if outcome.cancelled {
            ExitStatus::Cancelled
        } else if reduction.totals.failed > 0 {
            ExitStatus::Failed
        } else {
            ExitStatus::Success
        };
        let summary = JobSummary {
            job: outcome.job.clone(),
            totals: reduction.totals,
            root_failures: reduction.root_failures,
            cache_hits: reduction.cache_hits,
            timing: Timing {
                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            },
            status,
        };
        context
            .complete(outcome.job, outcome.result.clone())
            .map_err(KernelError::Emit)?;
        context
            .finish_job(summary.clone())
            .map_err(KernelError::Emit)?;
        Ok(DispatchOutcome {
            capability: CapabilityId::new(capability.descriptor().id).expect("validated ID"),
            result: outcome.result,
            summary,
        })
    }
    fn find(&self, kind: OperationKind) -> Result<&'a dyn Capability, KernelError> {
        self.capabilities
            .iter()
            .copied()
            .find(|c| c.descriptor().operations.contains(&kind))
            .ok_or(KernelError::UnsupportedOperation(kind))
    }
}

fn validate(capabilities: &[&dyn Capability]) -> Result<(), KernelError> {
    let mut ids = HashSet::new();
    let mut operations = HashSet::new();
    for capability in capabilities {
        let d = capability.descriptor();
        if d.id.is_empty() || d.operations.is_empty() {
            return Err(KernelError::InvalidDescriptor);
        }
        if !ids.insert(d.id) {
            return Err(KernelError::DuplicateCapability(d.id.to_owned()));
        }
        for kind in d.operations {
            if !operations.insert(*kind) {
                return Err(KernelError::DuplicateOperation(*kind));
            }
        }
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionState {
    Queued,
    Started,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
}
impl ActionState {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Blocked | Self::Cancelled
        )
    }
}
#[derive(Clone, Debug)]
struct ActionRecord {
    dependencies: Vec<ActionId>,
    state: ActionState,
    cache_hit: bool,
}
#[derive(Default)]
struct Lifecycle {
    job: Option<JobId>,
    planned: Option<u64>,
    actions: HashMap<ActionId, ActionRecord>,
    cache_hits: u64,
    operation_completed: bool,
    job_finished: bool,
}
struct Reduction {
    totals: ActionTotals,
    root_failures: u64,
    cache_hits: u64,
}
impl Lifecycle {
    fn observe(&mut self, event: &EventPayload) -> Result<(), LifecycleError> {
        if self.operation_completed {
            return Err(LifecycleError(
                "capability event followed operation completion".into(),
            ));
        }
        match event {
            EventPayload::PlanReady { job, actions } => self.plan(job, *actions),
            EventPayload::ActionQueued {
                job,
                action,
                dependencies,
                ..
            } => self.queue(job, action, dependencies),
            EventPayload::ActionStarted { job, action } => {
                self.transition(job, action, &[ActionState::Queued], ActionState::Started)
            }
            EventPayload::CacheHit {
                job,
                action,
                action_key,
                ..
            } => {
                if action_key.is_none() {
                    return Err(LifecycleError(
                        "new cache-hit emissions require a complete action key".into(),
                    ));
                }
                self.cache(job, action)?;
                self.cache_hits += 1;
                Ok(())
            }
            EventPayload::ActionSucceeded { job, action, .. } => {
                self.transition(job, action, &[ActionState::Started], ActionState::Succeeded)
            }
            EventPayload::ActionFailed { job, action, .. } => {
                self.transition(job, action, &[ActionState::Started], ActionState::Failed)
            }
            EventPayload::ActionBlocked {
                job,
                action,
                blocked_by,
            } => self.block(job, action, blocked_by),
            EventPayload::ActionCancelled { job, action, .. } => self.transition(
                job,
                action,
                &[ActionState::Queued, ActionState::Started],
                ActionState::Cancelled,
            ),
            EventPayload::Diagnostic(_) => Ok(()),
            EventPayload::OperationCompleted { .. } | EventPayload::JobFinished(_) => Err(
                LifecycleError("only the kernel may complete or finish a job".into()),
            ),
            _ => Err(LifecycleError(
                "kernel does not understand this additive lifecycle event".into(),
            )),
        }
    }
    fn complete(&mut self, job: &JobId) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        if self.operation_completed {
            return Err(LifecycleError(
                "operation-completed emitted more than once".into(),
            ));
        }
        if self.actions.values().any(|record| !record.state.terminal()) {
            return Err(LifecycleError(
                "operation completed with non-terminal actions".into(),
            ));
        }
        self.operation_completed = true;
        Ok(())
    }
    fn finish_job(&mut self, job: &JobId) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        if !self.operation_completed {
            return Err(LifecycleError(
                "job-finished preceded operation-completed".into(),
            ));
        }
        if self.job_finished {
            return Err(LifecycleError("job-finished emitted more than once".into()));
        }
        self.job_finished = true;
        Ok(())
    }
    fn plan(&mut self, job: &JobId, actions: u64) -> Result<(), LifecycleError> {
        if self.job.is_some() {
            return Err(LifecycleError("plan-ready emitted more than once".into()));
        }
        self.job = Some(job.clone());
        self.planned = Some(actions);
        Ok(())
    }
    fn queue(
        &mut self,
        job: &JobId,
        action: &ActionId,
        dependencies: &[ActionId],
    ) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        let mut unique = HashSet::new();
        for dependency in dependencies {
            if dependency == action
                || !unique.insert(dependency)
                || !self.actions.contains_key(dependency)
            {
                return Err(LifecycleError(format!(
                    "action `{action}` has invalid dependency `{dependency}`"
                )));
            }
        }
        if self
            .actions
            .insert(
                action.clone(),
                ActionRecord {
                    dependencies: dependencies.to_vec(),
                    state: ActionState::Queued,
                    cache_hit: false,
                },
            )
            .is_some()
        {
            return Err(LifecycleError(format!(
                "action `{action}` queued more than once"
            )));
        }
        Ok(())
    }
    fn transition(
        &mut self,
        job: &JobId,
        action: &ActionId,
        from: &[ActionState],
        to: ActionState,
    ) -> Result<(), LifecycleError> {
        self.require(job, action, from)?;
        self.actions.get_mut(action).expect("required action").state = to;
        Ok(())
    }
    fn cache(&mut self, job: &JobId, action: &ActionId) -> Result<(), LifecycleError> {
        self.require(job, action, &[ActionState::Started])?;
        let record = self.actions.get_mut(action).expect("required action");
        if record.cache_hit {
            return Err(LifecycleError(format!(
                "action `{action}` reported more than one cache hit"
            )));
        }
        record.cache_hit = true;
        Ok(())
    }
    fn block(
        &mut self,
        job: &JobId,
        action: &ActionId,
        blocked_by: &[ActionId],
    ) -> Result<(), LifecycleError> {
        if blocked_by.is_empty() {
            return Err(LifecycleError(format!(
                "blocked action `{action}` has no blocking predecessor"
            )));
        }
        let dependencies = &self
            .actions
            .get(action)
            .ok_or_else(|| LifecycleError(format!("action `{action}` was not queued")))?
            .dependencies;
        for predecessor in blocked_by {
            if !dependencies.contains(predecessor) {
                return Err(LifecycleError(format!(
                    "action `{action}` was blocked by non-dependency `{predecessor}`"
                )));
            }
            self.require(
                job,
                predecessor,
                &[ActionState::Failed, ActionState::Blocked],
            )?;
        }
        self.transition(job, action, &[ActionState::Queued], ActionState::Blocked)
    }
    fn require(
        &self,
        job: &JobId,
        action: &ActionId,
        states: &[ActionState],
    ) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        let record = self
            .actions
            .get(action)
            .ok_or_else(|| LifecycleError(format!("action `{action}` was not queued")))?;
        if states.contains(&record.state) {
            Ok(())
        } else {
            Err(LifecycleError(format!(
                "invalid transition for action `{action}` from {:?}",
                record.state
            )))
        }
    }
    fn same_job(&self, job: &JobId) -> Result<(), LifecycleError> {
        match &self.job {
            Some(expected) if expected == job => Ok(()),
            Some(expected) => Err(LifecycleError(format!(
                "event job `{job}` differs from plan `{expected}`"
            ))),
            None => Err(LifecycleError("action event preceded plan-ready".into())),
        }
    }
    fn finish(
        &self,
        outcome: &OperationOutcome,
        cancellation_requested: bool,
    ) -> Result<Reduction, LifecycleError> {
        self.same_job(&outcome.job)?;
        let planned = self.planned.expect("job implies planned count");
        if planned != self.actions.len() as u64 {
            return Err(LifecycleError(format!(
                "plan declared {planned} actions but {} were queued",
                self.actions.len()
            )));
        }
        if self.actions.values().any(|r| !r.state.terminal()) {
            return Err(LifecycleError("job ended with non-terminal actions".into()));
        }
        let mut totals = ActionTotals::default();
        let mut root_failures = 0;
        for record in self.actions.values() {
            match record.state {
                ActionState::Succeeded => totals.succeeded += 1,
                ActionState::Failed => {
                    totals.failed += 1;
                    root_failures += 1;
                }
                ActionState::Blocked => totals.blocked += 1,
                ActionState::Cancelled => totals.cancelled += 1,
                ActionState::Queued | ActionState::Started => unreachable!(),
            }
        }
        let cancelled = totals.cancelled > 0 || cancellation_requested;
        if totals != outcome.totals
            || root_failures != outcome.root_failures
            || cancelled != outcome.cancelled
        {
            return Err(LifecycleError(
                "declared outcome disagrees with reduced events".into(),
            ));
        }
        Ok(Reduction {
            totals,
            root_failures,
            cache_hits: self.cache_hits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_protocol::{
        ActionKind, BuildRequest, BuildResult, Diagnostic, DiagnosticId, EmitKind, InspectRequest,
        InspectResult, InspectView, LockMode, OpaqueSourceId, Phase, ProfileName,
        ProjectInspection, ProjectPath, Severity, WorkspaceScope,
    };
    use std::sync::Mutex;
    static DESC: CapabilityDescriptor = CapabilityDescriptor {
        id: "build",
        operations: &[OperationKind::Build],
        summary: "build",
    };
    static INSPECT_DESC: CapabilityDescriptor = CapabilityDescriptor {
        id: "inspect",
        operations: &[OperationKind::Inspect],
        summary: "inspect",
    };
    struct Build;
    impl Capability for Build {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let job = JobId::new("job").unwrap();
            let action = ActionId::new("root").unwrap();
            context
                .emit(EventPayload::PlanReady {
                    job: job.clone(),
                    actions: 1,
                })
                .unwrap();
            context
                .emit(EventPayload::ActionQueued {
                    job: job.clone(),
                    action: action.clone(),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                })
                .unwrap();
            context
                .emit(EventPayload::ActionStarted {
                    job: job.clone(),
                    action: action.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::ActionSucceeded {
                    job: job.clone(),
                    action,
                    timing: Timing::default(),
                    artifacts: vec![],
                })
                .unwrap();
            OperationOutcome {
                job,
                result: OperationResult::Build(BuildResult {
                    published: vec![],
                    build_record: None,
                }),
                totals: ActionTotals {
                    succeeded: 1,
                    ..ActionTotals::default()
                },
                root_failures: 0,
                cancelled: false,
            }
        }
    }
    #[derive(Default)]
    struct Sink(Mutex<Vec<Event>>);
    impl EventSink for Sink {
        fn emit(&self, event: Event) -> Result<(), SinkError> {
            lock(&self.0).push(event);
            Ok(())
        }
    }
    fn operation() -> OperationRequest {
        OperationRequest::Build(BuildRequest {
            project: ProjectPath::new(".").unwrap(),
            scope: WorkspaceScope::Current,
            targets: vec![],
            profile: ProfileName::new("dev").unwrap(),
            arguments: Default::default(),
            emit: vec![EmitKind::Prompt],
            lock: LockMode::Update,
        })
    }
    #[test]
    fn dispatch_reduces_success_and_finishes() {
        static BUILD: Build = Build;
        let capabilities: &[&dyn Capability] = &[&BUILD];
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        let outcome = Kernel::new(capabilities)
            .unwrap()
            .dispatch(&operation(), &context)
            .unwrap();
        assert_eq!(outcome.summary.status, ExitStatus::Success);
        assert_eq!(outcome.result.kind(), OperationKind::Build);
        let events = lock(&sink.0);
        assert!(matches!(
            events[events.len() - 2].payload,
            EventPayload::OperationCompleted { .. }
        ));
        assert!(matches!(
            events.last().unwrap().payload,
            EventPayload::JobFinished(_)
        ));
        assert_eq!(
            events[events.len() - 2].sequence + 1,
            events.last().unwrap().sequence
        );
    }
    struct WrongResult;
    impl Capability for WrongResult {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &DESC
        }
        fn execute(
            &self,
            operation: &OperationRequest,
            context: &InvocationContext,
        ) -> OperationOutcome {
            let mut outcome = Build.execute(operation, context);
            outcome.result = OperationResult::Format(squish_protocol::FormatResult {
                selected: vec![],
                changed: vec![],
                check: true,
                diffs: vec![],
            });
            outcome
        }
    }
    #[test]
    fn result_kind_mismatch_emits_neither_completion_nor_summary() {
        static WRONG: WrongResult = WrongResult;
        let capabilities: &[&dyn Capability] = &[&WRONG];
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        assert!(matches!(
            Kernel::new(capabilities)
                .unwrap()
                .dispatch(&operation(), &context),
            Err(KernelError::ResultKindMismatch { .. })
        ));
        assert!(!lock(&sink.0).iter().any(|event| matches!(
            event.payload,
            EventPayload::OperationCompleted { .. } | EventPayload::JobFinished(_)
        )));
    }
    fn early_failure(context: &InvocationContext, kind: ActionKind) -> JobId {
        let job = JobId::new("early-failure").unwrap();
        let action = ActionId::new("resolve").unwrap();
        context
            .emit(EventPayload::PlanReady {
                job: job.clone(),
                actions: 1,
            })
            .unwrap();
        context
            .emit(EventPayload::ActionQueued {
                job: job.clone(),
                action: action.clone(),
                kind,
                dependencies: vec![],
            })
            .unwrap();
        context
            .emit(EventPayload::ActionStarted {
                job: job.clone(),
                action: action.clone(),
            })
            .unwrap();
        context
            .emit(EventPayload::ActionFailed {
                job: job.clone(),
                action,
                timing: Timing::default(),
                diagnostic: Diagnostic {
                    id: DiagnosticId::new("resolution-failed").unwrap(),
                    code: "resolution_failed".into(),
                    severity: Severity::Error,
                    phase: Phase::Resolve,
                    message: "resolution failed before domain data existed".into(),
                    primary: None,
                    related: vec![],
                    help: None,
                },
            })
            .unwrap();
        job
    }
    struct ResolveFails;
    impl Capability for ResolveFails {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            OperationOutcome {
                job: early_failure(context, ActionKind::Resolve),
                result: OperationResult::Unavailable {
                    kind: OperationKind::Build,
                },
                totals: ActionTotals {
                    failed: 1,
                    ..ActionTotals::default()
                },
                root_failures: 1,
                cancelled: false,
            }
        }
    }
    #[test]
    fn resolve_failure_publishes_typed_unavailable_before_failed_summary() {
        static CAPABILITY: ResolveFails = ResolveFails;
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        let outcome = Kernel::new(&[&CAPABILITY])
            .unwrap()
            .dispatch(&operation(), &context)
            .unwrap();
        assert_eq!(outcome.summary.status, ExitStatus::Failed);
        assert_eq!(
            outcome.result,
            OperationResult::Unavailable {
                kind: OperationKind::Build
            }
        );
        let events = lock(&sink.0);
        assert!(matches!(
            events[events.len() - 2].payload,
            EventPayload::OperationCompleted {
                result: OperationResult::Unavailable {
                    kind: OperationKind::Build
                },
                ..
            }
        ));
        assert!(matches!(
            events.last().unwrap().payload,
            EventPayload::JobFinished(_)
        ));
    }
    struct InspectFails;
    impl Capability for InspectFails {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &INSPECT_DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            OperationOutcome {
                job: early_failure(context, ActionKind::ResolveCandidate),
                result: OperationResult::Unavailable {
                    kind: OperationKind::Inspect,
                },
                totals: ActionTotals {
                    failed: 1,
                    ..ActionTotals::default()
                },
                root_failures: 1,
                cancelled: false,
            }
        }
    }
    fn inspect_operation() -> OperationRequest {
        OperationRequest::Inspect(InspectRequest {
            project: ProjectPath::new(".").unwrap(),
            view: InspectView::Source(OpaqueSourceId::new("src/main.squish").unwrap()),
        })
    }
    #[test]
    fn inspect_early_failure_can_report_unavailable_without_fabricated_data() {
        static CAPABILITY: InspectFails = InspectFails;
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        Kernel::new(&[&CAPABILITY])
            .unwrap()
            .dispatch(&inspect_operation(), &context)
            .unwrap();
        assert!(matches!(
            lock(&sink.0)[4].payload,
            EventPayload::OperationCompleted {
                result: OperationResult::Unavailable {
                    kind: OperationKind::Inspect
                },
                ..
            }
        ));
    }
    struct WrongInspectIdentity;
    impl Capability for WrongInspectIdentity {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &INSPECT_DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let job = JobId::new("inspect").unwrap();
            context
                .emit(EventPayload::PlanReady {
                    job: job.clone(),
                    actions: 0,
                })
                .unwrap();
            OperationOutcome {
                job,
                result: OperationResult::Inspect(InspectResult::Project(ProjectInspection {
                    packages: vec![],
                    targets: vec![],
                })),
                totals: ActionTotals::default(),
                root_failures: 0,
                cancelled: false,
            }
        }
    }
    #[test]
    fn inspect_view_mismatch_is_rejected_before_completion() {
        static CAPABILITY: WrongInspectIdentity = WrongInspectIdentity;
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        assert_eq!(
            Kernel::new(&[&CAPABILITY])
                .unwrap()
                .dispatch(&inspect_operation(), &context),
            Err(KernelError::ResultRequestMismatch)
        );
        assert!(!lock(&sink.0).iter().any(|event| matches!(
            event.payload,
            EventPayload::OperationCompleted { .. } | EventPayload::JobFinished(_)
        )));
    }
    struct Liar;
    impl Capability for Liar {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let job = JobId::new("job").unwrap();
            context
                .emit(EventPayload::PlanReady {
                    job: job.clone(),
                    actions: 0,
                })
                .unwrap();
            OperationOutcome {
                job,
                result: OperationResult::Build(BuildResult {
                    published: vec![],
                    build_record: None,
                }),
                totals: ActionTotals {
                    failed: 1,
                    ..ActionTotals::default()
                },
                root_failures: 1,
                cancelled: false,
            }
        }
    }
    #[test]
    fn mismatch_is_rejected_without_finished_event() {
        static LIAR: Liar = Liar;
        let capabilities: &[&dyn Capability] = &[&LIAR];
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        assert!(matches!(
            Kernel::new(capabilities)
                .unwrap()
                .dispatch(&operation(), &context),
            Err(KernelError::Lifecycle(_))
        ));
        assert!(
            !lock(&sink.0)
                .iter()
                .any(|e| matches!(e.payload, EventPayload::JobFinished(_)))
        );
    }
    struct ChildFails;
    impl Capability for ChildFails {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &DESC
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let job = JobId::new("job").unwrap();
            let first = ActionId::new("first").unwrap();
            let child = ActionId::new("child").unwrap();
            context
                .emit(EventPayload::PlanReady {
                    job: job.clone(),
                    actions: 2,
                })
                .unwrap();
            context
                .emit(EventPayload::ActionQueued {
                    job: job.clone(),
                    action: first.clone(),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                })
                .unwrap();
            context
                .emit(EventPayload::ActionStarted {
                    job: job.clone(),
                    action: first.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::ActionSucceeded {
                    job: job.clone(),
                    action: first.clone(),
                    timing: Timing::default(),
                    artifacts: vec![],
                })
                .unwrap();
            context
                .emit(EventPayload::ActionQueued {
                    job: job.clone(),
                    action: child.clone(),
                    kind: ActionKind::Link,
                    dependencies: vec![first],
                })
                .unwrap();
            context
                .emit(EventPayload::ActionStarted {
                    job: job.clone(),
                    action: child.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::ActionFailed {
                    job: job.clone(),
                    action: child,
                    timing: Timing::default(),
                    diagnostic: Diagnostic {
                        id: DiagnosticId::new("link-failed").unwrap(),
                        code: "link_failed".into(),
                        severity: Severity::Error,
                        phase: Phase::Link,
                        message: "link failed".into(),
                        primary: None,
                        related: vec![],
                        help: None,
                    },
                })
                .unwrap();
            OperationOutcome {
                job,
                result: OperationResult::Build(BuildResult {
                    published: vec![],
                    build_record: None,
                }),
                totals: ActionTotals {
                    succeeded: 1,
                    failed: 1,
                    ..ActionTotals::default()
                },
                root_failures: 1,
                cancelled: false,
            }
        }
    }
    #[test]
    fn child_after_success_is_still_a_root_failure() {
        static CAPABILITY: ChildFails = ChildFails;
        let capabilities: &[&dyn Capability] = &[&CAPABILITY];
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("i").unwrap(),
            CancellationToken::default(),
            sink,
        );
        let outcome = Kernel::new(capabilities)
            .unwrap()
            .dispatch(&operation(), &context)
            .unwrap();
        assert_eq!(outcome.summary.status, ExitStatus::Failed);
        assert_eq!(outcome.summary.root_failures, 1);
    }
    #[test]
    fn duplicate_operation_handler_is_rejected() {
        static BUILD: Build = Build;
        let capabilities: &[&dyn Capability] = &[&BUILD, &BUILD];
        assert!(matches!(
            Kernel::new(capabilities),
            Err(KernelError::DuplicateCapability(_))
        ));
    }
}
