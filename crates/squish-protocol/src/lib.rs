//! prompt-squish 组件间的稳定协议。 / Stable protocols between prompt-squish components.
//!
//! 此 crate 只包含数据类型，不包含编排或领域逻辑。所有线上结构都显式带版本，
//! 以便终端、守护进程和缓存读取器独立演进。
//! This crate contains data types only, not orchestration or domain logic. Every
//! wire structure carries an explicit version so terminals, daemons, and cache
//! readers can evolve independently.

use std::{fmt, ops::Range, str::FromStr};

use serde::{Deserialize, Serialize};

/// 当前事件协议版本。 / Current event protocol version.
pub const EVENT_PROTOCOL_VERSION: u16 = 1;

/// 空标识符错误。 / Error returned for an empty identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyId;

impl fmt::Display for EmptyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("identifier must not be empty")
    }
}

impl std::error::Error for EmptyId {}

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// 从非空文本构造标识符。 / Creates an identifier from non-empty text.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
                let value = value.into();
                if value.is_empty() {
                    return Err(EmptyId);
                }
                Ok(Self(value))
            }

            /// 返回协议中的文本值。 / Returns the protocol text value.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = EmptyId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = EmptyId;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

string_id!(
    InvocationId,
    "一次入口调用的稳定标识符。 / Stable identifier for one entry-point invocation."
);
string_id!(
    CapabilityId,
    "静态注册能力的稳定标识符。 / Stable identifier for a statically registered capability."
);
string_id!(
    ArtifactId,
    "构建产物的稳定标识符。 / Stable identifier for a produced artifact."
);
string_id!(
    SourceId,
    "源码或虚拟输入的稳定标识符。 / Stable identifier for a source or virtual input."
);
string_id!(
    DiagnosticId,
    "诊断实例的稳定标识符。 / Stable identifier for a diagnostic instance."
);

/// 诊断严重级别。 / Diagnostic severity level.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 调试信息。 / Debugging information.
    Trace,
    /// 普通信息。 / Informational message.
    Info,
    /// 不阻止命令成功的警告。 / Warning that does not prevent success.
    Warning,
    /// 导致当前操作失败的错误。 / Error that fails the current operation.
    Error,
}

/// 产生诊断或事件的处理阶段。 / Processing phase that produced a diagnostic or event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 项目和输入发现。 / Project and input discovery.
    Discover,
    /// 源码解析。 / Source parsing.
    Parse,
    /// 语义分析及 IR 生成。 / Semantic analysis and IR generation.
    Analyze,
    /// 产物链接。 / Artifact linking.
    Link,
    /// 后端输出。 / Backend emission.
    Emit,
    /// 格式化。 / Formatting.
    Format,
    /// 依赖或项目管理。 / Dependency or project management.
    Manage,
    /// 内核编排。 / Kernel orchestration.
    Orchestrate,
}

/// 非法源码范围。 / Invalid source range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSpan {
    start: u64,
    end: u64,
}

impl fmt::Display for InvalidSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "span start {} exceeds end {}",
            self.start, self.end
        )
    }
}

impl std::error::Error for InvalidSpan {}

/// 源码中的半开字节范围。 / Half-open byte range in a source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "SpanWire", into = "SpanWire")]
pub struct Span {
    source: SourceId,
    bytes: Range<u64>,
}

impl Span {
    /// 构造满足 `start <= end` 的范围。 / Creates a range satisfying `start <= end`.
    pub fn new(source: SourceId, start: u64, end: u64) -> Result<Self, InvalidSpan> {
        if start > end {
            return Err(InvalidSpan { start, end });
        }
        Ok(Self {
            source,
            bytes: start..end,
        })
    }

    /// 返回源码标识符。 / Returns the source identifier.
    pub fn source(&self) -> &SourceId {
        &self.source
    }

    /// 返回半开字节范围。 / Returns the half-open byte range.
    pub fn bytes(&self) -> Range<u64> {
        self.bytes.clone()
    }
}

#[derive(Deserialize, Serialize)]
struct SpanWire {
    source: SourceId,
    start: u64,
    end: u64,
}

impl TryFrom<SpanWire> for Span {
    type Error = InvalidSpan;

    fn try_from(value: SpanWire) -> Result<Self, Self::Error> {
        Self::new(value.source, value.start, value.end)
    }
}

impl From<Span> for SpanWire {
    fn from(value: Span) -> Self {
        Self {
            source: value.source,
            start: value.bytes.start,
            end: value.bytes.end,
        }
    }
}

/// 附带说明的相关源码位置。 / Related source location with an explanatory label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelatedSpan {
    /// 相关范围。 / Related range.
    pub span: Span,
    /// 解释此范围为何相关。 / Explains why the range is relevant.
    pub label: String,
}

/// 可由机器分类、由人阅读的诊断。 / Machine-classifiable, human-readable diagnostic.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    /// 本次诊断实例的标识符。 / Identifier of this diagnostic instance.
    pub id: DiagnosticId,
    /// 跨版本稳定的机器代码。 / Machine code stable across versions.
    pub code: String,
    /// 严重级别。 / Severity level.
    pub severity: Severity,
    /// 产生诊断的阶段。 / Phase that produced the diagnostic.
    pub phase: Phase,
    /// 面向用户的主要消息。 / Primary user-facing message.
    pub message: String,
    /// 主要源码位置（若适用）。 / Primary source location, when applicable.
    pub primary: Option<Span>,
    /// 补充源码位置。 / Supplemental source locations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedSpan>,
    /// 可执行的修复建议。 / Actionable remediation hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// 产物在流水线中的语义类别。 / Semantic category of an artifact in the pipeline.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum ArtifactKind {
    /// 可缓存、可链接的二进制中间表示。 / Cacheable, linkable binary IR.
    BinaryIr,
    /// 最终 `.prompt` 文档。 / Final `.prompt` document.
    Prompt,
    /// 源码映射或其他调试信息。 / Source map or other debug information.
    DebugInfo,
    /// 供机器读取的构建元数据。 / Machine-readable build metadata.
    Metadata,
    /// 由扩展能力定义、且保留名称的类别。 / Named kind defined by an extension capability.
    Other(String),
}

