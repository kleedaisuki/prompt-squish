//! Frozen source discovery and static call contracts. / 冻结源码发现与静态调用契约。
use crate::compiler::{CompileError, CompileOptions, CompileResult, Compiler};
use std::{collections::BTreeMap, path::Path};

/// Wrap a compact fixture for expansion-stage assertions.
/// 包装紧凑样例，供展开阶段断言使用。
fn module(body: &str) -> String {
    crate::compiler::tests::module(body)
}

/// Wrap library declarations without entry. / 包装无入口的库声明。
fn library(body: &str) -> String {
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
fn qname_aliases_and_forward_references() {
    let result = compile(r#"<xs:macro name="m:first"><xs:expand xmlns:a="urn:test" ref="a:last"/></xs:macro><xs:macro name="m:last">yes</xs:macro><R><xs:expand ref="m:first"/></R>"#).unwrap();
    assert_eq!(text(&result.output), "yes");
    assert!(!result.output.contains("xs:expand"));
}

#[test]
fn dead_branches_and_unused_macros_are_statically_validated() {
    for body in [
        r#"<R><xs:ifr str="no" pattern="^yes$"><xs:expand ref="m:missing"/></xs:ifr></R>"#,
        r#"<xs:macro name="m:unused"/><xs:import src="missing.xml"/><R/>"#,
        r#"<R><xs:ifr str="no" pattern="^yes$"><xs:ifr str="x" pattern="("/></xs:ifr></R>"#,
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
}

#[test]
fn import_cycles_freeze_sources_without_executing_them() {
    let source = module(
        r#"<xs:import src="./lib/../lib/a.xml"/><xs:import src="lib/a.xml"/><R><xs:expand ref="m:a"/></R>"#,
    );
    let mut reads = BTreeMap::<String, usize>::new();
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |path| {
        let key = path.to_string_lossy().replace('\\', "/");
        *reads.entry(key.clone()).or_default() += 1;
        if key.ends_with("/lib/a.xml") {
            Ok(library(r#"<xs:import src="../main.xml"/><xs:macro name="m:unused"><xs:param name="not-executed"/>ignored</xs:macro><xs:macro name="m:a">A</xs:macro>"#))
        } else { Err(format!("unexpected {key}")) }
    }).unwrap();
    assert_eq!(text(&result.output), "A");
    assert_eq!(reads.values().sum::<usize>(), 1);
}

#[test]
fn macro_relative_sources_and_file_bindings_use_definition_site() {
    let source = module(r#"<xs:import src="lib/a.xml"/><R><xs:expand ref="m:a"/></R>"#);
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |path| {
        let path = path.to_string_lossy().replace('\\', "/");
        if path.ends_with("/lib/a.xml") {
            Ok(library(r#"<xs:macro name="m:a"><xs:insert get="file.name"/><xs:expand ref="m:helper"/></xs:macro><xs:import src="helper.xml"/>"#))
        } else if path.ends_with("/lib/helper.xml") {
            Ok(library(r#"<xs:macro name="m:helper"><H><xs:insert get="file.name"/></H></xs:macro>"#))
        } else { Err(format!("wrong definition base: {path}")) }
    }).unwrap();
    assert_eq!(text(&result.output), "a.xmlhelper.xml");
}

#[test]
fn required_arguments_are_exact_and_not_inherited() {
    for args in [
        "",
        r#"<xs:arg name="other" value="x"/>"#,
        r#"<xs:arg name="x" value="a"/><xs:arg name="x" value="b"/>"#,
    ] {
        let body = format!(
            r#"<xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:expand ref="m:f">{args}</xs:expand></R>"#
        );
        assert!(compile(&body).is_err());
    }
    let options = CompileOptions {
        args: BTreeMap::from([("x".into(), "parent".into())]),
        ..Default::default()
    };
    let source = module(
        r#"<xs:param name="x"/><xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:expand ref="m:f"/></R>"#,
    );
    assert!(
        Compiler::with_options(options)
            .compile(Path::new("main.xml"), &source, |_| unreachable!())
            .is_err()
    );
}

#[test]
fn slot_contracts_are_enforced() {
    for (slots, fills) in [
        (r#"<xs:slot name="s" required="true"/>"#, ""),
        (r#"<xs:slot name="s"/>"#, r#"<xs:fill name="unknown"/>"#),
        (
            r#"<xs:slot name="s"/>"#,
            r#"<xs:fill name="s"/><xs:fill name="s"/>"#,
        ),
        (r#"<xs:slot name="s"/><xs:slot name="s"/>"#, ""),
    ] {
        assert!(compile(&format!(r#"<xs:macro name="m:f"><R>{slots}</R></xs:macro><xs:expand ref="m:f">{fills}</xs:expand>"#)).is_err());
    }
}

#[test]
fn repeated_expansions_share_frozen_definitions_but_not_arguments() {
    let source = module(
        r#"<xs:import src="child.xml"/><xs:import src="./child.xml"/><R><xs:expand ref="m:child"><xs:arg name="x" value="one"/></xs:expand><xs:expand ref="m:child"><xs:arg name="x" value="two"/></xs:expand></R>"#,
    );
    let mut reads = 0;
    let result = Compiler::default()
        .compile(Path::new("fixtures/main.xml"), &source, |_| {
            reads += 1;
            Ok(library(
                r#"<xs:macro name="m:child"><xs:param name="x"/><C><xs:insert get="arg.x"/></C></xs:macro><xs:macro name="m:shared"/>"#,
            ))
        })
        .unwrap();
    assert_eq!(reads, 1);
    assert_eq!(text(&result.output), "onetwo");
}

#[test]
fn same_bytes_at_different_source_paths_are_distinct_definitions() {
    let source = module(r#"<xs:import src="a.xml"/><xs:import src="alias.xml"/><R/>"#);
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |_| {
        Ok(library(r#"<xs:macro name="m:f"/>"#))
    });
    assert!(result.is_err());
}

#[test]
fn percent_encoded_file_uri_and_relative_reference_share_identity() {
    let base = std::env::current_dir().unwrap().join("virtual/main.xml");
    let dependency = base.parent().unwrap().join("a b.xml");
    let uri = url::Url::from_file_path(&dependency).unwrap();
    let source = module(&format!(
        r#"<xs:import src="{uri}"/><xs:import src="./a%20b.xml"/><root/>"#
    ));
    let mut loads = 0;
    Compiler::default()
        .compile(&base, &source, |path| {
            loads += 1;
            assert_eq!(path, dependency);
            Ok(r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"/>"#.into())
        })
        .unwrap();
    assert_eq!(loads, 1);
}

#[test]
fn unsupported_uri_schemes_are_not_treated_as_local_filenames() {
    for src in [
        "https://example.test/a.xml",
        "urn:module:a",
        "data:text/plain,hello",
    ] {
        let error = compile(&format!(r#"<xs:import src="{src}"/><root/>"#)).unwrap_err();
        assert!(error.message.contains("unsupported URI scheme"), "{error}");
    }
}

/// A diamond plus back-edge freezes each normalized source once before execution.
/// 菱形依赖及回边在执行前各冻结一次规范化源码。
#[test]
fn diamond_and_cycle_imports_share_one_frozen_definition() {
    let source = module(
        r#"<xs:import src="a.xml"/><xs:import src="b.xml"/><R><xs:expand ref="m:a"/><xs:expand ref="m:b"/></R>"#,
    );
    let mut reads = BTreeMap::<String, usize>::new();
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |path| {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        *reads.entry(name.clone()).or_default() += 1;
        Ok(match name.as_str() {
            "a.xml" => library(r#"<xs:macro name="m:a"><xs:expand ref="m:c"/></xs:macro><xs:import src="c.xml"/>"#),
            "b.xml" => library(r#"<xs:import src="./c.xml"/><xs:macro name="m:b"><xs:expand ref="m:c"/></xs:macro>"#),
            "c.xml" => library(r#"<xs:import src="main.xml"/><xs:macro name="m:c">C</xs:macro>"#),
            _ => panic!("unexpected source {name}"),
        })
    }).unwrap();
    assert_eq!(text(&result.output), "CC");
    assert_eq!(
        reads,
        BTreeMap::from([
            ("a.xml".into(), 1),
            ("b.xml".into(), 1),
            ("c.xml".into(), 1)
        ])
    );
}

/// Library entries are validated, but importing never expands or binds them.
/// 库入口必须静态有效，但 import 既不展开也不绑定入口参数。
#[test]
fn library_entry_is_validated_but_not_executed() {
    let source = module(r#"<xs:import src="lib.xml"/><R><xs:expand ref="m:value"/></R>"#);
    let valid = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:library-main"><xs:macro name="m:library-main"><xs:param name="missing"/><xs:expand ref="m:library-main"><xs:arg name="missing" get="arg.missing"/></xs:expand></xs:macro><xs:macro name="m:value">ok</xs:macro></xs:module>"#;
    let result = Compiler::default()
        .compile(Path::new("main.xml"), &source, |_| Ok(valid.into()))
        .unwrap();
    assert_eq!(text(&result.output), "ok");
    let invalid = valid.replace("entry=\"m:library-main\"", "entry=\"m:undefined\"");
    assert!(
        Compiler::default()
            .compile(Path::new("main.xml"), &source, |_| Ok(invalid.clone()))
            .is_err()
    );
}

/// Root entry lookup uses expanded-name identity across the complete import closure.
/// 根入口通过完整导入闭包中的扩展名身份解析。
#[test]
fn root_entry_can_select_an_imported_macro_through_namespace_alias() {
    let source = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:entry="urn:test" entry="entry:start"><xs:import src="lib.xml"/></xs:module>"#;
    let result = Compiler::default()
        .compile(Path::new("main.xml"), source, |_| {
            Ok(library(
                r#"<xs:macro name="m:start"><R>imported entry</R></xs:macro>"#,
            ))
        })
        .unwrap();
    assert_eq!(text(&result.output), "imported entry");
}
