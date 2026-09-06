//! End-to-end language examples / 语言示例的端到端回归。
use std::ffi::OsString;
use std::fs;
use std::path::Path;

fn invoke(path: &Path, stage: &str) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = xmlsquish_cli::run(
        [
            OsString::from("xmlsquish"),
            OsString::from(stage),
            path.as_os_str().to_owned(),
        ],
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn documented_inheritance_example_compiles_in_both_stages() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("parts")).unwrap();
    let files = [
        (
            "prompt.xml",
            include_str!("../../../examples/inheritance/prompt.xml"),
        ),
        (
            "parts/section.xml",
            include_str!("../../../examples/inheritance/parts/section.xml"),
        ),
        (
            "parts/leaf.xml",
            include_str!("../../../examples/inheritance/parts/leaf.xml"),
        ),
    ];
    for (name, source) in files {
        fs::write(dir.path().join(name), source).unwrap();
    }
    let source = dir.path().join("prompt.xml");
    let (code, report, errors) = invoke(&source, "-I");
    assert_eq!(code, 0, "{errors}");
    assert!(report.contains("Optimization: not run (-I)"));
    let ir_path = source.with_extension("i.xml");
    let ir = fs::read_to_string(&ir_path).unwrap();
    assert!(ir.contains('\n'));
    assert!(!ir.contains("<xmlsquish:"));
    assert!(!ir.contains("<?"));
    assert!(!ir.contains("<!--"));
    assert!(!source.with_extension("o.xml").exists());

    let (code, report, errors) = invoke(&source, "-O");
    assert_eq!(code, 0, "{errors}");
    assert!(report.contains("Dependency loads: 6"), "{report}");
    assert!(report.contains("Unique dependency files: 2"), "{report}");
    let dependency_bytes = files[1].1.len() * 2 + files[2].1.len() * 4;
    assert!(report.contains(&format!("Dependency UTF-8 bytes read: {dependency_bytes} ")));
    assert!(!ir_path.exists());
    let output = fs::read_to_string(source.with_extension("o.xml")).unwrap();
    assert_eq!(
        output,
        concat!(
            "<prompt> <section> <author> klee </author> <source> section.xml </source> ",
            "<author> klee </author> <source> leaf.xml </source> ",
            "<leaf> <author> leaf-author </author> <source> leaf.xml </source> </leaf> </section> ",
            "<section> <author> section-author </author> <source> section.xml </source> ",
            "<author> section-author </author> <source> leaf.xml </source> ",
            "<leaf> <author> leaf-author </author> <source> leaf.xml </source> </leaf> </section> ",
            "<message> Hello &amp; welcome, &lt;researcher&gt;! </message> ",
            "<matched> The physical filename matches. </matched> </prompt>"
        )
    );
    for (name, original) in files {
        assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), original);
    }
}

#[test]
fn parent_adds_absent_metadata_without_relocating_physical_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.xml");
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(
        &path,
        r#"<?xmlsquish stamp="root" name="logical.xml" path="not-a-directory"?>
<r><xmlsquish:let mode="parent"/><xmlsquish:mount path="sub/child.xml" openat="$mode"/></r>"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("sub/child.xml"),
        r#"<?xmlsquish own="child"?><c><xmlsquish:insert get="meta:stamp"/>/<xmlsquish:insert get="meta:name"/>/<xmlsquish:insert get="file:name"/>/<xmlsquish:insert get="meta:own"/><xmlsquish:mount path="leaf.xml" openat="parent"/></c>"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("sub/leaf.xml"),
        r#"<leaf><xmlsquish:insert get="meta:stamp"/>/<xmlsquish:insert get="meta:own"/>/<xmlsquish:insert get="file:name"/></leaf>"#,
    )
    .unwrap();
    let (code, _, errors) = invoke(&path, "-I");
    assert_eq!(code, 0, "{errors}");
    assert!(
        fs::read_to_string(path.with_extension("i.xml"))
            .unwrap()
            .contains("<c>root/logical.xml/child.xml/child<leaf>root/child/leaf.xml</leaf></c>")
    );
}

#[test]
fn regex_unicode_and_insert_failures_follow_normal_output_safety() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.xml");
    fs::write(
        &path,
        r#"<r><xmlsquish:let value="你好 &amp; &lt;世界&gt;"/><xmlsquish:ifr str="你好" pattern="\A\p{Han}+\z"><xmlsquish:insert get="value"/></xmlsquish:ifr></r>"#,
    )
    .unwrap();
    let (code, _, errors) = invoke(&path, "-I");
    assert_eq!(code, 0, "{errors}");
    let ir_path = path.with_extension("i.xml");
    let original = fs::read_to_string(&ir_path).unwrap();
    assert_eq!(original, "<r>你好 &amp; &lt;世界&gt;</r>");

    for broken in [
        "<r><xmlsquish:insert get='missing'/></r>",
        "<r><xmlsquish:ifr str='x' pattern='['/></r>",
        "<r><xmlsquish:mount path='unused.xml' openat='invalid'/></r>",
    ] {
        fs::write(&path, broken).unwrap();
        let (code, report, errors) = invoke(&path, "-I");
        assert_eq!(code, 1, "{report}");
        assert!(errors.contains("main.xml:1"), "{errors}");
        assert!(report.contains("Succeeded: 0"));
        assert_eq!(fs::read_to_string(&ir_path).unwrap(), original);
    }
}

#[test]
fn renamed_mount_persists_both_stages_and_preserves_source_and_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.xml");
    let child = dir.path().join("child.xml");
    let child_source =
        "<?xmlsquish author='child'?><旧 a='旧'><旧/><xmlsquish:insert get='meta:author'/></旧>";
    fs::write(&child, child_source).unwrap();
    fs::write(&main, "<?xmlsquish author='parent'?><r><xmlsquish:let tag='Persona'/><xmlsquish:mount path='child.xml' rename='$tag' openat='parent'/></r>").unwrap();
    let (code, _, errors) = invoke(&main, "-I");
    assert_eq!(code, 0, "{errors}");
    assert_eq!(
        fs::read_to_string(main.with_extension("i.xml")).unwrap(),
        "<r><Persona a='旧'><旧/>parent</Persona></r>"
    );
    let (code, _, errors) = invoke(&main, "-O");
    assert_eq!(code, 0, "{errors}");
    let output = "<r> <Persona a='旧'> <旧/> parent </Persona> </r>";
    assert_eq!(
        fs::read_to_string(main.with_extension("o.xml")).unwrap(),
        output
    );
    assert!(!main.with_extension("i.xml").exists());
    assert_eq!(fs::read_to_string(&child).unwrap(), child_source);

    fs::write(
        &main,
        "<r>\n<xmlsquish:mount path='child.xml' rename='bad name'/></r>",
    )
    .unwrap();
    let (code, _, errors) = invoke(&main, "-O");
    assert_eq!(code, 1);
    assert!(errors.contains("main.xml:2"), "{errors}");
    assert_eq!(
        fs::read_to_string(main.with_extension("o.xml")).unwrap(),
        output
    );
    assert!(!main.with_extension("i.xml").exists());
}
