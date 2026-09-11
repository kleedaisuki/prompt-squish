//! Invocation, resource and provenance contracts. / 调用、资源与来源信息契约。
use crate::compiler::test_support::TestCompiler as Compiler;
use crate::compiler::{CompileError, CompileOptions, CompileResult};
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
fn scalar_body_preserves_whitespace_entities_and_escapes_markup() {
    let result = compile("<xs:macro name=\"m:f\"><xs:param name=\"x\"/><xs:insert get=\"arg.x\"/></xs:macro><R><xs:expand ref=\"m:f\"><xs:arg name=\"x\"> \n&lt;T&gt;&amp;\t </xs:arg></xs:expand></R>").unwrap();
    assert_eq!(text(&result.output), " \n<T>&\t ");
    assert!(!result.output.contains("<T>"));
}

#[test]
fn fills_capture_caller_environment_not_callee() {
    let result = compile(r#"<xs:macro name="m:f"><xs:param name="x"/><R><xs:slot name="body" required="true"/><xs:insert get="arg.x"/><xs:slot name="optional"/></R></xs:macro><xs:ifr str="caller" pattern="^(?&lt;x&gt;.*)$"><xs:expand ref="m:f"><xs:arg name="x" value="callee"/><xs:fill name="body"><V><xs:insert get="match.x"/></V></xs:fill></xs:expand></xs:ifr>"#).unwrap();
    assert_eq!(text(&result.output), "callercallee");
}

#[test]
fn nested_captures_shadow_then_restore_outer_binding() {
    let result = compile(r#"<R><xs:ifr str="outer" pattern="^(?&lt;x&gt;.*)$"><xs:insert get="match.x"/><xs:ifr str="inner" pattern="^(?&lt;x&gt;.*)$"><xs:insert get="match.x"/></xs:ifr><xs:insert get="match.x"/></xs:ifr></R>"#).unwrap();
    assert_eq!(text(&result.output), "outerinnerouter");
}

#[test]
fn captures_do_not_escape_blocks_or_invocation_frames() {
    for body in [
        r#"<R><xs:ifr str="a" pattern="(?&lt;x&gt;a)"/><xs:insert get="match.x"/></R>"#,
        r#"<R><xs:ifr str="" pattern="(?&lt;x&gt;a)?"><xs:insert get="match.x"/></xs:ifr></R>"#,
        r#"<xs:macro name="m:f"><xs:insert get="match.x"/></xs:macro><R><xs:ifr str="a" pattern="(?&lt;x&gt;a)"><xs:expand ref="m:f"/></xs:ifr></R>"#,
    ] {
        assert!(compile(body).is_err());
    }
}

#[test]
fn finite_recursive_scalar_computation() {
    let result = compile(r#"<xs:macro name="m:walk"><xs:param name="s"/><xs:ifr get="arg.s" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><xs:insert get="match.head"/><xs:expand ref="m:walk"><xs:arg name="s" get="match.tail"/></xs:expand></xs:ifr></xs:macro><R><xs:expand ref="m:walk"><xs:arg name="s" value="猫🙂A"/></xs:expand></R>"#).unwrap();
    assert_eq!(text(&result.output), "猫🙂A");
}

#[test]
fn resource_guards_reject_infinite_recursion_and_large_output() {
    let source = fixture(
        r#"<xs:macro name="m:loop"><xs:expand ref="m:loop"/></xs:macro><R><xs:expand ref="m:loop"/></R>"#,
    );
    for options in [
        CompileOptions {
            max_depth: 8,
            ..Default::default()
        },
        CompileOptions {
            max_expansions: 8,
            ..Default::default()
        },
    ] {
        let error = Compiler::with_options(options)
            .compile(Path::new("main.xml"), &source, |_| unreachable!())
            .unwrap_err();
        assert!(!error.to_string().is_empty());
    }
    let options = CompileOptions {
        max_output_bytes: 8,
        ..Default::default()
    };
    assert!(
        Compiler::with_options(options)
            .compile(
                Path::new("main.xml"),
                &fixture("<R>0123456789</R>"),
                |_| unreachable!()
            )
            .is_err()
    );
}

#[test]
fn final_document_requires_exactly_one_element_root() {
    for body in ["", "<A/><B/>", "text<R/>", "<R/>text"] {
        assert!(compile(body).is_err(), "accepted {body:?}");
    }
    assert!(compile("<!--before--><?user keep?><R/><!--after-->").is_ok());
}

#[test]
fn lowering_erases_namespaces_but_preserves_comments_and_processing_instructions() {
    let result =
        compile(r#"<?user keep?><u:R xmlns:u="urn:user" u:a="v"><!--keep--><u:C/></u:R>"#).unwrap();
    assert!(!result.output.contains("urn:user"));
    assert!(result.output.contains("<?user keep?>"));
    assert!(result.output.contains("<!--keep-->"));
    assert!(!result.output.contains("xs:module"));
    assert_ne!(result.intermediate, result.output);
}

#[test]
fn arguments_under_construction_do_not_see_each_other() {
    assert!(compile(r#"<xs:macro name="m:f"><xs:param name="x"/><xs:param name="y"/><R/></xs:macro><xs:expand ref="m:f"><xs:arg name="x" value="new"/><xs:arg name="y" get="arg.x"/></xs:expand>"#).is_err());
}

#[test]
fn arguments_and_fills_are_eager_and_evaluated_exactly_once() {
    let source = fixture(
        r#"<xs:macro name="m:value">v</xs:macro><xs:macro name="m:target"><xs:param name="x"/><R><xs:insert get="arg.x"/><xs:slot name="s"/></R></xs:macro><xs:expand ref="m:target"><xs:arg name="x"><xs:expand ref="m:value"/></xs:arg><xs:fill name="s"><xs:expand ref="m:value"/></xs:fill></xs:expand>"#,
    );
    let options = CompileOptions {
        max_expansions: 4,
        ..Default::default()
    };
    let result = Compiler::with_options(options)
        .compile(Path::new("main.xml"), &source, |_| unreachable!())
        .unwrap();
    assert_eq!(text(&result.output), "vv");
    let options = CompileOptions {
        max_expansions: 3,
        ..Default::default()
    };
    assert!(
        Compiler::with_options(options)
            .compile(Path::new("main.xml"), &source, |_| unreachable!())
            .is_err()
    );
    assert!(compile(r#"<xs:macro name="m:target"><R><xs:ifr str="no" pattern="^yes$"><xs:slot name="s"/></xs:ifr></R></xs:macro><xs:expand ref="m:target"><xs:fill name="s"><xs:insert get="arg.missing"/></xs:fill></xs:expand>"#).is_err());
}

#[test]
fn intermediate_has_valid_xml_and_distinct_execution_frames() {
    let result = compile(
        r#"<xs:macro name="m:f"><V/></xs:macro><R><xs:expand ref="m:f"/><xs:expand ref="m:f"/></R>"#,
    )
    .unwrap();
    let doc = roxmltree::Document::parse(&result.intermediate).unwrap();
    let frames: Vec<_> = doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "frame")
        .collect();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].attribute("kind"), Some("entry"));
    assert_eq!(frames[0].attribute("macro"), None);
    assert_eq!(frames[1].attribute("kind"), Some("macro"));
    assert!(frames[1].attribute("macro").is_some());
    assert_ne!(frames[1].attribute("id"), frames[2].attribute("id"));
    assert_eq!(frames[1].attribute("macro"), frames[2].attribute("macro"));
    assert_eq!(frames[1].attribute("parent"), frames[0].attribute("id"));
    for node in doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "node")
    {
        assert!(node.attribute("source").is_some());
        assert!(node.attribute("start").is_some());
        assert!(node.attribute("end").is_some());
        assert!(node.attribute("frame").is_some());
    }
}

/// Returned scalar results are ordinary eager argument values, including Unicode.
/// 返回的标量结果是普通的立即求值参数，包含 Unicode 字符。
#[test]
fn direct_recursive_return_is_passed_as_scalar_value() {
    let result = compile(r#"<xs:macro name="m:reverse"><xs:param name="s"/><xs:ifr get="arg.s" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><xs:expand ref="m:reverse"><xs:arg name="s" get="match.tail"/></xs:expand><xs:insert get="match.head"/></xs:ifr></xs:macro><xs:macro name="m:wrap"><xs:param name="s"/><R><xs:insert get="arg.s"/></R></xs:macro><xs:expand ref="m:wrap"><xs:arg name="s"><xs:expand ref="m:reverse"><xs:arg name="s" value="猫🙂A"/></xs:expand></xs:arg></xs:expand>"#).unwrap();
    assert_eq!(text(&result.output), "A🙂猫");
}

/// Mutual recursion composes returned text, without dynamic capture inheritance.
/// 相互递归组合返回文本，不动态继承捕获变量。
#[test]
fn mutually_recursive_returns_compose_through_argument_bodies() {
    let result = compile(r#"<xs:macro name="m:a"><xs:param name="s"/><xs:ifr get="arg.s" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><xs:expand ref="m:join"><xs:arg name="x"><xs:expand ref="m:b"><xs:arg name="s" get="match.tail"/></xs:expand></xs:arg><xs:arg name="y" get="match.head"/></xs:expand></xs:ifr></xs:macro><xs:macro name="m:b"><xs:param name="s"/><xs:ifr get="arg.s" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><xs:expand ref="m:join"><xs:arg name="x"><xs:expand ref="m:a"><xs:arg name="s" get="match.tail"/></xs:expand></xs:arg><xs:arg name="y" get="match.head"/></xs:expand></xs:ifr></xs:macro><xs:macro name="m:join"><xs:param name="x"/><xs:param name="y"/><xs:insert get="arg.x"/><xs:insert get="arg.y"/></xs:macro><R><xs:expand ref="m:a"><xs:arg name="s" value="A猫🙂Z"/></xs:expand></R>"#).unwrap();
    assert_eq!(text(&result.output), "Z🙂猫A");
}

/// The caller's parameter exists but must not leak into an unbound callee read.
/// 调用者参数存在，但不得泄漏给未绑定该参数的被展开宏。
#[test]
fn callee_cannot_implicitly_read_caller_parameters() {
    let source = fixture(
        r#"<xs:param name="secret"/><xs:macro name="m:read"><R><xs:insert get="arg.secret"/></R></xs:macro><xs:expand ref="m:read"/>"#,
    );
    let options = CompileOptions {
        args: std::collections::BTreeMap::from([("secret".into(), "caller-only".into())]),
        ..Default::default()
    };
    let error = Compiler::with_options(options)
        .compile(Path::new("main.xml"), &source, |_| unreachable!())
        .unwrap_err();
    assert!(error.message.contains("arg.secret"), "{error}");
}

/// Finite two-counter executions exercise zero tests, decrement, increment and state transfer.
/// 有限双计数器执行覆盖零测试、递减、递增及状态传递。
#[test]
fn two_counter_machine_transfers_unary_values_between_recursive_states() {
    let definitions = r#"<xs:macro name="m:transfer"><xs:param name="a"/><xs:param name="b"/><xs:ifr get="arg.a" pattern="^$"><xs:insert get="arg.b"/></xs:ifr><xs:ifr get="arg.a" pattern="^1(?&lt;rest&gt;1*)$"><xs:expand ref="m:increment"><xs:arg name="a" get="match.rest"/><xs:arg name="b" get="arg.b"/></xs:expand></xs:ifr></xs:macro><xs:macro name="m:increment"><xs:param name="a"/><xs:param name="b"/><xs:expand ref="m:transfer"><xs:arg name="a" get="arg.a"/><xs:arg name="b">1<xs:insert get="arg.b"/></xs:arg></xs:expand></xs:macro><xs:macro name="m:result"><xs:param name="value"/><R><xs:insert get="arg.value"/></R></xs:macro>"#;
    for (a, b, expected) in [
        ("", "", ""),
        ("", "11", "11"),
        ("11", "1", "111"),
        ("1111", "11", "111111"),
    ] {
        let result = compile(&format!(r#"{definitions}<xs:expand ref="m:result"><xs:arg name="value"><xs:expand ref="m:transfer"><xs:arg name="a" value="{a}"/><xs:arg name="b" value="{b}"/></xs:expand></xs:arg></xs:expand>"#)).unwrap();
        assert_eq!(text(&result.output), expected);
    }
}

/// A returned structure cannot masquerade as a scalar argument.
/// 返回的结构不能冒充标量参数。
#[test]
fn expanded_non_text_return_is_rejected_by_scalar_argument_boundary() {
    for returned in ["<E/>", "<!--comment-->", "<?user data?>"] {
        let error = compile(&format!(r#"<xs:macro name="m:value">{returned}</xs:macro><xs:macro name="m:consume"><xs:param name="x"/><R><xs:insert get="arg.x"/></R></xs:macro><xs:expand ref="m:consume"><xs:arg name="x"><xs:expand ref="m:value"/></xs:arg></xs:expand>"#)).unwrap_err();
        assert!(error.message.contains("non-text"), "{error}");
    }
}
