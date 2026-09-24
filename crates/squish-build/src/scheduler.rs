use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    Action, ActionEvent, ActionId, ActionKey, ActionResult, BuildPlan, ProducedOutput, Resources,
    WorkerFailure,
};

/// 动作的调度生命周期。 / Scheduling lifecycle of an action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionState {
    /// 尚有未完成前驱。 / Has unfinished predecessors.
    Pending,
    /// 前驱成功并等待资源。 / Predecessors succeeded; waiting for resources.
    Ready,
    /// 正在执行，或已加入相同键的执行。 / Executing or joined to execution of the same key.
    Running,
    /// 成功完成。 / Completed successfully.
    Succeeded,
    /// worker 报告失败。 / Worker reported failure.
    Failed(WorkerFailure),
    /// 因一个直接依赖失败或受阻而无法执行。 / Prevented by a failed or blocked direct dependency.
    Blocked {
        /// 阻止此动作的直接依赖；追踪其状态可还原完整失败链。 / Direct dependency preventing this action; follow its state to reconstruct the failure chain.
        dependency: ActionId,
    },
    /// 在执行前或协作式停止后被取消。 / Cancelled before execution or after cooperative shutdown.
    Cancelled,
}

/// 调度器产生、由宿主展示或记录的事件。 / Event produced by the scheduler for host rendering or recording.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ScheduleEvent {
    /// 生命周期发生改变。 / Lifecycle state changed.
    StateChanged {
        /// 改变状态的动作。 / Action whose state changed.
        action: ActionId,
        /// 新状态。 / New state.
        state: ActionState,
    },
    /// 动作加入同键 leader。 / An action joined a same-key leader.
    SingleFlight {
        /// 加入者。 / Joining action.
        action: ActionId,
        /// 实际执行者。 / Executing leader.
        leader: ActionId,
        /// 共享最终键。 / Shared final key.
        key: ActionKey,
    },
    /// worker 事件被保留。 / A worker event was retained.
    Worker {
        /// 产生事件的动作。 / Action producing the event.
        action: ActionId,
        /// 不带展示副作用的负载。 / Payload without presentation side effects.
        event: ActionEvent,
    },
    /// 成功结果可供每个逻辑动作发布或继续构建。 / A successful result is available for each logical action to publish or consume.
    ResultAvailable {
        /// 逻辑动作。 / Logical action.
        action: ActionId,
        /// 最终动作键。 / Final action key.
        key: ActionKey,
        /// 按声明顺序排列的产物摘要。 / Artifact digests in declaration order.
        outputs: Vec<ProducedOutput>,
        /// 结果来源。 / Result source.
        source: ResultSource,
    },
}

/// 成功动作结果的来源。 / Source of a successful action result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultSource {
    /// worker 执行。 / Worker execution.
    Worker,
    /// 同次运行的 single-flight leader。 / Same-run single-flight leader.
    SingleFlight,
    /// 此调度器运行内先前完成的键。 / Key completed earlier in this scheduler run.
    Cache,
}

/// 宿主应执行的一个动作。 / One action the host should execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dispatch {
    /// leader 动作的完整输入。 / Complete leader action input.
    pub action: Action,
    /// 前驱产物物化后的最终缓存键。 / Final cache key after predecessor artifacts materialize.
    pub key: ActionKey,
}

/// 调度器配置无法容纳计划。 / Scheduler configuration cannot accommodate the plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    /// 动作需求超过总配额，永远不可运行。 / An action exceeds total quotas and can never run.
    Unschedulable {
        /// 无法调度的动作。 / Unschedulable action.
        action: ActionId,
        /// 动作需求。 / Action demand.
        demand: Resources,
        /// 调度器总配额。 / Scheduler capacity.
        capacity: Resources,
    },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unschedulable { action, .. } => {
                write!(formatter, "action `{action}` exceeds scheduler capacity")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

/// 完成通知不符合当前运行状态。 / A completion notification contradicted current running state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionError {
    /// ID 不在计划中。 / ID is absent from the plan.
    UnknownAction(ActionId),
    /// ID 不是正在执行的 single-flight leader。 / ID is not the executing single-flight leader.
    NotLeader(ActionId),
    /// 成功结果与声明的具名输出模式不同。 / Successful result differs from the declared named-output schema.
    OutputSchema {
        /// 返回非法模式的动作。 / Action returning the invalid schema.
        action: ActionId,
    },
}

impl fmt::Display for CompletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAction(id) => write!(formatter, "unknown action `{id}`"),
            Self::NotLeader(id) => write!(formatter, "action `{id}` is not a running leader"),
            Self::OutputSchema { action } => write!(
                formatter,
                "action `{action}` returned outputs that differ from its declared schema"
            ),
        }
    }
}

