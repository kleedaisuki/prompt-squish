//! XML prompt compilation and lexical whitespace normalization.
//! XML 提示词编译与词法空白规范化。
//!
//! [`Compiler`] resolves macros with a caller-supplied include loader; [`squish`]
//! independently normalizes whitespace while preserving markup bytes.
//! 编译器通过调用方加载器解析宏；空白规范化独立运行，保持标记字节。
//! File discovery, token measurement and artifact persistence live in [`cli`].
//! 文件发现、令牌统计和产物持久化均封装在命令行模块。
//!
//! ```
//! use std::path::Path;
//! use xmlsquish::{Compiler, squish};
//!
//! let compiled = Compiler::new().compile(
//!     Path::new("prompt.xml"), "<p>  hello  </p>",
//!     |_| Err("no includes available".into()),
//! )?;
//! assert_eq!(squish(&compiled.output)?.output, "<p> hello </p>");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod cli;
mod compiler;
mod squish;

pub use compiler::{CompileError, CompileLog, CompileResult, Compiler};
pub use squish::{SquishError, SquishErrorKind, SquishOutput, WhitespaceStats, squish};
