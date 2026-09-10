//! Module-local regression tests. / 紧邻模块的回归测试。
use super::*;

#[test]
fn included_error_uses_loaded_snapshot_even_when_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let primary = dir.path().join("main.xml");
    let child = dir.path().join("child.xml");
    fs::write(&primary, r#"<r><xmlsquish:mount path="child.xml"/></r>"#).unwrap();
    fs::write(&child, "<r>\n<xmlsquish:log msg=\"$missing\"/>\n</r>").unwrap();
    let report = run(&[primary], OutputStage::Optimized, &mut Vec::new());
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
