//! Module-local regression tests. / 紧邻模块的回归测试。
use super::*;
fn compile(source: &str) -> Result<CompileResult, CompileError> {
    Compiler::new().compile(Path::new("prompt.xml"), source, |_| {
        Err("unexpected load".into())
    })
}
#[test]
fn metadata_and_macros_are_removed_without_touching_payload() {
    let r = compile("<?xml version=\"1.0\"?><?xmlsquish author='klee'?><?xmlsquish?><r a='$missing'><!--gone--><xmlsquish:let msg='Hello &amp; 世界'/><xmlsquish:log msg='$msg'/><x>$msg</x><![CDATA[$missing]]></r>").unwrap();
    assert_eq!(
        r.output,
        "<r a='$missing'><x>$msg</x><![CDATA[$missing]]></r>"
    );
    assert_eq!(r.logs[0].message, "Hello & 世界");
}
#[test]
fn execution_is_ordered_and_false_branches_are_lazy() {
    let r = compile("<r><xmlsquish:let a='yes' b='$a'/><xmlsquish:if lhs='$a' rhs='$b'><xmlsquish:let result='OK'/><ok/></xmlsquish:if><xmlsquish:ifn lhs='$a' rhs='yes'><xmlsquish:log msg='$missing'/></xmlsquish:ifn><xmlsquish:log msg='$result'/></r>").unwrap();
    assert_eq!(r.output, "<r><ok/></r>");
    assert_eq!(r.logs[0].message, "OK");
}
#[test]
fn diagnostics_report_lines_and_definition_failures() {
    let e = compile("<r>\n<xmlsquish:log msg='$missing'/></r>").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("undefined"));
    assert!(
        compile("<r><xmlsquish:let a='1'/><xmlsquish:let a='2'/></r>")
            .unwrap_err()
            .message
            .contains("duplicate")
    );
    assert!(
        compile("<?xmlsquish name='other'?><?xmlsquish name='again'?><r/>")
            .unwrap_err()
            .message
            .contains("duplicate")
    );
    assert!(compile("<r><xmlsquish:nope/></r>").is_err());
    assert!(compile("<r><xmlsquish:log wrong='x'/></r>").is_err());
}
#[test]
fn includes_have_independent_frames_and_relative_paths() {
    let source = "<r><xmlsquish:let x='parent'/><xmlsquish:mount path='sub/a.xml'/><xmlsquish:import path='sub/a.xml'/><xmlsquish:log msg='$x'/></r>";
    let r = Compiler::new().compile(Path::new("base/p.xml"), source, |p| {
        assert_eq!(p, Path::new("base/sub/a.xml"));
        Ok("<?xmlsquish who='child'?><child><xmlsquish:let x='child'/><xmlsquish:log msg='$file:name'/><a/><b/></child>".into())
    }).unwrap();
    assert_eq!(r.output, "<r><child><a/><b/></child><a/><b/></r>");
    assert_eq!(r.logs.len(), 3);
    assert_eq!(r.logs[0].message, "a.xml");
    assert_eq!(r.logs[2].message, "parent");
}
#[test]
fn child_cannot_see_parent_locals() {
    let e = Compiler::new()
        .compile(
            Path::new("p.xml"),
            "<r><xmlsquish:let x='parent'/><xmlsquish:mount path='a.xml'/></r>",
            |_| Ok("<r><xmlsquish:log msg='$x'/></r>".into()),
        )
        .unwrap_err();
    assert_eq!(e.path, Path::new("a.xml"));
    assert!(e.message.contains("undefined"));
}
#[test]
fn detects_cycles_and_invalid_included_documents() {
    let s = "<r><xmlsquish:mount path='./p.xml'/></r>";
    assert!(
        Compiler::new()
            .compile(Path::new("p.xml"), s, |_| Ok(s.into()))
            .unwrap_err()
            .message
            .contains("cycle")
    );
    assert!(
        Compiler::new()
            .compile(
                Path::new("p.xml"),
                "<r><xmlsquish:mount path='a.xml'/></r>",
                |_| Ok("<a/><b/>".into())
            )
            .is_err()
    );
}
#[test]
fn fragments_and_lexical_markup_remain_compatible() {
    assert_eq!(
        compile("<a  x = 'y'/> <b/>").unwrap().output,
        "<a  x = 'y'/> <b/>"
    );
    assert!(compile("<a><b></a>").is_err());
    assert!(compile("<a>").is_err());
}
#[test]
fn snapshot_and_metadata_expansion() {
    let c = Compiler::new();
    let s = "<?xmlsquish date='$sys:time'?><r><xmlsquish:log msg='$meta:date'/><xmlsquish:log msg='$$literal'/></r>";
    let a = c
        .compile(Path::new("p.xml"), s, |_| unreachable!())
        .unwrap();
    let b = c
        .compile(Path::new("p.xml"), s, |_| unreachable!())
        .unwrap();
    assert_eq!(a.logs, b.logs);
    assert_eq!(a.logs[1].message, "$literal");
}
#[test]
fn xml_line_endings_count_cr_and_crlf_once() {
    for newline in ["\r", "\n", "\r\n"] {
        let source =
            format!("<r>{newline}<!-- a{newline}b -->{newline}<xmlsquish:log msg='$missing'/></r>");
        assert_eq!(compile(&source).unwrap_err().line, 4);
    }
}
#[test]
fn environment_snapshot_is_read_only_and_missing_is_an_error() {
    let c = Compiler {
        sys: HashMap::new(),
        env: HashMap::from([(env_key("http_proxy"), "example".into())]),
    };
    let run = |s: &str| c.compile(Path::new("p.xml"), s, |_| unreachable!());
    assert_eq!(
        run("<r><xmlsquish:log msg='$env:http_proxy'/></r>")
            .unwrap()
            .logs[0]
            .message,
        "example"
    );
    assert!(
        run("<r><xmlsquish:log msg='$env:missing'/></r>")
            .unwrap_err()
            .message
            .contains("undefined")
    );
    for name in ["env:http_proxy", "sys:time", "file:name"] {
        assert!(run(&format!("<r><xmlsquish:let {name}='bad'/></r>")).is_err());
    }
    assert_eq!(c.env[&env_key("http_proxy")], "example");
}
#[test]
fn pragma_modes_are_explicit_and_fake_targets_do_not_execute() {
    let r = compile("<?xmlsquish version='adaptive' warnings='strict'?><?xmlsquish-other invalid='$missing'?><r><xmlsquish:log msg='$meta:version/$meta:warnings'/></r>").unwrap();
    assert_eq!(r.logs[0].message, "adaptive/strict");
    for attr in ["version='future'", "warnings='ignore'"] {
        assert!(
            compile(&format!("<?xmlsquish {attr}?><r/>"))
                .unwrap_err()
                .message
                .contains("unsupported")
        );
    }
}
#[test]
fn skipped_branches_never_load_files_and_dollar_values_are_not_recursive() {
    let r = compile("<r><xmlsquish:if lhs='a' rhs='b'><xmlsquish:mount path='$missing'/></xmlsquish:if><xmlsquish:let a='$$missing'/><xmlsquish:log msg='$a $$$$'/></r>").unwrap();
    assert_eq!(r.output, "<r></r>");
    assert_eq!(r.logs[0].message, "$missing $$");
    assert!(compile("<r><xmlsquish:log msg='$'/></r>").is_err());
}
#[test]
fn malformed_macro_arguments_are_rejected() {
    for body in [
        "<xmlsquish:if lhs='a'/>",
        "<xmlsquish:log msg='a' extra='b'/>",
        "<xmlsquish:mount/>",
        "<xmlsquish:let a='1' a='2'/>",
        "<xmlsquish:log msg='a'>text</xmlsquish:log>",
        "<xmlsquish:let a='1'><child/></xmlsquish:let>",
    ] {
        assert!(compile(&format!("<r>{body}</r>")).is_err(), "{body}");
    }
}
#[test]
fn includes_reject_outside_entities_and_duplicate_metadata() {
    for child in [
        "&amp;<r/>",
        "<r/>&amp;",
        "<?xmlsquish x='a'?><?xmlsquish x='b'?><r/>",
        "<xmlsquish:if lhs='a' rhs='a'><r/></xmlsquish:if>",
    ] {
        assert!(
            Compiler::new()
                .compile(
                    Path::new("p.xml"),
                    "<r><xmlsquish:import path='child.xml'/></r>",
                    |_| Ok(child.into())
                )
                .is_err(),
            "{child}"
        );
    }
}
#[test]
fn nesting_limits_fail_without_stack_overflow() {
    let deep = format!("{}{}", "<r>".repeat(257), "</r>".repeat(257));
    assert!(compile(&deep).unwrap_err().message.contains("nesting"));
    let mut n = 0;
    let error = Compiler::new()
        .compile(
            Path::new("p.xml"),
            "<r><xmlsquish:mount path='1.xml'/></r>",
            |_| {
                n += 1;
                Ok(format!("<r><xmlsquish:mount path='{}.xml'/></r>", n + 1))
            },
        )
        .unwrap_err();
    assert!(error.message.contains("nesting"));
}
#[test]
fn assignment_updates_only_existing_locals_left_to_right() {
    let r=compile("<r><xmlsquish:let a='old' b='old'/><xmlsquish:set a='new' b='$a'/><xmlsquish:log msg='$a/$b'/></r>").unwrap();
    assert_eq!(r.logs[0].message, "new/new");
    assert_eq!(r.output, "<r></r>");
    for body in [
        "<xmlsquish:set a='new'/>",
        "<xmlsquish:set sys:time='x'/>",
        "<xmlsquish:set env:PATH='x'/>",
        "<xmlsquish:set file:name='x'/>",
    ] {
        assert!(compile(&format!("<r>{body}</r>")).is_err());
    }
    let e = Compiler::new()
        .compile(
            Path::new("p.xml"),
            "<r><xmlsquish:let a='parent'/><xmlsquish:mount path='child.xml'/></r>",
            |_| Ok("<r><xmlsquish:set a='child'/></r>".into()),
        )
        .unwrap_err();
    assert!(e.message.contains("undefined"));
}
#[test]
fn combined_include_and_element_depth_is_bounded() {
    let source = format!(
        "{}<xmlsquish:mount path='child.xml'/>{}",
        "<r>".repeat(80),
        "</r>".repeat(80)
    );
    let child = format!("{}{}", "<r>".repeat(80), "</r>".repeat(80));
    let e = Compiler::new()
        .compile(Path::new("p.xml"), &source, |_| Ok(child.clone()))
        .unwrap_err();
    assert!(e.message.contains("combined render nesting"));
}
#[test]
fn environment_key_comparison_matches_platform() {
    let c = Compiler {
        sys: HashMap::new(),
        env: HashMap::from([(env_key("HTTP_PROXY"), "proxy".into())]),
    };
    let result = c.compile(
        Path::new("p.xml"),
        "<r><xmlsquish:log msg='$env:http_proxy'/></r>",
        |_| unreachable!(),
    );
    if cfg!(windows) {
        assert_eq!(result.unwrap().logs[0].message, "proxy");
    } else {
        assert!(result.is_err());
    }
}
#[test]
fn metadata_and_physical_file_namespaces_are_separate() {
    let r=compile("<?xmlsquish name='logical' author='klee'?><r><xmlsquish:log msg='$file:name/$meta:name/$meta:author'/></r>").unwrap();
    assert_eq!(r.logs[0].message, "prompt.xml/logical/klee");
    assert!(
        compile("<?xmlsquish author='klee'?><r><xmlsquish:log msg='$file:author'/></r>").is_err()
    );
}
#[test]
fn inherited_metadata_wins_but_local_duplicate_definitions_still_fail() {
    let parent = "<?xmlsquish owner='parent' warnings='strict'?><r><xmlsquish:mount path='child.xml' openat='parent'/></r>";
    let child = "<?xmlsquish owner='child' warnings='ignored' seen='$meta:owner' local='childonly'?><c><xmlsquish:log msg='$meta:owner/$meta:seen/$meta:local/$file:name'/></c>";
    let r = Compiler::new()
        .compile(Path::new("parent.xml"), parent, |_| Ok(child.into()))
        .unwrap();
    assert_eq!(r.logs[0].message, "parent/parent/childonly/child.xml");
    let child = "<?xmlsquish owner='child'?><?xmlsquish owner='again'?><c/>";
    assert!(
        Compiler::new()
            .compile(Path::new("parent.xml"), parent, |_| Ok(child.into()))
            .unwrap_err()
            .message
            .contains("duplicate")
    );
}
#[test]
fn inheritance_is_per_edge_and_sibling_local() {
    let parent = "<?xmlsquish owner='parent'?><r><xmlsquish:import path='sub/child.xml' openat='parent'/><xmlsquish:mount path='sibling.xml'/></r>";
    let r=Compiler::new().compile(Path::new("base/parent.xml"),parent, |path| {
        let source=match path.to_str().unwrap().replace('\\', "/").as_str() {
            "base/sub/child.xml" => "<?xmlsquish owner='child'?><c><xmlsquish:mount path='grand.xml'/><xmlsquish:mount path='grand.xml' openat='parent'/><xmlsquish:mount path='grand.xml' openat='self'/></c>",
            "base/sub/grand.xml" => "<?xmlsquish owner='grand'?><g><xmlsquish:log msg='$meta:owner/$file:name'/></g>",
            "base/sibling.xml" => "<?xmlsquish owner='sibling'?><s><xmlsquish:log msg='$meta:owner'/></s>",
            other=>panic!("unexpected path {other}"),
        }; Ok(source.into())
    }).unwrap();
    assert_eq!(
        r.logs
            .iter()
            .map(|l| l.message.as_str())
            .collect::<Vec<_>>(),
        [
            "grand/grand.xml",
            "parent/grand.xml",
            "grand/grand.xml",
            "sibling"
        ]
    );
    assert_eq!(r.output, "<r><g></g><g></g><g></g><s></s></r>");
}
#[test]
fn regex_search_anchors_errors_and_lazy_children() {
    let r=compile("<r><xmlsquish:let value='abc123'/><xmlsquish:ifr str='$value' pattern='[0-9]+$'><yes/></xmlsquish:ifr><xmlsquish:ifr str='$value' pattern='^[0-9]+$'><xmlsquish:mount path='$missing'/></xmlsquish:ifr></r>").unwrap();
    assert_eq!(r.output, "<r><yes/></r>");
    let e = compile("<r>\n<xmlsquish:ifr str='a' pattern='['/></r>").unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("invalid regex"));
    assert!(compile("<r><xmlsquish:ifr str='x'/></r>").is_err());
    assert!(
        compile("<r><xmlsquish:mount path='a.xml' openat='bad'/></r>")
            .unwrap_err()
            .message
            .contains("openat")
    );
}
#[test]
fn insert_outputs_escaped_nonrecursive_text_and_requires_a_reference() {
    let r=compile("<?xmlsquish text='&lt;xmlsquish:log msg=&quot;hello&quot;/&gt;&amp;'?><r><xmlsquish:let x='$$missing'/><xmlsquish:insert get='meta:text'/><xmlsquish:insert get='x'/></r>").unwrap();
    assert_eq!(
        r.output,
        "<r>&lt;xmlsquish:log msg=\"hello\"/&gt;&amp;$missing</r>"
    );
    assert!(r.logs.is_empty());
    for get in ["missing", "$x", "meta:absent"] {
        assert!(compile(&format!("<r><xmlsquish:insert get='{get}'/></r>")).is_err());
    }
    let c = Compiler {
        sys: HashMap::new(),
        env: HashMap::from([(env_key("INVALID"), "\0".into())]),
    };
    assert!(
        c.compile(
            Path::new("p.xml"),
            "<r><xmlsquish:insert get='env:INVALID'/></r>",
            |_| unreachable!()
        )
        .unwrap_err()
        .message
        .contains("XML 1.0")
    );
}
#[test]
fn missing_include_path_diagnostic_does_not_require_optional_openat() {
    for macro_name in ["mount", "import"] {
        let e = compile(&format!("<r><xmlsquish:{macro_name}/></r>")).unwrap_err();
        assert_eq!(
            e.message,
            format!("xmlsquish:{macro_name} requires attributes path")
        );
    }
}

