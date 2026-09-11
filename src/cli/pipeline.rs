//! Native two-stage compilation / 本地两阶段编译。
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{CompileOptions, Compiler};
use tiktoken_rs::o200k_base_singleton;

use super::diagnostics::{Diagnostic, Stage, painted, safe_text};
use super::files::{atomic_write, read_xml};
use super::paths::{intermediate_path, logical_absolute, output_path_for};

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
            tokens: u64::try_from(o200k_base_singleton().encode_ordinary(text).len())
                .map_err(|_| "token count exceeds u64".to_owned())?,
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
pub(crate) fn run(paths: &[PathBuf], stage: OutputStage, logs: &mut dyn Write) -> Report {
    run_with_color(paths, stage, logs, false, CompileOptions::default(), false)
}

pub(crate) fn run_with_color(
    paths: &[PathBuf],
    stage: OutputStage,
    logs: &mut dyn Write,
    color: bool,
    options: CompileOptions,
    debug: bool,
) -> Report {
    // Each compile freezes its own source closure and immutable root arguments.
    // 每次编译冻结独立源码闭包，复用只读根参数和预算配置。
    let max_output_bytes = options.max_output_bytes;
    let compiler = Compiler::with_options(options);
    let mut report = Report {
        stage,
        ..Report::default()
    };
    for path in paths {
        match process_one(&compiler, path, stage, logs, color, debug, max_output_bytes) {
            Ok(file) => report.stats.include(file),
            Err(error) => report.failures.push(*error),
        }
    }
    report
}

fn process_one(
    compiler: &Compiler,
    path: &Path,
    stage: OutputStage,
    logs: &mut dyn Write,
    color: bool,
    debug: bool,
    max_output_bytes: usize,
) -> Result<Stats, Box<Diagnostic>> {
    let logical_path =
        logical_absolute(path).map_err(|error| Diagnostic::new(Stage::Read, path, error))?;
    let path = logical_path.as_path();
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
            stats.dependency_paths.insert(path.to_path_buf());
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
    // Validate final-stage growth before committing either artifact.
    // 最终压缩可能新增分隔空格，写入任何产物前也必须验证其预算。
    let final_output = if stage == OutputStage::Optimized {
        let output = crate::squish(&compiled.output)
            .map_err(|error| Diagnostic::new(Stage::Squish, path, error))?
            .output;
        if output.len() > max_output_bytes {
            return Err(Box::new(Diagnostic::compile(
                compiled.output_error(format!(
                    "Expansion: final output exceeds max-output-bytes budget {max_output_bytes}"
                )),
                path,
                Some(&source),
            )));
        }
        output
    } else {
        String::new()
    };
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
    stats.ir = Size::measure(&compiled.intermediate)
        .map_err(|error| Diagnostic::new(Stage::Measure, path, error))?;
    atomic_write(&ir_path, bom, &compiled.intermediate)
        .map_err(|error| Diagnostic::new(Stage::WriteIntermediate, &ir_path, error))?;
    if stage == OutputStage::Intermediate {
        stats.final_prompt = stats.ir;
    } else {
        stats.final_prompt = Size::measure(&final_output)
            .map_err(|error| Diagnostic::new(Stage::Measure, &ir_path, error))?;
        let output_path = output_path_for(path);
        atomic_write(&output_path, bom, &final_output)
            .map_err(|error| Diagnostic::new(Stage::WriteOutput, &output_path, error))?;
        // Only discard this input's IR after its final output is persisted.
        // 仅在最终文件成功持久化后删除本输入的中间产物。
        if !debug {
            fs::remove_file(&ir_path)
                .map_err(|error| Diagnostic::new(Stage::Cleanup, &ir_path, error))?;
        }
    }
    stats.processed_files = 1;
    Ok(stats)
}

#[cfg(test)]
#[path = "pipeline.test.rs"]
mod diagnostic_tests;
