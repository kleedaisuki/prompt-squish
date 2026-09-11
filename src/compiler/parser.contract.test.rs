//! Parser syntax and namespace contracts. / 解析器语法与命名空间契约。
use crate::compiler::test_support::TestCompiler as Compiler;
use crate::compiler::{CompileError, CompileResult};
use std::path::Path;

/// Wrap a compact fixture for expansion-stage assertions.
/// 包装紧凑样例，供展开阶段断言使用。
fn fixture(body: &str) -> String {
    crate::compiler::tests::fixture(body)
}

/// Compile without allowing unexpected source discovery.
/// 编译并拒绝意外源码装载。
fn compile(body: &str) -> Result<CompileResult, CompileError> {
    Compiler::default().compile(Path::new("fixtures/main.xml"), &fixture(body), |p| {
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
    let source = r#"<z:entry xmlns:z="https://xmlsquish.moesegfault.dev/ns" xmlns:xs="urn:user"><xs:R><z:insert get="file.name"/></xs:R></z:entry>"#;
    let result = Compiler::default()
        .compile(Path::new("main.xml"), source, |_| unreachable!())
        .unwrap();
    assert_eq!(text(&result.output), "main.xml");
    assert!(!result.output.contains("urn:user"));
}

/// Modules are libraries; entries own execution and reject macro declarations.
/// 模块是库，入口独占执行职责并拒绝宏声明。
#[test]
fn source_kinds_reject_implicit_entries_and_cross_kind_declarations() {
    let ns = "https://xmlsquish.moesegfault.dev/ns";
    for (kind, attributes, body) in [
        ("module", "", r#"<xs:macro name="m:main"><R/></xs:macro>"#),
        (
            "module",
            r#" entry="m:main""#,
            r#"<xs:macro name="m:main"><R/></xs:macro>"#,
        ),
        ("module", "", "<R/>"),
        ("module", "", r#"<xs:param name="x"/>"#),
        ("module", "", "text"),
        (
            "entry",
            "",
            r#"<xs:macro name="m:main"><R/></xs:macro><R/>"#,
        ),
        ("entry", r#" entry="m:main""#, "<R/>"),
        ("entry", "", r#"<R/><xs:import src="lib.xml"/>"#),
        ("entry", "", r#"<R/><xs:param name="x"/>"#),
    ] {
        let source = format!(
            r#"<xs:{kind} xmlns:xs="{ns}" xmlns:m="urn:test"{attributes}>{body}</xs:{kind}>"#
        );
        assert!(
            Compiler::default()
                .compile(Path::new("main.xml"), &source, |_| Err(
                    "unexpected load".into()
                ))
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

/// Source classification follows the root kind, not an optional entry attribute.
/// 源码分类取决于根种类，而非可选入口属性。
#[test]
fn module_is_a_library_and_entry_is_executable() {
    let path = Path::new("main.xml");
    assert!(
        crate::compiler::is_library(
            path,
            r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"/>"#
        )
        .unwrap()
    );
    assert!(
        !crate::compiler::is_library(
            path,
            r#"<xs:entry xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><R/></xs:entry>"#
        )
        .unwrap()
    );
}