#[test]
fn mount_rename_changes_only_root_name_spans() {
    let child = "<?xmlsquish author='child'?><old  a = 'old' >old<old/>\n<xmlsquish:log msg='$file:name'/></old >";
    let r = Compiler::new().compile(
        Path::new("main.xml"),
        "<r><xmlsquish:mount path='child.xml' rename='Persona'/><xmlsquish:mount path='child.xml'/></r>",
        |_| Ok(child.into()),
    ).unwrap();
    assert_eq!(
        r.output,
        "<r><Persona  a = 'old' >old<old/>\n</Persona ><old  a = 'old' >old<old/>\n</old ></r>"
    );
    assert_eq!(
        r.logs
            .iter()
            .map(|l| l.message.as_str())
            .collect::<Vec<_>>(),
        ["child.xml", "child.xml"]
    );
}

#[test]
fn mount_rename_supports_empty_unicode_and_qualified_roots() {
    for replacement in ["New", "角色", "p:New", "e\u{301}"] {
        let r = Compiler::new()
            .compile(
                Path::new("main.xml"),
                &format!("<r><xmlsquish:mount path='child.xml' rename='{replacement}'/></r>"),
                |_| Ok("<旧 xmlns:p='urn:test' a=\"旧\" />".into()),
            )
            .unwrap();
        assert_eq!(
            r.output,
            format!("<r><{replacement} xmlns:p='urn:test' a=\"旧\" /></r>")
        );
    }
}

