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

/// A source file is not an implicit executable macro.
/// 源文件不是隐式可执行宏；库模块必须由显式入口引用。
#[test]
fn library_without_entry_is_not_a_cli_program() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("library.xml"),
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:macro name="m:main"><p/></xs:macro></xs:module>"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(dir.path())
        .args(["--color=never", "library.xml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("explicit entry"), "{diagnostic}");
    assert!(!dir.path().join("library.o.xml").exists());
    assert!(!dir.path().join("library.i.xml").exists());
}

#[test]
fn binary_compiles_from_its_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("prompt.xml"), r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:main"><xs:macro name="m:main"><p>  hello  </p></xs:macro></xs:module>"#).unwrap();
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

/// Final prompts contain no attribute metadata, even with debug enabled.
/// 即使启用调试，最终提示词也不包含属性元数据。
#[test]
fn final_prompt_strips_attributes_and_keeps_whitespace_compression() {
    let dir = tempfile::tempdir().unwrap();
    let source = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:demo="https://example.com/macros/demo" xmlns:m="urn:test" entry="m:main"><xs:macro name="m:main"><prompt id="discard"><Persona xml:space="preserve"><audience>  everyone  </audience><voice> clear &amp; kind </voice></Persona><demo:task role="ignored"> Explain the trade-offs. </demo:task></prompt></xs:macro></xs:module>"#;
    std::fs::write(dir.path().join("prompt.xml"), source).unwrap();
    for debug in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xmlsquish"));
        command
            .current_dir(dir.path())
            .args(["--color=never", "prompt.xml"]);
        if debug {
            command.arg("--debug");
        }
        let result = command.output().unwrap();
        assert!(result.status.success(), "{:?}", result);
        let output = std::fs::read_to_string(dir.path().join("prompt.o.xml")).unwrap();
        assert_eq!(
            output,
            "<prompt> <Persona> <audience> everyone </audience> <voice> clear &amp; kind </voice> </Persona> <task> Explain the trade-offs. </task> </prompt>"
        );
    }
    let intermediate = std::fs::read_to_string(dir.path().join("prompt.i.xml")).unwrap();
    assert!(intermediate.contains("discard"));
}
