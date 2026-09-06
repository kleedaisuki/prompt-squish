//! Structured terminal-safe diagnostics / 结构化且终端安全的诊断。
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Stage {
    Discovery,
    Read,
    Compile,
    Measure,
    Squish,
    WriteIntermediate,
    WriteOutput,
    Cleanup,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Read => "read",
            Self::Compile => "compile",
            Self::Measure => "measure",
            Self::Squish => "squish",
            Self::WriteIntermediate => "write intermediate",
            Self::WriteOutput => "write output",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Diagnostic {
    stage: Stage,
    path: Option<PathBuf>,
    line: Option<usize>,
    message: String,
    snippet: Option<String>,
    origin: Option<PathBuf>,
}

impl Diagnostic {
    pub(crate) fn new(stage: Stage, path: &Path, message: impl ToString) -> Self {
        Self {
            stage,
            path: Some(path.to_path_buf()),
            line: None,
            message: message.to_string(),
            snippet: None,
            origin: None,
        }
    }

    pub(crate) fn discovery(message: impl Into<String>) -> Self {
        Self {
            stage: Stage::Discovery,
            path: None,
            line: None,
            message: message.into(),
            snippet: None,
            origin: None,
        }
    }

    /// Capture the compiler's original source, never re-read a possibly changed file.
    /// 捕获编译器实际读取的源码，不重新读取可能已变化的文件。
    pub(crate) fn compile(
        error: xmlsquish_core::CompileError,
        primary: &Path,
        source: Option<&str>,
    ) -> Self {
        let snippet = source
            .and_then(|text| source_line(text, error.line))
            .map(str::to_owned);
        Self {
            stage: Stage::Compile,
            origin: (error.path != primary).then(|| primary.to_path_buf()),
            path: Some(error.path),
            line: Some(error.line),
            message: error.message,
            snippet,
        }
    }

    pub(crate) fn render(&self, out: &mut dyn Write, color: bool) -> io::Result<()> {
        // Group writes to reduce interleaving with separately captured stdout.
        // 整条诊断合并写出，减少与单独捕获的 stdout 发生交错的机会。
        let mut rendered = Vec::new();
        self.render_into(&mut rendered, color)?;
        out.write_all(&rendered)
    }

    fn render_into(&self, out: &mut dyn Write, color: bool) -> io::Result<()> {
        let message = self.message.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines = message.split('\n');
        let first = safe_text(lines.next().unwrap_or_default());
        writeln!(
            out,
            "{}: {first}",
            painted(&format!("error[{}]", self.stage.name()), "1;31", color)
        )?;
        // Every continuation is visibly nested, including regex error diagrams.
        // 所有续行明确缩进，包括正则表达式的多行错误说明。
        for line in lines {
            writeln!(out, "  {}", safe_text(line))?;
        }
        if let Some(path) = &self.path {
            let location = match self.line {
                Some(line) => format!("{}:{line}", pretty_path(path)),
                None => pretty_path(path),
            };
            writeln!(out, " {} {location}", painted("-->", "1;34", color))?;
        }
        if let (Some(line), Some(snippet)) = (self.line, &self.snippet) {
            let width = line.to_string().len();
            writeln!(
                out,
                " {}",
                painted(&format!("{:width$} |", ""), "1;34", color)
            )?;
            writeln!(
                out,
                " {} {}",
                painted(&format!("{line} |"), "1;34", color),
                clipped_source(snippet)
            )?;
            // Core only supplies a line; a fabricated column/caret would mislead.
            // 核心仅提供行号，不能虚构列号或插入符位置。
        }
        if let Some(origin) = &self.origin {
            writeln!(
                out,
                " {} while compiling {}",
                painted("note:", "1;36", color),
                pretty_path(origin)
            )?;
        }
        Ok(())
    }
}

fn source_line(text: &str, line: usize) -> Option<&str> {
    if line == 0 {
        return None;
    }
    // XML treats CRLF and bare CR as line endings / XML 将 CRLF 和单独 CR 视为换行。
    let mut lines = text.split_inclusive(['\n', '\r']).peekable();
    let mut number = 1;
    while let Some(part) = lines.next() {
        if number == line {
            return Some(part.trim_end_matches(['\n', '\r']));
        }
        if part.ends_with('\r') && lines.peek() == Some(&"\n") {
            lines.next();
        }
        number += 1;
    }
    (number == line && text.ends_with(['\n', '\r'])).then_some("")
}

pub(crate) fn safe_text(text: &str) -> String {
    text.chars().map(safe_character).collect()
}

fn safe_character(character: char) -> String {
    if character.is_control()
        || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    {
        character.escape_default().to_string()
    } else {
        character.to_string()
    }
}

fn clipped_source(text: &str) -> String {
    let mut result = String::new();
    let mut length = 0;
    for character in text.chars() {
        let escaped = safe_character(character);
        let count = escaped.chars().count();
        if length + count > 240 {
            result.push('…');
            break;
        }
        result.push_str(&escaped);
        length += count;
    }
    result
}

pub(crate) fn pretty_path(path: &Path) -> String {
    let cwd = std::env::current_dir().ok();
    let canonical = cwd.as_ref().and_then(|cwd| std::fs::canonicalize(cwd).ok());
    let relative = canonical
        .as_ref()
        .and_then(|cwd| path.strip_prefix(cwd).ok())
        .or_else(|| cwd.as_ref().and_then(|cwd| path.strip_prefix(cwd).ok()));
    let text = relative.unwrap_or(path).to_string_lossy();
    #[cfg(windows)]
    let text = readable_windows_path(&text);
    safe_text(text.as_ref())
}

#[cfg(windows)]
fn readable_windows_path(text: &str) -> String {
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(text).to_owned()
    }
}

