//! Parser syntax and namespace contracts. / 解析器语法与命名空间契约。
use crate::compiler::{CompileError, CompileResult, Compiler};
use std::path::Path;

/// Wrap a compact fixture for expansion-stage assertions.
/// 包装紧凑样例，供展开阶段断言使用。
fn module(body: &str) -> String {
    crate::compiler::tests::module(body)
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
        assert!(compile(&format!(r#"<xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:expand ref="m:f">{arg}</xs:expand></R>"#)).is_err(), "accepted {arg}");
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
    let source = r#"<z:module xmlns:z="https://xmlsquish.moesegfault.dev/ns" xmlns:xs="urn:user" entry="xs:main"><z:macro name="xs:main"><xs:R><z:insert get="file.name"/></xs:R></z:macro></z:module>"#;
    let result = Compiler::default()
        .compile(Path::new("main.xml"), source, |_| unreachable!())
        .unwrap();
    assert_eq!(text(&result.output), "main.xml");
    assert!(!result.output.contains("urn:user"));
}

/// A module is declarations only; no positional implicit entry survives.
/// 模块只能包含声明，不保留按位置推导的隐式入口。
#[test]
fn modules_require_explicit_root_entry_and_reject_old_executable_forms() {
    let ns = "https://xmlsquish.moesegfault.dev/ns";
    for (entry, body) in [
        ("", r#"<xs:macro name="m:main"><R/></xs:macro>"#),
        (
            r#" entry="m:main""#,
            r#"<xs:macro name="m:main"><R/></xs:macro><R/>"#,
        ),
        (
            r#" entry="m:main""#,
            r#"<xs:param name="x"/><xs:macro name="m:main"><R/></xs:macro>"#,
        ),
        (
            r#" entry="m:main""#,
            r#"text<xs:macro name="m:main"><R/></xs:macro>"#,
        ),
        (
            r#" entry="m:missing""#,
            r#"<xs:macro name="m:main"><R/></xs:macro>"#,
        ),
        (
            r#" entry="main""#,
            r#"<xs:macro name="m:main"><R/></xs:macro>"#,
        ),
    ] {
        let source =
            format!(r#"<xs:module xmlns:xs="{ns}" xmlns:m="urn:test"{entry}>{body}</xs:module>"#);
        assert!(
            Compiler::default()
                .compile(Path::new("main.xml"), &source, |_| unreachable!())
                .is_err(),
            "accepted {source}"
        );
    }
    for body in [
        r#"<xs:macro name="m:f"><R/></xs:macro><xs:call ref="m:f"/>"#,
        r#"<xs:mount src="child.xml"/>"#,
        r#"<xs:fragment><R/></xs:fragment>"#,
        r#"<xs:expand src="child.xml"/>"#,
        r#"<R><xs:import src="child.xml"/></R>"#,
    ] {
        let error = compile(body).unwrap_err();
        assert!(
            !error.message.contains("unexpected load"),
            "invalid executable import loaded a file: {error}"
        );
    }
}
