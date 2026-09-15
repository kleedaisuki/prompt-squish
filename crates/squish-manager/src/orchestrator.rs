//! 统一调度和事件映射。 / Unified scheduling and event mapping.

use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
    time::Instant,
};

use squish_build::{
    Action, ActionEvent, ActionKey, ActionResult, ActionState, BuildPlan, Dispatch, ProducedOutput,
    Resources, ResultSource, ScheduleEvent, Scheduler,
};
use squish_kernel::{CancellationToken, InvocationContext, OperationOutcome};
use squish_protocol::{
    ActionId, ActionKeyId, ActionTotals, Artifact, CacheKind, Diagnostic, DiagnosticId, Digest,
    DigestAlgorithm, EventPayload, FinalizationId, FinalizationKind, JobId, OperationKind,
    OperationResult, Phase, PlanCloseReason, PlanDigest, PlanId, PlanInspection, PlanMode,
    PlanScopeId, PlannedAction, PlanningAttemptId, PlanningIssueId, PlanningStepId,
    PlanningStepKind, Severity, SupersedeReason, Timing,
};

use crate::{Effect, InvocationSettings, ManagerError, PlannedWork, PreparedPlan};

/// 持久动作缓存的一项已校验命中。 / One validated persistent action-cache hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedResult {
    /// 结果记录的内容摘要。 / Content digest of the result record.
    pub digest: squish_protocol::Digest,
    /// 调度器消费的完整具名输出。 / Complete named outputs consumed by the scheduler.
    pub outputs: Vec<ProducedOutput>,
    /// 协议事件公开的完整产物。 / Complete artifacts exposed by protocol events.
    pub artifacts: Vec<Artifact>,
    /// 缓存所在位置。 / Cache location.
    pub cache: CacheKind,
}

/// 派发时冻结的完整传递前驱输出。 / Complete transitive-prerequisite outputs frozen at dispatch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedInputs {
    outputs: BTreeMap<ActionId, Vec<ProducedOutput>>,
}

impl ResolvedInputs {
    /// 返回一个传递前驱的有序输出。 / Returns the ordered outputs of one transitive prerequisite.
    pub fn outputs(&self, action: &ActionId) -> Option<&[ProducedOutput]> {
        self.outputs.get(action).map(Vec::as_slice)
    }

    /// 按动作 ID 稳定迭代全部传递前驱输出。 / Iterates all transitive-prerequisite outputs in stable action-ID order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&ActionId, &[ProducedOutput])> {
        self.outputs
            .iter()
            .map(|(action, outputs)| (action, outputs.as_slice()))
    }
}

/// 无展示副作用的同步工作执行端口。 / Synchronous work-execution port without presentation side effects.
pub trait WorkExecutor<W: PlannedWork>: Sync {
    /// 仅对纯变换查找持久缓存。 / Looks up persistent cache only for a pure transform.
    fn lookup(
        &self,
        dispatch: &Dispatch,
        work: &W,
        plan: &PreparedPlan<W>,
        inputs: &ResolvedInputs,
    ) -> Result<Option<CachedResult>, ManagerError>;

    /// 执行一个调度器 dispatch；worker 不得打印。 / Executes one scheduler dispatch; the worker must not print.
    fn execute(
        &self,
        dispatch: &Dispatch,
        work: &W,
        plan: &PreparedPlan<W>,
        inputs: &ResolvedInputs,
        cancellation: CancellationToken,
    ) -> WorkDisposition;

    /// 记录成功纯变换的缓存结果。 / Records the cache result of a successful pure transform.
    fn record(
        &self,
        dispatch: &Dispatch,
        work: &W,
        result: &ActionResult,
    ) -> Result<(), ManagerError>;
}

/// Worker 的类型化执行处置。 / Typed execution disposition returned by a worker.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkDisposition {
    /// 普通成功或失败结果。 / Ordinary success or failure result.
    Complete(ActionResult),
    /// 权威修订竞争使计划失效。 / An authoritative-revision race invalidated the plan.
    Superseded {
        /// 封闭原因。 / Closed reason.
        reason: SupersedeReason,
        /// 检测竞争前产生的事实。 / Facts produced before detecting contention.
        events: Vec<ActionEvent>,
    },
}

impl From<ActionResult> for WorkDisposition {
    fn from(result: ActionResult) -> Self {
        Self::Complete(result)
    }
}

/// 一次统一调度的归约结果。 / Reduced result of one unified scheduling run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    /// 动作终态计数。 / Terminal action counts.
    pub totals: ActionTotals,
    /// 非阻塞根失败数。 / Root failure count excluding blocked actions.
    pub root_failures: u64,
    /// 是否响应了取消。 / Whether cancellation was observed.
    pub cancelled: bool,
    /// 需要开始新计划尝试的原因。 / Reason a fresh planning attempt is required.
    pub superseded: Option<SupersedeReason>,
    /// 按动作 ID 排序的权威执行事实。 / Authoritative execution facts sorted by action ID.
    pub actions: Vec<ActionExecutionFact>,
}