#[test]
fn mount_rename_accepts_bom_from_public_api_loaders() {
    for child in ["\u{feff}<旧/>", "\u{feff}<旧></旧>"] {
        let r = Compiler::new()
            .compile(
                Path::new("main.xml"),
                "\u{feff}<r><xmlsquish:mount path='child.xml' rename='New'/></r>",
                |_| Ok(child.into()),
            )
            .unwrap();
        assert!(matches!(
            r.output.as_str(),
            "<r><New/></r>" | "<r><New></New></r>"
        ));
    }
}

#[test]
fn mount_rename_uses_caller_variables_with_parent_metadata_and_nested_mounts() {
    let r = Compiler::new().compile(
        Path::new("main.xml"),
        "<?xmlsquish tag='Outer' author='parent'?><r><xmlsquish:mount path='child.xml' rename='$meta:tag' openat='parent'/></r>",
        |path| match path.to_str().unwrap() {
            "child.xml" => Ok("<?xmlsquish tag='Ignored' author='child'?><old><xmlsquish:let tag='Inner'/><xmlsquish:insert get='meta:author'/><xmlsquish:mount path='leaf.xml' rename='$tag'/></old>".into()),
            "leaf.xml" => Ok("<leaf/>".into()),
            _ => unreachable!(),
        },
    ).unwrap();
    assert_eq!(r.output, "<r><Outer>parent<Inner/></Outer></r>");
}

