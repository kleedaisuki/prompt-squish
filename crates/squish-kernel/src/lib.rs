//! prompt-squish 的领域无关静态微内核。 / Domain-neutral static microkernel for prompt-squish.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use squish_protocol::{
    ActionId, ActionKind, ActionTotals, CapabilityId, Event, EventPayload, ExitStatus,
    InvocationId, JobId, JobSummary, OperationKind, OperationRequest, OperationResult,
    PlanCloseReason, PlanId, PlanMode, PlanningAttemptId, PlanningIssueId, PlanningStepId,
    SupersedeReason, Timing,
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
    /// 最终计划问题、独立动作失败或致命计划失败的根失败数。 /
    /// Root failures from final-plan issues, independent action failures, or a fatal planning failure.
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
        if let Err(error) = Event::new(self.id.clone(), 0, payload.clone()).validate() {
            let error = EmitError::Lifecycle(LifecycleError(format!(
                "non-canonical native v2 event: {error}"
            )));
            self.remember(error.clone());
            return Err(error);
        }
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
            .finish(&outcome)
            .map_err(KernelError::Lifecycle)?;
        if outcome.result.is_unavailable() && reduction.root_failures == 0 && !outcome.cancelled {
            return Err(KernelError::UnavailableResultOnSuccess);
        }
        let status = if outcome.cancelled {
            ExitStatus::Cancelled
        } else if reduction.root_failures > 0 {
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
    Declared,
    Started,
    Succeeded,
    Failed,
    Blocked,
    Cancelled,
    Superseded,
}
impl ActionState {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Blocked | Self::Cancelled | Self::Superseded
        )
    }
}
#[derive(Clone, Debug)]
struct ActionRecord {
    kind: ActionKind,
    dependencies: Vec<ActionId>,
    state: ActionState,
    cache_hit: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StepState {
    Started,
    Succeeded,
    Failed,
    Cancelled,
}
impl StepState {
    fn terminal(self) -> bool {
        self != Self::Started
    }
}
#[derive(Debug)]
struct AttemptRecord {
    steps: HashMap<PlanningStepId, StepState>,
    issues: HashSet<PlanningIssueId>,
}
#[derive(Debug)]
struct PlanRecord {
    mode: PlanMode,
    expected_actions: u64,
    expected_issues: u64,
    actions: HashMap<ActionId, ActionRecord>,
    any_started: bool,
    closed: Option<PlanCloseReason>,
}
#[derive(Clone, Debug)]
enum Active {
    Attempt(PlanningAttemptId),
    Plan(PlanId),
}
#[derive(Default)]
struct Lifecycle {
    job: Option<JobId>,
    active: Option<Active>,
    attempts: HashMap<PlanningAttemptId, AttemptRecord>,
    plans: HashMap<PlanId, PlanRecord>,
    final_plan: Option<PlanId>,
    retry_allowed: bool,
    fatal_planning_failure: bool,
    planning_cancelled: bool,
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
            return self.invalid("capability event followed operation completion");
        }
        match event {
            EventPayload::PlanningStarted { job, attempt } => self.start_attempt(job, attempt),
            EventPayload::PlanningStepStarted {
                job, attempt, step, ..
            } => self.start_step(job, attempt, step),
            EventPayload::PlanningStepSucceeded {
                job, attempt, step, ..
            } => self.finish_step(job, attempt, step, StepState::Succeeded),
            EventPayload::PlanningStepFailed {
                job, attempt, step, ..
            } => self.finish_step(job, attempt, step, StepState::Failed),
            EventPayload::PlanningStepCancelled {
                job, attempt, step, ..
            } => self.finish_step(job, attempt, step, StepState::Cancelled),
            EventPayload::PlanningIssue {
                job,
                attempt,
                issue,
                ..
            } => self.issue(job, attempt, issue),
            EventPayload::PlanningFailed { job, attempt, .. } => {
                self.end_attempt(job, attempt, false)
            }
            EventPayload::PlanningCancelled { job, attempt } => {
                self.end_attempt(job, attempt, true)
            }
            EventPayload::PlanReady {
                job,
                attempt,
                plan,
                mode,
                actions,
                issues,
                ..
            } => self.seal(job, attempt, plan, *mode, *actions, *issues),
            EventPayload::ActionDeclared {
                job,
                plan,
                action,
                kind,
                dependencies,
            } => self.declare(job, plan, action, *kind, dependencies),
            EventPayload::ActionStarted { job, plan, action } => {
                self.start_action(job, plan, action)
            }
            EventPayload::CacheHit {
                job,
                plan,
                action,
                action_key,
                ..
            } => {
                if action_key.is_none() {
                    return self.invalid("native v2 cache hit requires a complete action key");
                }
                self.cache(job, plan, action)
            }
            EventPayload::ActionSucceeded {
                job, plan, action, ..
            } => self.action_transition(
                job,
                plan,
                action,
                &[ActionState::Started],
                ActionState::Succeeded,
            ),
            EventPayload::ActionFailed {
                job, plan, action, ..
            } => self.action_transition(
                job,
                plan,
                action,
                &[ActionState::Started],
                ActionState::Failed,
            ),
            EventPayload::ActionBlocked {
                job,
                plan,
                action,
                blocked_by,
            } => self.block(job, plan, action, blocked_by),
            EventPayload::ActionCancelled {
                job, plan, action, ..
            } => self.action_transition(
                job,
                plan,
                action,
                &[ActionState::Declared, ActionState::Started],
                ActionState::Cancelled,
            ),
            EventPayload::ActionSuperseded {
                job,
                plan,
                action,
                reason,
                ..
            } => {
                if *reason != SupersedeReason::AuthoritativeRevisionChanged {
                    return self.invalid("unsupported supersede reason");
                }
                self.action_transition(
                    job,
                    plan,
                    action,
                    &[ActionState::Declared, ActionState::Started],
                    ActionState::Superseded,
                )
            }
            EventPayload::PlanClosed { job, plan, reason } => self.close_plan(job, plan, *reason),
            EventPayload::Diagnostic(_) => Ok(()),
            EventPayload::OperationCompleted { .. } | EventPayload::JobFinished(_) => {
                self.invalid("only the kernel may complete or finish a job")
            }
            _ => self.invalid("kernel does not understand this lifecycle event"),
        }
    }

    fn invalid<T>(&self, message: impl Into<String>) -> Result<T, LifecycleError> {
        Err(LifecycleError(message.into()))
    }
    fn bind_job(&mut self, job: &JobId) -> Result<(), LifecycleError> {
        match &self.job {
            Some(expected) if expected != job => {
                self.invalid(format!("event job `{job}` differs from job `{expected}`"))
            }
            Some(_) => Ok(()),
            None => {
                self.job = Some(job.clone());
                Ok(())
            }
        }
    }
    fn same_job(&self, job: &JobId) -> Result<(), LifecycleError> {
        match &self.job {
            Some(expected) if expected == job => Ok(()),
            Some(expected) => {
                self.invalid(format!("event job `{job}` differs from job `{expected}`"))
            }
            None => self.invalid("event preceded planning-started"),
        }
    }
    fn start_attempt(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
    ) -> Result<(), LifecycleError> {
        self.bind_job(job)?;
        if self.active.is_some()
            || (!self.attempts.is_empty() && !self.retry_allowed)
            || self.final_plan.is_some()
            || self.fatal_planning_failure
            || self.planning_cancelled
        {
            return self.invalid("planning attempt is not permitted in the current job state");
        }
        if self.attempts.contains_key(attempt) {
            return self.invalid(format!("planning attempt `{attempt}` was reused"));
        }
        self.attempts.insert(
            attempt.clone(),
            AttemptRecord {
                steps: HashMap::new(),
                issues: HashSet::new(),
            },
        );
        self.active = Some(Active::Attempt(attempt.clone()));
        self.retry_allowed = false;
        Ok(())
    }
    fn active_attempt(
        &self,
        job: &JobId,
        attempt: &PlanningAttemptId,
    ) -> Result<&AttemptRecord, LifecycleError> {
        self.same_job(job)?;
        match &self.active {
            Some(Active::Attempt(active)) if active == attempt => {
                Ok(self.attempts.get(attempt).expect("active attempt exists"))
            }
            _ => self.invalid(format!("planning attempt `{attempt}` is not active")),
        }
    }
    fn start_step(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
        step: &PlanningStepId,
    ) -> Result<(), LifecycleError> {
        self.active_attempt(job, attempt)?;
        let record = self
            .attempts
            .get_mut(attempt)
            .expect("active attempt exists");
        if record
            .steps
            .insert(step.clone(), StepState::Started)
            .is_some()
        {
            return self.invalid(format!("planning step `{step}` was reused"));
        }
        Ok(())
    }
    fn finish_step(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
        step: &PlanningStepId,
        state: StepState,
    ) -> Result<(), LifecycleError> {
        self.active_attempt(job, attempt)?;
        let current = self
            .attempts
            .get_mut(attempt)
            .expect("active attempt exists")
            .steps
            .get_mut(step)
            .ok_or_else(|| LifecycleError(format!("planning step `{step}` was not started")))?;
        if *current != StepState::Started {
            return self.invalid(format!("planning step `{step}` terminated more than once"));
        }
        *current = state;
        Ok(())
    }
    fn issue(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
        issue: &PlanningIssueId,
    ) -> Result<(), LifecycleError> {
        self.active_attempt(job, attempt)?;
        if !self
            .attempts
            .get_mut(attempt)
            .expect("active attempt exists")
            .issues
            .insert(issue.clone())
        {
            return self.invalid(format!("planning issue `{issue}` was reused"));
        }
        Ok(())
    }
    fn require_terminal_steps(&self, attempt: &PlanningAttemptId) -> Result<(), LifecycleError> {
        if self
            .attempts
            .get(attempt)
            .expect("attempt exists")
            .steps
            .values()
            .all(|state| state.terminal())
        {
            Ok(())
        } else {
            self.invalid("planning attempt ended with a non-terminal step")
        }
    }
    fn end_attempt(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
        cancelled: bool,
    ) -> Result<(), LifecycleError> {
        self.active_attempt(job, attempt)?;
        self.require_terminal_steps(attempt)?;
        self.active = None;
        if cancelled {
            self.planning_cancelled = true;
        } else {
            self.fatal_planning_failure = true;
        }
        Ok(())
    }
    fn seal(
        &mut self,
        job: &JobId,
        attempt: &PlanningAttemptId,
        plan: &PlanId,
        mode: PlanMode,
        actions: u64,
        issues: u64,
    ) -> Result<(), LifecycleError> {
        self.active_attempt(job, attempt)?;
        if mode == PlanMode::Legacy {
            return self.invalid("inspection-only legacy plans are not native v2 lifecycle events");
        }
        self.require_terminal_steps(attempt)?;
        let actual_issues = self
            .attempts
            .get(attempt)
            .expect("attempt exists")
            .issues
            .len() as u64;
        if issues != actual_issues {
            return self.invalid(format!(
                "plan declared {issues} issues but observed {actual_issues}"
            ));
        }
        if self.plans.contains_key(plan) {
            return self.invalid(format!("plan `{plan}` was reused"));
        }
        self.plans.insert(
            plan.clone(),
            PlanRecord {
                mode,
                expected_actions: actions,
                expected_issues: issues,
                actions: HashMap::new(),
                any_started: false,
                closed: None,
            },
        );
        self.active = Some(Active::Plan(plan.clone()));
        Ok(())
    }
    fn active_plan(&self, job: &JobId, plan: &PlanId) -> Result<&PlanRecord, LifecycleError> {
        self.same_job(job)?;
        match &self.active {
            Some(Active::Plan(active)) if active == plan => {
                Ok(self.plans.get(plan).expect("active plan exists"))
            }
            _ => self.invalid(format!("plan `{plan}` is not active")),
        }
    }
    fn declarations_complete(record: &PlanRecord) -> bool {
        record.actions.len() as u64 == record.expected_actions
    }
    fn declare(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
        kind: ActionKind,
        dependencies: &[ActionId],
    ) -> Result<(), LifecycleError> {
        let record = self.active_plan(job, plan)?;
        if Self::declarations_complete(record) {
            return self.invalid("action declaration followed the plan's declared count");
        }
        let mut unique = HashSet::new();
        if dependencies
            .iter()
            .any(|dependency| dependency == action || !unique.insert(dependency))
        {
            return self.invalid(format!("action `{action}` has invalid dependencies"));
        }
        let record = self.plans.get_mut(plan).expect("active plan exists");
        if record
            .actions
            .insert(
                action.clone(),
                ActionRecord {
                    kind,
                    dependencies: dependencies.to_vec(),
                    state: ActionState::Declared,
                    cache_hit: false,
                },
            )
            .is_some()
        {
            return self.invalid(format!("action `{action}` was declared more than once"));
        }
        if Self::declarations_complete(record) {
            Self::validate_dag(record)?;
        }
        Ok(())
    }
    fn validate_dag(record: &PlanRecord) -> Result<(), LifecycleError> {
        for (action, item) in &record.actions {
            for dependency in &item.dependencies {
                if !record.actions.contains_key(dependency) {
                    return Err(LifecycleError(format!(
                        "action `{action}` refers to undeclared dependency `{dependency}`"
                    )));
                }
            }
        }
        fn visit(
            id: &ActionId,
            plan: &PlanRecord,
            visiting: &mut HashSet<ActionId>,
            done: &mut HashSet<ActionId>,
        ) -> Result<(), LifecycleError> {
            if done.contains(id) {
                return Ok(());
            }
            if !visiting.insert(id.clone()) {
                return Err(LifecycleError(format!(
                    "action graph contains a cycle at `{id}`"
                )));
            }
            for dependency in &plan.actions[id].dependencies {
                visit(dependency, plan, visiting, done)?;
            }
            visiting.remove(id);
            done.insert(id.clone());
            Ok(())
        }
        let mut visiting = HashSet::new();
        let mut done = HashSet::new();
        for id in record.actions.keys() {
            visit(id, record, &mut visiting, &mut done)?;
        }
        Ok(())
    }
    fn start_action(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
    ) -> Result<(), LifecycleError> {
        let record = self.active_plan(job, plan)?;
        if record.mode != PlanMode::Execute {
            return self.invalid("actions may start only in an execute plan");
        }
        if !Self::declarations_complete(record) {
            return self.invalid("action started before every plan vertex was declared");
        }
        let item = record
            .actions
            .get(action)
            .ok_or_else(|| LifecycleError(format!("action `{action}` was not declared")))?;
        if item.state != ActionState::Declared {
            return self.invalid(format!(
                "invalid transition for action `{action}` from {:?}",
                item.state
            ));
        }
        for dependency in &item.dependencies {
            if record.actions[dependency].state != ActionState::Succeeded {
                return self.invalid(format!(
                    "action `{action}` started before dependency `{dependency}` succeeded"
                ));
            }
        }
        let record = self.plans.get_mut(plan).expect("active plan exists");
        record
            .actions
            .get_mut(action)
            .expect("declared action")
            .state = ActionState::Started;
        record.any_started = true;
        Ok(())
    }
    fn action_transition(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
        from: &[ActionState],
        to: ActionState,
    ) -> Result<(), LifecycleError> {
        let record = self.active_plan(job, plan)?;
        if !Self::declarations_complete(record) {
            return self.invalid("action transition preceded complete declaration");
        }
        let state = record
            .actions
            .get(action)
            .ok_or_else(|| LifecycleError(format!("action `{action}` was not declared")))?
            .state;
        if !from.contains(&state) {
            return self.invalid(format!(
                "invalid transition for action `{action}` from {state:?}"
            ));
        }
        self.plans
            .get_mut(plan)
            .expect("active plan exists")
            .actions
            .get_mut(action)
            .expect("declared action")
            .state = to;
        Ok(())
    }
    fn cache(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
    ) -> Result<(), LifecycleError> {
        self.active_plan(job, plan)?;
        self.action_transition(
            job,
            plan,
            action,
            &[ActionState::Started],
            ActionState::Started,
        )?;
        let item = self
            .plans
            .get_mut(plan)
            .expect("active plan exists")
            .actions
            .get_mut(action)
            .expect("declared action");
        if item.cache_hit {
            return self.invalid(format!(
                "action `{action}` reported more than one cache hit"
            ));
        }
        item.cache_hit = true;
        Ok(())
    }
    fn block(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
        blocked_by: &[ActionId],
    ) -> Result<(), LifecycleError> {
        let record = self.active_plan(job, plan)?;
        if blocked_by.is_empty() {
            return self.invalid(format!(
                "blocked action `{action}` has no blocking predecessor"
            ));
        }
        let item = record
            .actions
            .get(action)
            .ok_or_else(|| LifecycleError(format!("action `{action}` was not declared")))?;
        if item.state != ActionState::Declared {
            return self.invalid(format!(
                "invalid transition for action `{action}` from {:?}",
                item.state
            ));
        }
        for blocker in blocked_by {
            if !item.dependencies.contains(blocker)
                || !matches!(
                    record.actions.get(blocker).map(|x| x.state),
                    Some(ActionState::Failed | ActionState::Blocked)
                )
            {
                return self.invalid(format!("action `{action}` has invalid blocker `{blocker}`"));
            }
        }
        self.action_transition(
            job,
            plan,
            action,
            &[ActionState::Declared],
            ActionState::Blocked,
        )
    }
    fn close_plan(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        reason: PlanCloseReason,
    ) -> Result<(), LifecycleError> {
        let record = self.active_plan(job, plan)?;
        if !Self::declarations_complete(record) {
            return self.invalid("plan closed before every action was declared");
        }
        match reason {
            PlanCloseReason::Executed
                if record.mode != PlanMode::Execute
                    || record
                        .actions
                        .values()
                        .any(|x| !x.state.terminal() || x.state == ActionState::Superseded) =>
            {
                return self.invalid("executed plan has invalid mode or non-executed actions");
            }
            PlanCloseReason::Reported
                if record.mode != PlanMode::ReportOnly
                    || record.any_started
                    || record
                        .actions
                        .values()
                        .any(|x| x.state != ActionState::Declared) =>
            {
                return self.invalid("reported plan must have declarations and zero starts");
            }
            PlanCloseReason::Superseded
                if record.mode != PlanMode::Execute
                    || record
                        .actions
                        .values()
                        .any(|x| x.state == ActionState::Started)
                    || !record
                        .actions
                        .values()
                        .any(|x| x.state == ActionState::Superseded)
                    || record.actions.values().any(|x| {
                        x.kind == ActionKind::CommitTransaction && x.state == ActionState::Succeeded
                    }) =>
            {
                return self.invalid(
                    "superseded plan requires discarded work before any commit succeeded",
                );
            }
            PlanCloseReason::Executed | PlanCloseReason::Reported | PlanCloseReason::Superseded => {
            }
        }
        let record = self.plans.get_mut(plan).expect("active plan exists");
        if reason == PlanCloseReason::Superseded {
            for action in record.actions.values_mut() {
                if action.state == ActionState::Declared {
                    action.state = ActionState::Superseded;
                }
            }
            self.retry_allowed = true;
        } else {
            if self.final_plan.is_some() {
                return self.invalid("job has more than one non-superseded plan");
            }
            self.final_plan = Some(plan.clone());
        }
        record.closed = Some(reason);
        self.active = None;
        Ok(())
    }
    fn complete(&mut self, job: &JobId) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        if self.operation_completed {
            return self.invalid("operation-completed emitted more than once");
        }
        if self.active.is_some() || self.retry_allowed {
            return self.invalid("operation completed with an open lifecycle state");
        }
        self.operation_completed = true;
        Ok(())
    }
    fn finish_job(&mut self, job: &JobId) -> Result<(), LifecycleError> {
        self.same_job(job)?;
        if !self.operation_completed {
            return self.invalid("job-finished preceded operation-completed");
        }
        if self.job_finished {
            return self.invalid("job-finished emitted more than once");
        }
        self.job_finished = true;
        Ok(())
    }
    fn finish(&self, outcome: &OperationOutcome) -> Result<Reduction, LifecycleError> {
        self.same_job(&outcome.job)?;
        if self.active.is_some() || self.retry_allowed {
            return self.invalid("job ended with an open planning attempt or plan");
        }
        let mut totals = ActionTotals::default();
        let mut root_failures = 0;
        let mut cancelled = self.planning_cancelled;
        let mut cache_hits = 0;
        if let Some(plan) = &self.final_plan {
            let record = &self.plans[plan];
            root_failures += record.expected_issues;
            for action in record.actions.values() {
                cache_hits += u64::from(action.cache_hit);
                match action.state {
                    ActionState::Succeeded => totals.succeeded += 1,
                    ActionState::Failed => {
                        totals.failed += 1;
                        root_failures += 1;
                    }
                    ActionState::Blocked => totals.blocked += 1,
                    ActionState::Cancelled => {
                        totals.cancelled += 1;
                        cancelled = true;
                    }
                    ActionState::Declared | ActionState::Started | ActionState::Superseded => {}
                }
            }
        } else if self.fatal_planning_failure {
            root_failures = 1;
        } else if !self.planning_cancelled {
            return self.invalid("job has neither a final plan nor a terminal planning outcome");
        }
        if totals != outcome.totals
            || root_failures != outcome.root_failures
            || cancelled != outcome.cancelled
        {
            return self.invalid("declared outcome disagrees with reduced events");
        }
        Ok(Reduction {
            totals,
            root_failures,
            cache_hits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_protocol::{
        ActionKind, BuildRequest, BuildResult, Diagnostic, DiagnosticId, Digest, DigestAlgorithm,
        EmitKind, InspectRequest, InspectResult, InspectView, LockMode, OpaqueSourceId, Phase,
        PlanDigest, PlanningStepKind, ProfileName, ProjectInspection, ProjectPath, Severity,
        WorkspaceScope,
    };
    use std::sync::Mutex;

    fn job() -> JobId {
        JobId::new("job").unwrap()
    }
    fn attempt(value: &str) -> PlanningAttemptId {
        PlanningAttemptId::new(value).unwrap()
    }
    fn plan(value: &str) -> PlanId {
        PlanId::new(value).unwrap()
    }
    fn action(value: &str) -> ActionId {
        ActionId::new(value).unwrap()
    }
    fn plan_digest() -> PlanDigest {
        PlanDigest::new(Digest::new(DigestAlgorithm::Sha256, vec![0; 32]).unwrap())
    }
    fn diagnostic() -> Diagnostic {
        Diagnostic {
            id: DiagnosticId::new("d").unwrap(),
            code: "failed".into(),
            severity: Severity::Error,
            phase: Phase::Analyze,
            message: "failed".into(),
            primary: None,
            related: vec![],
            help: None,
        }
    }
    fn start(lifecycle: &mut Lifecycle, attempt: &PlanningAttemptId) {
        lifecycle
            .observe(&EventPayload::PlanningStarted {
                job: job(),
                attempt: attempt.clone(),
            })
            .unwrap();
    }
    fn seal(
        lifecycle: &mut Lifecycle,
        attempt: &PlanningAttemptId,
        plan: &PlanId,
        mode: PlanMode,
        actions: u64,
        issues: u64,
    ) {
        lifecycle
            .observe(&EventPayload::PlanReady {
                job: job(),
                attempt: attempt.clone(),
                plan: plan.clone(),
                digest: plan_digest(),
                mode,
                actions,
                issues,
            })
            .unwrap();
    }
    fn declare(
        lifecycle: &mut Lifecycle,
        plan: &PlanId,
        action: &ActionId,
        dependencies: Vec<ActionId>,
    ) {
        declare_kind(lifecycle, plan, action, ActionKind::Compile, dependencies);
    }
    fn declare_kind(
        lifecycle: &mut Lifecycle,
        plan: &PlanId,
        action: &ActionId,
        kind: ActionKind,
        dependencies: Vec<ActionId>,
    ) {
        lifecycle
            .observe(&EventPayload::ActionDeclared {
                job: job(),
                plan: plan.clone(),
                action: action.clone(),
                kind,
                dependencies,
            })
            .unwrap();
    }
    fn outcome(totals: ActionTotals, root_failures: u64, cancelled: bool) -> OperationOutcome {
        OperationOutcome {
            job: job(),
            result: OperationResult::Build(BuildResult {
                published: vec![],
                build_record: None,
            }),
            totals,
            root_failures,
            cancelled,
        }
    }

    #[test]
    fn executable_plan_requires_complete_declaration_and_closes() {
        let mut lifecycle = Lifecycle::default();
        let a = attempt("a");
        let p = plan("p");
        let x = action("x");
        let y = action("y");
        start(&mut lifecycle, &a);
        seal(&mut lifecycle, &a, &p, PlanMode::Execute, 2, 0);
        declare(&mut lifecycle, &p, &x, vec![]);
        assert!(
            lifecycle
                .observe(&EventPayload::ActionStarted {
                    job: job(),
                    plan: p.clone(),
                    action: x.clone()
                })
                .is_err()
        );
        declare(&mut lifecycle, &p, &y, vec![x.clone()]);
        lifecycle
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p.clone(),
                action: x.clone(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionSucceeded {
                job: job(),
                plan: p.clone(),
                action: x,
                timing: Timing::default(),
                artifacts: vec![],
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p.clone(),
                action: y.clone(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionSucceeded {
                job: job(),
                plan: p.clone(),
                action: y,
                timing: Timing::default(),
                artifacts: vec![],
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p,
                reason: PlanCloseReason::Executed,
            })
            .unwrap();
        lifecycle
            .finish(&outcome(
                ActionTotals {
                    succeeded: 2,
                    ..Default::default()
                },
                0,
                false,
            ))
            .unwrap();
    }

    #[test]
    fn planning_steps_must_be_terminal_and_plan_counts_must_match() {
        let mut lifecycle = Lifecycle::default();
        let a = attempt("a");
        let p = plan("p");
        let step = PlanningStepId::new("locate").unwrap();
        start(&mut lifecycle, &a);
        lifecycle
            .observe(&EventPayload::PlanningStepStarted {
                job: job(),
                attempt: a.clone(),
                step: step.clone(),
                kind: PlanningStepKind::Locate,
            })
            .unwrap();
        assert!(
            lifecycle
                .observe(&EventPayload::PlanReady {
                    job: job(),
                    attempt: a.clone(),
                    plan: p.clone(),
                    digest: plan_digest(),
                    mode: PlanMode::Execute,
                    actions: 0,
                    issues: 0
                })
                .is_err()
        );
        lifecycle
            .observe(&EventPayload::PlanningStepSucceeded {
                job: job(),
                attempt: a.clone(),
                step,
                timing: Timing::default(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::PlanningIssue {
                job: job(),
                attempt: a.clone(),
                issue: PlanningIssueId::new("issue").unwrap(),
                affected: vec![],
                diagnostic: diagnostic(),
            })
            .unwrap();
        assert!(
            lifecycle
                .observe(&EventPayload::PlanReady {
                    job: job(),
                    attempt: a.clone(),
                    plan: p,
                    digest: plan_digest(),
                    mode: PlanMode::Execute,
                    actions: 0,
                    issues: 0
                })
                .is_err()
        );
    }

    #[test]
    fn report_only_declares_graph_but_never_starts_actions() {
        let mut lifecycle = Lifecycle::default();
        let a = attempt("a");
        let p = plan("p");
        let x = action("x");
        start(&mut lifecycle, &a);
        seal(&mut lifecycle, &a, &p, PlanMode::ReportOnly, 1, 0);
        declare(&mut lifecycle, &p, &x, vec![]);
        assert!(
            lifecycle
                .observe(&EventPayload::ActionStarted {
                    job: job(),
                    plan: p.clone(),
                    action: x
                })
                .is_err()
        );
        lifecycle
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p,
                reason: PlanCloseReason::Reported,
            })
            .unwrap();
        lifecycle
            .finish(&outcome(Default::default(), 0, false))
            .unwrap();
    }

    #[test]
    fn pre_plan_failure_and_cancellation_have_zero_actions() {
        let mut failed = Lifecycle::default();
        let a = attempt("failure");
        start(&mut failed, &a);
        failed
            .observe(&EventPayload::PlanningFailed {
                job: job(),
                attempt: a,
                diagnostic: diagnostic(),
            })
            .unwrap();
        failed
            .finish(&outcome(Default::default(), 1, false))
            .unwrap();
        let mut cancelled = Lifecycle::default();
        let a = attempt("cancel");
        start(&mut cancelled, &a);
        cancelled
            .observe(&EventPayload::PlanningCancelled {
                job: job(),
                attempt: a,
            })
            .unwrap();
        cancelled
            .finish(&outcome(Default::default(), 0, true))
            .unwrap();
    }

    #[test]
    fn revision_conflict_supersedes_old_plan_then_replans() {
        let mut lifecycle = Lifecycle::default();
        let a1 = attempt("a1");
        let p1 = plan("p1");
        let commit = action("commit");
        start(&mut lifecycle, &a1);
        seal(&mut lifecycle, &a1, &p1, PlanMode::Execute, 1, 0);
        declare(&mut lifecycle, &p1, &commit, vec![]);
        lifecycle
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p1.clone(),
                action: commit.clone(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionSuperseded {
                job: job(),
                plan: p1.clone(),
                action: commit,
                timing: Timing::default(),
                reason: SupersedeReason::AuthoritativeRevisionChanged,
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p1,
                reason: PlanCloseReason::Superseded,
            })
            .unwrap();
        let a2 = attempt("a2");
        let p2 = plan("p2");
        let x = action("new");
        start(&mut lifecycle, &a2);
        seal(&mut lifecycle, &a2, &p2, PlanMode::Execute, 1, 0);
        declare(&mut lifecycle, &p2, &x, vec![]);
        lifecycle
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p2.clone(),
                action: x.clone(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionSucceeded {
                job: job(),
                plan: p2.clone(),
                action: x,
                timing: Timing::default(),
                artifacts: vec![],
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p2,
                reason: PlanCloseReason::Executed,
            })
            .unwrap();
        lifecycle
            .finish(&outcome(
                ActionTotals {
                    succeeded: 1,
                    ..Default::default()
                },
                0,
                false,
            ))
            .unwrap();
    }

    #[test]
    fn supersede_requires_discarded_work_and_rejects_a_succeeded_commit() {
        let mut valid = Lifecycle::default();
        let a = attempt("valid-attempt");
        let p = plan("valid-plan");
        let transform = action("transform");
        let commit = action("commit");
        start(&mut valid, &a);
        seal(&mut valid, &a, &p, PlanMode::Execute, 2, 0);
        declare(&mut valid, &p, &transform, vec![]);
        declare_kind(
            &mut valid,
            &p,
            &commit,
            ActionKind::CommitTransaction,
            vec![transform.clone()],
        );
        valid
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p.clone(),
                action: transform.clone(),
            })
            .unwrap();
        valid
            .observe(&EventPayload::ActionSucceeded {
                job: job(),
                plan: p.clone(),
                action: transform,
                timing: Timing::default(),
                artifacts: vec![],
            })
            .unwrap();
        valid
            .observe(&EventPayload::ActionSuperseded {
                job: job(),
                plan: p.clone(),
                action: commit,
                timing: Timing::default(),
                reason: SupersedeReason::AuthoritativeRevisionChanged,
            })
            .unwrap();
        valid
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p,
                reason: PlanCloseReason::Superseded,
            })
            .unwrap();

        let mut invalid = Lifecycle::default();
        let a = attempt("invalid-attempt");
        let p = plan("invalid-plan");
        let commit = action("commit");
        let discarded = action("discarded");
        start(&mut invalid, &a);
        seal(&mut invalid, &a, &p, PlanMode::Execute, 2, 0);
        declare_kind(
            &mut invalid,
            &p,
            &commit,
            ActionKind::CommitTransaction,
            vec![],
        );
        declare(&mut invalid, &p, &discarded, vec![]);
        invalid
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p.clone(),
                action: commit.clone(),
            })
            .unwrap();
        invalid
            .observe(&EventPayload::ActionSucceeded {
                job: job(),
                plan: p.clone(),
                action: commit,
                timing: Timing::default(),
                artifacts: vec![],
            })
            .unwrap();
        invalid
            .observe(&EventPayload::ActionSuperseded {
                job: job(),
                plan: p.clone(),
                action: discarded,
                timing: Timing::default(),
                reason: SupersedeReason::AuthoritativeRevisionChanged,
            })
            .unwrap();
        assert!(
            invalid
                .observe(&EventPayload::PlanClosed {
                    job: job(),
                    plan: p,
                    reason: PlanCloseReason::Superseded,
                })
                .is_err()
        );
    }

    #[test]
    fn plan_identity_and_invalid_interleavings_are_rejected() {
        let mut lifecycle = Lifecycle::default();
        let a = attempt("a");
        let p = plan("p");
        start(&mut lifecycle, &a);
        seal(&mut lifecycle, &a, &p, PlanMode::Execute, 1, 0);
        assert!(
            lifecycle
                .observe(&EventPayload::ActionDeclared {
                    job: job(),
                    plan: plan("wrong"),
                    action: action("x"),
                    kind: ActionKind::Compile,
                    dependencies: vec![]
                })
                .is_err()
        );
        assert!(
            lifecycle
                .observe(&EventPayload::ActionDeclared {
                    job: job(),
                    plan: p,
                    action: action("x"),
                    kind: ActionKind::Compile,
                    dependencies: vec![action("missing")]
                })
                .is_err()
        );
    }

    #[test]
    fn legacy_mode_and_cyclic_closed_graph_are_rejected() {
        let mut legacy = Lifecycle::default();
        let a = attempt("legacy-attempt");
        start(&mut legacy, &a);
        assert!(
            legacy
                .observe(&EventPayload::PlanReady {
                    job: job(),
                    attempt: a,
                    plan: plan("legacy-plan"),
                    digest: plan_digest(),
                    mode: PlanMode::Legacy,
                    actions: 0,
                    issues: 0,
                })
                .is_err()
        );

        let mut cyclic = Lifecycle::default();
        let a = attempt("cycle-attempt");
        let p = plan("cycle-plan");
        let x = action("x");
        let y = action("y");
        start(&mut cyclic, &a);
        seal(&mut cyclic, &a, &p, PlanMode::Execute, 2, 0);
        declare(&mut cyclic, &p, &x, vec![y.clone()]);
        assert!(
            cyclic
                .observe(&EventPayload::ActionDeclared {
                    job: job(),
                    plan: p,
                    action: y,
                    kind: ActionKind::Compile,
                    dependencies: vec![x],
                })
                .is_err()
        );
    }

    #[test]
    fn final_plan_issues_and_failures_drive_root_failure_reduction() {
        let mut lifecycle = Lifecycle::default();
        let a = attempt("a");
        let p = plan("p");
        let x = action("x");
        start(&mut lifecycle, &a);
        lifecycle
            .observe(&EventPayload::PlanningIssue {
                job: job(),
                attempt: a.clone(),
                issue: PlanningIssueId::new("issue").unwrap(),
                affected: vec![],
                diagnostic: diagnostic(),
            })
            .unwrap();
        seal(&mut lifecycle, &a, &p, PlanMode::Execute, 1, 1);
        declare(&mut lifecycle, &p, &x, vec![]);
        lifecycle
            .observe(&EventPayload::ActionStarted {
                job: job(),
                plan: p.clone(),
                action: x.clone(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::ActionFailed {
                job: job(),
                plan: p.clone(),
                action: x,
                timing: Timing::default(),
                diagnostic: diagnostic(),
            })
            .unwrap();
        lifecycle
            .observe(&EventPayload::PlanClosed {
                job: job(),
                plan: p,
                reason: PlanCloseReason::Executed,
            })
            .unwrap();
        lifecycle
            .finish(&outcome(
                ActionTotals {
                    failed: 1,
                    ..Default::default()
                },
                2,
                false,
            ))
            .unwrap();
    }

    #[test]
    fn inspect_result_identity_validation_is_preserved() {
        let request = OperationRequest::Inspect(InspectRequest {
            project: ProjectPath::new(".").unwrap(),
            view: InspectView::Source(OpaqueSourceId::new("src/main.squish").unwrap()),
        });
        let wrong = OperationResult::Inspect(InspectResult::Project(ProjectInspection {
            packages: vec![],
            targets: vec![],
        }));
        assert!(!wrong.matches_request(&request));
    }

    #[test]
    fn build_request_result_identity_still_matches() {
        let request = OperationRequest::Build(BuildRequest {
            project: ProjectPath::new(".").unwrap(),
            scope: WorkspaceScope::Current,
            targets: vec![],
            profile: ProfileName::new("dev").unwrap(),
            arguments: Default::default(),
            emit: vec![EmitKind::Prompt],
            lock: LockMode::Update,
        });
        let result = OperationResult::Build(BuildResult {
            published: vec![],
            build_record: None,
        });
        assert!(result.matches_request(&request));
    }

    static BUILD_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
        id: "planning-failure",
        operations: &[OperationKind::Build],
        summary: "planning failure",
    };
    struct PlanningFailure;
    impl Capability for PlanningFailure {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &BUILD_DESCRIPTOR
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let attempt = attempt("fatal");
            context
                .emit(EventPayload::PlanningStarted {
                    job: job(),
                    attempt: attempt.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::PlanningFailed {
                    job: job(),
                    attempt,
                    diagnostic: diagnostic(),
                })
                .unwrap();
            OperationOutcome {
                job: job(),
                result: OperationResult::Unavailable {
                    kind: OperationKind::Build,
                },
                totals: ActionTotals::default(),
                root_failures: 1,
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

    #[test]
    fn fatal_planning_failure_with_zero_actions_drives_failed_exit() {
        static CAPABILITY: PlanningFailure = PlanningFailure;
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("invocation").unwrap(),
            CancellationToken::default(),
            sink,
        );
        let request = OperationRequest::Build(BuildRequest {
            project: ProjectPath::new(".").unwrap(),
            scope: WorkspaceScope::Current,
            targets: vec![],
            profile: ProfileName::new("dev").unwrap(),
            arguments: Default::default(),
            emit: vec![EmitKind::Prompt],
            lock: LockMode::Update,
        });
        let dispatched = Kernel::new(&[&CAPABILITY])
            .unwrap()
            .dispatch(&request, &context)
            .unwrap();
        assert_eq!(dispatched.summary.status, ExitStatus::Failed);
        assert_eq!(dispatched.summary.totals, ActionTotals::default());
        assert_eq!(dispatched.summary.root_failures, 1);
    }

    struct LateCancellation;
    impl Capability for LateCancellation {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &BUILD_DESCRIPTOR
        }
        fn execute(&self, _: &OperationRequest, context: &InvocationContext) -> OperationOutcome {
            let attempt = attempt("late-cancel-attempt");
            let plan = plan("late-cancel-plan");
            let action = action("work");
            context
                .emit(EventPayload::PlanningStarted {
                    job: job(),
                    attempt: attempt.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::PlanReady {
                    job: job(),
                    attempt,
                    plan: plan.clone(),
                    digest: plan_digest(),
                    mode: PlanMode::Execute,
                    actions: 1,
                    issues: 0,
                })
                .unwrap();
            context
                .emit(EventPayload::ActionDeclared {
                    job: job(),
                    plan: plan.clone(),
                    action: action.clone(),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                })
                .unwrap();
            context
                .emit(EventPayload::ActionStarted {
                    job: job(),
                    plan: plan.clone(),
                    action: action.clone(),
                })
                .unwrap();
            context
                .emit(EventPayload::ActionSucceeded {
                    job: job(),
                    plan: plan.clone(),
                    action,
                    timing: Timing::default(),
                    artifacts: vec![],
                })
                .unwrap();
            context
                .emit(EventPayload::PlanClosed {
                    job: job(),
                    plan,
                    reason: PlanCloseReason::Executed,
                })
                .unwrap();
            context.cancellation().cancel();
            OperationOutcome {
                job: job(),
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

    #[test]
    fn advisory_cancellation_after_plan_close_does_not_rewrite_success() {
        static CAPABILITY: LateCancellation = LateCancellation;
        let sink = Arc::new(Sink::default());
        let context = InvocationContext::new(
            InvocationId::new("late-cancel-invocation").unwrap(),
            CancellationToken::default(),
            sink.clone(),
        );
        let request = OperationRequest::Build(BuildRequest {
            project: ProjectPath::new(".").unwrap(),
            scope: WorkspaceScope::Current,
            targets: vec![],
            profile: ProfileName::new("dev").unwrap(),
            arguments: Default::default(),
            emit: vec![EmitKind::Prompt],
            lock: LockMode::Update,
        });
        let dispatched = Kernel::new(&[&CAPABILITY])
            .unwrap()
            .dispatch(&request, &context)
            .unwrap();
        assert_eq!(dispatched.summary.status, ExitStatus::Success);
        let events = lock(&sink.0);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.payload, EventPayload::OperationCompleted { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.payload, EventPayload::JobFinished(_)))
                .count(),
            1
        );
    }
}