pub(crate) fn painted(text: &str, style: &str, color: bool) -> String {
    if color {
        format!("\x1b[{style}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(diagnostic: &Diagnostic, color: bool) -> String {
        let mut out = Vec::new();
        diagnostic.render(&mut out, color).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn windows_verbatim_paths_remain_readable_and_unc_stays_absolute() {
        assert_eq!(
            readable_windows_path(r"\\?\C:\work\a.xml"),
            r"C:\work\a.xml"
        );
        assert_eq!(
            readable_windows_path(r"\\?\UNC\server\share\a.xml"),
            r"\\server\share\a.xml"
        );
    }

    #[test]
    fn canonical_working_directory_paths_are_relative() {
        let cwd = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(pretty_path(&cwd.join("fixture.xml")), "fixture.xml");
    }

    #[test]
    fn plain_diagnostic_has_source_and_no_fabricated_column() {
        let diagnostic = Diagnostic::compile(
            xmlsquish_core::CompileError {
                path: "part.xml".into(),
                line: 2,
                message: "undefined variable $x".into(),
            },
            Path::new("main.xml"),
            Some("<r>\r\n<xmlsquish:log msg=\"$x\"/>\r\n</r>"),
        );
        assert_eq!(
            render(&diagnostic, false),
            "error[compile]: undefined variable $x\n --> part.xml:2\n   |\n 2 | <xmlsquish:log msg=\"$x\"/>\n note: while compiling main.xml\n"
        );
    }

    #[test]
    fn color_only_wraps_the_same_plain_text() {
        let diagnostic = Diagnostic::new(Stage::Read, Path::new("missing.xml"), "not found");
        let color = render(&diagnostic, true);
        assert!(color.contains("\x1b[1;31m"));
        let stripped = color
            .replace("\x1b[1;31m", "")
            .replace("\x1b[1;34m", "")
            .replace("\x1b[0m", "");
        assert_eq!(stripped, render(&diagnostic, false));
    }

    #[test]
    fn messages_paths_and_snippets_cannot_inject_terminal_controls() {
        let diagnostic = Diagnostic::compile(
            xmlsquish_core::CompileError {
                path: "bad\x1b[2J.xml".into(),
                line: 1,
                message: "bad\x1b[31m\nsecond\rthird\u{202e}".into(),
            },
            Path::new("bad\x1b[2J.xml"),
            Some("\t<r>\x07\u{2066}</r>"),
        );
        let plain = render(&diagnostic, false);
        assert!(!plain.contains('\x1b') && !plain.contains('\x07') && !plain.contains('\u{202e}'));
        assert!(plain.contains("\n  second\n  third\\u{202e}"));
        assert!(plain.contains("\\t<r>\\u{7}\\u{2066}</r>"));
        assert!(!plain.contains("note:"));
    }

    #[test]
    fn snippet_is_an_owned_snapshot_and_long_lines_are_clipped() {
        let mut source = "original".repeat(100);
        let diagnostic = Diagnostic::compile(
            xmlsquish_core::CompileError {
                path: "a.xml".into(),
                line: 1,
                message: "bad".into(),
            },
            Path::new("a.xml"),
            Some(&source),
        );
        source.clear();
        let plain = render(&diagnostic, false);
        assert!(plain.contains("original"));
        let source_line = plain.lines().find(|line| line.starts_with(" 1 |")).unwrap();
        assert!(source_line.ends_with('…') && source_line.chars().count() <= 246);
        assert_eq!(super::source_line("a\rb\r\nc\nd", 3), Some("c"));
    }
}
