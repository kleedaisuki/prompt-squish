//! Native two-stage compilation / 本地两阶段编译。
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use xmlsquish_app::{Squasher, TokenCounter, output_path_for};
use xmlsquish_core::Compiler;

use crate::diagnostics::{Diagnostic, Stage, painted, safe_text};
use crate::{CoreSquasher, O200kTokens, UTF8_BOM, atomic_write};

/// Text metrics exclude the encoding envelope (BOM) / 文本统计不包含 BOM。
#[derive(Clone, Copy, Default)]
pub(crate) struct Size {
    pub tokens: u64,
    pub bytes: u64,
    pub characters: u64,
}

impl Size {
    fn measure(text: &str) -> Result<Self, String> {
        Ok(Self {
            tokens: O200kTokens.count(text).map_err(|error| error.to_string())?,
            bytes: text.len() as u64,
            characters: text.chars().count() as u64,
        })
    }

    fn include(&mut self, other: Self) {
        self.tokens = self.tokens.saturating_add(other.tokens);
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.characters = self.characters.saturating_add(other.characters);
    }
}

#[derive(Default)]
pub(crate) struct Stats {
    pub processed_files: u64,
    pub source: Size,
    pub ir: Size,
    pub final_prompt: Size,
    pub dependency_loads: u64,
    pub dependency_bytes: u64,
    pub dependency_paths: BTreeSet<PathBuf>,
}

impl Stats {
    fn include(&mut self, other: Self) {
        self.processed_files = self.processed_files.saturating_add(other.processed_files);
        self.source.include(other.source);
        self.ir.include(other.ir);
        self.final_prompt.include(other.final_prompt);
        self.dependency_loads = self.dependency_loads.saturating_add(other.dependency_loads);
        self.dependency_bytes = self.dependency_bytes.saturating_add(other.dependency_bytes);
        self.dependency_paths.extend(other.dependency_paths);
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum OutputStage {
    Intermediate,
    #[default]
    Optimized,
}

#[derive(Default)]
pub(crate) struct Report {
    pub stats: Stats,
    pub stage: OutputStage,
    pub failures: Vec<Diagnostic>,
}

#[cfg(test)]
pub(crate) fn run(paths: &[PathBuf], intermediate: bool, logs: &mut dyn Write) -> Report {
    run_with_color(paths, intermediate, logs, false)
}

pub(crate) fn run_with_color(
    paths: &[PathBuf],
    intermediate: bool,
    logs: &mut dyn Write,
    color: bool,
) -> Report {
    // One compiler snapshots system/environment values for the whole invocation.
    // 整次调用共享系统和环境快照；每个源文件仍有独立词法环境。
    let compiler = Compiler::new();
    let mut report = Report {
        stage: if intermediate {
            OutputStage::Intermediate
        } else {
            OutputStage::Optimized
        },
        ..Report::default()
    };
    for path in paths {
        match process_one(&compiler, path, intermediate, logs, color) {
            Ok(file) => report.stats.include(file),
            Err(error) => report.failures.push(*error),
        }
    }
    report
}

fn read_xml(path: &Path) -> Result<(String, bool), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let (bytes, bom) = match bytes.strip_prefix(UTF8_BOM) {
        Some(text) => (text, true),
        None => (bytes.as_slice(), false),
    };
    let text = std::str::from_utf8(bytes).map_err(|error| format!("not valid UTF-8: {error}"))?;
    Ok((text.to_owned(), bom))
}

fn intermediate_path(input: &Path) -> PathBuf {
    input.with_extension("i.xml")
}

fn process_one(
    compiler: &Compiler,
    path: &Path,
    intermediate: bool,
    logs: &mut dyn Write,
    color: bool,
) -> Result<Stats, Box<Diagnostic>> {
    let (source, bom) =
        read_xml(path).map_err(|error| Diagnostic::new(Stage::Read, path, error))?;
    let mut stats = Stats {
        source: Size::measure(&source)
            .map_err(|error| Diagnostic::new(Stage::Measure, path, error))?,
        ..Stats::default()
    };
    let mut snapshots = HashMap::new();
    let compiled = compiler
        .compile(path, &source, |path| {
            let (text, _) = read_xml(path)?;
            stats.dependency_loads = stats.dependency_loads.saturating_add(1);
            stats.dependency_bytes = stats.dependency_bytes.saturating_add(text.len() as u64);
            stats
                .dependency_paths
                .insert(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
            snapshots.insert(path.to_path_buf(), text.clone());
            Ok(text)
        })
        .map_err(|error| {
            let text = if error.path == path {
                Some(source.as_str())
            } else {
                snapshots.get(&error.path).map(String::as_str)
            };
            Diagnostic::compile(error, path, text)
        })?;
    for log in compiled.logs {
        let name = log.path.file_name().unwrap_or(log.path.as_os_str());
        let prefix = format!("{}:{}:", safe_text(&name.to_string_lossy()), log.line);
        let _ = writeln!(
            logs,
            "{} {}",
            painted(&prefix, "1;36", color),
            safe_text(&log.message)
        );
    }
    let ir_path = intermediate_path(path);
    stats.ir = Size::measure(&compiled.output)
        .map_err(|error| Diagnostic::new(Stage::Measure, path, error))?;
    atomic_write(&ir_path, bom, &compiled.output)
        .map_err(|error| Diagnostic::new(Stage::WriteIntermediate, &ir_path, error))?;
    if intermediate {
        stats.final_prompt = stats.ir;
    } else {
        let squished = CoreSquasher
            .squish(&compiled.output)
            .map_err(|error| Diagnostic::new(Stage::Squish, &ir_path, error))?;
        stats.final_prompt = Size::measure(&squished.text)
            .map_err(|error| Diagnostic::new(Stage::Measure, &ir_path, error))?;
        let output_path = output_path_for(path);
        atomic_write(&output_path, bom, &squished.text)
            .map_err(|error| Diagnostic::new(Stage::WriteOutput, &output_path, error))?;
        // Only discard this input's IR after its final output is persisted.
        // 仅在最终文件成功持久化后删除本输入的中间产物。
        fs::remove_file(&ir_path)
            .map_err(|error| Diagnostic::new(Stage::Cleanup, &ir_path, error))?;
    }
    stats.processed_files = 1;
    Ok(stats)
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn included_error_uses_loaded_snapshot_even_when_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let primary = dir.path().join("main.xml");
        let child = dir.path().join("child.xml");
        fs::write(&primary, r#"<r><xmlsquish:mount path="child.xml"/></r>"#).unwrap();
        fs::write(&child, "<r>\n<xmlsquish:log msg=\"$missing\"/>\n</r>").unwrap();
        let report = run(&[primary], false, &mut Vec::new());
        assert_eq!(report.failures.len(), 1);
        fs::write(&child, "replaced after compilation").unwrap();
        let mut output = Vec::new();
        report.failures[0].render(&mut output, false).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("child.xml:2"), "{output}");
        assert!(
            output.contains("2 | <xmlsquish:log msg=\"$missing\"/>"),
            "{output}"
        );
        assert!(output.contains("note: while compiling"), "{output}");
        assert!(!output.contains("replaced after compilation"));
        assert!(!output.contains("[compile] error"));
    }
}
