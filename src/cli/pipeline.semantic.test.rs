//! End-to-end module composition / 模块组合端到端回归。
use std::{ffi::OsString, fs, path::Path};

/// Write an explicitly classified source / 写入显式区分入口与库的源码。
fn write(path: &Path, kind: &str, body: &str) {
    fs::write(path, format!(r#"<xs:{kind} xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test">{body}</xs:{kind}>"#)).unwrap();
}

/// Run with retained intermediate artifact / 运行并保留中间产物。
fn invoke(path: &Path) -> (i32, String) {
    let mut err = Vec::new();
    let code = crate::cli::run(
        [
            OsString::from("xmlsquish"),
            OsString::from("--debug"),
            path.as_os_str().to_owned(),
        ],
        &mut Vec::new(),
        &mut err,
    );
    (code, String::from_utf8(err).unwrap())
}

#[test]
fn imported_macro_uses_definition_site_and_explicit_slots() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("lib")).unwrap();
    let main = dir.path().join("main.xml");
    write(
        &main,
        "entry",
        r#"<xs:import src="lib/macros.xml"/><root><xs:expand ref="m:panel"><xs:arg name="title" value="你好 &amp; &lt;世界&gt;"/><xs:fill name="body"><Body> keep </Body></xs:fill></xs:expand></root>"#,
    );
    write(
        &dir.path().join("lib/macros.xml"),
        "module",
        r#"<xs:import src="leaf.xml"/><xs:macro name="m:panel"><xs:param name="title"/><Panel><Title><xs:insert get="arg.title"/></Title><xs:expand ref="m:leaf"/><xs:slot name="body" required="true"/></Panel></xs:macro>"#,
    );
    write(
        &dir.path().join("lib/leaf.xml"),
        "module",
        r#"<xs:macro name="m:leaf"><Source><xs:insert get="file.name"/></Source></xs:macro>"#,
    );
    let (code, err) = invoke(&main);
    assert_eq!(code, 0, "{err}");
    let output = fs::read_to_string(main.with_extension("o.xml"))
        .unwrap()
        .replace(" xmlns=\"\"", "")
        .replace(" xmlns:m=\"urn:test\"", "");
    assert!(output.contains("你好 &amp; &lt;世界&gt;"), "{output}");
    assert!(output.contains("<Source> leaf.xml </Source>"), "{output}");
    assert!(output.contains("<Body> keep </Body>"), "{output}");
    assert!(!output.contains("<xs:"));
    let ir = fs::read_to_string(main.with_extension("i.xml")).unwrap();
    assert_ne!(ir, output);
}

#[test]
fn unicode_recursion_and_failure_do_not_commit_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.xml");
    write(
        &dir.path().join("chars.xml"),
        "module",
        r#"<xs:macro name="m:chars"><xs:param name="text"/><xs:ifr get="arg.text" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><C><xs:insert get="match.head"/></C><xs:expand ref="m:chars"><xs:arg name="text" get="match.tail"/></xs:expand></xs:ifr></xs:macro>"#,
    );
    write(
        &main,
        "entry",
        r#"<xs:import src="chars.xml"/><r><xs:expand ref="m:chars"><xs:arg name="text" value="你好"/></xs:expand></r>"#,
    );
    let (code, err) = invoke(&main);
    assert_eq!(code, 0, "{err}");
    let original = fs::read_to_string(main.with_extension("o.xml")).unwrap();
    assert!(
        original
            .replace(" xmlns=\"\"", "")
            .replace(" xmlns:m=\"urn:test\"", "")
            .contains("<C> 你 </C> <C> 好 </C>"),
        "{original}"
    );
    for broken in [
        r#"<r><xs:insert get="arg.missing"/></r>"#,
        r#"<r><xs:ifr str="x" pattern="["/></r>"#,
        "<a/><b/>",
    ] {
        write(&main, "entry", broken);
        let (code, err) = invoke(&main);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("main.xml"), "{err}");
        assert_eq!(
            fs::read_to_string(main.with_extension("o.xml")).unwrap(),
            original
        );
    }
}
