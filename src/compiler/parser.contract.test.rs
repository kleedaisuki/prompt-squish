//! Parser syntax and namespace contracts. / 解析器语法与命名空间契约。
use crate::compiler::{CompileError, CompileResult, Compiler};
use std::path::Path;

/// Wrap a compact fixture for expansion-stage assertions.
/// 包装紧凑样例，供展开阶段断言使用。
fn module(body: &str) -> String {
    format!(
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test">{body}</xs:module>"#
    )
}

/// Compile without allowing unexpected source discovery.
/// 编译并拒绝意外源码装载。
fn compile(body: &str) -> Result<CompileResult, CompileError> {
    Compiler::default().compile(Path::new("fixtures/main.xml"), &module(body), |p| {
        Err(format!("unexpected load: {}", p.display()))
    })
}

/// Collect decoded XML character data, ignoring serialization choices.
/// 收集解码字符数据，不依赖序列化形式。
fn text(xml: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut result = String::new();
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Text(t) => {
                result.push_str(&quick_xml::escape::unescape(&t.decode().unwrap()).unwrap());
            }
            quick_xml::events::Event::CData(t) => result.push_str(&t.decode().unwrap()),
            quick_xml::events::Event::GeneralRef(r) => {
                let reference = format!("&{};", r.decode().unwrap());
                result.push_str(&quick_xml::escape::unescape(&reference).unwrap());
            }
            quick_xml::events::Event::Eof => return result,
            _ => {}
        }
    }
}

#[test]
fn duplicate_expanded_names_and_builtin_names_are_errors() {
    for body in [
        r#"<xs:macro name="m:f"/><xs:macro xmlns:a="urn:test" name="a:f"/><R/>"#,
        r#"<xs:macro name="xs:f"/><R/>"#,
        r#"<xs:macro name="unbound:f"/><R/>"#,
        r#"<xs:macro name="f"/><R/>"#,
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
}

#[test]
fn scalar_arguments_reject_non_text_and_conflicting_forms() {
    for arg in [
        r#"<xs:arg name="x"><E/></xs:arg>"#,
        r#"<xs:arg name="x"><!--comment--></xs:arg>"#,
        r#"<xs:arg name="x"><?user data?></xs:arg>"#,
        r#"<xs:arg name="x" value="a" get="file.name"/>"#,
        r#"<xs:arg name="x" value="a">b</xs:arg>"#,
    ] {
        assert!(compile(&format!(r#"<xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:call ref="m:f">{arg}</xs:call></R>"#)).is_err(), "accepted {arg}");
    }
}

#[test]
fn regex_dialect_rejects_positional_captures_and_backtracking_features() {
    for pattern in ["(a)", "(?=a)", r"(a)\1", "(?&lt;x&gt;a)(?&lt;x&gt;b)"] {
        assert!(
            compile(&format!(r#"<R><xs:ifr str="ab" pattern="{pattern}"/></R>"#)).is_err(),
            "accepted {pattern}"
        );
    }
}

#[test]
fn builtin_prefix_is_only_a_lexical_alias() {
    let source = r#"<z:module xmlns:z="https://xmlsquish.moesegfault.dev/ns" xmlns:xs="urn:user"><xs:R><z:insert get="file.name"/></xs:R></z:module>"#;
    let result = Compiler::default()
        .compile(Path::new("main.xml"), source, |_| unreachable!())
        .unwrap();
    assert_eq!(text(&result.output), "main.xml");
    assert!(result.output.contains("urn:user"));
}
