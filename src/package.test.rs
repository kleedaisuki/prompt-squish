//! Public library and installed-style binary smoke tests.
//! 公共库接口与独立二进制的冒烟测试。
use std::path::Path;
use std::process::Command;
use xmlsquish::Compiler;

#[test]
fn public_library_compiles_with_an_in_memory_loader() {
    let source = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><root><xs:mount src="child.xml"/></root></xs:module>"#;
    let mut loaded = Vec::new();
    let compiled = Compiler::new()
        .compile(Path::new("virtual/main.xml"), source, |path| {
            loaded.push(path.to_owned());
            Ok(r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><child>  hello  </child></xs:module>"#.into())
        })
        .unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].ends_with(Path::new("virtual/child.xml")));
    assert!(compiled.output.contains("  hello  "));
    assert!(!compiled.output.contains("xs:mount"));
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
