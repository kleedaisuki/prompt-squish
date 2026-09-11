//! CLI DSL contracts and artifact safety / 命令行 DSL 契约与产物安全。
use super::*;

/// Build a module fixture / 构造模块样例。
fn module(body: &str) -> String {
    format!(
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:main"><xs:macro name="m:main">{body}</xs:macro></xs:module>"#
    )
}

/// Invoke with optional flags / 使用可选参数调用。
fn invoke(path: &Path, flags: &[&str]) -> (i32, String, String) {
    let mut args = vec![OsString::from("xmlsquish")];
    args.extend(flags.iter().map(OsString::from));
    args.push(path.as_os_str().to_owned());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn stages_separate_macro_text_from_final_compression() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let source = module("<a>  text  <!--keep--><?user data?></a>");
    fs::write(&input, &source).unwrap();
    fs::write(dir.path().join("orphan.i.xml"), "untouched").unwrap();
    for flag in ["--debug", "--explain", "-I"] {
        let (code, _, err) = invoke(&input, &[flag]);
        assert_eq!(code, 0, "{err}");
        let ir = fs::read_to_string(input.with_extension("i.xml")).unwrap();
        assert!(ir.contains("  text  "), "{ir}");
        if flag != "-I" {
            let output = fs::read_to_string(input.with_extension("o.xml")).unwrap();
            assert!(output.contains(" text "), "{output}");
            assert!(
                !output.contains("https://xmlsquish.moesegfault.dev/ns"),
                "{output}"
            );
            assert_ne!(ir, output);
        }
    }
    let before = fs::read_to_string(input.with_extension("o.xml")).unwrap();
    let (code, _, err) = invoke(&input, &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        fs::read_to_string(input.with_extension("o.xml")).unwrap(),
        before
    );
    assert!(!input.with_extension("i.xml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("orphan.i.xml")).unwrap(),
        "untouched"
    );
    assert_eq!(fs::read_to_string(input).unwrap(), source);
}

#[test]
fn root_arguments_preserve_empty_equals_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(
        &input,
        module(r#"<xs:param name="x"/><a><xs:insert get="arg.x"/></a>"#),
    )
    .unwrap();
    for value in ["", "萌=a & <b>"] {
        let arg = format!("x={value}");
        let (code, _, err) = invoke(&input, &["--arg", &arg]);
        assert_eq!(code, 0, "{err}");
        let output = fs::read_to_string(input.with_extension("o.xml")).unwrap();
        assert!(
            output.contains(
                &value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
            ),
            "{output}"
        );
    }
    for flags in [
        vec!["--arg", "x=1", "--arg", "x=2"],
        vec!["--arg", "bad"],
        vec!["--arg", "=x"],
    ] {
        assert_eq!(invoke(&input, &flags).0, 2);
    }
}

#[test]
fn budgets_fail_without_overwriting_previous_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, module(r#"<xs:expand ref="m:main"/>"#)).unwrap();
    for flag in ["--max-depth", "--max-expansions"] {
        fs::write(input.with_extension("i.xml"), "old ir").unwrap();
        fs::write(input.with_extension("o.xml"), "old output").unwrap();
        let (code, _, err) = invoke(&input, &[flag, "3"]);
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("frame"), "{err}");
        assert_eq!(
            fs::read_to_string(input.with_extension("i.xml")).unwrap(),
            "old ir"
        );
        assert_eq!(
            fs::read_to_string(input.with_extension("o.xml")).unwrap(),
            "old output"
        );
    }
    fs::write(&input, module("<r>long output</r>")).unwrap();
    assert_eq!(invoke(&input, &["--max-output-bytes", "1"]).0, 1);
    assert_eq!(
        fs::read_to_string(input.with_extension("o.xml")).unwrap(),
        "old output"
    );
    assert_eq!(invoke(&input, &["--max-output-bytes", "100000"]).0, 0);
}

#[test]
fn output_failure_keeps_ir_and_bom_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(module("<a>萌</a>").as_bytes());
    fs::write(&input, bytes).unwrap();
    fs::create_dir(input.with_extension("o.xml")).unwrap();
    let (code, _, err) = invoke(&input, &[]);
    assert_eq!(code, 1);
    assert!(err.contains("write output"), "{err}");
    assert!(
        fs::read(input.with_extension("i.xml"))
            .unwrap()
            .starts_with(UTF8_BOM)
    );
    let (code, out, err) = invoke(&input.with_extension("i.xml"), &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Processed files: 0"));
}

