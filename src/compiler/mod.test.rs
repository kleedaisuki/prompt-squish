//! DSL contract and source-closure regressions. / DSL 契约与源码闭包回归测试。
use super::*;
use crate::compiler::test_support::TestCompiler as Compiler;
const NS: &str = "https://xmlsquish.moesegfault.dev/ns";
/// Build a compact fixture, materialized as entry plus imported definitions.
/// 构造紧凑样例，实际生成入口与被导入的定义。
pub(super) fn fixture(body: &str) -> String {
    crate::compiler::test_support::fixture(body)
}
/// Wrap declarations without an executable entry. / 包装无可执行入口的声明。
fn library(body: &str) -> String {
    format!(r#"<xs:module xmlns:xs="{NS}" xmlns:m="urn:test">{body}</xs:module>"#)
}
/// Compile a standalone source. / 编译独立源码。
fn compile(body: &str) -> Result<CompileResult, CompileError> {
    Compiler::default().compile(Path::new("entry.xml"), &fixture(body), |p| {
        Err(format!("unexpected load {}", p.display()))
    })
}
#[test]
fn named_macros_are_forward_resolved_and_scalar_escaped() {
    let r=compile(r#"<xs:macro name="m:emit"><xs:param name="x"/><xs:insert get="arg.x"/></xs:macro><r><xs:expand ref="m:emit"><xs:arg name="x" value="&lt;safe&gt;&amp;"/></xs:expand></r>"#).unwrap();
    assert!(r.output.contains("&lt;safe&gt;&amp;"));
    assert!(!r.intermediate.is_empty());
}
#[test]
fn root_arguments_are_explicit_and_required() {
    let source = fixture(r#"<xs:param name="x"/><r><xs:insert get="arg.x"/></r>"#);
    let opts = CompileOptions {
        args: BTreeMap::from([("x".into(), "hello".into())]),
        ..Default::default()
    };
    assert!(
        Compiler::with_options(opts)
            .compile(Path::new("entry.xml"), &source, |_| unreachable!())
            .unwrap()
            .output
            .contains("hello")
    );
    assert!(
        Compiler::default()
            .compile(Path::new("entry.xml"), &source, |_| unreachable!())
            .is_err()
    );
}
#[test]
fn import_cycles_freeze_each_logical_source_once() {
    let source = fixture(
        r#"<xs:import src="./lib/../lib.xml"/><xs:import src="lib.xml"/><r><xs:expand ref="m:x"/></r>"#,
    );
    let lib = library(r#"<xs:import src="lib.xml"/><xs:macro name="m:x">ok</xs:macro>"#);
    let mut reads = 0;
    let result = Compiler::default()
        .compile(Path::new("entry.xml"), &source, |p| {
            assert_eq!(p.file_name().unwrap(), "lib.xml");
            reads += 1;
            Ok(lib.clone())
        })
        .unwrap();
    assert_eq!(reads, 1);
    assert!(result.output.contains("ok"));
}
#[test]
fn unused_import_is_discovered_and_invalid_expands_rejected() {
    let mut reads = 0;
    let source = fixture(r#"<xs:macro name="m:unused"/><xs:import src="dead.xml"/><r/>"#);
    Compiler::default()
        .compile(Path::new("entry.xml"), &source, |_| {
            reads += 1;
            Ok(library(""))
        })
        .unwrap();
    assert_eq!(reads, 1);
    assert!(
        compile(r#"<xs:macro name="m:unused"><xs:expand ref="m:missing"/></xs:macro><r/>"#)
            .is_err()
    );
}
#[test]
fn duplicate_expanded_names_are_rejected() {
    let error = compile(r#"<xs:macro name="m:x"/><xs:macro xmlns:n="urn:test" name="n:x"/><r/>"#)
        .unwrap_err();
    assert!(error.message.contains("definition"));
}
#[test]
fn scalar_body_cannot_carry_xml_nodes() {
    let error=compile(r#"<xs:macro name="m:f"><xs:param name="x"/><xs:insert get="arg.x"/></xs:macro><r><xs:expand ref="m:f"><xs:arg name="x"><x/></xs:arg></xs:expand></r>"#).unwrap_err();
    assert!(error.message.contains("non-text"));
}
#[test]
fn output_erases_namespaces_after_macro_resolution() {
    let result=compile(r#"<xs:macro name="m:f"><u:item xmlns:u="urn:child"/></xs:macro><r xmlns="urn:root"><xs:expand ref="m:f"/></r>"#).unwrap();
    let doc = roxmltree::Document::parse(&result.output).unwrap();
    assert_eq!(doc.root_element().tag_name().namespace(), None);
    assert_eq!(
        doc.root_element()
            .first_element_child()
            .unwrap()
            .tag_name()
            .namespace(),
        None
    );
}
#[test]
fn bounded_recursive_execution_fails_with_frame_chain() {
    let source = fixture(
        r#"<xs:macro name="m:loop"><xs:expand ref="m:loop"/></xs:macro><r><xs:expand ref="m:loop"/></r>"#,
    );
    let options = CompileOptions {
        max_depth: 8,
        ..Default::default()
    };
    let e = Compiler::with_options(options)
        .compile(Path::new("entry.xml"), &source, |_| unreachable!())
        .unwrap_err();
    assert!(e.message.contains("depth"));
    assert!(e.message.contains("frame"));
}

#[test]
fn imported_macro_references_and_file_bindings_use_definition_site() {
    let entry = fixture(r#"<xs:import src="lib/macros.xml"/><r><xs:expand ref="m:where"/></r>"#);
    let library = library(
        r#"<xs:macro name="m:where"><xs:expand ref="m:helper"/></xs:macro><xs:import src="helper.xml"/>"#,
    );
    let helper = self::library(
        r#"<xs:macro name="m:helper"><location><xs:insert get="file.name"/></location></xs:macro>"#,
    );
    let result = Compiler::default()
        .compile(Path::new("entry.xml"), &entry, |path| {
            match path.file_name().unwrap().to_str().unwrap() {
                "macros.xml" => Ok(library.clone()),
                "helper.xml" => {
                    assert_eq!(path.parent().unwrap().file_name().unwrap(), "lib");
                    Ok(helper.clone())
                }
                _ => Err("unexpected source".into()),
            }
        })
        .unwrap();
    assert!(result.output.contains("helper.xml"));
}

#[test]
fn invocation_reuses_definition_not_execution_frame() {
    let source = fixture(
        r#"<xs:import src="child.xml"/><r><xs:expand ref="m:child"><xs:arg name="x" value="first"/></xs:expand><xs:expand ref="m:child"><xs:arg name="x" value="second"/></xs:expand></r>"#,
    );
    let child = library(
        r#"<xs:macro name="m:child"><xs:param name="x"/><x><xs:insert get="arg.x"/></x></xs:macro>"#,
    );
    let mut reads = 0;
    let result = Compiler::default()
        .compile(Path::new("entry.xml"), &source, |_| {
            reads += 1;
            Ok(child.clone())
        })
        .unwrap();
    assert_eq!(reads, 1);
    let doc = roxmltree::Document::parse(&result.output).unwrap();
    let values: Vec<_> = doc
        .root_element()
        .children()
        .filter(|n| n.is_element())
        .map(|n| n.text().unwrap())
        .collect();
    assert_eq!(values, ["first", "second"]);
}

#[test]
fn compiler_loads_dependencies_with_an_in_memory_loader() {
    let source = fixture(r#"<xs:import src="child.xml"/><root><xs:expand ref="m:child"/></root>"#);
    let mut loaded = Vec::new();
    let compiled = Compiler::default()
        .compile(Path::new("virtual/main.xml"), &source, |path| {
            loaded.push(path.to_owned());
            Ok(library(
                r#"<xs:macro name="m:child"><child>  hello  </child></xs:macro>"#,
            ))
        })
        .unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].ends_with(Path::new("virtual/child.xml")));
    assert!(compiled.output.contains("  hello  "));
    assert!(!compiled.output.contains("xs:mount"));
}
