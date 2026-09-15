use std::{collections::BTreeMap, fmt};

use squish_protocol::{ActionId, Artifact, ArtifactKind, Diagnostic, DigestAlgorithm};
pub use squish_protocol::{ActionKind, Digest as ContentDigest};

macro_rules! string_value {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);
        impl $name {
            /// 从非空文本构造值。 / Creates the value from non-empty text.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyValue> {
                let value = value.into();
                if value.is_empty() {
                    Err(EmptyValue)
                } else {
                    Ok(Self(value))
                }
            }
            /// 返回稳定文本表示。 / Returns the stable textual representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

/// 字符串值为空。 / A string value was empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyValue;
impl fmt::Display for EmptyValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("value must not be empty")
    }
}
impl std::error::Error for EmptyValue {}

string_value!(
    ActionKey,
    "动作全部语义输入的摘要键；相同键可 single-flight。 / Digest key of every semantic action input; equal keys may be single-flighted."
);
string_value!(
    OutputName,
    "动作内稳定且唯一的输出名称。 / Stable action-local output name."
);

/// 对另一个动作具名输出的引用。 / Reference to a named output of another action.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OutputRef {
    /// 生产动作。 / Producer action.
    pub action: ActionId,
    /// 动作局部输出名。 / Action-local output name.
    pub output: OutputName,
}

/// 键配方中的一个有序语义输入。 / One ordered semantic input in a key recipe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputRef {
    /// 已解析内容 blob。 / Resolved content blob.
    Blob(ContentDigest),
    /// 前驱的具名产物；只解析后的内容摘要进入最终键。 / Named predecessor artifact; only its resolved content digest enters the final key.
    Output(OutputRef),
}

/// 动作声明的纯输出模式，不含发布位置。 / Pure output schema declared by an action, excluding publication locations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Output {
    /// 稳定输出名。 / Stable output name.
    pub name: OutputName,
    /// 产物语义类别。 / Semantic artifact kind.
    pub kind: ArtifactKind,
}

/// 冻结于计划中的键配方。 / Key recipe frozen in a plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRecipe {
    semantic_epoch: String,
    inputs: Vec<InputRef>,
    options: BTreeMap<String, String>,
}

impl KeyRecipe {
    /// 创建语义 epoch、类型化输入及规范选项的配方。 / Creates a recipe from a semantic epoch, typed inputs, and canonical options.
    pub fn new(
        semantic_epoch: impl Into<String>,
        inputs: Vec<InputRef>,
        options: BTreeMap<String, String>,
    ) -> Result<Self, EmptyValue> {
        let semantic_epoch = semantic_epoch.into();
        if semantic_epoch.is_empty() {
            return Err(EmptyValue);
        }
        Ok(Self {
            semantic_epoch,
            inputs,
            options,
        })
    }

    /// 返回有序输入，供计划验证引用。 / Returns ordered inputs for plan reference validation.
    pub fn inputs(&self) -> &[InputRef] {
        &self.inputs
    }

    /// 将未物化的完整配方写入计划语义哈希。 / Writes the complete, unmaterialized recipe into a plan-semantic hash.
    ///
    /// 与 [`Self::materialize`] 不同，这会保留 [`OutputRef`] 的生产者和输出名身份；计划指纹不得
    /// 被运行时解析出的产物摘要替代。 / Unlike [`Self::materialize`], this preserves the producer
    /// and output-name identity of every [`OutputRef`]; a plan fingerprint must not substitute a
    /// runtime-resolved artifact digest.
    pub(crate) fn hash_semantics(&self, hash: &mut blake3::Hasher) {
        hash.update(b"key-recipe\0");
        hash.update(b"semantic-epoch\0");
        hash_field(hash, self.semantic_epoch.as_bytes());
        hash.update(b"typed-inputs\0");
        hash_count(hash, self.inputs.len());
        for input in &self.inputs {
            match input {
                InputRef::Blob(digest) => {
                    hash.update(b"input\0blob\0");
                    hash_digest(hash, digest);
                }
                InputRef::Output(reference) => {
                    hash.update(b"input\0output-ref\0");
                    hash_field(hash, reference.action.as_str().as_bytes());
                    hash_field(hash, reference.output.as_str().as_bytes());
                }
            }
        }
        hash.update(b"canonical-options\0");
        hash_count(hash, self.options.len());
        for (name, value) in &self.options {
            hash_field(hash, name.as_bytes());
            hash_field(hash, value.as_bytes());
        }
    }

