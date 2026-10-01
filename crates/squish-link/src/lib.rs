//! prompt-squish 的链接与实例化领域。 / Linking and instantiation for prompt-squish.
//!
//! 本 crate 实施两个明确阶段：[`StaticLinker`] 对冻结单元闭包做全量符号验证，
//! [`Instantiator`] 仅求值已链接程序并产生后端中立文档 IR。它不是后端，也不序列化 XML。
//! [`MiddleEnd`] specializes unbound portable units before [`StaticLinker`] binds symbols
//! and builds indexed executable views. [`Instantiator`] consumes those linked views into
//! backend-neutral documents and source-owned archive directives. No stage emits XML bytes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod directives;
mod error;
mod instantiate;
mod key;
mod linker;
mod middle;
mod program;

pub use directives::{ArchiveDirective, decode_archive_directives, encode_archive_directives};
pub use error::{InstantiateError, InstantiationFailure, InstantiationFrame, LinkError};
pub use instantiate::{Budgets, InstantiateOutput, Instantiator};
pub use key::{InstantiateKeyProjection, LinkKeyProjection};
pub use linker::{LinkOutput, PreparedUnitClosure, SharedUnitClosure, StaticLinker, UnitClosure};
pub use middle::{
    MiddleEnd, MiddleError, OptimizationStats, OptimizedUnit, StaticMatch, StaticScalar,
    StaticScalarSegment,
};
pub use program::{LinkedProgram, PreparedUnit};

#[cfg(test)]
mod tests;
