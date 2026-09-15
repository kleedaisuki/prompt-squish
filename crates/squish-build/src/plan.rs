use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    Action, ActionId, InputRef, OutputName, ResourceClass,
    model::{hash_action_kind, hash_count, hash_field, hash_output_kind},
};

/// 封闭构建 DAG 的 BLAKE3 语义指纹。 / BLAKE3 semantic fingerprint of a sealed build DAG.
///
/// 该独立类型防止将图身份误作内容或已物化产物摘要。编码算法由
/// [`BuildPlan::semantic_digest`] 的契约固定。 / This distinct type prevents graph identity from
/// being confused with content or materialized-output digests. The encoding algorithm is fixed by
/// the contract of [`BuildPlan::semantic_digest`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SemanticGraphDigest([u8; 32]);

impl SemanticGraphDigest {
    /// 返回固定算法名。 / Returns the fixed algorithm name.
    pub const fn algorithm(&self) -> &'static str {
        "blake3"
    }

    /// 返回 32 字节规范摘要。 / Returns the canonical 32-byte digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 消耗值并返回 32 字节规范摘要。 / Consumes the value and returns the canonical 32-byte digest.
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for SemanticGraphDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("blake3:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// 构建计划不满足图或产物不变量。 / A build plan violated graph or artifact invariants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    /// 动作 ID 重复。 / An action ID was repeated.
    DuplicateId(ActionId),
    /// 动作引用不存在的前驱。 / An action referenced an absent predecessor.
    MissingDependency {
        /// 引用者。 / Referencing action.
        action: ActionId,
        /// 缺失前驱。 / Missing predecessor.
        dependency: ActionId,
    },
    /// 依赖图包含环；向量给出一个闭合环。 / Dependency graph contains a cycle; the vector is one closed cycle.
    Cycle(Vec<ActionId>),
    /// 一个动作重复声明同一具名输出。 / An action declared the same named output twice.
    OutputCollision {
        /// 声明者。 / Declaring action.
        action: ActionId,
        /// 重复输出名。 / Repeated output name.
        output: OutputName,
    },
    /// 键配方引用不存在或非直接依赖的输出。 / A key recipe references an absent or non-dependent output.
    InvalidInput {
        /// 引用者。 / Referencing action.
        action: ActionId,
        /// 预期生产者。 / Expected producer.
        producer: ActionId,
        /// 引用输出名。 / Referenced output name.
        output: OutputName,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(formatter, "duplicate action id `{id}`"),
            Self::MissingDependency { action, dependency } => write!(
                formatter,
                "action `{action}` depends on missing `{dependency}`"
            ),
            Self::Cycle(ids) => write!(
                formatter,
                "dependency cycle: {}",
                ids.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ),
            Self::OutputCollision { action, output } => write!(
                formatter,
                "action `{action}` declares output `{output}` twice"
            ),
            Self::InvalidInput {
                action,
                producer,
                output,
            } => write!(
                formatter,
                "action `{action}` references unavailable output `{producer}:{output}`"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// 已验证且构造后不可变的动作有向无环图。 / Validated, immutable action directed acyclic graph.
#[derive(Clone, Debug)]
pub struct BuildPlan {
    actions: BTreeMap<ActionId, Action>,
    dependents: BTreeMap<ActionId, Vec<ActionId>>,
}

impl BuildPlan {
    /// 验证 ID、边、环和输出所有权后创建计划。 / Creates a plan after validating IDs, edges, cycles, and output ownership.
    pub fn new(actions: impl IntoIterator<Item = Action>) -> Result<Self, PlanError> {
        let mut actions = collect_unique(actions)?;
        validate_edges(&actions)?;
        canonicalize_dependencies(&mut actions);
        validate_outputs(&actions)?;
        validate_inputs(&actions)?;
        detect_cycle(&actions)?;
        let dependents = reverse_edges(&actions);
        Ok(Self {
            actions,
            dependents,
        })
    }

    /// 按 ID 的确定顺序迭代动作。 / Iterates actions in deterministic ID order.
    pub fn actions(&self) -> impl ExactSizeIterator<Item = &Action> {
        self.actions.values()
    }

    /// 返回指定动作。 / Returns the selected action.
    pub fn action(&self, id: &ActionId) -> Option<&Action> {
        self.actions.get(id)
    }

    /// 计算封闭 DAG 的完整规范语义指纹。 / Computes the complete canonical semantic fingerprint of the sealed DAG.
    ///
    /// 不变量如下：动作按 ID 排序；依赖作为排序且去重的集合；键配方保留语义 epoch、有序类型化
    /// 输入（包括 [`crate::OutputRef`] 身份）及规范选项；动作种类、资源类别与向量、输出顺序/名称/
    /// 类别各编码一次。调用方必须另行组合冻结快照、规划问题和执行模式。物理路径、调度状态、
    /// 已解析/已生成输出摘要均不进入本指纹。 / Invariants: actions are ordered by ID;
    /// dependencies are a sorted, duplicate-free set; key recipes retain the semantic epoch,
    /// ordered typed inputs (including [`crate::OutputRef`] identity), and canonical options; action
    /// kind, resource class/vector, and ordered output names/kinds are each encoded exactly once.
    /// Callers must separately combine the frozen snapshot, planning issues, and execution mode.
    /// Physical paths, scheduler state, and resolved/produced output digests are excluded.
    ///
    /// 编码使用带域分隔的长度前缀 BLAKE3；变更此格式需要新的域版本。 / The encoding uses
    /// domain-separated, length-prefixed BLAKE3; changing it requires a new domain version.
    pub fn semantic_digest(&self) -> SemanticGraphDigest {
        let mut hash = blake3::Hasher::new();
        hash.update(b"prompt-squish-build-plan-semantic\0v1");
        hash.update(b"ordered-actions\0");
        hash_count(&mut hash, self.actions.len());
        for action in self.actions.values() {
            hash.update(b"action\0");
            hash_field(&mut hash, action.id.as_str().as_bytes());
            hash_action_kind(&mut hash, &action.kind);
            hash_resource_class(&mut hash, action.class);
            hash.update(b"resources\0");
            hash.update(&action.resources.cpu.to_le_bytes());
            hash.update(&action.resources.io.to_le_bytes());
            hash.update(&action.resources.memory.to_le_bytes());
            hash.update(b"dependencies\0");
            hash_count(&mut hash, action.dependencies.len());
            for dependency in &action.dependencies {
                hash_field(&mut hash, dependency.as_str().as_bytes());
            }
            action.key.hash_semantics(&mut hash);
            hash.update(b"output-schema\0");
            hash_count(&mut hash, action.outputs.len());
            for output in &action.outputs {
                hash_field(&mut hash, output.name.as_str().as_bytes());
                hash_output_kind(&mut hash, &output.kind);
            }
        }
        SemanticGraphDigest(*hash.finalize().as_bytes())
    }

    pub(crate) fn dependents(&self, id: &ActionId) -> &[ActionId] {
        self.dependents.get(id).map_or(&[], Vec::as_slice)
    }
}

fn canonicalize_dependencies(actions: &mut BTreeMap<ActionId, Action>) {
    for action in actions.values_mut() {
        action.dependencies.sort();
        action.dependencies.dedup();
    }
}

fn hash_resource_class(hash: &mut blake3::Hasher, class: ResourceClass) {
    hash.update(b"resource-class\0");
    hash.update(match class {
        ResourceClass::Cpu => b"cpu\0",
        ResourceClass::Io => b"io\0",
        ResourceClass::Memory => b"memory\0",
    });
}

fn collect_unique(
    actions: impl IntoIterator<Item = Action>,
) -> Result<BTreeMap<ActionId, Action>, PlanError> {
    let mut result = BTreeMap::new();
    for action in actions {
        let id = action.id.clone();
        if result.insert(id.clone(), action).is_some() {
            return Err(PlanError::DuplicateId(id));
        }
    }
    Ok(result)
}

fn validate_edges(actions: &BTreeMap<ActionId, Action>) -> Result<(), PlanError> {
    for action in actions.values() {
        for dependency in &action.dependencies {
            if !actions.contains_key(dependency) {
                return Err(PlanError::MissingDependency {
                    action: action.id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_outputs(actions: &BTreeMap<ActionId, Action>) -> Result<(), PlanError> {
    for action in actions.values() {
        let mut names = BTreeSet::new();
        for output in &action.outputs {
            if !names.insert(&output.name) {
                return Err(PlanError::OutputCollision {
                    action: action.id.clone(),
                    output: output.name.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_inputs(actions: &BTreeMap<ActionId, Action>) -> Result<(), PlanError> {
    for action in actions.values() {
        for input in action.key.inputs() {
            let InputRef::Output(reference) = input else {
                continue;
            };
            let valid_edge = action.dependencies.contains(&reference.action);
            let valid_output = actions.get(&reference.action).is_some_and(|producer| {
                producer
                    .outputs
                    .iter()
                    .any(|output| output.name == reference.output)
            });
            if !valid_edge || !valid_output {
                return Err(PlanError::InvalidInput {
                    action: action.id.clone(),
                    producer: reference.action.clone(),
                    output: reference.output.clone(),
                });
            }
        }
    }
    Ok(())
}

fn reverse_edges(actions: &BTreeMap<ActionId, Action>) -> BTreeMap<ActionId, Vec<ActionId>> {
    let mut result: BTreeMap<ActionId, Vec<ActionId>> =
        actions.keys().cloned().map(|id| (id, Vec::new())).collect();
    for action in actions.values() {
        for dependency in &action.dependencies {
            result
                .get_mut(dependency)
                .expect("dependencies were validated")
                .push(action.id.clone());
        }
    }
    result
}

fn detect_cycle(actions: &BTreeMap<ActionId, Action>) -> Result<(), PlanError> {
    let dependents = reverse_edges(actions);
    let mut indegree: BTreeMap<_, _> = actions
        .values()
        .map(|action| (action.id.clone(), action.dependencies.len()))
        .collect();
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect();
    while let Some(id) = ready.pop_first() {
        for child in &dependents[&id] {
            let degree = indegree.get_mut(child).expect("dependent exists");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(child.clone());
            }
        }
    }
    let cyclic: BTreeSet<_> = indegree
        .into_iter()
        .filter(|(_, degree)| *degree != 0)
        .map(|(id, _)| id)
        .collect();
    if cyclic.is_empty() {
        return Ok(());
    }
    let mut positions = BTreeMap::new();
    let mut path = Vec::new();
    let mut current = cyclic.first().expect("cyclic set is non-empty").clone();
    loop {
        if let Some(&start) = positions.get(&current) {
            let mut cycle = path[start..].to_vec();
            cycle.push(current);
            return Err(PlanError::Cycle(cycle));
        }
        positions.insert(current.clone(), path.len());
        path.push(current.clone());
        current = actions[&current]
            .dependencies
            .iter()
            .find(|dependency| cyclic.contains(*dependency))
            .expect("each remaining node reaches a remaining predecessor")
            .clone();
    }
}