/// 聚合记录使用的动作终态。 / Action terminal state used by aggregate records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionState {
    /// 仅声明，未执行。 / Declared but not executed.
    Declared,
    /// 成功。 / Succeeded.
    Succeeded,
    /// Worker 失败。 / Worker failure.
    Failed {
        /// 稳定机器代码。 / Stable machine code.
        code: String,
        /// 面向用户的消息。 / User-facing message.
        message: String,
    },
    /// 被直接前驱阻塞。 / Blocked by a direct prerequisite.
    Blocked {
        /// 阻塞前驱。 / Blocking prerequisite.
        dependency: ActionId,
    },
    /// 已取消。 / Cancelled.
    Cancelled,
    /// 因权威状态竞争被废弃。 / Superseded by an authoritative-state race.
    Superseded,
}

/// 一个动作的权威调度事实。 / Authoritative scheduling fact for one action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionExecutionFact {
    /// 动作 ID。 / Action ID.
    pub action: ActionId,
    /// 动作类别。 / Action kind.
    pub kind: squish_protocol::ActionKind,
    /// 规范顺序的直接前驱。 / Direct prerequisites in canonical order.
    pub dependencies: Vec<ActionId>,
    /// 最终状态。 / Final state.
    pub state: ExecutionState,
    /// 已物化时的完整动作键。 / Complete action key when materialized.
    pub key: Option<ActionKeyId>,
    /// 已成功物化的具名输出。 / Named outputs materialized on success.
    pub outputs: Vec<ProducedOutput>,
    /// 成功结果的来源；非成功动作为空。 / Source of a successful result; absent otherwise.
    pub source: Option<ResultSource>,
}

/// 活动计划尝试的唯一事件记录器。 / Sole event recorder for an active planning attempt.
pub struct PlanningRecorder<'a> {
    job: JobId,
    attempt: PlanningAttemptId,
    context: &'a InvocationContext,
    issues: Vec<Vec<u8>>,
    ended: bool,
}

/// 计划步骤的类型化非成功结果。 / Typed non-success result of a planning step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanningFailure {
    /// 领域或适配器失败。 / Domain or adapter failure.
    Failed(ManagerError),
    /// 协作式取消。 / Cooperative cancellation.
    Cancelled,
}

/// 必须计作一个根失败的收尾失败。 / Finalization failure that contributes one root failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizationFailure {
    error: ManagerError,
}

impl FinalizationFailure {
    /// 返回结构化管理器错误。 / Returns the structured manager error.
    pub fn error(&self) -> &ManagerError {
        &self.error
    }

    /// 返回内核归约所需的根失败增量。 / Returns the root-failure increment required by kernel reduction.
    pub const fn root_failures(&self) -> u64 {
        1
    }

    /// 消耗包装并返回管理器错误。 / Consumes the wrapper and returns the manager error.
    pub fn into_error(self) -> ManagerError {
        self.error
    }
}

impl std::fmt::Display for FinalizationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for FinalizationFailure {}

/// 在最终计划关闭后执行一项不可取消的终态收尾工作。 / Runs one non-cancellable terminal finalization after the final plan closes.
pub fn finalize<T>(
    job: &JobId,
    id: FinalizationId,
    kind: FinalizationKind,
    context: &InvocationContext,
    operation: impl FnOnce() -> Result<T, ManagerError>,
) -> Result<T, FinalizationFailure> {
    emit(
        context,
        EventPayload::FinalizationStarted {
            job: job.clone(),
            id: id.clone(),
            kind,
        },
    )
    .map_err(finalization_failure)?;
    let started = Instant::now();
    match operation() {
        Ok(value) => {
            emit(
                context,
                EventPayload::FinalizationSucceeded {
                    job: job.clone(),
                    id,
                    timing: elapsed(started),
                },
            )
            .map_err(finalization_failure)?;
            Ok(value)
        }
        Err(error) => {
            let diagnostic_id = DiagnosticId::new(format!("finalization-{id}-failed"))
                .expect("non-empty finalization ID");
            let payload = EventPayload::FinalizationFailed {
                job: job.clone(),
                id,
                timing: elapsed(started),
                diagnostic: error.diagnostic(diagnostic_id),
            };
            emit(context, payload).map_err(finalization_failure)?;
            Err(finalization_failure(error))
        }
    }
}

fn finalization_failure(error: ManagerError) -> FinalizationFailure {
    FinalizationFailure { error }
}

impl<'a> PlanningRecorder<'a> {
    /// 开始一个具名计划尝试。 / Starts a named planning attempt.
    pub fn start(
        job: JobId,
        attempt: PlanningAttemptId,
        context: &'a InvocationContext,
    ) -> Result<Self, ManagerError> {
        emit(
            context,
            EventPayload::PlanningStarted {
                job: job.clone(),
                attempt: attempt.clone(),
            },
        )?;
        Ok(Self {
            job,
            attempt,
            context,
            issues: Vec::new(),
            ended: false,
        })
    }

