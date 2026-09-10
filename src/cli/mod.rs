//! Command-line orchestration and terminal presentation.
//! 命令行编排与终端展示；文件系统操作不属于编译器的公共契约。

mod console;
mod diagnostics;
mod files;
mod paths;
mod pipeline;

#[cfg(test)]
use self::files::UTF8_BOM;
use clap::{CommandFactory, Parser};
use console::{ColorMode, section, styled};
use std::ffi::OsString;
#[cfg(test)]
use std::fs;
use std::io::{Write, stderr, stdout};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "xmlsquish",
    version,
    about = "Compile XML prompt macros and compress whitespace",
    long_about = "Compile XML macros to *.i.xml, then compress whitespace to *.o.xml.\n\
                  Use -I to retain only the intermediate stage; -O is the default.\n\
                  Directories are searched recursively, ignoring *.i.xml and *.o.xml."
)]
struct Args {
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
/// 使用可注入的输出流运行命令行，返回进程退出码。
///
/// ```
/// let (mut out, mut err) = (Vec::new(), Vec::new());
/// assert_eq!(xmlsquish::cli::run(["xmlsquish", "--help"], &mut out, &mut err), 0);
/// ```
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
    let mut stdout = color.stream(stdout()).lock();
    let mut stderr = color.stream(stderr()).lock();
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

    let stage = if args.intermediate {
        pipeline::OutputStage::Intermediate
    } else {
        pipeline::OutputStage::Optimized
    };
    let report = pipeline::run_with_color(&discovery.files, stage, &mut stdout, out_color);
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

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

#[cfg(test)]
#[path = "mod.pipeline_tests.test.rs"]
mod pipeline_tests;
