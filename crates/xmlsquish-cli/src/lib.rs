mod paths;

use clap::{CommandFactory, Parser};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::NamedTempFile;
use tiktoken_rs::o200k_base_singleton;
use xmlsquish_app::{
    BatchProcessor, FileFailure, FileStore, ProcessingStage, Squasher, SquishResult, TokenCounter,
    output_path_for,
};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Parser)]
#[command(
    name = "xmlsquish",
    version,
    about = "Compress XML prompt whitespace without changing markup",
    long_about = "Compress XML prompt whitespace using a lexical finite-state machine.\n\
                  Files are written beside their inputs as *.o.xml; directories are searched recursively."
)]
pub struct Args {
    /// Input XML files, directories, or glob patterns
    #[arg(value_name = "PATH", allow_hyphen_values = true)]
    paths: Vec<PathBuf>,
}

/// Runs the CLI with injectable streams. Returns the intended process exit code.
pub fn run<I, T>(args: I, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = match Args::try_parse_from(args) {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            let _ = if error.use_stderr() {
                write!(stderr, "{error}")
            } else {
                write!(stdout, "{error}")
            };
            return code;
        }
    };

    if args.paths.is_empty() {
        let mut command = Args::command();
        let mut help = Vec::new();
        let _ = command.write_long_help(&mut help);
        let _ = stdout.write_all(&help);
        let _ = writeln!(stdout);
        return 0;
    }

    let discovery = paths::discover(&args.paths);
    for error in &discovery.errors {
        let _ = writeln!(stderr, "discovery: {error}");
    }

    let processor = BatchProcessor::new(NativeFiles::default(), CoreSquasher, O200kTokens);
    let report = processor.run(&discovery.files);

    for failure in &report.failures {
        let _ = writeln!(
            stderr,
            "{} [{}]: {}",
            failure.path.display(),
            stage_name(failure.stage),
            failure.error
        );
    }
    print_report(
        stdout,
        discovery.files.len(),
        discovery.errors.len(),
        &report,
    );

    if discovery.errors.is_empty() && report.failures.is_empty() {
        0
    } else {
        1
    }
}

fn stage_name(stage: ProcessingStage) -> &'static str {
    match stage {
        ProcessingStage::Read => "read",
        ProcessingStage::Squish => "squish",
        ProcessingStage::CountInputTokens => "input tokens",
        ProcessingStage::CountOutputTokens => "output tokens",
        ProcessingStage::Write => "write",
    }
}

fn print_report(
    out: &mut dyn Write,
    discovered: usize,
    discovery_failures: usize,
    report: &xmlsquish_app::BatchReport,
) {
    let stats = report.stats;
    let failures = discovery_failures.saturating_add(report.failures.len());
    let _ = writeln!(out, "Encoding: o200k_base");
    let _ = writeln!(out, "Processed files: {discovered}");
    let _ = writeln!(out, "Succeeded: {}", stats.processed_files);
    let _ = writeln!(out, "Failed: {failures}");
    let _ = writeln!(out, "Discovery errors: {discovery_failures}");
    let _ = writeln!(out, "Input tokens: {}", stats.input_tokens);
    let _ = writeln!(out, "Output tokens: {}", stats.output_tokens);
    let _ = writeln!(out, "Input characters: {}", stats.input_characters);
    let _ = writeln!(out, "Output characters: {}", stats.output_characters);
    let _ = writeln!(
        out,
        "Recognized whitespace: {}",
        stats.recognized_whitespace
    );
    let _ = writeln!(out, "Removed whitespace: {}", stats.removed_whitespace);
    let _ = writeln!(out, "Inserted whitespace: {}", stats.inserted_whitespace);
    if stats.input_tokens == 0 {
        let _ = writeln!(out, "Token compression rate: N/A");
    } else {
        let rate = 100.0 * (stats.input_tokens as f64 - stats.output_tokens as f64)
            / stats.input_tokens as f64;
        let _ = writeln!(out, "Token compression rate: {rate:.2}%");
    }
}

#[derive(Default)]
struct NativeFiles {
    /// `FileStore` intentionally exposes text, so remember the encoding envelope
    /// between read(input) and write(output). BatchProcessor calls these in order.
    output_boms: Mutex<HashMap<PathBuf, bool>>,
}

impl FileStore for NativeFiles {
    fn read(&self, path: &Path) -> Result<String, FileFailure> {
        let bytes = fs::read(path)
            .map_err(|error| FileFailure::new(format!("{}: {error}", path.display())))?;
        let (has_bom, xml) = match bytes.strip_prefix(UTF8_BOM) {
            Some(xml) => (true, xml),
            None => (false, bytes.as_slice()),
        };
        let text = std::str::from_utf8(xml).map_err(|error| {
            FileFailure::new(format!(
                "{} is not valid UTF-8 (byte {}): {error}",
                path.display(),
                error.valid_up_to()
            ))
        })?;
        self.output_boms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(output_path_for(path), has_bom);
        Ok(text.to_owned())
    }

    fn write(&self, path: &Path, contents: &str) -> Result<(), FileFailure> {
        let has_bom = self
            .output_boms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(path)
            .unwrap_or(false);
        atomic_write(path, has_bom, contents)
            .map_err(|error| FileFailure::new(format!("{}: {error}", path.display())))
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