#[test]
fn repeated_import_loads_dependency_once_and_expansion_repeats_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(
        &input,
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test" entry="m:main"><xs:import src="part.xml"/><xs:import src="./part.xml"/><xs:macro name="m:main"><r><xs:expand ref="m:part"/><xs:expand ref="m:part"/></r></xs:macro></xs:module>"#,
    )
    .unwrap();
    fs::write(dir.path().join("part.xml"), r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:macro name="m:part"><part/></xs:macro></xs:module>"#).unwrap();
    let (code, out, err) = invoke(&input, &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Dependency loads: 1"), "{out}");
    let output = fs::read_to_string(input.with_extension("o.xml")).unwrap();
    assert_eq!(output.matches("<part").count(), 2);
}

#[test]
fn stage_options_are_mutually_exclusive() {
    assert_eq!(
        run(
            ["xmlsquish", "-I", "-O", "x.xml"],
            &mut Vec::new(),
            &mut Vec::new()
        ),
        2
    );
}

#[test]
fn empty_report_avoids_nan_and_infinite_ratios() {
    let mut out = Vec::new();
    print_report(&mut out, 0, 0, &pipeline::Report::default());
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("N/A"));
    assert!(!out.contains("NaN"));
}

#[test]
fn final_compression_growth_is_budgeted_before_artifact_commit() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let source = module("<r><a/></r>");
    fs::write(&input, &source).unwrap();
    let compiled = crate::compiler::Compiler::default()
        .compile(&input, &source, |_| unreachable!())
        .unwrap();
    let compressed = crate::squish::squish(&compiled.output).unwrap();
    assert!(compressed.output.len() > compiled.output.len());
    fs::write(input.with_extension("i.xml"), "old ir").unwrap();
    fs::write(input.with_extension("o.xml"), "old output").unwrap();
    let limit = compiled.output.len().to_string();
    let (code, _, err) = invoke(&input, &["--max-output-bytes", &limit]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("final output"), "{err}");
    assert!(err.contains("frame") && err.contains("bytes"), "{err}");
    assert_eq!(
        fs::read_to_string(input.with_extension("i.xml")).unwrap(),
        "old ir"
    );
    assert_eq!(
        fs::read_to_string(input.with_extension("o.xml")).unwrap(),
        "old output"
    );
}

/// Discovery compiles entries, not every source file in their closure.
/// 批量发现只编译显式入口，不把闭包中的库源码当作程序。
#[test]
fn directory_and_glob_skip_libraries_but_compile_entries() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("entry.xml");
    let library = dir.path().join("library.xml");
    fs::write(&entry, module("<r> ready </r>")).unwrap();
    let source = r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:macro name="m:helper"><part/></xs:macro></xs:module>"#;
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(source.as_bytes());
    fs::write(&library, bytes).unwrap();
    for input in [dir.path().to_path_buf(), dir.path().join("*.xml")] {
        let (code, out, err) = invoke(&input, &[]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("Succeeded: 1"), "{out}");
        assert!(out.contains("Skipped libraries: 1"), "{out}");
        assert!(entry.with_extension("o.xml").exists());
        assert!(!library.with_extension("i.xml").exists());
        assert!(!library.with_extension("o.xml").exists());
    }
}

/// Explicit operands cannot be hidden by a duplicate directory/glob match.
/// 显式文件参数不能被重复的目录或通配符匹配隐藏。
#[test]
fn explicit_library_wins_over_discovery_in_either_argument_order() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library.xml");
    fs::write(
        &library,
        r#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"/>"#,
    )
    .unwrap();
    for discovered in [dir.path().to_path_buf(), dir.path().join("*.xml")] {
        for paths in [
            [library.clone(), discovered.clone()],
            [discovered.clone(), library.clone()],
        ] {
            let mut args = vec![OsString::from("xmlsquish")];
            args.extend(paths.into_iter().map(PathBuf::into_os_string));
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(run(args, &mut out, &mut err), 1);
            let err = String::from_utf8(err).unwrap();
            assert!(err.contains("explicit entry"), "{err}");
            assert_eq!(err.matches("error[compile]").count(), 1, "{err}");
            assert!(
                String::from_utf8(out)
                    .unwrap()
                    .contains("Skipped libraries: 0")
            );
        }
    }
}

/// Missing entry never masks syntax or encoding failures during discovery.
/// 未声明入口不能掩盖批量发现时的语法和编码错误。
#[test]
fn discovered_invalid_sources_are_errors_not_libraries() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = dir.path().join("bad.xml");
    let cases: &[&[u8]] = &[
        b"<xs:module",
        b"<not-a-module/>",
        br#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns"><p/></xs:module>"#,
        br#"<xs:module xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:test"><xs:macro name="m:bad"><xs:unknown/></xs:macro></xs:module>"#,
        &[0xff, 0xfe, b'<', 0],
    ];
    for source in cases {
        fs::write(&invalid, source).unwrap();
        for input in [dir.path().to_path_buf(), dir.path().join("*.xml")] {
            let (code, out, err) = invoke(&input, &[]);
            assert_eq!(code, 1, "{out}\n{err}");
            assert!(err.contains("bad.xml"), "{err}");
            assert!(out.contains("Skipped libraries: 0"), "{out}");
        }
    }
}