impl std::error::Error for CompletionError {}

#[derive(Clone, Debug)]
struct Flight {
    leader: ActionId,
    members: BTreeSet<ActionId>,
}

#[derive(Clone, Debug)]
enum CachedOutcome {
    Success(Vec<ProducedOutput>),
    Failed(WorkerFailure),
}

/// 确定性、有界、同步的 DAG 调度核心。 / Deterministic, bounded, synchronous DAG scheduling core.
///
/// `next_dispatch` 总是选择当前能放入配额的最小 ID。并发由宿主决定：反复调用可取得
/// 多个 dispatch，而 `complete` 释放配额。 / `next_dispatch` always chooses the least ID
/// currently fitting the quotas. The host controls concurrency: repeated calls
/// obtain multiple dispatches, while `complete` releases quotas.
pub struct Scheduler {
    plan: BuildPlan,
    capacity: Resources,
    available: Resources,
    keep_going: bool,
    states: BTreeMap<ActionId, ActionState>,
    ready: BTreeSet<ActionId>,
    flights: BTreeMap<ActionKey, Flight>,
    leaders: BTreeMap<ActionId, ActionKey>,
    cache: BTreeMap<ActionKey, CachedOutcome>,
    completed_outputs: BTreeMap<ActionId, Vec<ProducedOutput>>,
    final_keys: BTreeMap<ActionId, ActionKey>,
    events: Vec<ScheduleEvent>,
    cancellation_requested: bool,
}

impl Scheduler {
    /// 创建调度器，并拒绝任何永远不能满足的资源需求。 / Creates a scheduler and rejects demands that can never fit.
    pub fn new(
        plan: BuildPlan,
        capacity: Resources,
        keep_going: bool,
    ) -> Result<Self, SchedulerError> {
        for action in plan.actions() {
            if !action.resources.fits(capacity) {
                return Err(SchedulerError::Unschedulable {
                    action: action.id.clone(),
                    demand: action.resources,
                    capacity,
                });
            }
        }
        let mut scheduler = Self {
            states: plan
                .actions()
                .map(|action| (action.id.clone(), ActionState::Pending))
                .collect(),
            plan,
            capacity,
            available: capacity,
            keep_going,
            ready: BTreeSet::new(),
            flights: BTreeMap::new(),
            leaders: BTreeMap::new(),
            cache: BTreeMap::new(),
            completed_outputs: BTreeMap::new(),
            final_keys: BTreeMap::new(),
            events: Vec::new(),
            cancellation_requested: false,
        };
        let roots: Vec<_> = scheduler
            .plan
            .actions()
            .filter(|action| action.dependencies.is_empty())
            .map(|action| action.id.clone())
            .collect();
        for id in roots {
            scheduler.make_ready(id);
        }
        Ok(scheduler)
    }

    /// 返回动作状态。 / Returns an action state.
    pub fn state(&self, id: &ActionId) -> Option<&ActionState> {
        self.states.get(id)
    }

    /// 返回总资源配额。 / Returns total resource quotas.
    pub const fn capacity(&self) -> Resources {
        self.capacity
    }

    /// 返回当前可用资源。 / Returns currently available resources.
    pub const fn available(&self) -> Resources {
        self.available
    }

    /// 返回成功动作的有序输出摘要。 / Returns ordered output digests of a successful action.
    pub fn completed_outputs(&self, id: &ActionId) -> Option<&[ProducedOutput]> {
        self.completed_outputs.get(id).map(Vec::as_slice)
    }

    /// 返回已经物化的最终动作键。 / Returns a final action key once materialized.
    pub fn final_key(&self, id: &ActionId) -> Option<&ActionKey> {
        self.final_keys.get(id)
    }

    /// 取得下一项确定性工作；`None` 表示当前没有可派发项。 / Takes the next deterministic work item; `None` means none is dispatchable now.
    pub fn next_dispatch(&mut self) -> Option<Dispatch> {
        if self.cancellation_requested {
            return None;
        }
        loop {
            let ids: Vec<_> = self.ready.iter().cloned().collect();
            let mut resolved = false;
            for id in ids {
                if self.resolve_single_flight(&id) {
                    self.ready.remove(&id);
                    resolved = true;
                    break;
                }
                let demand = self.plan.action(&id).expect("ready ID exists").resources;
                if demand.fits(self.available) {
                    self.ready.remove(&id);
                    return Some(self.start(id));
                }
            }
            if !resolved {
                return None;
            }
        }
    }

    /// 完成一个 leader，并将结果传播给已加入的同键动作。 / Completes a leader and propagates its result to joined same-key actions.
    pub fn complete(
        &mut self,
        leader: &ActionId,
        result: ActionResult,
    ) -> Result<(), CompletionError> {
        self.complete_with_source(leader, result, ResultSource::Worker)
    }