    /// 执行并记录一项真实计划工作。 / Executes and records one real planning step.
    pub fn step<T>(
        &mut self,
        step: PlanningStepId,
        kind: PlanningStepKind,
        operation: impl FnOnce() -> Result<T, ManagerError>,
    ) -> Result<T, PlanningFailure> {
        emit(
            self.context,
            EventPayload::PlanningStepStarted {
                job: self.job.clone(),
                attempt: self.attempt.clone(),
                step: step.clone(),
                kind,
            },
        )
        .map_err(PlanningFailure::Failed)?;
        let started = Instant::now();
        if self.context.is_cancelled() {
            self.cancel_step(step, started)?;
            return Err(PlanningFailure::Cancelled);
        }
        match operation() {
            Ok(value) => {
                if self.context.is_cancelled() {
                    self.cancel_step(step, started)?;
                    return Err(PlanningFailure::Cancelled);
                }
                emit(
                    self.context,
                    EventPayload::PlanningStepSucceeded {
                        job: self.job.clone(),
                        attempt: self.attempt.clone(),
                        step,
                        timing: elapsed(started),
                    },
                )
                .map_err(PlanningFailure::Failed)?;
                Ok(value)
            }
            Err(error) => {
                if self.context.is_cancelled() {
                    self.cancel_step(step, started)?;
                    return Err(PlanningFailure::Cancelled);
                }
                emit(
                    self.context,
                    EventPayload::PlanningStepFailed {
                        job: self.job.clone(),
                        attempt: self.attempt.clone(),
                        step: step.clone(),
                        timing: elapsed(started),
                        diagnostic: error.diagnostic(planning_diagnostic(&step)),
                    },
                )
                .map_err(PlanningFailure::Failed)?;
                emit(
                    self.context,
                    EventPayload::PlanningFailed {
                        job: self.job.clone(),
                        attempt: self.attempt.clone(),
                        diagnostic: error.diagnostic(
                            DiagnosticId::new(format!("planning-{}-failed", self.attempt))
                                .expect("non-empty"),
                        ),
                    },
                )
                .map_err(PlanningFailure::Failed)?;
                self.ended = true;
                Err(PlanningFailure::Failed(error))
            }
        }
    }

    fn cancel_step(
        &mut self,
        step: PlanningStepId,
        started: Instant,
    ) -> Result<(), PlanningFailure> {
        emit(
            self.context,
            EventPayload::PlanningStepCancelled {
                job: self.job.clone(),
                attempt: self.attempt.clone(),
                step,
                timing: elapsed(started),
            },
        )
        .map_err(PlanningFailure::Failed)?;
        emit(
            self.context,
            EventPayload::PlanningCancelled {
                job: self.job.clone(),
                attempt: self.attempt.clone(),
            },
        )
        .map_err(PlanningFailure::Failed)?;
        self.ended = true;
        Ok(())
    }

    /// 记录不阻止独立计划范围的规范问题。 / Records a canonical issue that does not prevent independent scopes.
    pub fn issue(
        &mut self,
        issue: PlanningIssueId,
        mut affected: Vec<PlanScopeId>,
        diagnostic: Diagnostic,
    ) -> Result<(), ManagerError> {
        affected.sort();
        affected.dedup();
        self.issues.push(
            serde_json::to_vec(&(issue.as_str(), &affected, &diagnostic))
                .map_err(|error| orchestration(error.to_string()))?,
        );
        let payload = EventPayload::PlanningIssue {
            job: self.job.clone(),
            attempt: self.attempt.clone(),
            issue,
            affected,
            diagnostic,
        };
        emit(self.context, payload)
    }

    /// 原子封闭计划并声明完整执行图。 / Atomically seals a plan and declares its complete execution graph.
    pub fn seal<W: PlannedWork>(
        mut self,
        prepared: PreparedPlan<W>,
        snapshot: &Digest,
        mode: PlanMode,
    ) -> Result<SealedPlan<W>, PlanningFailure> {
        if self.ended {
            return Err(PlanningFailure::Failed(orchestration(
                "planning attempt already ended",
            )));
        }
        if mode == PlanMode::Legacy {
            return Err(self.end_failed(orchestration(
                "legacy plans cannot be sealed as native v2 plans",
            )));
        }
        if self.context.is_cancelled() {
            emit(
                self.context,
                EventPayload::PlanningCancelled {
                    job: self.job.clone(),
                    attempt: self.attempt.clone(),
                },
            )
            .map_err(PlanningFailure::Failed)?;
            self.ended = true;
            return Err(PlanningFailure::Cancelled);
        }
        if let Err(error) = reject_planning_placeholders(prepared.graph()) {
            return Err(self.end_failed(error));
        }
        let identity = plan_identity(
            &self.job,
            &self.attempt,
            snapshot,
            prepared.graph(),
            &self.issues,
            mode,
        );
        emit(
            self.context,
            EventPayload::PlanReady {
                job: self.job.clone(),
                attempt: self.attempt.clone(),
                plan: identity.plan.clone(),
                digest: identity.digest.clone(),
                mode,
                actions: prepared.graph().actions().len() as u64,
                issues: self.issues.len() as u64,
            },
        )
        .map_err(PlanningFailure::Failed)?;
        announce(&self.job, &identity.plan, &prepared, self.context)
            .map_err(PlanningFailure::Failed)?;
        self.ended = true;
        Ok(SealedPlan {
            job: self.job,
            identity,
            prepared,
        })
    }

