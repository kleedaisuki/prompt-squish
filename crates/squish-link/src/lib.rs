//! prompt-squish 的链接与实例化领域。 / Linking and instantiation for prompt-squish.
//!
//! 本 crate 实施两个明确阶段：[`StaticLinker`] 对冻结单元闭包做全量符号验证，
//! [`Instantiator`] 仅求值已链接程序并产生后端中立文档 IR。它不是后端，也不序列化 XML。
//! This crate implements two explicit phases: [`StaticLinker`] validates and relocates a frozen
//! unit closure, while [`Instantiator`] evaluates a linked program into backend-neutral document
//! IR. It is not a backend and never serializes XML.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod instantiate;
mod key;
mod linker;
mod program;

pub use error::{InstantiateError, LinkError};
pub use instantiate::{Budgets, InstantiateOutput, Instantiator};
pub use key::{InstantiateKeyProjection, LinkKeyProjection};
pub use linker::{LinkOutput, StaticLinker, UnitClosure};
pub use program::LinkedProgram;

#[cfg(test)]
mod tests;