    /// 以持久缓存命中完成 leader；宿主应先用 dispatch key 查询 [`crate::ActionIndex`]。 / Completes a leader from a persistent cache hit; the host should query [`crate::ActionIndex`] with the dispatch key first.
    pub fn complete_cached(
        &mut self,
        leader: &ActionId,
        outputs: Vec<ProducedOutput>,
    ) -> Result<(), CompletionError> {
        self.complete_with_source(
            leader,
            ActionResult {
                outcome: Ok(()),
                outputs,
                events: Vec::new(),
            },
            ResultSource::Cache,
        )
    }

    /// 请求协作式取消并返回仍需宿主停止的 worker leader。 / Requests cooperative cancellation and returns worker leaders the host must still stop.
    ///
    /// 尚未派发的动作立即进入 [`ActionState::Cancelled`]，此后不再产生新 dispatch。
    /// 已运行的 single-flight 组保持 `Running`，直到宿主停止对应 worker 并调用
    /// [`Scheduler::complete_cancelled`]；因此资源不会在 worker 真正停止前被错误复用。
    /// Actions not yet dispatched become [`ActionState::Cancelled`] immediately and no new
    /// dispatches are produced. Running single-flight groups remain `Running` until the host
    /// stops each corresponding worker and calls [`Scheduler::complete_cancelled`], preventing
    /// resources from being reused while work is still active.
    pub fn request_cancellation(&mut self) -> Vec<ActionId> {
        self.cancellation_requested = true;
        self.cancel_queued();
        self.leaders.keys().cloned().collect()
    }

    /// 确认一个 worker leader 已协作式停止，并取消整个 single-flight 组。 / Confirms a worker leader stopped cooperatively and cancels its whole single-flight group.
    ///
    /// 只有 [`Scheduler::request_cancellation`] 之后才能确认取消。资源在这里释放，而非在
    /// 请求时释放。 / Cancellation can only be acknowledged after
    /// [`Scheduler::request_cancellation`]. Resources are released here, not at request time.
    pub fn complete_cancelled(&mut self, leader: &ActionId) -> Result<(), CompletionError> {
        if !self.states.contains_key(leader) {
            return Err(CompletionError::UnknownAction(leader.clone()));
        }
        if !self.cancellation_requested {
            return Err(CompletionError::NotLeader(leader.clone()));
        }
        let flight = self.retire_flight(leader)?;
        for member in flight.members {
            self.change(member, ActionState::Cancelled);
        }
        Ok(())
    }