#[test]
fn invalid_rename_is_source_located_and_never_loads_a_file() {
    for name in [
        "",
        "1root",
        "bad name",
        "a/b",
        "a&gt;b",
        ":a",
        "a:",
        "a:b:c",
        "xmlsquish:let",
        "xmlns:tag",
        "\u{300}a",
        "\u{f0000}",
    ] {
        let e = Compiler::new()
            .compile(
                Path::new("main.xml"),
                &format!("<r>\n<xmlsquish:mount path='child.xml' rename='{name}'/></r>"),
                |_| panic!("invalid name must fail before loading"),
            )
            .unwrap_err();
        assert_eq!(e.path, Path::new("main.xml"));
        assert_eq!(e.line, 2);
        assert!(e.message.contains("rename"), "{e}");
    }
    assert!(compile("<r><xmlsquish:import path='child.xml' rename='New'/></r>").is_err());
    assert!(
        compile("<r><xmlsquish:mount path='child.xml' rename='$missing'/></r>")
            .unwrap_err()
            .message
            .contains("undefined")
    );
    assert_eq!(compile("<r><xmlsquish:if lhs='a' rhs='b'><xmlsquish:mount path='missing.xml' rename='bad name'/></xmlsquish:if></r>").unwrap().output, "<r></r>");
}