    /// 返回规划失败后的诚实不可用结果。 / Returns an honest unavailable result after planning failed.
    pub fn failed(self, operation: OperationKind) -> OperationOutcome {
        let cancelled = self.context.is_cancelled();
        OperationOutcome {
            job: self.job,
            result: OperationResult::Unavailable { kind: operation },
            totals: ActionTotals::default(),
            root_failures: u64::from(!cancelled),
            cancelled,
        }
    }

    /// 返回一次计划失败或取消对应的不可用结果。 / Returns the unavailable result for a planning failure or cancellation.
    pub fn unavailable(
        self,
        operation: OperationKind,
        failure: &PlanningFailure,
    ) -> OperationOutcome {
        OperationOutcome {
            job: self.job,
            result: OperationResult::Unavailable { kind: operation },
            totals: ActionTotals::default(),
            root_failures: u64::from(matches!(failure, PlanningFailure::Failed(_))),
            cancelled: matches!(failure, PlanningFailure::Cancelled),
        }
    }

    fn end_failed(&mut self, error: ManagerError) -> PlanningFailure {
        let diagnostic = error.diagnostic(
            DiagnosticId::new(format!("planning-{}-failed", self.attempt)).expect("non-empty"),
        );
        let payload = EventPayload::PlanningFailed {
            job: self.job.clone(),
            attempt: self.attempt.clone(),
            diagnostic,
        };
        self.ended = true;
        match emit(self.context, payload) {
            Ok(()) => PlanningFailure::Failed(error),
            Err(emission) => PlanningFailure::Failed(emission),
        }
    }
}

/// 封闭计划的统一身份。 / Unified identity of a sealed plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanIdentity {
    /// 作业内的计划 ID。 / Job-scoped plan ID.
    pub plan: PlanId,
    /// 计划的规范语义摘要。 / Canonical semantic plan digest.
    pub digest: PlanDigest,
    /// 执行或仅报告模式。 / Execute or report-only mode.
    pub mode: PlanMode,
}

/// 从已闭合计划失败构造领域不可用结果。 / Constructs an unavailable outcome from a closed planning failure.
pub fn planning_outcome(
    job: JobId,
    operation: OperationKind,
    failure: &PlanningFailure,
) -> OperationOutcome {
    OperationOutcome {
        job,
        result: OperationResult::Unavailable { kind: operation },
        totals: ActionTotals::default(),
        root_failures: u64::from(matches!(failure, PlanningFailure::Failed(_))),
        cancelled: matches!(failure, PlanningFailure::Cancelled),
    }
}

/// 已发布声明、可交给调度器的计划。 / Declared sealed plan ready for scheduling.
pub struct SealedPlan<W> {
    job: JobId,
    identity: PlanIdentity,
    prepared: PreparedPlan<W>,
}

impl<W: PlannedWork> SealedPlan<W> {
    /// 返回作业身份。 / Returns the job identity.
    pub fn job(&self) -> &JobId {
        &self.job
    }

    /// 返回计划身份。 / Returns the sealed plan identity.
    pub fn identity(&self) -> &PlanIdentity {
        &self.identity
    }

    /// 返回已验证工作图。 / Returns the validated work graph.
    pub fn prepared(&self) -> &PreparedPlan<W> {
        &self.prepared
    }

    /// 投影为协议查询结果，复用完全相同的身份。 / Projects to protocol inspection using exactly the same identity.
    pub fn inspection(&self) -> PlanInspection {
        PlanInspection {
            job: self.job.clone(),
            plan: self.identity.plan.clone(),
            digest: self.identity.digest.clone(),
            mode: self.identity.mode,
            actions: topological_actions(self.prepared.graph())
                .into_iter()
                .map(|action| PlannedAction {
                    action: action.id.clone(),
                    kind: action.kind,
                    action_key: None,
                    dependencies: canonical_dependencies(action),
                })
                .collect(),
        }
    }
}

/// 将计划前失败正规化为 v2 计划失败。 / Normalizes a pre-plan failure into a v2 planning failure.
pub fn fail(
    job: JobId,
    operation: OperationKind,
    _action_kind: squish_protocol::ActionKind,
    error: ManagerError,
    context: &InvocationContext,
    _settings: &InvocationSettings,
) -> OperationOutcome {
    let attempt = PlanningAttemptId::new("attempt-1").expect("non-empty");
    let _ = emit(
        context,
        EventPayload::PlanningStarted {
            job: job.clone(),
            attempt: attempt.clone(),
        },
    );
    let _ = emit(
        context,
        EventPayload::PlanningFailed {
            job: job.clone(),
            attempt,
            diagnostic: error.diagnostic(DiagnosticId::new("planning-failed").expect("non-empty")),
        },
    );
    OperationOutcome {
        job,
        result: OperationResult::Unavailable { kind: operation },
        totals: ActionTotals::default(),
        root_failures: 1,
        cancelled: false,
    }
}