    /// 解析具名输出并计算最终 BLAKE3 键。 / Resolves named outputs and computes the final BLAKE3 key.
    ///
    /// 前驱动作 key 和纯顺序依赖从不进入哈希，因而前驱重算但内容不变时可 early cutoff。
    /// Predecessor action keys and order-only dependencies never enter the hash,
    /// enabling early cutoff when predecessors rerun without changing content.
    pub fn materialize(
        &self,
        kind: &ActionKind,
        outputs: &[Output],
        mut resolve: impl FnMut(&OutputRef) -> Option<ContentDigest>,
    ) -> Option<ActionKey> {
        let mut hash = blake3::Hasher::new();
        hash.update(b"xmlsquish-action-key\0v2");
        hash_kind(&mut hash, kind);
        hash.update(b"epoch\0");
        hash_field(&mut hash, self.semantic_epoch.as_bytes());
        hash.update(b"resolved-inputs\0");
        hash_count(&mut hash, self.inputs.len());
        for input in &self.inputs {
            let digest = match input {
                InputRef::Blob(value) => value.clone(),
                InputRef::Output(reference) => resolve(reference)?,
            };
            hash_digest(&mut hash, &digest);
        }
        hash.update(b"options\0");
        hash_count(&mut hash, self.options.len());
        for (name, value) in &self.options {
            hash_field(&mut hash, name.as_bytes());
            hash_field(&mut hash, value.as_bytes());
        }
        hash.update(b"output-schema\0");
        hash_count(&mut hash, outputs.len());
        for output in outputs {
            hash_field(&mut hash, output.name.as_str().as_bytes());
            hash_artifact_kind(&mut hash, &output.kind);
        }
        Some(
            ActionKey::new(format!("blake3:{}", hash.finalize().to_hex()))
                .expect("digest is non-empty"),
        )
    }
}

fn hash_kind(hash: &mut blake3::Hasher, kind: &ActionKind) {
    let tag = match kind {
        ActionKind::CreateProject => b"create-project".as_slice(),
        ActionKind::Resolve => b"resolve".as_slice(),
        ActionKind::Snapshot => b"snapshot",
        ActionKind::Scan => b"scan",
        ActionKind::Compile => b"compile",
        ActionKind::Instantiate => b"instantiate",
        ActionKind::Link => b"link",
        ActionKind::Publish => b"publish",
        ActionKind::Format => b"format",
        ActionKind::Inspect => b"inspect",
        ActionKind::ResolveCandidate => b"resolve-candidate",
        ActionKind::CommitTransaction => b"commit-transaction",
        ActionKind::Backend => b"backend",
    };
    hash.update(b"kind\0builtin\0");
    hash_field(hash, tag);
}

pub(crate) fn hash_action_kind(hash: &mut blake3::Hasher, kind: &ActionKind) {
    hash_kind(hash, kind);
}

pub(crate) fn hash_output_kind(hash: &mut blake3::Hasher, kind: &ArtifactKind) {
    hash_artifact_kind(hash, kind);
}

fn hash_artifact_kind(hash: &mut blake3::Hasher, kind: &ArtifactKind) {
    let tag = match kind {
        ArtifactKind::BinaryIr => b"binary-ir".as_slice(),
        ArtifactKind::Prompt => b"prompt",
        ArtifactKind::DebugInfo => b"debug-info",
        ArtifactKind::Metadata => b"metadata",
        ArtifactKind::Other(name) => {
            hash.update(b"artifact\0other\0");
            hash_field(hash, name.as_bytes());
            return;
        }
    };
    hash.update(b"artifact\0builtin\0");
    hash_field(hash, tag);
}

pub(crate) fn hash_count(hash: &mut blake3::Hasher, count: usize) {
    hash.update(&(count as u64).to_le_bytes());
}
pub(crate) fn hash_field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
fn hash_digest(hash: &mut blake3::Hasher, digest: &ContentDigest) {
    match digest.algorithm() {
        DigestAlgorithm::Sha256 => {
            hash.update(b"algorithm\0sha256\0");
        }
        DigestAlgorithm::Blake3 => {
            hash.update(b"algorithm\0blake3\0");
        }
        DigestAlgorithm::Other(name) => {
            hash.update(b"algorithm\0other\0");
            hash_field(hash, name.as_bytes());
        }
    }
    hash_field(hash, digest.bytes());
}

