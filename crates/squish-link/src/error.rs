//! 链接和求值错误。 / Link and evaluation errors.

use squish_ir::{FrameId, FrameIdentity, QualifiedOriginRef, SourceKey, SymbolKey};
use std::{error::Error, fmt};

/// 结构化链接错误；`code` 是稳定的机器识别值。 / Structured link error with a stable machine code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkError {
    /// 稳定诊断代码。 / Stable diagnostic code.
    pub code: &'static str,
    /// 人类可读详情。 / Human-readable detail.
    pub message: String,
    /// 相关逻辑源。 / Related logical source.
    pub source: Option<Box<SourceKey>>,
    /// 相关宏符号。 / Related macro symbol.
    pub symbol: Option<Box<SymbolKey>>,
    /// 最精确的可用源位置。 / Most precise available source origin.
    pub origin: Option<QualifiedOriginRef>,
}

impl LinkError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
            symbol: None,
            origin: None,
        }
    }
    pub(crate) fn at_source(mut self, source: SourceKey) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    pub(crate) fn at_symbol(mut self, symbol: SymbolKey) -> Self {
        self.symbol = Some(Box::new(symbol));
        self
    }
}
impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for LinkError {}
impl LinkError {
    /// 返回协议中的固定阶段。 / Returns the stable protocol phase.
    #[must_use]
    pub fn phase(&self) -> squish_protocol::Phase {
        squish_protocol::Phase::Link
    }
}

/// 结构化实例化错误，保留完整展开帧链。 / Structured instantiation error retaining the expansion frame chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstantiateError {
    /// 稳定诊断代码。 / Stable diagnostic code.
    pub code: &'static str,
    /// 人类可读详情。 / Human-readable detail.
    pub message: String,
    /// 触发失败的源位置。 / Origin which triggered the failure.
    pub origin: Option<QualifiedOriginRef>,
    /// 从 entry 到失败点的帧链。 / Frame chain from entry to failure.
    pub frame_chain: Vec<FrameId>,
}

impl InstantiateError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            origin: None,
            frame_chain: Vec::new(),
        }
    }
}
impl fmt::Display for InstantiateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for InstantiateError {}
impl InstantiateError {
    /// 返回协议中的固定阶段。 / Returns the stable protocol phase.
    #[must_use]
    pub fn phase(&self) -> squish_protocol::Phase {
        squish_protocol::Phase::Instantiate
    }
}

/// Source-qualified provenance of one actual invocation on a failed expansion's parent chain.
///
/// These are dynamic frames, not a static symbol walk: repeated recursive invocations retain
/// distinct frame IDs even when their definition and call-site origins are equal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstantiationFrame {
    /// Actual dynamic frame ID, matching the corresponding legacy error chain entry.
    pub id: FrameId,
    /// Entry or exact linked macro address invoked by this frame.
    pub identity: FrameIdentity,
    /// Actual caller operation origin; the entry has no call site.
    pub call_origin: Option<QualifiedOriginRef>,
    /// Source origin of the entry root or macro declaration that owns this invocation.
    pub definition_origin: QualifiedOriginRef,
}

/// Additive failure evidence for diagnostic-aware consumers, without changing legacy errors.
///
/// `frames` follows the actual entry-to-failure parent chain in `error.frame_chain`. It is
/// empty when a failure occurs before a machine/frame exists or has no dynamic frame context.
/// No successful trace, partial document or fabricated source origin is returned on failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstantiationFailure {
    /// The unchanged public error, including its stable code, primary origin and frame IDs.
    pub error: InstantiateError,
    /// Immutable dense invocation snapshot needed to explain call and declaration sites.
    /// A boxed slice keeps the additive failure small without boxing the legacy error itself.
    pub frames: Box<[InstantiationFrame]>,
}

impl From<InstantiateError> for InstantiationFailure {
    /// Wraps a pre-machine or context-free error without inventing invocation provenance.
    fn from(error: InstantiateError) -> Self {
        Self {
            error,
            frames: Box::default(),
        }
    }
}

impl fmt::Display for InstantiationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for InstantiationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
