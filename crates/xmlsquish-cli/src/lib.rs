mod console;
mod diagnostics;
mod paths;
mod pipeline;

use clap::{CommandFactory, Parser};
use console::{ColorMode, section, styled};
use std::ffi::OsString;
#[cfg(test)]
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tiktoken_rs::o200k_base_singleton;
use xmlsquish_app::{FileFailure, Squasher, SquishResult, TokenCounter};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Parser)]
#[command(
    name = "xmlsquish",
    version,
    about = "Compile XML prompt macros and compress whitespace",
    long_about = "Compile XML macros to *.i.xml, then compress whitespace to *.o.xml.\n\
                  Use -I to retain only the intermediate stage; -O is the default.\n\
                  Directories are searched recursively, ignoring *.i.xml and *.o.xml."
)]
pub struct Args {
    /// Color output: auto detects each terminal / 颜色模式，auto 按终端能力决定
    #[arg(long, value_enum, default_value = "auto")]
    color: ColorMode,
    /// Compile only to *.i.xml / 仅生成中间表示
    #[arg(short = 'I', conflicts_with = "optimized")]
    intermediate: bool,
    /// Compile and optimize to *.o.xml (default) / 编译并压缩
    #[arg(short = 'O')]
    optimized: bool,
    /// Input XML files, directories, or glob patterns
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

/// Runs the CLI with injectable streams. Returns the intended process exit code.
pub fn run<I, T>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let stream = ColorMode::from_args(&args).stream(Vec::<u8>::new());
    let color = stream.current_choice() != anstream::ColorChoice::Never;
    execute(args, stdout, stderr, color, color)
}

/// Native terminal entry point; injected `run` writers have no TTY capability.
/// 原生终端入口；run 的注入 writer 不宣称自己具有终端能力。
pub fn run_stdio<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let color = ColorMode::from_args(&args);
    let mut stdout = color.stream(io::stdout()).lock();
    let mut stderr = color.stream(io::stderr()).lock();
    let out_color = stdout.current_choice() != anstream::ColorChoice::Never;
    let err_color = stderr.current_choice() != anstream::ColorChoice::Never;
    execute(args, &mut stdout, &mut stderr, out_color, err_color)
}

fn execute(
    args: Vec<OsString>,
    mut stdout: &mut dyn Write,
    mut stderr: &mut dyn Write,
    out_color: bool,
    err_color: bool,
) -> i32 {
    let args = match Args::try_parse_from(&args) {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            if error.use_stderr() {
                // Re-render only the failed parse with escaped argument values.
                // 仅为错误展示重放已失败的参数解析；不改变实际处理的路径或参数。
                let display_args = args
                    .iter()
                    .map(|arg| OsString::from(diagnostics::safe_text(&arg.to_string_lossy())));
                let message = Args::try_parse_from(display_args)
                    .err()
                    .filter(|error| error.use_stderr())
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| {
                        format!("error: {}", diagnostics::safe_text(&error.to_string()))
                    });
                console::clap_text(&mut stderr, &message, err_color);
            } else {
                console::clap_text(&mut stdout, &error.to_string(), out_color);
            }
            return code;
        }
    };

    if args.paths.is_empty() {
        let mut command = Args::command();
        console::clap_text(
            &mut stdout,
            &command.render_long_help().to_string(),
            out_color,
        );
        return 0;
    }

    let discovery = paths::discover(&args.paths);
    for error in &discovery.errors {
        let _ = diagnostics::Diagnostic::discovery(error.clone()).render(&mut stderr, err_color);
        let _ = writeln!(stderr);
    }

    let report =
        pipeline::run_with_color(&discovery.files, args.intermediate, &mut stdout, out_color);
    let _ = stdout.flush();
    for failure in &report.failures {
        let _ = failure.render(&mut stderr, err_color);
        let _ = writeln!(stderr);
    }
    let _ = stderr.flush();
    print_report_colored(
        &mut stdout,
        discovery.files.len(),
        discovery.errors.len(),
        &report,
        out_color,
    );

    if discovery.errors.is_empty() && report.failures.is_empty() {
        0
    } else {
        1
    }
}

#[cfg(test)]
fn print_report(
    out: &mut dyn Write,
    discovered: usize,
    discovery_failures: usize,
    report: &pipeline::Report,
) {
    print_report_colored(out, discovered, discovery_failures, report, false);
}

