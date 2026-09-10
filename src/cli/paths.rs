//! Input discovery and sibling artifact names.
//! 输入发现与同目录产物命名；全程保留非 UTF-8 路径。
use glob::{MatchOptions, Pattern, PatternError};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug)]
pub struct Discovery {
    pub files: Vec<PathBuf>,
    pub errors: Vec<String>,
}

pub fn discover(inputs: &[PathBuf]) -> Discovery {
    let mut files = BTreeSet::new();
    let mut errors = Vec::new();

    for input in inputs {
        if has_glob_syntax(input.as_os_str()) {
            expand_glob(input, &mut files, &mut errors);
        } else {
            collect_path(input, &mut files, &mut errors);
        }
    }

    reject_output_collisions(&mut files, &mut errors);

    Discovery {
        files: files.into_iter().collect(),
        errors,
    }
}

fn has_glob_syntax(value: &OsStr) -> bool {
    value
        .to_string_lossy()
        .bytes()
        .any(|b| matches!(b, b'*' | b'?' | b'['))
}

fn expand_glob(pattern: &Path, files: &mut BTreeSet<PathBuf>, errors: &mut Vec<String>) {
    let pattern_text = pattern.to_string_lossy();
    let pattern = match Pattern::new(&pattern_text) {
        Ok(pattern) => pattern,
        Err(error) => {
            errors.push(format_glob_error(&pattern_text, error));
            return;
        }
    };
    let options = MatchOptions {
        case_sensitive: cfg!(not(windows)),
        // A single `*` is local to one path component. Recursive matching is
        // expressed explicitly with `**`, just like conventional shell globs.
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    let root = glob_root(&pattern_text);

    let mut found = false;
    let mut matched_directories = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                let relative = path.strip_prefix(".").unwrap_or(path);
                if pattern.matches_path_with(path, options)
                    || pattern.matches_path_with(relative, options)
                {
                    found = true;
                    if entry.file_type().is_file() && is_input_xml(path) {
                        insert_normalized(path, files);
                    } else if entry.file_type().is_dir() {
                        matched_directories.push(path.to_path_buf());
                    }
                }
            }
            Err(error) => errors.push(format!("glob 遍历失败：{error}")),
        }
    }
    for directory in matched_directories {
        collect_path(&directory, files, errors);
    }
    if !found {
        errors.push(format!("通配符未匹配任何路径：{pattern_text}"));
    }
}

fn glob_root(pattern: &str) -> PathBuf {
    let wildcard = pattern
        .char_indices()
        .find_map(|(index, character)| matches!(character, '*' | '?' | '[').then_some(index))
        .unwrap_or(pattern.len());
    let literal_prefix = &pattern[..wildcard];
    let directory_end = literal_prefix
        .char_indices()
        .rev()
        .find_map(|(index, character)| matches!(character, '/' | '\\').then_some(index));

    match directory_end {
        Some(0) => PathBuf::from(&literal_prefix[..1]),
        Some(2) if literal_prefix.as_bytes().get(1) == Some(&b':') => {
            PathBuf::from(&literal_prefix[..3])
        }
        Some(index) => PathBuf::from(&literal_prefix[..index]),
        None => PathBuf::from("."),
    }
}

fn format_glob_error(pattern: &str, error: PatternError) -> String {
    format!("无效通配符 {pattern:?}：{error}")
}

fn collect_path(path: &Path, files: &mut BTreeSet<PathBuf>, errors: &mut Vec<String>) {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            errors.push(format!("无法访问 {}：{error}", path.display()));
            return;
        }
    };

    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        if is_input_xml(path) {
            insert_normalized(path, files);
        }
        return;
    }
    if !metadata.is_dir() {
        errors.push(format!("不是普通文件或目录：{}", path.display()));
        return;
    }

    for entry in WalkDir::new(path).follow_links(false).sort_by_file_name() {
        match entry {
            Ok(entry) if entry.file_type().is_file() && is_input_xml(entry.path()) => {
                insert_normalized(entry.path(), files);
            }
            Ok(_) => {}
            Err(error) => errors.push(format!("遍历 {} 失败：{error}", path.display())),
        }
    }
}

fn insert_normalized(path: &Path, files: &mut BTreeSet<PathBuf>) {
    // Canonicalization makes duplicate spellings of the same input collapse. If the
    // filesystem cannot canonicalize an otherwise readable path, retain that path so
    // processing can report the more useful I/O error later.
    files.insert(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
}

pub fn is_input_xml(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    os_ends_with_ascii_case_insensitive(name, b".xml")
        && !os_ends_with_ascii_case_insensitive(name, b".o.xml")
        && !os_ends_with_ascii_case_insensitive(name, b".i.xml")
}

#[cfg(unix)]
fn os_ends_with_ascii_case_insensitive(value: &OsStr, suffix: &[u8]) -> bool {
    use std::os::unix::ffi::OsStrExt;

    value
        .as_bytes()
        .get(value.as_bytes().len().saturating_sub(suffix.len())..)
        .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
}

#[cfg(windows)]
fn os_ends_with_ascii_case_insensitive(value: &OsStr, suffix: &[u8]) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let suffix: Vec<u16> = suffix.iter().map(|&byte| u16::from(byte)).collect();
    let value: Vec<u16> = value.encode_wide().collect();
    value
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|ending| {
            ending.iter().zip(&suffix).all(|(&left, &right)| {
                u8::try_from(left)
                    .is_ok_and(|left| left.eq_ignore_ascii_case(&u8::try_from(right).unwrap()))
            })
        })
}

#[cfg(not(any(unix, windows)))]
fn os_ends_with_ascii_case_insensitive(value: &OsStr, suffix: &[u8]) -> bool {
    value
        .to_str()
        .is_some_and(|value| value.as_bytes().ends_with(suffix))
}

fn reject_output_collisions(files: &mut BTreeSet<PathBuf>, errors: &mut Vec<String>) {
    let mut by_output: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for input in files.iter() {
        by_output
            .entry(output_path_for(input))
            .or_default()
            .push(input.clone());
    }

    for (output, inputs) in by_output {
        if inputs.len() < 2 {
            continue;
        }
        for input in &inputs {
            files.remove(input);
        }
        let inputs = inputs
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        errors.push(format!("输出路径冲突 {}：{inputs}", output.display()));
    }
}

/// Derives the optimized sibling path without decoding its name.
/// 无需解码文件名，生成同目录优化产物路径。
pub(super) fn output_path_for(input: &Path) -> PathBuf {
    input.with_extension("o.xml")
}

/// Derives the compiled intermediate sibling path.
/// 生成同目录编译中间产物路径。
pub(super) fn intermediate_path(input: &Path) -> PathBuf {
    input.with_extension("i.xml")
}

#[cfg(test)]
#[path = "paths.test.rs"]
mod tests;
