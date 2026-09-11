//! Frozen source discovery and static call contracts. / 冻结源码发现与静态调用契约。
use crate::compiler::{CompileError, CompileOptions, CompileResult, Compiler};
use std::{collections::BTreeMap, path::Path};

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
fn qname_aliases_and_forward_references() {
    let result = compile(r#"<xs:macro name="m:first"><xs:call xmlns:a="urn:test" ref="a:last"/></xs:macro><xs:macro name="m:last">yes</xs:macro><R><xs:call ref="m:first"/></R>"#).unwrap();
    assert_eq!(text(&result.output), "yes");
    assert!(!result.output.contains("xs:call"));
}

#[test]
fn dead_branches_and_unused_macros_are_statically_validated() {
    for body in [
        r#"<R><xs:ifr str="no" pattern="^yes$"><xs:call ref="m:missing"/></xs:ifr></R>"#,
        r#"<xs:macro name="m:unused"><xs:mount src="missing.xml"/></xs:macro><R/>"#,
        r#"<R><xs:ifr str="no" pattern="^yes$"><xs:ifr str="x" pattern="("/></xs:ifr></R>"#,
    ] {
        assert!(compile(body).is_err(), "accepted {body}");
    }
}

#[test]
fn import_cycles_freeze_sources_without_executing_them() {
    let source = module(
        r#"<xs:import src="./lib/../lib/a.xml"/><xs:import src="lib/a.xml"/><R><xs:call ref="m:a"/></R>"#,
    );
    let mut reads = BTreeMap::<String, usize>::new();
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |path| {
        let key = path.to_string_lossy().replace('\\', "/");
        *reads.entry(key.clone()).or_default() += 1;
        if key.ends_with("/lib/a.xml") {
            Ok(module(r#"<xs:import src="../main.xml"/><xs:param name="not-executed"/><xs:macro name="m:a">A</xs:macro>ignored"#))
        } else { Err(format!("unexpected {key}")) }
    }).unwrap();
    assert_eq!(text(&result.output), "A");
    assert_eq!(reads.values().sum::<usize>(), 1);
}

#[test]
fn macro_relative_sources_and_file_bindings_use_definition_site() {
    let source = module(r#"<xs:import src="lib/a.xml"/><R><xs:call ref="m:a"/></R>"#);
    let result = Compiler::default().compile(Path::new("fixtures/main.xml"), &source, |path| {
        let path = path.to_string_lossy().replace('\\', "/");
        if path.ends_with("/lib/a.xml") {
            Ok(module(r#"<xs:macro name="m:a"><xs:insert get="file.name"/><xs:mount src="helper.xml"/></xs:macro>"#))
        } else if path.ends_with("/lib/helper.xml") {
            Ok(module(r#"<H><xs:insert get="file.name"/></H>"#))
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
            r#"<xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:call ref="m:f">{args}</xs:call></R>"#
        );
        assert!(compile(&body).is_err());
    }
    let options = CompileOptions {
        args: BTreeMap::from([("x".into(), "parent".into())]),
        ..Default::default()
    };
    let source = module(
        r#"<xs:param name="x"/><xs:macro name="m:f"><xs:param name="x"/></xs:macro><R><xs:call ref="m:f"/></R>"#,
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
        assert!(compile(&format!(r#"<xs:macro name="m:f"><R>{slots}</R></xs:macro><xs:call ref="m:f">{fills}</xs:call>"#)).is_err());
    }
}

#[test]
fn repeated_mounts_share_frozen_definitions_but_not_arguments() {
    let source = module(
        r#"<R><xs:mount src="child.xml"><xs:arg name="x" value="one"/></xs:mount><xs:mount src="./child.xml"><xs:arg name="x" value="two"/></xs:mount></R>"#,
    );
    let mut reads = 0;
    let result = Compiler::default()
        .compile(Path::new("fixtures/main.xml"), &source, |_| {
            reads += 1;
            Ok(module(
                r#"<xs:param name="x"/><xs:macro name="m:shared"/><C><xs:insert get="arg.x"/></C>"#,
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
        Ok(module(r#"<xs:macro name="m:f"/>"#))
    });
    assert!(result.is_err());
}

#[test]
fn percent_encoded_file_uri_and_relative_reference_share_identity() {
    let base = std::env::current_dir().unwrap().join("virtual/main.xml");
    let dependency = base.parent().unwrap().join("a b.xml");
    let uri = url::Url::from_file_path(&dependency).unwrap();
    let source = format!(
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><xs:import src="{uri}"/><xs:import src="./a%20b.xml"/><root/></xs:module>"#
    );
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
