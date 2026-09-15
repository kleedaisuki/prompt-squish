use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use squish_build::{ActionKind, BuildPlan};
use squish_protocol::{ActionId, ActionKeyId};

/// 产物路径定位器为空。 / Artifact path locator is empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidArtifactLocator;

impl fmt::Display for InvalidArtifactLocator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("artifact path must not be empty")
    }
}

impl std::error::Error for InvalidArtifactLocator {}

/// 由组合根验证、供查询使用的产物路径。 / Composition-root-validated artifact path used for inspection.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactLocator(PathBuf);

impl ArtifactLocator {
    /// 创建非空路径定位器。 / Creates a non-empty path locator.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, InvalidArtifactLocator> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            Err(InvalidArtifactLocator)
        } else {
            Ok(Self(path))
        }
    }

    /// 返回宿主路径。 / Returns the host path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// 协议视图之外的类型化查询对象。 / Typed inspection subject supplementing the protocol view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectSubject {
    /// 完整动作缓存键。 / Complete action-cache key.
    CacheKey(ActionKeyId),
    /// 用户指定的产物路径。 / User-selected artifact path.
    ArtifactPath(ArtifactLocator),
}

/// 动作对外部世界的影响类别。 / Kind of effect an action has on the external world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// 可持久缓存的纯变换。 / Pure transform eligible for persistent caching.
    Transform,
    /// 读取外部状态。 / Reads external state.
    ReadEffect,
    /// 改变外部状态。 / Changes external state.
    WriteEffect,
    /// 协调锁、事务或租约。 / Coordinates locks, transactions, or leases.
    Coordination,
}

impl Effect {
    /// 仅纯变换可进入持久动作缓存。 / Only pure transforms may enter the persistent action cache.
    pub const fn cacheable(self) -> bool {
        matches!(self, Self::Transform)
    }
}

/// 可与调度图动作一一对应的类型化工作。 / Typed work corresponding one-to-one with a scheduled action.
pub trait PlannedWork {
    /// 返回协议动作类别。 / Returns the protocol action kind.
    fn kind(&self) -> ActionKind;

    /// 返回外部副作用类别。 / Returns the external-effect class.
    fn effect(&self) -> Effect;
}

/// 图与领域工作的映射无效。 / Invalid mapping between the graph and domain work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedPlanError {
    /// 图动作缺少工作。 / A graph action has no work.
    MissingWork(ActionId),
    /// 工作没有对应的图动作。 / Work has no graph action.
    ExtraWork(ActionId),
    /// 同一 ID 的图与工作类别不一致。 / Graph and work kinds disagree for one ID.
    KindMismatch {
        /// 动作 ID。 / Action ID.
        action: ActionId,
        /// 图声明的类别。 / Kind declared by the graph.
        graph: ActionKind,
        /// 工作声明的类别。 / Kind declared by the work.
        work: ActionKind,
    },
}

impl fmt::Display for PreparedPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWork(id) => write!(formatter, "action `{id}` has no work"),
            Self::ExtraWork(id) => write!(formatter, "work `{id}` has no graph action"),
            Self::KindMismatch {
                action,
                graph,
                work,
            } => write!(
                formatter,
                "action `{action}` has graph kind `{graph:?}` but work kind `{work:?}`"
            ),
        }
    }
}

impl std::error::Error for PreparedPlanError {}

/// 已验证的不可变图及其领域工作。 / Validated immutable graph and its domain work.
#[derive(Clone, Debug)]
pub struct PreparedPlan<W> {
    graph: BuildPlan,
    work: BTreeMap<ActionId, W>,
}

impl<W: PlannedWork> PreparedPlan<W> {
    /// 验证 ID 集合和动作类别严格一一对应后创建计划。 / Creates a plan after validating exact ID and kind correspondence.
    pub fn new(graph: BuildPlan, work: BTreeMap<ActionId, W>) -> Result<Self, PreparedPlanError> {
        validate_work(&graph, &work)?;
        Ok(Self { graph, work })
    }

    /// 返回调度图。 / Returns the scheduling graph.
    pub const fn graph(&self) -> &BuildPlan {
        &self.graph
    }

    /// 返回动作对应的工作。 / Returns work corresponding to an action.
    pub fn work(&self, action: &ActionId) -> Option<&W> {
        self.work.get(action)
    }

    /// 按动作 ID 的稳定顺序迭代工作。 / Iterates work in stable action-ID order.
    pub fn works(&self) -> impl ExactSizeIterator<Item = (&ActionId, &W)> {
        self.work.iter()
    }
}

fn validate_work<W: PlannedWork>(
    graph: &BuildPlan,
    work: &BTreeMap<ActionId, W>,
) -> Result<(), PreparedPlanError> {
    for action in graph.actions() {
        let item = work
            .get(&action.id)
            .ok_or_else(|| PreparedPlanError::MissingWork(action.id.clone()))?;
        if action.kind != item.kind() {
            return Err(PreparedPlanError::KindMismatch {
                action: action.id.clone(),
                graph: action.kind,
                work: item.kind(),
            });
        }
    }
    if let Some(id) = work.keys().find(|id| graph.action(id).is_none()) {
        return Err(PreparedPlanError::ExtraWork(id.clone()));
    }
    Ok(())
}
