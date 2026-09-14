//! 依赖解析器端口；本 crate 不实现网络或缓存。 / Dependency-resolver port; this crate implements no network or cache.

use std::collections::BTreeMap;

use crate::{Lockfile, Manifest};

/// 解析时的外部状态策略。 / External-state policy during resolution.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResolutionMode {
    /// May refresh indexes/content and update the lock. / 可刷新索引/内容并更新锁。
    #[default]
    Online,
    /// No network, but locally available state may produce a new lock. / 无网络，但本地状态可产生新锁。
    Offline,
    /// Exact prior lock is required and cannot change. / 必须使用且不能修改原精确锁。
    Locked,
    /// Locked plus no network or mutable-source refresh. / Locked 且禁止网络和可变源刷新。
    Frozen,
}

/// 一次确定性解析的完整输入快照。 / Complete input snapshot for one deterministic resolution.
#[derive(Clone, Debug)]
pub struct ResolutionInput<'a> {
    /// Workspace-relative manifest path to validated intent. / 工作区相对清单路径到已验证意图。
    pub manifests: &'a BTreeMap<String, Manifest>,
    /// Digest of the normalized complete manifest set. / 归一化完整清单集摘要。
    pub manifest_digest: &'a str,
    /// Prior exact graph used for minimal-change preference. / 用于最小变更偏好的旧精确图。
    pub prior_lock: Option<&'a Lockfile>,
    /// Network/mutation policy. / 网络/变更策略。
    pub mode: ResolutionMode,
}

/// 将人工意图解析为精确图的微内核端口。 / Microkernel port resolving human intent into an exact graph.
///
/// Implementations may access registries, Git, or local indexes according to
/// [`ResolutionMode`]. They must not mutate manifests or locks; callers first
/// inspect the returned candidate and later commit it transactionally.
pub trait DependencyResolver {
    /// Adapter-specific error. / 适配器特定错误。
    type Error;

    /// Produces a validated candidate lock without authoritative writes. / 在不写权威状态时产生已验证候选锁。
    fn resolve(&self, input: ResolutionInput<'_>) -> Result<Lockfile, Self::Error>;
}