/// 用唯一调度路径执行计划并发布合法协议事件。 / Executes a plan through the sole scheduling path and publishes valid protocol events.
pub fn run<W: PlannedWork + Sync>(
    sealed: SealedPlan<W>,
    executor: &dyn WorkExecutor<W>,
    context: &InvocationContext,
    settings: &InvocationSettings,
) -> Result<ExecutionReport, ManagerError> {
    let SealedPlan {
        job,
        identity,
        prepared,
    } = sealed;
    if identity.mode == PlanMode::ReportOnly {
        emit(
            context,
            EventPayload::PlanClosed {
                job,
                plan: identity.plan,
                reason: PlanCloseReason::Reported,
            },
        )?;
        return Ok(ExecutionReport {
            totals: ActionTotals::default(),
            root_failures: 0,
            cancelled: false,
            superseded: None,
            actions: Vec::new(),
        });
    }
    let capacity = capacity(prepared.graph(), settings.jobs);
    let mut scheduler = Scheduler::new(prepared.graph().clone(), capacity, settings.keep_going)
        .map_err(|error| orchestration(error.to_string()))?;
    let mut mapping = EventMapping::new(job.clone(), identity.plan.clone());
    mapping.publish(scheduler.drain_events(), context)?;
    let superseded = drive(
        &prepared,
        executor,
        context,
        &mut scheduler,
        &mut mapping,
        settings.jobs.max(1),
    )?;
    let mut report = mapping.report(
        context.is_cancelled(),
        &scheduler,
        prepared.graph(),
        superseded.is_some(),
    );
    report.superseded = superseded;
    emit(
        context,
        EventPayload::PlanClosed {
            job,
            plan: identity.plan,
            reason: if superseded.is_some() {
                PlanCloseReason::Superseded
            } else {
                PlanCloseReason::Executed
            },
        },
    )?;
    Ok(report)
}

fn announce<W: PlannedWork>(
    job: &JobId,
    plan_id: &PlanId,
    plan: &PreparedPlan<W>,
    context: &InvocationContext,
) -> Result<(), ManagerError> {
    for action in topological_actions(plan.graph()) {
        emit(
            context,
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan_id.clone(),
                action: action.id.clone(),
                kind: action.kind,
                dependencies: canonical_dependencies(action),
            },
        )?;
    }
    Ok(())
}

fn canonical_dependencies(action: &Action) -> Vec<ActionId> {
    let mut dependencies = action.dependencies.clone();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

fn reject_planning_placeholders(graph: &BuildPlan) -> Result<(), ManagerError> {
    if graph.actions().any(|action| {
        matches!(
            action.kind,
            squish_protocol::ActionKind::Resolve
                | squish_protocol::ActionKind::Snapshot
                | squish_protocol::ActionKind::Scan
                | squish_protocol::ActionKind::ResolveCandidate
        )
    }) {
        Err(orchestration(
            "planning work cannot appear in a v2 execution plan",
        ))
    } else {
        Ok(())
    }
}

fn plan_identity(
    job: &JobId,
    attempt: &PlanningAttemptId,
    snapshot: &Digest,
    graph: &BuildPlan,
    issues: &[Vec<u8>],
    mode: PlanMode,
) -> PlanIdentity {
    let mut hash = blake3::Hasher::new();
    hash.update(b"xmlsquish-plan\0v2");
    hash_field(&mut hash, digest_algorithm(snapshot));
    hash_field(&mut hash, snapshot.bytes());
    hash_field(&mut hash, graph.semantic_digest().as_bytes());
    hash_field(
        &mut hash,
        match mode {
            PlanMode::Execute => b"execute",
            PlanMode::ReportOnly => b"report-only",
            PlanMode::Legacy => b"legacy",
        },
    );
    let mut issues = issues.to_vec();
    issues.sort();
    for issue in &issues {
        hash_field(&mut hash, issue);
    }
    let bytes = *hash.finalize().as_bytes();
    let digest = PlanDigest::new(
        Digest::new(DigestAlgorithm::Blake3, bytes.to_vec()).expect("BLAKE3 is 32 bytes"),
    );
    let plan = PlanId::new(format!(
        "{}:{}:{}",
        job,
        attempt,
        &digest.digest().hex()[..16]
    ))
    .expect("non-empty");
    PlanIdentity { plan, digest, mode }
}

fn digest_algorithm(digest: &Digest) -> &[u8] {
    match digest.algorithm() {
        DigestAlgorithm::Sha256 => b"sha256",
        DigestAlgorithm::Blake3 => b"blake3",
        DigestAlgorithm::Other(name) => name.as_bytes(),
    }
}

fn hash_field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn planning_diagnostic(step: &PlanningStepId) -> DiagnosticId {
    DiagnosticId::new(format!("planning-{step}-failed")).expect("non-empty")
}

fn topological_actions(graph: &BuildPlan) -> Vec<&Action> {
    let mut degrees = BTreeMap::new();
    let mut dependents: BTreeMap<ActionId, Vec<ActionId>> = BTreeMap::new();
    let mut ready = BTreeSet::new();
    for action in graph.actions() {
        degrees.insert(action.id.clone(), action.dependencies.len());
        if action.dependencies.is_empty() {
            ready.insert(action.id.clone());
        }
        for dependency in &action.dependencies {
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(action.id.clone());
        }
    }
    let mut ordered = Vec::with_capacity(graph.actions().len());
    while let Some(id) = ready.pop_first() {
        ordered.push(graph.action(&id).expect("topological ID exists"));
        for dependent in dependents.get(&id).into_iter().flatten() {
            let degree = degrees.get_mut(dependent).expect("dependent ID exists");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(dependent.clone());
            }
        }
    }
    assert_eq!(ordered.len(), degrees.len(), "BuildPlan is acyclic");
    ordered
}