fn print_report_colored(
    out: &mut dyn Write,
    discovered: usize,
    discovery_failures: usize,
    report: &pipeline::Report,
    color: bool,
) {
    let stats = &report.stats;
    let failures = discovery_failures.saturating_add(report.failures.len());
    section(out, "Summary", color);
    let _ = writeln!(out, "Processed files: {discovered}");
    let _ = writeln!(
        out,
        "{}",
        styled(
            &format!("Succeeded: {}", stats.processed_files),
            "32",
            color
        )
    );
    let _ = writeln!(
        out,
        "{}",
        styled(
            &format!("Failed: {failures}"),
            if failures == 0 { "2" } else { "1;31" },
            color
        )
    );
    let _ = writeln!(out, "Discovery errors: {discovery_failures}");
    section(out, "Prompt size", color);
    let _ = writeln!(out, "Encoding: o200k_base");
    let _ = writeln!(
        out,
        "Measurements: successful artifacts only; UTF-8 text bytes exclude BOM"
    );
    let _ = writeln!(
        out,
        "{}",
        styled("Stage                Tokens    UTF-8 bytes", "1", color)
    );
    print_size(out, "Primary source", stats.source);
    print_size(out, "Compiled IR", stats.ir);
    print_size(out, "Final prompt", stats.final_prompt);
    let _ = writeln!(out, "Final prompt tokens: {}", stats.final_prompt.tokens);
    let _ = writeln!(
        out,
        "Final prompt UTF-8 bytes: {}",
        stats.final_prompt.bytes
    );
    let _ = writeln!(out, "Input characters: {}", stats.source.characters);
    let _ = writeln!(out, "Output characters: {}", stats.final_prompt.characters);
    section(out, "Dependencies", color);
    let _ = writeln!(
        out,
        "Primary source: each successful input counted once; dependencies excluded"
    );
    let _ = writeln!(out, "Dependency loads: {}", stats.dependency_loads);
    let _ = writeln!(
        out,
        "Unique dependency files: {}",
        stats.dependency_paths.len()
    );
    let _ = writeln!(
        out,
        "Dependency UTF-8 bytes read: {} (repeated loads counted)",
        stats.dependency_bytes
    );
    if stats.source.tokens == 0 {
        let _ = writeln!(out, "Assembly token ratio (IR / source): N/A");
    } else {
        let ratio = stats.ir.tokens as f64 / stats.source.tokens as f64;
        let _ = writeln!(out, "Assembly token ratio (IR / source): {ratio:.2}x");
    }
    section(out, "Optimization", color);
    if stats.processed_files == 0 {
        let _ = writeln!(out, "Optimization: N/A (no successful files)");
        return;
    }
    if report.stage == pipeline::OutputStage::Intermediate {
        let _ = writeln!(
            out,
            "Optimization: not run (-I); final prompt is compiled IR"
        );
        return;
    }
    let _ = writeln!(out, "Optimization baseline: compiled IR -> final prompt");
    print_optimization(out, "tokens", stats.ir.tokens, stats.final_prompt.tokens);
    print_optimization(out, "UTF-8 bytes", stats.ir.bytes, stats.final_prompt.bytes);
}

fn print_size(out: &mut dyn Write, name: &str, size: pipeline::Size) {
    let _ = writeln!(out, "{name:<18} {:>8} {:>13}", size.tokens, size.bytes);
}

fn print_optimization(out: &mut dyn Write, unit: &str, before: u64, after: u64) {
    if after > before {
        let _ = writeln!(out, "Optimization {unit} added: {}", after - before);
    } else {
        let _ = writeln!(out, "Optimization {unit} saved: {}", before - after);
    }
    if before == 0 || after > before {
        let _ = writeln!(out, "Optimization {unit} savings: N/A");
    } else {
        let percent = 100.0 * (before - after) as f64 / before as f64;
        let _ = writeln!(out, "Optimization {unit} savings: {percent:.2}%");
    }
}

