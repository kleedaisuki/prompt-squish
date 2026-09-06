use glob::{MatchOptions, Pattern, PatternError};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use xmlsquish_app::output_path_for;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn recognizes_inputs_and_output_names_case_insensitively() {
        assert!(is_input_xml(Path::new("one.XML")));
        assert!(!is_input_xml(Path::new("one.O.XML")));
        assert!(!is_input_xml(Path::new("one.I.XML")));
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
        fs::write(temp.path().join("nested/a.i.xml"), "ignored").unwrap();

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

    #[test]
    fn single_star_does_not_cross_a_separator_but_double_star_does() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("top.xml"), "<top/>").unwrap();
        fs::write(temp.path().join("nested/deep.xml"), "<deep/>").unwrap();

        let local = discover(&[temp.path().join("*.xml")]);
        assert!(local.errors.is_empty(), "{:?}", local.errors);
        assert_eq!(local.files.len(), 1);
        assert!(local.files[0].ends_with("top.xml"));

        let recursive = discover(&[temp.path().join("**/*.xml")]);
        assert!(recursive.errors.is_empty(), "{:?}", recursive.errors);
        assert_eq!(recursive.files.len(), 2);
    }

    #[test]
    fn a_globbed_directory_is_processed_recursively() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("chosen/nested")).unwrap();
        fs::write(temp.path().join("chosen/nested/deep.xml"), "<deep/>").unwrap();

        let result = discover(&[temp.path().join("cho*")]);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].ends_with("deep.xml"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_file_names_keep_ascii_suffix_semantics() {
        use std::os::unix::ffi::OsStringExt;

        let input = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff.XML".to_vec()));
        let output = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff.O.XML".to_vec()));
        assert!(is_input_xml(&input));
        assert!(!is_input_xml(&output));
    }

    #[cfg(unix)]
    #[test]
    fn colliding_outputs_reject_both_inputs_deterministically() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.xml"), "<lower/>").unwrap();
        fs::write(temp.path().join("a.XML"), "<upper/>").unwrap();

        let result = discover(&[temp.path().to_path_buf()]);
        assert!(result.files.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("a.o.xml"));
        let lower = result.errors[0].find("a.XML").unwrap();
        let upper = result.errors[0].find("a.xml").unwrap();
        assert!(
            lower < upper,
            "conflict inputs must be sorted: {}",
            result.errors[0]
        );
    }
}