/// 动作占用最多的资源类别，仅用于可观测性与策略。 / Dominant resource class, used for observability and policy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ResourceClass {
    /// 处理器密集。 / Processor intensive.
    Cpu,
    /// 输入输出密集。 / Input/output intensive.
    Io,
    /// 内存容量密集。 / Memory-capacity intensive.
    Memory,
}

/// 三维不可超售资源向量。 / Three-dimensional non-oversubscribable resource vector.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Resources {
    /// CPU 槽数量。 / Number of CPU slots.
    pub cpu: u32,
    /// IO 槽数量。 / Number of I/O slots.
    pub io: u32,
    /// 内存字节数。 / Number of memory bytes.
    pub memory: u64,
}
impl Resources {
    /// 创建资源向量。 / Creates a resource vector.
    pub const fn new(cpu: u32, io: u32, memory: u64) -> Self {
        Self { cpu, io, memory }
    }
    pub(crate) fn fits(self, available: Self) -> bool {
        self.cpu <= available.cpu && self.io <= available.io && self.memory <= available.memory
    }
    pub(crate) fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            cpu: self.cpu.checked_add(other.cpu)?,
            io: self.io.checked_add(other.io)?,
            memory: self.memory.checked_add(other.memory)?,
        })
    }
    pub(crate) fn subtract(self, other: Self) -> Self {
        Self::new(
            self.cpu - other.cpu,
            self.io - other.io,
            self.memory - other.memory,
        )
    }
}

/// 不可变计划的数学动作输入。 / Mathematical action input of an immutable plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Action {
    /// 计划局部 ID。 / Plan-local ID.
    pub id: ActionId,
    /// 物化完整键的冻结配方。 / Frozen recipe materializing the complete key.
    pub key: KeyRecipe,
    /// 领域动作种类。 / Domain action kind.
    pub kind: ActionKind,
    /// 主要资源类别。 / Dominant resource class.
    pub class: ResourceClass,
    /// 同时执行时保留的资源量。 / Resources reserved while executing.
    pub resources: Resources,
    /// 必须成功的前驱，包括不参与键的 order-only 边。 / Required predecessors, including order-only edges excluded from the key.
    pub dependencies: Vec<ActionId>,
    /// 纯输出模式。 / Pure output schema.
    pub outputs: Vec<Output>,
}

/// worker 返回的结构化事件；展示层决定如何渲染。 / Structured worker event rendered by the presentation layer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ActionEvent {
    /// 结构化诊断。 / Structured diagnostic.
    Diagnostic(Diagnostic),
    /// 已生成产物。 / Produced artifact.
    Artifact(Artifact),
    /// 稳定机器代码及人类文本。 / Stable machine code and human text.
    Message {
        /// 稳定机器代码。 / Stable machine code.
        code: String,
        /// 面向人的文本。 / Human-facing text.
        message: String,
    },
}

/// worker 报告的动作失败。 / Action failure reported by a worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerFailure {
    /// 跨版本稳定代码。 / Code stable across versions.
    pub code: String,
    /// 面向用户的说明。 / User-facing explanation.
    pub message: String,
}
impl WorkerFailure {
    /// 创建失败值。 / Creates a failure value.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// worker 的完整返回值，不产生终端副作用。 / Complete worker return value with no terminal side effects.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionResult {
    /// 成功或结构化失败。 / Success or structured failure.
    pub outcome: Result<(), WorkerFailure>,
    /// 成功动作按声明顺序生成的结构化输出。 / Structured outputs produced by a successful action, in declaration order.
    pub outputs: Vec<ProducedOutput>,
    /// 动作执行期间产生的结构化事件。 / Structured events produced during execution.
    pub events: Vec<ActionEvent>,
}
impl ActionResult {
    /// 创建不带事件的成功结果。 / Creates a successful result without events.
    pub fn success() -> Self {
        Self {
            outcome: Ok(()),
            outputs: Vec::new(),
            events: Vec::new(),
        }
    }
    /// 创建不带事件的失败结果。 / Creates a failed result without events.
    pub fn failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            outcome: Err(WorkerFailure::new(code, message)),
            outputs: Vec::new(),
            events: Vec::new(),
        }
    }
}

/// 一个已生成输出的内容身份。 / Content identity of one produced output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducedOutput {
    /// 稳定输出名。 / Stable output name.
    pub name: OutputName,
    /// 产物类别。 / Artifact kind.
    pub kind: ArtifactKind,
    /// 内容摘要。 / Content digest.
    pub digest: ContentDigest,
    /// 内容字节数。 / Content size in bytes.
    pub size: u64,
}
