//! Module-local regression tests. / 紧邻模块的回归测试。
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
        vec![logical_absolute(&temp.path().join("a.xml")).unwrap()]
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

#[test]
fn colliding_outputs_reject_both_inputs_deterministically() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.xml"), "<lower/>").unwrap();
    fs::write(temp.path().join("a.XML"), "<upper/>").unwrap();

    // Explicit logical spellings collide even on case-insensitive volumes.
    // 显式逻辑路径在大小写不敏感卷上也产生同名输出，不依赖目录能否存储两个文件。
    let result = discover(&[temp.path().join("a.xml"), temp.path().join("a.XML")]);
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

#[test]
fn lexical_dot_segments_collapse_without_filesystem_identity() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("a.xml");
    fs::write(&file, "<a/>").unwrap();
    let result = discover(&[file.clone(), temp.path().join("unused/../a.xml")]);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.files, vec![logical_absolute(&file).unwrap()]);
}

/// Symlink permissions are optional on Windows, mandatory on Unix.
/// Windows 符号链接权限可能不可用；Unix 创建失败必须使测试失败。
#[cfg(any(unix, windows))]
fn link_file(source: &Path, alias: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, alias).unwrap();
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(source, alias) {
            Ok(()) => true,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1314) =>
            {
                eprintln!("SKIP: Windows symlink privilege unavailable: {error}");
                false
            }
            Err(error) => panic!("cannot create symlink: {error}"),
        }
    }
}

#[cfg(any(unix, windows))]
#[test]
fn symlink_root_keeps_logical_definition_base_and_distinct_identity() {
    let temp = tempfile::tempdir().unwrap();
    let physical = temp.path().join("physical");
    let logical = temp.path().join("logical");
    fs::create_dir(&physical).unwrap();
    fs::create_dir(&logical).unwrap();
    let source = physical.join("main.xml");
    let alias = logical.join("alias.xml");
    fs::write(&source, r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:import src="child.xml"/><xs:import src="child-alias.xml"/><r><xs:insert get="file.name"/><child/></r></xs:entry>"#).unwrap();
    fs::write(
        logical.join("child.xml"),
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"/>"#,
    )
    .unwrap();
    if !link_file(&source, &alias) {
        return;
    }
    assert!(link_file(
        &logical.join("child.xml"),
        &logical.join("child-alias.xml")
    ));
    let found = discover(&[source.clone(), alias.clone()]);
    assert!(found.errors.is_empty(), "{:?}", found.errors);
    assert_eq!(found.files.len(), 2);
    assert!(found.files.contains(&logical_absolute(&alias).unwrap()));
    let mut err = Vec::new();
    let mut out = Vec::new();
    let code = crate::cli::run(
        [
            std::ffi::OsString::from("xmlsquish"),
            alias.into_os_string(),
        ],
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
    let report = String::from_utf8(out).unwrap();
    assert!(report.contains("Unique dependency files: 2"), "{report}");
    let output = fs::read_to_string(logical.join("alias.o.xml")).unwrap();
    assert!(output.contains("alias.xml"), "{output}");
    assert!(output.contains("child"), "{output}");
    assert!(!physical.join("main.o.xml").exists());
}