/// 已发布产物的描述。 / Description of a published artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Artifact {
    /// 产物标识符。 / Artifact identifier.
    pub id: ArtifactId,
    /// 产物类别。 / Artifact category.
    pub kind: ArtifactKind,
    /// UTF-8 URI；路径由宿主解释。 / UTF-8 URI; paths are interpreted by the host.
    pub uri: String,
    /// 内容字节数。 / Content size in bytes.
    pub size: u64,
    /// 可选内容摘要，例如 `sha256:<hex>`。 / Optional content digest such as `sha256:<hex>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// 已完成单位超过总单位。 / Completed work units exceeded total work units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidProgress {
    completed: u64,
    total: u64,
}

impl fmt::Display for InvalidProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "completed work {} exceeds total work {}",
            self.completed, self.total
        )
    }
}

impl std::error::Error for InvalidProgress {}

/// 满足 `completed <= total` 的有界进度。 / Bounded progress satisfying `completed <= total`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "ProgressWire", into = "ProgressWire")]
pub struct Progress {
    message: String,
    completed: u64,
    total: u64,
}

impl Progress {
    /// 创建保持有界进度不变量的更新。 / Creates an update preserving the bounded-progress invariant.
    pub fn new(
        message: impl Into<String>,
        completed: u64,
        total: u64,
    ) -> Result<Self, InvalidProgress> {
        if completed > total {
            return Err(InvalidProgress { completed, total });
        }
        Ok(Self {
            message: message.into(),
            completed,
            total,
        })
    }

    /// 返回当前工作说明。 / Returns the current-work description.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// 返回已完成单位。 / Returns completed work units.
    pub const fn completed(&self) -> u64 {
        self.completed
    }

    /// 返回总单位。 / Returns total work units.
    pub const fn total(&self) -> u64 {
        self.total
    }
}

#[derive(Deserialize, Serialize)]
struct ProgressWire {
    message: String,
    completed: u64,
    total: u64,
}

impl TryFrom<ProgressWire> for Progress {
    type Error = InvalidProgress;

    fn try_from(value: ProgressWire) -> Result<Self, Self::Error> {
        Self::new(value.message, value.completed, value.total)
    }
}

impl From<Progress> for ProgressWire {
    fn from(value: Progress) -> Self {
        Self {
            message: value.message,
            completed: value.completed,
            total: value.total,
        }
    }
}

/// 命令的终止状态。 / Terminal status of a command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ExitStatus {
    /// 成功完成。 / Completed successfully.
    Success,
    /// 领域操作失败。 / Domain operation failed.
    Failed,
    /// 由调用者取消。 / Cancelled by the caller.
    Cancelled,
}

impl ExitStatus {
    /// 返回跨前端一致的进程退出码。 / Returns a process exit code consistent across frontends.
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failed => 1,
            Self::Cancelled => 130,
        }
    }
}

/// 内核向前端发布的版本化事件。 / Versioned event published by the kernel to a frontend.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Event {
    /// 线上协议版本，当前必须是 [`EVENT_PROTOCOL_VERSION`]。 / Wire protocol version, currently [`EVENT_PROTOCOL_VERSION`].
    pub version: u16,
    /// 所属调用。 / Owning invocation.
    pub invocation: InvocationId,
    /// 调用内严格递增的序号。 / Strictly increasing sequence within the invocation.
    pub sequence: u64,
    /// 事件负载。 / Event payload.
    pub payload: EventPayload,
}

impl Event {
    /// 使用当前协议版本创建事件。 / Creates an event using the current protocol version.
    pub fn new(invocation: InvocationId, sequence: u64, payload: EventPayload) -> Self {
        Self {
            version: EVENT_PROTOCOL_VERSION,
            invocation,
            sequence,
            payload,
        }
    }
}

/// 事件的向前兼容负载集合。 / Forward-extensible set of event payloads.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
#[non_exhaustive]
pub enum EventPayload {
    /// 命令已被调度。 / A command has been scheduled.
    CommandStarted {
        /// 处理命令的能力。 / Capability handling the command.
        capability: CapabilityId,
        /// 稳定命令名称。 / Stable command name.
        command: String,
    },
    /// 有界进度更新。 / Bounded progress update.
    Progress(Progress),
    /// 结构化诊断。 / Structured diagnostic.
    Diagnostic(Diagnostic),
    /// 新产物已发布。 / A new artifact was published.
    Artifact(Artifact),
    /// 命令已终止。 / The command terminated.
    CommandFinished {
        /// 最终状态。 / Final status.
        status: ExitStatus,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_empty_wire_values() {
        let result = serde_json::from_str::<InvocationId>(r#"""#);
        assert!(result.is_err());
    }

    #[test]
    fn spans_reject_reversed_wire_ranges() {
        let json = r#"{"source":"main","start":9,"end":4}"#;
        assert!(serde_json::from_str::<Span>(json).is_err());
    }

    #[test]
    fn progress_rejects_values_past_total() {
        let json = r#"{"message":"compile","completed":2,"total":1}"#;
        assert!(serde_json::from_str::<Progress>(json).is_err());
    }

    #[test]
    fn event_round_trips_with_explicit_version() {
        let event = Event::new(
            InvocationId::new("run-1").unwrap(),
            0,
            EventPayload::CommandFinished {
                status: ExitStatus::Success,
            },
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""version":1"#));
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }
}