fn atomic_write(path: &Path, has_bom: bool, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    if has_bom {
        temporary.write_all(UTF8_BOM)?;
    }
    temporary.write_all(contents.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

struct CoreSquasher;

impl Squasher for CoreSquasher {
    fn squish(&self, input: &str) -> Result<SquishResult, FileFailure> {
        let result =
            xmlsquish_core::squish(input).map_err(|error| FileFailure::new(error.to_string()))?;
        Ok(SquishResult::new(
            result.output,
            result.stats.recognized as u64,
            result.stats.removed as u64,
            result.stats.inserted as u64,
        ))
    }
}

struct O200kTokens;

impl TokenCounter for O200kTokens {
    fn count(&self, text: &str) -> Result<u64, FileFailure> {
        u64::try_from(o200k_base_singleton().encode_ordinary(text).len())
            .map_err(|_| FileFailure::new("token count exceeds u64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_prints_help_and_succeeds() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(run(["xmlsquish"], &mut out, &mut err), 0);
        assert!(String::from_utf8(out).unwrap().contains("Usage:"));
        assert!(err.is_empty());
    }

    #[test]
    fn processes_bom_without_counting_it() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("bom.xml");
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(b" <a>  x </a> ");
        fs::write(&input, bytes).unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            run(
                [OsString::from("xmlsquish"), input.as_os_str().to_owned()],
                &mut out,
                &mut err
            ),
            0
        );
        let output = fs::read(temp.path().join("bom.o.xml")).unwrap();
        assert!(output.starts_with(UTF8_BOM));
        assert_eq!(&output[UTF8_BOM.len()..], b"<a> x </a>");
        let report = String::from_utf8(out).unwrap();
        assert!(report.contains("Input characters: 13"), "{report}");
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    }

    #[test]
    fn one_bad_file_does_not_prevent_another_file() {
        let temp = tempfile::tempdir().unwrap();
        let bad = temp.path().join("a.xml");
        let good = temp.path().join("b.xml");
        fs::write(&bad, b"\xff").unwrap();
        fs::write(&good, " <b/> ").unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();

        assert_eq!(
            run(
                [
                    OsString::from("xmlsquish"),
                    bad.as_os_str().to_owned(),
                    good.as_os_str().to_owned(),
                ],
                &mut out,
                &mut err
            ),
            1
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("b.o.xml")).unwrap(),
            "<b/>"
        );
        let report = String::from_utf8(out).unwrap();
        assert!(report.contains("Succeeded: 1"));
        assert!(report.contains("Failed: 1"));
    }

    #[test]
    fn replaces_an_existing_output() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("a.xml");
        let output = temp.path().join("a.o.xml");
        fs::write(&input, " <a/> ").unwrap();
        fs::write(&output, "stale output that must disappear").unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(
            run(
                [OsString::from("xmlsquish"), input.as_os_str().to_owned()],
                &mut stdout,
                &mut stderr
            ),
            0
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "<a/>");
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    fn invoke(path: &Path, flag: Option<&str>) -> (i32, String, String) {
        let mut args = vec![OsString::from("xmlsquish")];
        if let Some(flag) = flag {
            args.push(flag.into());
        }
        args.push(path.as_os_str().to_owned());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn intermediate_then_default_output_and_scoped_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        let source = "<?xml version=\"1.0\"?><a><!--gone--><xmlsquish:let x=\"hello\"/><xmlsquish:log msg=\"$x\"/>  text  </a>";
        fs::write(&input, source).unwrap();
        fs::write(dir.path().join("orphan.i.xml"), "untouched").unwrap();
        let (code, out, err) = invoke(dir.path(), Some("-I"));
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("a.xml:1: hello"), "{out}");
        let ir = fs::read_to_string(dir.path().join("a.i.xml")).unwrap();
        assert!(!ir.contains("xmlsquish") && !ir.contains("gone"));
        assert!(ir.contains("  text  "));
        assert!(!dir.path().join("a.o.xml").exists());
        let (code, out, err) = invoke(dir.path(), None);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Processed files: 1"), "{out}");
        assert_eq!(
            fs::read_to_string(dir.path().join("a.o.xml")).unwrap(),
            "<a> text </a>"
        );
        assert!(!dir.path().join("a.i.xml").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("orphan.i.xml")).unwrap(),
            "untouched"
        );
        assert_eq!(fs::read_to_string(input).unwrap(), source);
    }

    #[test]
    fn compile_failure_preserves_previous_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, "<a><xmlsquish:log msg=\"$missing\"/></a>").unwrap();
        fs::write(dir.path().join("a.i.xml"), "old ir").unwrap();
        fs::write(dir.path().join("a.o.xml"), "old output").unwrap();
        let (code, _, err) = invoke(&input, Some("-O"));
        assert_eq!(code, 1);
        assert!(err.contains("undefined"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
            "old ir"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("a.o.xml")).unwrap(),
            "old output"
        );
    }

    #[test]
    fn output_failure_retains_compiled_intermediate() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, "<a>  x  </a>").unwrap();
        fs::create_dir(dir.path().join("a.o.xml")).unwrap();
        let (code, _, err) = invoke(&input, None);
        assert_eq!(code, 1);
        assert!(err.contains("write output"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
            "<a>  x  </a>"
        );
    }

    #[test]
    fn intermediate_preserves_bom_and_explicit_artifacts_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        let mut source = UTF8_BOM.to_vec();
        source.extend_from_slice(b"<a/>");
        fs::write(&input, source).unwrap();
        assert_eq!(invoke(&input, Some("-I")).0, 0);
        let ir = dir.path().join("a.i.xml");
        assert!(fs::read(&ir).unwrap().starts_with(UTF8_BOM));
        let (code, out, err) = invoke(&ir, Some("-O"));
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Processed files: 0"), "{out}");
        assert!(!dir.path().join("a.i.o.xml").exists());
    }

    #[test]
    fn loads_relative_dependencies_with_independent_frames_and_boms() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("parts")).unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, "<root><xmlsquish:let x=\"parent\"/><xmlsquish:mount path=\"parts/mount.xml\"/><xmlsquish:import path=\"parts/import.xml\"/></root>").unwrap();
        let mut mounted = UTF8_BOM.to_vec();
        mounted.extend_from_slice(
            b"<mounted><xmlsquish:let x=\"child\"/><xmlsquish:log msg=\"$x\"/></mounted>",
        );
        fs::write(dir.path().join("parts/mount.xml"), mounted).unwrap();
        fs::write(
            dir.path().join("parts/import.xml"),
            "<discard><child/></discard>",
        )
        .unwrap();
        let (code, out, err) = invoke(&input, Some("-I"));
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("mount.xml:1: child"), "{out}");
        assert_eq!(
            fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
            "<root><mounted></mounted><child/></root>"
        );
    }

    #[test]
    fn set_assigns_existing_locals_in_branch_and_attribute_order() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, r#"<r><xmlsquish:let result="" next=""/><xmlsquish:if lhs="yes" rhs="yes"><xmlsquish:set result="OK" next="$result"/></xmlsquish:if><xmlsquish:log msg="$result/$next"/></r>"#).unwrap();
        for flag in ["-I", "-O"] {
            let (code, out, err) = invoke(&input, Some(flag));
            assert_eq!(code, 0, "{err}");
            assert!(out.contains("a.xml:1: OK/OK"), "{out}");
            let suffix = if flag == "-I" { "a.i.xml" } else { "a.o.xml" };
            assert_eq!(
                fs::read_to_string(dir.path().join(suffix)).unwrap(),
                if flag == "-I" { "<r></r>" } else { "<r> </r>" }
            );
        }
    }

    #[test]
    fn undeclared_set_preserves_existing_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, r#"<r><xmlsquish:set missing="value"/></r>"#).unwrap();
        let output = dir.path().join("a.o.xml");
        fs::write(&output, "previous output").unwrap();
        let (code, _, err) = invoke(&input, Some("-O"));
        assert_eq!(code, 1);
        assert!(
            err.contains("undefined") && err.contains("missing"),
            "{err}"
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "previous output");
        assert!(!dir.path().join("a.i.xml").exists());
    }

    #[test]
    fn included_file_cannot_set_parent_local() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(
            dir.path().join("child.xml"),
            r#"<child><xmlsquish:set result="child"/></child>"#,
        )
        .unwrap();
        for operation in ["mount", "import"] {
            fs::write(&input, format!(r#"<r><xmlsquish:let result="parent"/><xmlsquish:{operation} path="child.xml"/></r>"#)).unwrap();
            let (code, _, err) = invoke(&input, Some("-O"));
            assert_eq!(code, 1);
            assert!(
                err.contains("undefined") && err.contains("result") && err.contains("child.xml"),
                "{err}"
            );
            assert!(!dir.path().join("a.o.xml").exists());
        }
    }

    #[test]
    fn assembly_growth_is_not_reported_as_negative_compression() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        let dependency = format!(
            "<part>{}</part>",
            "a long repeated passage with many tokens. ".repeat(100)
        );
        fs::write(dir.path().join("part.xml"), &dependency).unwrap();
        let source =
            r#"<r><xmlsquish:mount path="part.xml"/><xmlsquish:mount path="part.xml"/></r>"#;
        fs::write(&input, source).unwrap();
        let (code, out, err) = invoke(&input, Some("-O"));
        assert_eq!(code, 0, "{err}");
        let source_tokens = O200kTokens.count(source).unwrap();
        let final_text = fs::read_to_string(dir.path().join("a.o.xml")).unwrap();
        assert!(O200kTokens.count(&final_text).unwrap() > 10 * source_tokens);
        assert!(out.contains("Assembly token ratio (IR / source):"), "{out}");
        assert!(
            !out.contains("compression rate") && !out.contains("-900%"),
            "{out}"
        );
        assert!(out.contains("Dependency loads: 2"), "{out}");
        assert!(out.contains("Unique dependency files: 1"), "{out}");
        assert!(
            out.contains(&format!(
                "Dependency UTF-8 bytes read: {}",
                dependency.len() * 2
            )),
            "{out}"
        );
        assert!(
            out.contains(&format!("Final prompt UTF-8 bytes: {}", final_text.len())),
            "{out}"
        );
    }

    #[test]
    fn actual_optimization_token_growth_is_reported_as_added() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        let source = "<r></r>";
        fs::write(&input, source).unwrap();
        let (code, out, err) = invoke(&input, None);
        assert_eq!(code, 0, "{err}");
        let final_text = fs::read_to_string(dir.path().join("a.o.xml")).unwrap();
        let before = O200kTokens.count(source).unwrap();
        let after = O200kTokens.count(&final_text).unwrap();
        assert!(
            after > before,
            "fixture must increase tokens: {before} -> {after}"
        );
        assert!(
            out.contains(&format!("Optimization tokens added: {}", after - before)),
            "{out}"
        );
        assert!(out.contains("Optimization tokens savings: N/A"), "{out}");
    }

    #[test]
    fn intermediate_report_does_not_claim_optimization() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        fs::write(&input, "<r>    untouched    </r>").unwrap();
        let (code, out, err) = invoke(&input, Some("-I"));
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Optimization: not run (-I)"), "{out}");
        assert!(!out.contains("Optimization tokens saved"), "{out}");
    }

    #[test]
    fn zero_denominators_and_no_successes_are_explicit() {
        let mut out = Vec::new();
        print_report(&mut out, 0, 0, &pipeline::Report::default());
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("Assembly token ratio (IR / source): N/A"),
            "{out}"
        );
        assert!(
            out.contains("Optimization: N/A (no successful files)"),
            "{out}"
        );
        let mut out = Vec::new();
        print_optimization(&mut out, "tokens", 0, 0);
        print_optimization(&mut out, "UTF-8 bytes", 0, 1);
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("Optimization tokens savings: N/A"), "{out}");
        assert!(out.contains("Optimization UTF-8 bytes added: 1"), "{out}");
        assert!(!out.contains("NaN") && !out.contains("inf"), "{out}");
    }

    #[test]
    fn failed_artifacts_do_not_contribute_stage_or_dependency_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.xml");
        let good = dir.path().join("good.xml");
        fs::write(dir.path().join("part.xml"), "<part/>").unwrap();
        fs::write(&bad, r#"<r><xmlsquish:mount path="part.xml"/></r>"#).unwrap();
        fs::create_dir(dir.path().join("bad.o.xml")).unwrap();
        let source = "<good/>";
        fs::write(&good, source).unwrap();
        let report = pipeline::run(&[bad, good], false, &mut Vec::new());
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.stats.processed_files, 1);
        assert_eq!(
            report.stats.source.tokens,
            O200kTokens.count(source).unwrap()
        );
        assert_eq!(report.stats.source.bytes, source.len() as u64);
        assert_eq!(report.stats.final_prompt.bytes, source.len() as u64);
        assert_eq!(report.stats.dependency_loads, 0);
        assert_eq!(report.stats.dependency_bytes, 0);
        assert!(report.stats.dependency_paths.is_empty());
    }

    #[test]
    fn stage_byte_counts_measure_utf8_text_without_bom() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.xml");
        let source = "<r>萌</r>";
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice(source.as_bytes());
        fs::write(&input, bytes).unwrap();
        let report = pipeline::run(&[input], true, &mut Vec::new());
        assert!(report.failures.is_empty());
        assert_eq!(report.stats.source.bytes, source.len() as u64);
        assert_eq!(report.stats.ir.bytes, source.len() as u64);
        assert_eq!(report.stats.final_prompt.bytes, source.len() as u64);
        assert!(report.stats.source.bytes > report.stats.source.characters);
    }

    #[test]
    fn stage_options_are_mutually_exclusive() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            run(["xmlsquish", "-I", "-O", "x.xml"], &mut out, &mut err),
            2
        );
    }

    #[test]
    fn files_have_independent_locals_but_share_compilation_time() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a", "b"] {
            fs::write(
                dir.path().join(format!("{name}.xml")),
                "<r><xmlsquish:let x=\"same\"/><xmlsquish:log msg=\"$sys:time\"/></r>",
            )
            .unwrap();
        }
        let (code, out, err) = invoke(dir.path(), None);
        assert_eq!(code, 0, "{err}");
        let a = out
            .lines()
            .find_map(|line| line.strip_prefix("a.xml:1: "))
            .unwrap();
        let b = out
            .lines()
            .find_map(|line| line.strip_prefix("b.xml:1: "))
            .unwrap();
        assert_eq!(a, b);
    }
}
