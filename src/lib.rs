//! Namespace-aware XML macros with frozen sources and explicit invocation frames.
//! 基于命名空间的 XML 宏，使用冻结源码与显式调用帧。
//!
//! [`Compiler`] resolves macros with a caller-supplied include loader; [`squish`]
//! independently normalizes whitespace while preserving markup bytes.
//! 编译器通过调用方加载器解析宏；空白规范化独立运行，保持标记字节。
//! File discovery, token measurement and artifact persistence live in [`cli`].
//! 文件发现、令牌统计和产物持久化均封装在命令行模块。
//!
//! ```
//! use std::path::Path;
//! use xmlsquish::Compiler;
//!
//! let compiled = Compiler::new().compile(
//!     Path::new("prompt.xml"),
//!     r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><p>  hello  </p></xs:module>"#,
//!     |_| Err("no includes available".into()),
//! )?;
//! assert!(compiled.output.contains("  hello  "));
//! // Normal compilation preserves text; squish is an independent, lossy utility.
//! // 正常编译保留文本；squish 是独立、有损的工具。
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod cli;
mod compiler;
mod squish;

pub use compiler::{CompileError, CompileLog, CompileOptions, CompileResult, Compiler};
pub use squish::{SquishError, SquishErrorKind, SquishOutput, WhitespaceStats, squish};
