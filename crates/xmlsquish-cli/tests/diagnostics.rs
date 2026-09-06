//! Process-level color policy tests avoid mutating the test runner environment.
//! 用子进程测试颜色策略，不修改并发测试运行器的全局环境。
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xmlsquish"));
    for name in ["NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "TERM", "CI"] {
        command.env_remove(name);
    }
    command.env("TERM", "xterm-256color");
    command
}

fn has_color(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|bytes| bytes == b"\x1b[")
}

fn strip(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::new();
    anstream::StripStream::new(&mut text)
        .write_all(bytes)
        .unwrap();
    text
}

fn failure_fixture(dir: &Path) -> std::path::PathBuf {
    let main = dir.join("main.xml");
    fs::write(&main, "<r><xmlsquish:mount path='child.xml'/></r>").unwrap();
    fs::write(
        dir.join("child.xml"),
        "<child>\r\n    <xmlsquish:log msg='$missing'/>\r\n</child>",
    )
    .unwrap();
    main
}

#[test]
fn redirected_auto_is_plain_and_forced_color_preserves_diagnostic_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = failure_fixture(dir.path());
    let automatic = command().arg(&path).output().unwrap();
    let never = command()
        .args(["--color", "never"])
        .arg(&path)
        .output()
        .unwrap();
    let always = command().arg("--color=always").arg(&path).output().unwrap();
    for output in [&automatic, &never, &always] {
        assert_eq!(output.status.code(), Some(1));
    }
    assert!(!has_color(&automatic.stdout) && !has_color(&automatic.stderr));
    assert_eq!(automatic.stdout, never.stdout);
    assert_eq!(automatic.stderr, never.stderr);
    assert!(has_color(&always.stdout) && has_color(&always.stderr));
    assert_eq!(strip(&always.stdout), never.stdout);
    assert_eq!(strip(&always.stderr), never.stderr);
    let diagnostic = String::from_utf8(never.stderr).unwrap();
    assert!(
        diagnostic.starts_with("error[compile]: undefined variable '$missing'\n"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("child.xml:2"));
    assert!(diagnostic.contains("2 |     <xmlsquish:log msg='$missing'/>"));
    assert!(diagnostic.contains("while compiling"));
    assert_eq!(diagnostic.matches("[compile]").count(), 1);
}

#[test]
fn environment_preferences_and_explicit_overrides_work_for_help() {
    type ColorCase<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str], bool);
    let cases: &[ColorCase<'_>] = &[
        (&[("NO_COLOR", "1")], &["--help"], false),
        (&[("CLICOLOR_FORCE", "1")], &["--help"], true),
        (
            &[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")],
            &["--help"],
            false,
        ),
        (&[("CLICOLOR", "0")], &["--help"], false),
        (&[("TERM", "dumb")], &["--help"], false),
        (&[("CI", "1")], &["--help"], false),
        (&[("NO_COLOR", "1")], &["--color=always", "--help"], true),
        (
            &[("CLICOLOR_FORCE", "1")],
            &["--color=never", "--help"],
            false,
        ),
    ];
    for (environment, args, colored) in cases {
        let result = command()
            .envs(environment.iter().copied())
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0));
        assert!(result.stderr.is_empty());
        assert_eq!(
            has_color(&result.stdout),
            *colored,
            "{environment:?} {args:?}"
        );
        assert!(
            String::from_utf8(strip(&result.stdout))
                .unwrap()
                .contains("--color")
        );
    }
}

#[test]
fn argument_errors_follow_color_choice_and_keep_exit_code() {
    for args in [
        vec!["--color=always", "--unknown"],
        vec!["--unknown", "--color", "always"],
    ] {
        let result = command().args(args).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        assert!(has_color(&result.stderr));
        assert!(
            String::from_utf8(strip(&result.stderr))
                .unwrap()
                .starts_with("error:")
        );
    }
    let result = command().args(["--color=bad"]).output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(!has_color(&result.stderr));
}

#[test]
fn argument_newlines_cannot_forge_colored_diagnostic_labels() {
    let result = command()
        .args([
            "--color=always",
            "--bad\nerror: forged diagnostic\rOVERWRITE",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let text = String::from_utf8(strip(&result.stderr)).unwrap();
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("error:"))
            .count(),
        1,
        "{text}"
    );
    assert!(
        text.contains(r"--bad\nerror: forged diagnostic\rOVERWRITE"),
        "{text}"
    );
}

#[test]
fn user_control_sequences_never_become_terminal_commands() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log.xml");
    fs::write(&path, "<r><xmlsquish:log msg='hello\x1b[2J\u{202e}'/></r>").unwrap();
    let Output {
        status,
        stdout,
        stderr,
    } = command().arg("--color=always").arg(path).output().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    let text = String::from_utf8(stdout).unwrap();
    assert!(!text.contains("\x1b[2J"));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains(r"\u{1b}[2J\u{202e}"), "{text}");
}

#[test]
fn read_and_discovery_errors_are_structured_without_duplicate_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.xml");
    fs::write(&path, [0xff]).unwrap();
    let result = command().arg(&path).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let text = String::from_utf8(result.stderr).unwrap();
    assert!(text.starts_with("error[read]:"), "{text}");
    assert_eq!(text.matches("invalid.xml").count(), 1, "{text}");
    let result = command()
        .arg(dir.path().join("missing.xml"))
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .starts_with("error[discovery]:")
    );
}
