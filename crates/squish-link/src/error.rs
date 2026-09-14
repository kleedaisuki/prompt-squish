//! 链接和求值错误。 / Link and evaluation errors.

use squish_ir::{FrameId, QualifiedOriginRef, SourceKey, SymbolKey};
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
