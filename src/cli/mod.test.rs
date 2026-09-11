//! Module-local regression tests. / 紧邻模块的回归测试。
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
    bytes.extend_from_slice(
        b"<xs:module xmlns:xs=\"https://xmlsquish.moesegfault.dev/ns\" xmlns:m=\"urn:test\" entry=\"m:main\"><xs:macro name=\"m:main\"><a>  x </a></xs:macro></xs:module>",
    );
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
    assert_eq!(
        String::from_utf8(output[UTF8_BOM.len()..].to_vec())
            .unwrap()
            .replace(" xmlns=\"\"", ""),
        "<a> x </a>"
    );
    let report = String::from_utf8(out).unwrap();
    assert!(report.contains("Input characters:"), "{report}");
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
}

#[test]
fn one_bad_file_does_not_prevent_another_file() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("a.xml");
    let good = temp.path().join("b.xml");
    fs::write(&bad, b"\xff").unwrap();
    fs::write(
        &good,
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:main"><xs:macro name="m:main"><b/></xs:macro></xs:module>"#,
    )
    .unwrap();
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
        fs::read_to_string(temp.path().join("b.o.xml"))
            .unwrap()
            .replace(" xmlns=\"\"", ""),
        "<b> </b>"
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
    fs::write(
        &input,
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:main"><xs:macro name="m:main"><a/></xs:macro></xs:module>"#,
    )
    .unwrap();
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
    assert_eq!(
        fs::read_to_string(output)
            .unwrap()
            .replace(" xmlns=\"\"", ""),
        "<a> </a>"
    );
}