fn drive<W: PlannedWork + Sync>(
    plan: &PreparedPlan<W>,
    executor: &dyn WorkExecutor<W>,
    context: &InvocationContext,
    scheduler: &mut Scheduler,
    mapping: &mut EventMapping,
    jobs: usize,
) -> Result<Option<SupersedeReason>, ManagerError> {
    while !scheduler.is_finished() {
        if context.is_cancelled() {
            scheduler.request_cancellation();
            mapping.publish(scheduler.drain_events(), context)?;
            continue;
        }
        let mut batch = Vec::with_capacity(jobs);
        while batch.len() < jobs {
            let Some(dispatch) = scheduler.next_dispatch() else {
                break;
            };
            let inputs = resolved_inputs(plan, scheduler, &dispatch);
            batch.push(DispatchTask { dispatch, inputs });
        }
        mapping.publish(scheduler.drain_events(), context)?;
        if batch.is_empty() {
            return Err(orchestration("scheduler made no progress"));
        }
        let completed = execute_batch(plan, executor, context.cancellation(), batch)?;
        if let Some(reason) = complete_batch(scheduler, mapping, context, completed)? {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

enum DispatchOutcome {
    Cached(CachedResult),
    Worker(ActionResult),
    Superseded {
        reason: SupersedeReason,
        events: Vec<ActionEvent>,
    },
}

struct CompletedDispatch {
    dispatch: Dispatch,
    outcome: DispatchOutcome,
    timing: Timing,
}

struct DispatchTask {
    dispatch: Dispatch,
    inputs: ResolvedInputs,
}

fn resolved_inputs<W: PlannedWork>(
    plan: &PreparedPlan<W>,
    scheduler: &Scheduler,
    dispatch: &Dispatch,
) -> ResolvedInputs {
    let mut prerequisites = BTreeSet::new();
    let mut pending = dispatch.action.dependencies.clone();
    while let Some(dependency) = pending.pop() {
        if prerequisites.insert(dependency.clone()) {
            pending.extend(
                plan.graph()
                    .action(&dependency)
                    .expect("dependency exists")
                    .dependencies
                    .iter()
                    .cloned(),
            );
        }
    }
    let outputs = prerequisites
        .into_iter()
        .map(|dependency| {
            let outputs = scheduler
                .completed_outputs(&dependency)
                .expect("ready action has successful dependency outputs")
                .to_vec();
            (dependency, outputs)
        })
        .collect();
    ResolvedInputs { outputs }
}

fn execute_batch<W: PlannedWork + Sync>(
    plan: &PreparedPlan<W>,
    executor: &dyn WorkExecutor<W>,
    cancellation: CancellationToken,
    batch: Vec<DispatchTask>,
) -> Result<Vec<CompletedDispatch>, ManagerError> {
    let count = batch.len();
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for task in batch {
            let sender = sender.clone();
            let cancellation = cancellation.clone();
            scope.spawn(move || {
                let panic_dispatch = task.dispatch.clone();
                let completed = catch_unwind(AssertUnwindSafe(|| {
                    execute_dispatch(plan, executor, cancellation, task)
                }))
                .unwrap_or_else(|_| CompletedDispatch {
                    timing: Timing::default(),
                    dispatch: panic_dispatch,
                    outcome: DispatchOutcome::Worker(ActionResult::failure(
                        "manager_worker_panicked",
                        "worker terminated unexpectedly",
                    )),
                });
                let _ = sender.send(completed);
            });
        }
        drop(sender);
        (0..count)
            .map(|_| {
                receiver
                    .recv()
                    .map_err(|error| orchestration(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .map(|mut completed| {
        completed.sort_by(|left, right| left.dispatch.action.id.cmp(&right.dispatch.action.id));
        completed
    })
}

fn execute_dispatch<W: PlannedWork + Sync>(
    plan: &PreparedPlan<W>,
    executor: &dyn WorkExecutor<W>,
    cancellation: CancellationToken,
    task: DispatchTask,
) -> CompletedDispatch {
    let DispatchTask { dispatch, inputs } = task;
    let work = plan
        .work(&dispatch.action.id)
        .expect("PreparedPlan guarantees graph/work correspondence");
    let started = Instant::now();
    let lookup_error = if work.effect() == Effect::Transform {
        match executor.lookup(&dispatch, work, plan, &inputs) {
            Ok(Some(hit)) => {
                return CompletedDispatch {
                    dispatch,
                    outcome: DispatchOutcome::Cached(hit),
                    timing: elapsed(started),
                };
            }
            Ok(None) => None,
            Err(error) => Some(error),
        }
    } else {
        None
    };
    let disposition = executor.execute(&dispatch, work, plan, &inputs, cancellation);
    let WorkDisposition::Complete(mut result) = disposition else {
        let WorkDisposition::Superseded { reason, events } = disposition else {
            unreachable!()
        };
        return CompletedDispatch {
            dispatch,
            outcome: DispatchOutcome::Superseded { reason, events },
            timing: elapsed(started),
        };
    };
    if let Some(error) = lookup_error {
        result.events.push(ActionEvent::Message {
            code: error.code().to_owned(),
            message: error.message().to_owned(),
        });
    }
    if result.outcome.is_ok()
        && work.effect() == Effect::Transform
        && let Err(error) = executor.record(&dispatch, work, &result)
    {
        result.events.push(ActionEvent::Message {
            code: error.code().to_owned(),
            message: error.message().to_owned(),
        });
    }
    CompletedDispatch {
        dispatch,
        outcome: DispatchOutcome::Worker(result),
        timing: elapsed(started),
    }
}

fn complete_batch(
    scheduler: &mut Scheduler,
    mapping: &mut EventMapping,
    context: &InvocationContext,
    completed: Vec<CompletedDispatch>,
) -> Result<Option<SupersedeReason>, ManagerError> {
    let mut superseded = None;
    for completed in completed {
        let action = completed.dispatch.action.id;
        mapping.timing.insert(action.clone(), completed.timing);
        match completed.outcome {
            DispatchOutcome::Cached(hit) => {
                mapping.cache.insert(action.clone(), hit.clone());
                scheduler
                    .complete_cached(&action, hit.outputs)
                    .map_err(|error| orchestration(error.to_string()))?;
            }
            DispatchOutcome::Worker(result) => scheduler
                .complete(&action, result)
                .map_err(|error| orchestration(error.to_string()))?,
            DispatchOutcome::Superseded { reason, events } => {
                mapping.superseded_actions.insert(action.clone());
                for event in events {
                    mapping.publish(
                        vec![ScheduleEvent::Worker {
                            action: action.clone(),
                            event,
                        }],
                        context,
                    )?;
                }
                emit(
                    context,
                    EventPayload::ActionSuperseded {
                        job: mapping.job.clone(),
                        plan: mapping.plan.clone(),
                        action,
                        timing: completed.timing,
                        reason,
                    },
                )?;
                superseded = Some(reason);
                continue;
            }
        }
        mapping.publish(scheduler.drain_events(), context)?;
    }
    Ok(superseded)
}

fn capacity(graph: &squish_build::BuildPlan, jobs: usize) -> Resources {
    let one = graph
        .actions()
        .fold(Resources::default(), |capacity, action| {
            Resources::new(
                capacity.cpu.max(action.resources.cpu),
                capacity.io.max(action.resources.io),
                capacity.memory.max(action.resources.memory),
            )
        });
    let jobs = u32::try_from(jobs.max(1)).unwrap_or(u32::MAX);
    Resources::new(
        one.cpu.saturating_mul(jobs),
        one.io.saturating_mul(jobs),
        one.memory.saturating_mul(u64::from(jobs)),
    )
}

fn elapsed(started: Instant) -> Timing {
    Timing {
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

struct EventMapping {
    job: JobId,
    plan: PlanId,
    timing: BTreeMap<ActionId, Timing>,
    artifacts: BTreeMap<ActionId, Vec<Artifact>>,
    cache: BTreeMap<ActionId, CachedResult>,
    totals: ActionTotals,
    root_failures: u64,
    joined: BTreeSet<ActionId>,
    superseded_actions: BTreeSet<ActionId>,
    sources: BTreeMap<ActionId, ResultSource>,
    messages: u64,
}

impl EventMapping {
    fn new(job: JobId, plan: PlanId) -> Self {
        Self {
            job,
            plan,
            timing: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            cache: BTreeMap::new(),
            totals: ActionTotals::default(),
            root_failures: 0,
            joined: BTreeSet::new(),
            superseded_actions: BTreeSet::new(),
            sources: BTreeMap::new(),
            messages: 0,
        }
    }

    fn publish(
        &mut self,
        events: Vec<ScheduleEvent>,
        context: &InvocationContext,
    ) -> Result<(), ManagerError> {
        for event in events {
            if let Some(payload) = self.map(event) {
                emit(context, payload)?;
            }
        }
        Ok(())
    }

    fn map(&mut self, event: ScheduleEvent) -> Option<EventPayload> {
        match event {
            ScheduleEvent::StateChanged { action, state } => self.state(action, state),
            ScheduleEvent::Worker { action, event } => self.worker(action, event),
            ScheduleEvent::ResultAvailable {
                action,
                key,
                source,
                ..
            } => self.result(action, key, source),
            ScheduleEvent::SingleFlight { action, .. } => {
                self.joined.insert(action);
                None
            }
            _ => None,
        }
    }

    fn state(&mut self, action: ActionId, state: ActionState) -> Option<EventPayload> {
        let timing = self.timing.get(&action).copied().unwrap_or_default();
        match state {
            ActionState::Running => Some(EventPayload::ActionStarted {
                job: self.job.clone(),
                plan: self.plan.clone(),
                action,
            }),
            ActionState::Succeeded => {
                self.totals.succeeded += 1;
                Some(EventPayload::ActionSucceeded {
                    job: self.job.clone(),
                    plan: self.plan.clone(),
                    artifacts: self.artifacts.remove(&action).unwrap_or_default(),
                    action,
                    timing,
                })
            }
            ActionState::Failed(failure) => {
                self.totals.failed += 1;
                if !self.joined.contains(&action) {
                    self.root_failures += 1;
                }
                let diagnostic = failure_diagnostic(&action, failure.code, failure.message);
                Some(EventPayload::ActionFailed {
                    job: self.job.clone(),
                    plan: self.plan.clone(),
                    action,
                    timing,
                    diagnostic,
                })
            }
            ActionState::Blocked { dependency } => {
                self.totals.blocked += 1;
                Some(EventPayload::ActionBlocked {
                    job: self.job.clone(),
                    plan: self.plan.clone(),
                    action,
                    blocked_by: vec![dependency],
                })
            }
            ActionState::Cancelled => {
                self.totals.cancelled += 1;
                Some(EventPayload::ActionCancelled {
                    job: self.job.clone(),
                    plan: self.plan.clone(),
                    action,
                    timing,
                })
            }
            ActionState::Pending | ActionState::Ready => None,
        }
    }

    fn worker(&mut self, action: ActionId, event: ActionEvent) -> Option<EventPayload> {
        match event {
            ActionEvent::Diagnostic(diagnostic) => Some(EventPayload::Diagnostic(diagnostic)),
            ActionEvent::Artifact(artifact) => {
                self.artifacts.entry(action).or_default().push(artifact);
                None
            }
            ActionEvent::Message { code, message } => {
                self.messages += 1;
                Some(EventPayload::Diagnostic(Diagnostic {
                    id: DiagnosticId::new(format!("manager-message-{}", self.messages))
                        .expect("non-empty"),
                    code,
                    severity: Severity::Warning,
                    phase: Phase::Orchestrate,
                    message,
                    primary: None,
                    related: Vec::new(),
                    help: None,
                }))
            }
            _ => None,
        }
    }

    fn result(
        &mut self,
        action: ActionId,
        key: ActionKey,
        source: ResultSource,
    ) -> Option<EventPayload> {
        self.sources.insert(action.clone(), source);
        if source != ResultSource::Cache {
            return None;
        }
        let hit = self.cache.remove(&action)?;
        self.artifacts.insert(action.clone(), hit.artifacts.clone());
        Some(EventPayload::CacheHit {
            job: self.job.clone(),
            plan: self.plan.clone(),
            action,
            cache: hit.cache,
            digest: hit.digest,
            action_key: Some(ActionKeyId::new(key.as_str()).expect("scheduler keys are non-empty")),
            outputs: hit.artifacts,
        })
    }

    fn report(
        &self,
        cancelled: bool,
        scheduler: &Scheduler,
        graph: &BuildPlan,
        plan_superseded: bool,
    ) -> ExecutionReport {
        ExecutionReport {
            totals: self.totals,
            root_failures: self.root_failures,
            cancelled,
            superseded: None,
            actions: execution_facts(self, scheduler, graph, plan_superseded),
        }
    }
}

fn execution_facts(
    mapping: &EventMapping,
    scheduler: &Scheduler,
    graph: &BuildPlan,
    plan_superseded: bool,
) -> Vec<ActionExecutionFact> {
    graph
        .actions()
        .map(|action| {
            let state = scheduler
                .state(&action.id)
                .expect("graph action has scheduler state");
            let state = execution_state(
                state,
                plan_superseded,
                mapping.superseded_actions.contains(&action.id),
            );
            ActionExecutionFact {
                action: action.id.clone(),
                kind: action.kind,
                dependencies: canonical_dependencies(action),
                state,
                key: scheduler.final_key(&action.id).map(|key| {
                    ActionKeyId::new(key.as_str()).expect("scheduler keys are non-empty")
                }),
                outputs: scheduler
                    .completed_outputs(&action.id)
                    .map(<[ProducedOutput]>::to_vec)
                    .unwrap_or_default(),
                source: mapping.sources.get(&action.id).copied(),
            }
        })
        .collect()
}

fn execution_state(
    state: &ActionState,
    plan_superseded: bool,
    explicitly_superseded: bool,
) -> ExecutionState {
    match state {
        ActionState::Succeeded => ExecutionState::Succeeded,
        ActionState::Failed(failure) => ExecutionState::Failed {
            code: failure.code.clone(),
            message: failure.message.clone(),
        },
        ActionState::Blocked { dependency } => ExecutionState::Blocked {
            dependency: dependency.clone(),
        },
        ActionState::Cancelled => ExecutionState::Cancelled,
        ActionState::Pending | ActionState::Ready | ActionState::Running
            if plan_superseded || explicitly_superseded =>
        {
            ExecutionState::Superseded
        }
        ActionState::Pending | ActionState::Ready | ActionState::Running => {
            ExecutionState::Declared
        }
    }
}

fn failure_diagnostic(action: &ActionId, code: String, message: String) -> Diagnostic {
    Diagnostic {
        id: DiagnosticId::new(format!("action-{action}-failure")).expect("non-empty"),
        code,
        severity: Severity::Error,
        phase: Phase::Orchestrate,
        message,
        primary: None,
        related: Vec::new(),
        help: None,
    }
}

fn emit(context: &InvocationContext, payload: EventPayload) -> Result<(), ManagerError> {
    context
        .emit(payload)
        .map_err(|error| orchestration(error.to_string()))
}

fn orchestration(message: impl Into<String>) -> ManagerError {
    ManagerError::new("manager_orchestration", Phase::Orchestrate, message)
}
