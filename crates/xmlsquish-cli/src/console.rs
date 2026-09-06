//! Per-stream styling policy / 按输出流决定终端样式。
use std::ffi::OsString;
use std::io::Write;

use anstream::{AutoStream, ColorChoice, stream::RawStream};
use clap::ValueEnum;

pub(crate) use crate::diagnostics::painted as styled;
use crate::diagnostics::safe_text;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    // Inspect only the color option before clap can exit on help or an error.
    // 提前读取颜色选项，使帮助和参数错误也遵循同一策略；语法仍由 clap 验证。
    pub(crate) fn from_args(args: &[OsString]) -> Self {
        let mut mode = Self::Auto;
        let mut args = args.iter().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--" {
                break;
            }
            let value = if arg == "--color" {
                args.next().and_then(|value| value.to_str())
            } else {
                arg.to_str()
                    .and_then(|value| value.strip_prefix("--color="))
            };
            match value {
                Some("auto") => mode = Self::Auto,
                Some("always") => mode = Self::Always,
                Some("never") => mode = Self::Never,
                _ => {}
            }
        }
        mode
    }

    pub(crate) fn stream<S: RawStream>(self, raw: S) -> AutoStream<S> {
        let choice = match self {
            Self::Auto => ColorChoice::Auto,
            Self::Always => ColorChoice::Always,
            Self::Never => ColorChoice::Never,
        };
        AutoStream::new(raw, choice)
    }
}

pub(crate) fn section(out: &mut dyn Write, title: &str, color: bool) {
    let _ = writeln!(out, "\n{}", styled(title, "1;36", color));
}

/// Preserve clap's prose while styling only known labels, never user arguments.
/// 保留 clap 文案，仅给已知标签着色，用户参数中的控制字符可视化显示。
pub(crate) fn clap_text(out: &mut dyn Write, text: &str, color: bool) {
    for line in text.lines() {
        let line = safe_text(line);
        let prefix = ["error:", "Usage:", "Arguments:", "Options:"]
            .into_iter()
            .find(|prefix| line.starts_with(prefix));
        if let Some(prefix) = prefix {
            let code = if prefix == "error:" { "1;31" } else { "1;36" };
            let _ = writeln!(
                out,
                "{}{}",
                styled(prefix, code, color),
                &line[prefix.len()..]
            );
        } else {
            let _ = writeln!(out, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_preparse_respects_argument_boundary_and_forms() {
        for (args, expected) in [
            (
                vec!["xmlsquish", "--color=always", "--help"],
                ColorMode::Always,
            ),
            (
                vec!["xmlsquish", "--color", "never", "--bad"],
                ColorMode::Never,
            ),
            (vec!["xmlsquish", "--", "--color=always"], ColorMode::Auto),
            (vec!["xmlsquish", "--color=invalid"], ColorMode::Auto),
        ] {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(ColorMode::from_args(&args), expected);
        }
    }
}
