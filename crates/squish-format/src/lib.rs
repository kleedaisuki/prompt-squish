//! prompt-squish 的无损 XML 语法与格式化领域。 / Lossless XML syntax and formatting domain.
//!
//! 本 crate 有意不依赖语义前端。解析结果保存源文件的精确字节切片；格式化器只为标签内部的
//! 空白生成编辑计划。字符数据、注释、CDATA、处理指令和属性值从不被重写。
//! This crate intentionally has no dependency on the semantic frontend. Parsed tokens retain exact
//! source byte slices; the formatter only plans edits to whitespace inside tags. Character data,
//! comments, CDATA, processing instructions, and attribute values are never rewritten.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod format;
mod syntax;

#[cfg(test)]
mod tests;

pub use format::{
    FormatEdit, FormatPlan, FormatPlanError, StyleEdition, UnknownStyleEdition, format,
};
pub use syntax::{Diagnostic, DiagnosticCode, LosslessXml, Span, Token, TokenKind};
