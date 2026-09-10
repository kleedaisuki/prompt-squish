//! Module-local regression tests. / 紧邻模块的回归测试。
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
        crate::CompileError {
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
        crate::CompileError {
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
        crate::CompileError {
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