    /// 是否已经请求取消。 / Whether cancellation has been requested.
    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }

    fn complete_with_source(
        &mut self,
        leader: &ActionId,
        result: ActionResult,
        leader_source: ResultSource,
    ) -> Result<(), CompletionError> {
        if !self.states.contains_key(leader) {
            return Err(CompletionError::UnknownAction(leader.clone()));
        }
        let key = self
            .leaders
            .get(leader)
            .cloned()
            .ok_or_else(|| CompletionError::NotLeader(leader.clone()))?;
        let action = self.plan.action(leader).expect("leader exists");
        let schema_matches = action.outputs.len() == result.outputs.len()
            && action
                .outputs
                .iter()
                .zip(&result.outputs)
                .all(|(declared, actual)| {
                    declared.name == actual.name && declared.kind == actual.kind
                });
        if result.outcome.is_ok() && !schema_matches {
            return Err(CompletionError::OutputSchema {
                action: leader.clone(),
            });
        }
        let flight = self.retire_flight(leader)?;
        for event in result.events {
            self.events.push(ScheduleEvent::Worker {
                action: leader.clone(),
                event,
            });
        }
        let cached = match result.outcome {
            Ok(()) => CachedOutcome::Success(result.outputs),
            Err(error) => CachedOutcome::Failed(error),
        };
        self.cache.insert(key.clone(), cached.clone());
        for member in flight.members {
            let source = if &member == leader {
                leader_source
            } else {
                ResultSource::SingleFlight
            };
            self.finish(member, key.clone(), cached.clone(), source);
        }
        Ok(())
    }

    /// 在结果验证后一次性退役 leader，并归还其资源。 / Retires a validated leader and returns its resources exactly once.
    fn retire_flight(&mut self, leader: &ActionId) -> Result<Flight, CompletionError> {
        let key = self
            .leaders
            .remove(leader)
            .ok_or_else(|| CompletionError::NotLeader(leader.clone()))?;
        let flight = self.flights.remove(&key).expect("leader has a flight");
        let demand = self.plan.action(leader).expect("leader exists").resources;
        self.available = self
            .available
            .checked_add(demand)
            .expect("released resources fit integer domains");
        Ok(flight)
    }

    /// 排空结构化事件。 / Drains structured events.
    pub fn drain_events(&mut self) -> Vec<ScheduleEvent> {
        std::mem::take(&mut self.events)
    }

    /// 所有动作是否已进入终态。 / Whether every action reached a terminal state.
    pub fn is_finished(&self) -> bool {
        self.states.values().all(is_terminal)
    }

    fn resolve_single_flight(&mut self, id: &ActionId) -> bool {
        let key = self.materialized_key(id);
        self.final_keys.insert(id.clone(), key.clone());
        if let Some(outcome) = self.cache.get(&key).cloned() {
            self.finish(id.clone(), key, outcome, ResultSource::Cache);
            return true;
        }
        if let Some(flight) = self.flights.get_mut(&key) {
            let leader = flight.leader.clone();
            flight.members.insert(id.clone());
            self.change(id.clone(), ActionState::Running);
            self.events.push(ScheduleEvent::SingleFlight {
                action: id.clone(),
                leader,
                key,
            });
            return true;
        }
        false
    }

    fn start(&mut self, id: ActionId) -> Dispatch {
        let action = self.plan.action(&id).expect("ready action exists").clone();
        let key = self.materialized_key(&id);
        self.final_keys.insert(id.clone(), key.clone());
        self.available = self.available.subtract(action.resources);
        let mut members = BTreeSet::new();
        members.insert(id.clone());
        self.flights.insert(
            key.clone(),
            Flight {
                leader: id.clone(),
                members,
            },
        );
        self.leaders.insert(id.clone(), key.clone());
        self.change(id, ActionState::Running);
        Dispatch { action, key }
    }

    fn finish(
        &mut self,
        id: ActionId,
        key: ActionKey,
        outcome: CachedOutcome,
        source: ResultSource,
    ) {
        match outcome {
            CachedOutcome::Success(outputs) => {
                self.completed_outputs.insert(id.clone(), outputs.clone());
                self.events.push(ScheduleEvent::ResultAvailable {
                    action: id.clone(),
                    key,
                    outputs,
                    source,
                });
                self.change(id.clone(), ActionState::Succeeded);
                self.advance_dependents(&id);
            }
            CachedOutcome::Failed(error) => {
                self.change(id.clone(), ActionState::Failed(error));
                self.block_dependents(&id);
                if !self.keep_going {
                    self.cancel_queued();
                }
            }
        }
    }

    fn advance_dependents(&mut self, id: &ActionId) {
        let dependents = self.plan.dependents(id).to_vec();
        for dependent in dependents {
            let action = self.plan.action(&dependent).expect("dependent exists");
            if matches!(self.states[&dependent], ActionState::Pending)
                && action
                    .dependencies
                    .iter()
                    .all(|dep| matches!(self.states[dep], ActionState::Succeeded))
            {
                self.make_ready(dependent);
            }
        }
    }

    fn block_dependents(&mut self, dependency: &ActionId) {
        let dependents = self.plan.dependents(dependency).to_vec();
        for dependent in dependents {
            if is_terminal(&self.states[&dependent]) {
                continue;
            }
            self.ready.remove(&dependent);
            self.change(
                dependent.clone(),
                ActionState::Blocked {
                    dependency: dependency.clone(),
                },
            );
            self.block_dependents(&dependent);
        }
    }

    fn cancel_queued(&mut self) {
        let ids: Vec<_> = self
            .states
            .iter()
            .filter(|(_, state)| matches!(state, ActionState::Pending | ActionState::Ready))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.ready.remove(&id);
            self.change(id, ActionState::Cancelled);
        }
    }

    fn make_ready(&mut self, id: ActionId) {
        self.ready.insert(id.clone());
        self.change(id, ActionState::Ready);
    }

    fn change(&mut self, id: ActionId, state: ActionState) {
        self.states.insert(id.clone(), state.clone());
        self.events
            .push(ScheduleEvent::StateChanged { action: id, state });
    }

    fn materialized_key(&self, id: &ActionId) -> ActionKey {
        let action = self.plan.action(id).expect("scheduled action exists");
        action
            .key
            .materialize(&action.kind, &action.outputs, |reference| {
                self.completed_outputs
                    .get(&reference.action)?
                    .iter()
                    .find(|output| output.name == reference.output)
                    .map(|output| output.digest.clone())
            })
            .expect("ready action references validated successful outputs")
    }
}

fn is_terminal(state: &ActionState) -> bool {
    matches!(
        state,
        ActionState::Succeeded
            | ActionState::Failed(_)
            | ActionState::Blocked { .. }
            | ActionState::Cancelled
    )
}
