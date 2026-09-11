//! XML data and invocation isolation regressions. / XML 数据与调用隔离回归。
use crate::compiler::{CompileError, CompileOptions, CompileResult, Compiler};
use std::path::Path;

/// Compile a complete in-memory module. / 编译完整的内存模块。
fn compile(body: &str) -> Result<CompileResult, CompileError> {
    Compiler::default().compile(
        Path::new("boundaries.xml"),
        &format!(r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:macro">{body}</xs:module>"#),
        |_| Err("unexpected dependency".into()),
    )
}

#[test]
fn ordinary_comments_and_processing_instructions_are_data() {
    let result = compile("<root><!--keep--><?user keep?><![CDATA[a<&b]]></root>").unwrap();
    let parsed = roxmltree::Document::parse(&result.output).unwrap();
    let root = parsed.root_element();
    assert!(
        root.children()
            .any(|node| node.is_comment() && node.text() == Some("keep"))
    );
    assert!(root.children().any(|node| {
        node.pi()
            .is_some_and(|pi| pi.target == "user" && pi.value == Some("keep"))
    }));
    assert!(
        root.children()
            .any(|node| node.is_text() && node.text() == Some("a<&b"))
    );
}

#[test]
fn callee_cannot_observe_caller_capture_implicitly() {
    let error = compile(r#"<xs:macro name="m:read"><xs:insert get="match.x"/></xs:macro><root><xs:ifr str="x" pattern="(?&lt;x&gt;x)"><xs:call ref="m:read"/></xs:ifr></root>"#).unwrap_err();
    assert!(error.message.contains("match.x"), "{error}");
}

#[test]
fn scalar_argument_rejects_comment_and_pi_nodes() {
    for body in ["<!--not text-->", "<?user not-text?>", "<element/>"] {
        let error = compile(&format!(r#"<xs:macro name="m:text"><xs:param name="x"/><xs:insert get="arg.x"/></xs:macro><root><xs:call ref="m:text"><xs:arg name="x">{body}</xs:arg></xs:call></root>"#)).unwrap_err();
        assert!(error.message.contains("non-text"), "{error}");
    }
}

#[test]
fn fill_output_erases_both_caller_and_callee_namespaces() {
    let result = compile(r#"<xs:macro name="m:panel"><panel xmlns="urn:panel"><xs:slot name="body"/></panel></xs:macro><root><xs:call ref="m:panel"><xs:fill name="body"><item/></xs:fill></xs:call></root>"#).unwrap();
    let parsed = roxmltree::Document::parse(&result.output).unwrap();
    let item = parsed
        .descendants()
        .find(|node| node.has_tag_name("item"))
        .unwrap();
    assert_eq!(item.tag_name().namespace().unwrap_or_default(), "");
    assert_eq!(item.parent_element().unwrap().tag_name().namespace(), None);
}

#[test]
fn scalar_insertion_is_not_reparsed_as_control_markup() {
    let mut options = CompileOptions::default();
    options
        .args
        .insert("payload".into(), "<xs:call ref=\"m:evil\"/>&".into());
    let result = Compiler::with_options(options).compile(Path::new("in.xml"), r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><xs:param name="payload"/><root><xs:insert get="arg.payload"/></root></xs:module>"#, |_| unreachable!()).unwrap();
    let parsed = roxmltree::Document::parse(&result.output).unwrap();
    assert_eq!(
        parsed.root_element().text(),
        Some("<xs:call ref=\"m:evil\"/>&")
    );
    assert_eq!(
        parsed
            .root_element()
            .children()
            .filter(|node| node.is_element())
            .count(),
        0
    );
}

#[test]
fn xml_invalid_entry_argument_fails_instead_of_emitting_invalid_output() {
    let mut options = CompileOptions::default();
    options.args.insert("payload".into(), "\u{0}".into());
    let result = Compiler::with_options(options).compile(Path::new("in.xml"), r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><xs:param name="payload"/><root><xs:insert get="arg.payload"/></root></xs:module>"#, |_| unreachable!());
    assert!(result.is_err());
}

#[test]
fn final_output_strips_namespaced_attributes_and_prefixes() {
    let result = compile(
        r#"<n:root xmlns:n="urn:node" xmlns:a="urn:attr" a:key="value"><n:child/></n:root>"#,
    )
    .unwrap();
    let parsed = roxmltree::Document::parse(&result.output).unwrap();
    assert!(parsed.root_element().has_tag_name("root"));
    assert_eq!(parsed.root_element().attribute(("urn:attr", "key")), None);
}

/// All ordinary metadata is removed, but the IR remains inspectable.
/// 普通元数据全部移除，中间表示仍可检查。
#[test]
fn all_attributes_are_removed_without_reinterpreting_text() {
    let result = compile(r#"<n:root xmlns:n="urn:node" id="discard" xml:space="preserve" xml:lang="en"><child title="&gt; &quot;"/><n:child> a="b" &amp; &lt;x id="v"/&gt; </n:child></n:root>"#).unwrap();
    assert_eq!(
        result.output,
        r#"<root><child></child><child> a="b" &amp; &lt;x id="v"/&gt; </child></root>"#
    );
    let doc = roxmltree::Document::parse(&result.output).unwrap();
    for node in doc.descendants().filter(|n| n.is_element()) {
        assert_eq!(node.attributes().len(), 0);
        assert_eq!(node.tag_name().namespace(), None);
    }
    assert!(result.intermediate.contains("discard"));
    assert!(result.intermediate.contains("urn:node"));
}
