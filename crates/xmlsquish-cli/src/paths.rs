use glob::{MatchOptions, Pattern, PatternError};
use std::collections::BTreeSet;
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
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let root = glob_root(&pattern_text);

    let mut found = false;
    for entry in WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        match entry {
            Ok(entry) if entry.file_type().is_file() => {
                let path = entry.path();
                let relative = path.strip_prefix(".").unwrap_or(path);
                if (pattern.matches_path_with(path, options)
                    || pattern.matches_path_with(relative, options))
                    && is_input_xml(path)
                {
                    found = true;
                    insert_normalized(path, files);
                }
            }
            Ok(_) => {}
            Err(error) => errors.push(format!("glob 遍历失败：{error}")),
        }
    }
    if !found {
        errors.push(format!("通配符未匹配任何 XML 文件：{pattern_text}"));
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
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name.ends_with(".xml") && !name.ends_with(".o.xml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn recognizes_inputs_and_output_names_case_insensitively() {
        assert!(is_input_xml(Path::new("one.XML")));
        assert!(!is_input_xml(Path::new("one.O.XML")));
        assert!(!is_input_xml(Path::new("one.txt")));
    }

    #[test]
    fn discovers_recursively_sorted_and_excludes_outputs() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("z.xml"), "<z/>").unwrap();
        fs::write(temp.path().join("nested/a.XML"), "<a/>").unwrap();
        fs::write(temp.path().join("nested/a.o.xml"), "ignored").unwrap();
        fs::write(temp.path().join("ignored.txt"), "ignored").unwrap();

        let result = discover(&[temp.path().to_path_buf()]);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.files.len(), 2);
        assert!(result.files[0] < result.files[1]);
    }

    #[test]
    fn duplicate_inputs_collapse() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.xml");
        fs::write(&file, "<a/>").unwrap();
        let result = discover(&[file.clone(), file]);
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn expands_globs_and_filters_output_documents() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.xml"), "<a/>").unwrap();
        fs::write(temp.path().join("b.o.xml"), "<b/>").unwrap();
        fs::write(temp.path().join("c.txt"), "ignored").unwrap();
        let pattern = temp.path().join("*");

        let result = discover(&[pattern]);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.files,
            vec![fs::canonicalize(temp.path().join("a.xml")).unwrap()]
        );
    }
}
