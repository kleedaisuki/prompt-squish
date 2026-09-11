//! Installed-style CLI smoke test. / 独立 CLI 二进制冒烟测试。
use std::process::Command;

#[test]
fn version_matches_release_package_metadata() {
    let output = Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        concat!("xmlsquish ", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn binary_compiles_from_its_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("prompt.xml"), r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><p>  hello  </p></xs:module>"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(dir.path())
        .args(["--color=never", "prompt.xml"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let xml = std::fs::read_to_string(dir.path().join("prompt.o.xml")).unwrap();
    assert!(xml.contains(" hello "));
    assert!(!xml.contains("  hello  "));
    assert!(!xml.contains("xs:module"));
    assert!(!dir.path().join("prompt.i.xml").exists());
}
