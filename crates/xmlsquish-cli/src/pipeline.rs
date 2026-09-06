//! Native two-stage compilation / 本地两阶段编译。
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use xmlsquish_app::{
    BatchStats, FileReport, Squasher, SquishResult, TokenCounter, output_path_for,
};
use xmlsquish_core::Compiler;

use crate::{CoreSquasher, O200kTokens, UTF8_BOM, atomic_write};

#[derive(Default)]
pub(crate) struct Report {
    pub stats: BatchStats,
    pub failures: Vec<String>,
}

pub(crate) fn run(paths: &[PathBuf], intermediate: bool, logs: &mut dyn Write) -> Report {
    // One compiler snapshots system/environment values for the whole invocation.
    // 整次调用共享系统和环境快照；每个源文件仍有独立词法环境。
    let compiler = Compiler::new();
    let mut report = Report::default();
    for path in paths {
        match process_one(&compiler, path, intermediate, logs) {
            Ok(file) => report.stats.include(&file),
            Err(error) => report.failures.push(format!("{}: {error}", path.display())),
        }
    }
    report
}

fn read_xml(path: &Path) -> Result<(String, bool), String> {
    let bytes = fs::read(path).map_err(|error| format!("[read] {}: {error}", path.display()))?;
    let (bytes, bom) = match bytes.strip_prefix(UTF8_BOM) {
        Some(text) => (text, true),
        None => (bytes.as_slice(), false),
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("[read] {} is not valid UTF-8: {error}", path.display()))?;
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
) -> Result<FileReport, String> {
    let (source, bom) = read_xml(path)?;
    let compiled = compiler
        .compile(path, &source, |path| read_xml(path).map(|(text, _)| text))
        .map_err(|error| format!("[compile] {error}"))?;
    for log in compiled.logs {
        let name = log.path.file_name().unwrap_or(log.path.as_os_str());
        let _ = writeln!(
            logs,
            "{}:{}: {}",
            name.to_string_lossy(),
            log.line,
            log.message
        );
    }
    let ir_path = intermediate_path(path);
    let input_tokens = O200kTokens
        .count(&source)
        .map_err(|error| error.to_string())?;
    atomic_write(&ir_path, bom, &compiled.output)
        .map_err(|error| format!("[write intermediate] {}: {error}", ir_path.display()))?;
    let squished = if intermediate {
        SquishResult::new(compiled.output.clone(), 0, 0, 0)
    } else {
        CoreSquasher
            .squish(&compiled.output)
            .map_err(|error| format!("[squish] {error}"))?
    };
    let output_tokens = O200kTokens
        .count(&squished.text)
        .map_err(|error| error.to_string())?;
    let output_path = if intermediate {
        ir_path.clone()
    } else {
        output_path_for(path)
    };
    if !intermediate {
        atomic_write(&output_path, bom, &squished.text)
            .map_err(|error| format!("[write output] {}: {error}", output_path.display()))?;
        // Only discard this input's IR after its final output is persisted.
        // 仅在最终文件成功持久化后删除本输入的中间产物。
        fs::remove_file(&ir_path)
            .map_err(|error| format!("[cleanup] {}: {error}", ir_path.display()))?;
    }
    Ok(FileReport {
        input_path: path.to_path_buf(),
        output_path,
        input_tokens,
        output_tokens,
        input_characters: source.chars().count() as u64,
        output_characters: squished.text.chars().count() as u64,
        recognized_whitespace: squished.recognized_whitespace,
        removed_whitespace: squished.removed_whitespace,
        inserted_whitespace: squished.inserted_whitespace,
    })
}
