//! Public library and installed-style binary smoke tests.
//! 公共库接口与独立二进制的冒烟测试。
use std::path::Path;
use std::process::Command;
use xmlsquish::{Compiler, squish};

#[test]
fn public_library_compiles_with_an_in_memory_loader() {
    let source = "<root><xmlsquish:mount path=\"child.xml\"/></root>";
    let mut loaded = Vec::new();
    let compiled = Compiler::new()
        .compile(Path::new("virtual/main.xml"), source, |path| {
            loaded.push(path.to_owned());
            Ok("<child>  hello  </child>".into())
        })
        .unwrap();
    assert_eq!(loaded, [Path::new("virtual/child.xml")]);
    assert_eq!(
        squish(&compiled.output).unwrap().output,
        "<root> <child> hello </child> </root>"
    );
}

#[test]
fn binary_compiles_from_its_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("prompt.xml"), "<p>  hello  </p>").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xmlsquish"))
        .current_dir(dir.path())
        .args(["--color=never", "prompt.xml"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("prompt.o.xml")).unwrap(),
        "<p> hello </p>"
    );
    assert!(!dir.path().join("prompt.i.xml").exists());
}
