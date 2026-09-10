//! Module-local regression tests. / 紧邻模块的回归测试。
use super::*;

fn invoke(path: &Path, flag: Option<&str>) -> (i32, String, String) {
    let mut args = vec![OsString::from("xmlsquish")];
    if let Some(flag) = flag {
        args.push(flag.into());
    }
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
fn intermediate_then_default_output_and_scoped_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let source = "<?xml version=\"1.0\"?><a><!--gone--><xmlsquish:let x=\"hello\"/><xmlsquish:log msg=\"$x\"/>  text  </a>";
    fs::write(&input, source).unwrap();
    fs::write(dir.path().join("orphan.i.xml"), "untouched").unwrap();
    let (code, out, err) = invoke(dir.path(), Some("-I"));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("a.xml:1: hello"), "{out}");
    let ir = fs::read_to_string(dir.path().join("a.i.xml")).unwrap();
    assert!(!ir.contains("xmlsquish") && !ir.contains("gone"));
    assert!(ir.contains("  text  "));
    assert!(!dir.path().join("a.o.xml").exists());
    let (code, out, err) = invoke(dir.path(), None);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Processed files: 1"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.o.xml")).unwrap(),
        "<a> text </a>"
    );
    assert!(!dir.path().join("a.i.xml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("orphan.i.xml")).unwrap(),
        "untouched"
    );
    assert_eq!(fs::read_to_string(input).unwrap(), source);
}

#[test]
fn compile_failure_preserves_previous_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, "<a><xmlsquish:log msg=\"$missing\"/></a>").unwrap();
    fs::write(dir.path().join("a.i.xml"), "old ir").unwrap();
    fs::write(dir.path().join("a.o.xml"), "old output").unwrap();
    let (code, _, err) = invoke(&input, Some("-O"));
    assert_eq!(code, 1);
    assert!(err.contains("undefined"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
        "old ir"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.o.xml")).unwrap(),
        "old output"
    );
}

#[test]
fn output_failure_retains_compiled_intermediate() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, "<a>  x  </a>").unwrap();
    fs::create_dir(dir.path().join("a.o.xml")).unwrap();
    let (code, _, err) = invoke(&input, None);
    assert_eq!(code, 1);
    assert!(err.contains("write output"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
        "<a>  x  </a>"
    );
}

#[test]
fn intermediate_preserves_bom_and_explicit_artifacts_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let mut source = UTF8_BOM.to_vec();
    source.extend_from_slice(b"<a/>");
    fs::write(&input, source).unwrap();
    assert_eq!(invoke(&input, Some("-I")).0, 0);
    let ir = dir.path().join("a.i.xml");
    assert!(fs::read(&ir).unwrap().starts_with(UTF8_BOM));
    let (code, out, err) = invoke(&ir, Some("-O"));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Processed files: 0"), "{out}");
    assert!(!dir.path().join("a.i.o.xml").exists());
}

#[test]
fn loads_relative_dependencies_with_independent_frames_and_boms() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("parts")).unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, "<root><xmlsquish:let x=\"parent\"/><xmlsquish:mount path=\"parts/mount.xml\"/><xmlsquish:import path=\"parts/import.xml\"/></root>").unwrap();
    let mut mounted = UTF8_BOM.to_vec();
    mounted.extend_from_slice(
        b"<mounted><xmlsquish:let x=\"child\"/><xmlsquish:log msg=\"$x\"/></mounted>",
    );
    fs::write(dir.path().join("parts/mount.xml"), mounted).unwrap();
    fs::write(
        dir.path().join("parts/import.xml"),
        "<discard><child/></discard>",
    )
    .unwrap();
    let (code, out, err) = invoke(&input, Some("-I"));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("mount.xml:1: child"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.i.xml")).unwrap(),
        "<root><mounted></mounted><child/></root>"
    );
}

#[test]
fn set_assigns_existing_locals_in_branch_and_attribute_order() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, r#"<r><xmlsquish:let result="" next=""/><xmlsquish:if lhs="yes" rhs="yes"><xmlsquish:set result="OK" next="$result"/></xmlsquish:if><xmlsquish:log msg="$result/$next"/></r>"#).unwrap();
    for flag in ["-I", "-O"] {
        let (code, out, err) = invoke(&input, Some(flag));
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("a.xml:1: OK/OK"), "{out}");
        let suffix = if flag == "-I" { "a.i.xml" } else { "a.o.xml" };
        assert_eq!(
            fs::read_to_string(dir.path().join(suffix)).unwrap(),
            if flag == "-I" { "<r></r>" } else { "<r> </r>" }
        );
    }
}

#[test]
fn undeclared_set_preserves_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, r#"<r><xmlsquish:set missing="value"/></r>"#).unwrap();
    let output = dir.path().join("a.o.xml");
    fs::write(&output, "previous output").unwrap();
    let (code, _, err) = invoke(&input, Some("-O"));
    assert_eq!(code, 1);
    assert!(
        err.contains("undefined") && err.contains("missing"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "previous output");
    assert!(!dir.path().join("a.i.xml").exists());
}

#[test]
fn included_file_cannot_set_parent_local() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(
        dir.path().join("child.xml"),
        r#"<child><xmlsquish:set result="child"/></child>"#,
    )
    .unwrap();
    for operation in ["mount", "import"] {
        fs::write(&input, format!(r#"<r><xmlsquish:let result="parent"/><xmlsquish:{operation} path="child.xml"/></r>"#)).unwrap();
        let (code, _, err) = invoke(&input, Some("-O"));
        assert_eq!(code, 1);
        assert!(
            err.contains("undefined") && err.contains("result") && err.contains("child.xml"),
            "{err}"
        );
        assert!(!dir.path().join("a.o.xml").exists());
    }
}

#[test]
fn assembly_growth_is_not_reported_as_negative_compression() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let dependency = format!(
        "<part>{}</part>",
        "a long repeated passage with many tokens. ".repeat(100)
    );
    fs::write(dir.path().join("part.xml"), &dependency).unwrap();
    let source = r#"<r><xmlsquish:mount path="part.xml"/><xmlsquish:mount path="part.xml"/></r>"#;
    fs::write(&input, source).unwrap();
    let (code, out, err) = invoke(&input, Some("-O"));
    assert_eq!(code, 0, "{err}");
    let source_tokens = tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(source)
        .len() as u64;
    let final_text = fs::read_to_string(dir.path().join("a.o.xml")).unwrap();
    assert!(
        tiktoken_rs::o200k_base_singleton()
            .encode_ordinary(&final_text)
            .len() as u64
            > 10 * source_tokens
    );
    assert!(out.contains("Assembly token ratio (IR / source):"), "{out}");
    assert!(
        !out.contains("compression rate") && !out.contains("-900%"),
        "{out}"
    );
    assert!(out.contains("Dependency loads: 2"), "{out}");
    assert!(out.contains("Unique dependency files: 1"), "{out}");
    assert!(
        out.contains(&format!(
            "Dependency UTF-8 bytes read: {}",
            dependency.len() * 2
        )),
        "{out}"
    );
    assert!(
        out.contains(&format!("Final prompt UTF-8 bytes: {}", final_text.len())),
        "{out}"
    );
}

#[test]
fn actual_optimization_token_growth_is_reported_as_added() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let source = "<r></r>";
    fs::write(&input, source).unwrap();
    let (code, out, err) = invoke(&input, None);
    assert_eq!(code, 0, "{err}");
    let final_text = fs::read_to_string(dir.path().join("a.o.xml")).unwrap();
    let before = tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(source)
        .len() as u64;
    let after = tiktoken_rs::o200k_base_singleton()
        .encode_ordinary(&final_text)
        .len() as u64;
    assert!(
        after > before,
        "fixture must increase tokens: {before} -> {after}"
    );
    assert!(
        out.contains(&format!("Optimization tokens added: {}", after - before)),
        "{out}"
    );
    assert!(out.contains("Optimization tokens savings: N/A"), "{out}");
}

#[test]
fn intermediate_report_does_not_claim_optimization() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    fs::write(&input, "<r>    untouched    </r>").unwrap();
    let (code, out, err) = invoke(&input, Some("-I"));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Optimization: not run (-I)"), "{out}");
    assert!(!out.contains("Optimization tokens saved"), "{out}");
}

#[test]
fn zero_denominators_and_no_successes_are_explicit() {
    let mut out = Vec::new();
    print_report(&mut out, 0, 0, &pipeline::Report::default());
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("Assembly token ratio (IR / source): N/A"),
        "{out}"
    );
    assert!(
        out.contains("Optimization: N/A (no successful files)"),
        "{out}"
    );
    let mut out = Vec::new();
    print_optimization(&mut out, "tokens", 0, 0);
    print_optimization(&mut out, "UTF-8 bytes", 0, 1);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("Optimization tokens savings: N/A"), "{out}");
    assert!(out.contains("Optimization UTF-8 bytes added: 1"), "{out}");
    assert!(!out.contains("NaN") && !out.contains("inf"), "{out}");
}

#[test]
fn failed_artifacts_do_not_contribute_stage_or_dependency_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.xml");
    let good = dir.path().join("good.xml");
    fs::write(dir.path().join("part.xml"), "<part/>").unwrap();
    fs::write(&bad, r#"<r><xmlsquish:mount path="part.xml"/></r>"#).unwrap();
    fs::create_dir(dir.path().join("bad.o.xml")).unwrap();
    let source = "<good/>";
    fs::write(&good, source).unwrap();
    let report = pipeline::run(
        &[bad, good],
        pipeline::OutputStage::Optimized,
        &mut Vec::new(),
    );
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.stats.processed_files, 1);
    assert_eq!(
        report.stats.source.tokens,
        tiktoken_rs::o200k_base_singleton()
            .encode_ordinary(source)
            .len() as u64
    );
    assert_eq!(report.stats.source.bytes, source.len() as u64);
    assert_eq!(report.stats.final_prompt.bytes, source.len() as u64);
    assert_eq!(report.stats.dependency_loads, 0);
    assert_eq!(report.stats.dependency_bytes, 0);
    assert!(report.stats.dependency_paths.is_empty());
}

#[test]
fn stage_byte_counts_measure_utf8_text_without_bom() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("a.xml");
    let source = "<r>萌</r>";
    let mut bytes = UTF8_BOM.to_vec();
    bytes.extend_from_slice(source.as_bytes());
    fs::write(&input, bytes).unwrap();
    let report = pipeline::run(
        &[input],
        pipeline::OutputStage::Intermediate,
        &mut Vec::new(),
    );
    assert!(report.failures.is_empty());
    assert_eq!(report.stats.source.bytes, source.len() as u64);
    assert_eq!(report.stats.ir.bytes, source.len() as u64);
    assert_eq!(report.stats.final_prompt.bytes, source.len() as u64);
    assert!(report.stats.source.bytes > report.stats.source.characters);
}

#[test]
fn stage_options_are_mutually_exclusive() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run(["xmlsquish", "-I", "-O", "x.xml"], &mut out, &mut err),
        2
    );
}

#[test]
fn files_have_independent_locals_but_share_compilation_time() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        fs::write(
            dir.path().join(format!("{name}.xml")),
            "<r><xmlsquish:let x=\"same\"/><xmlsquish:log msg=\"$sys:time\"/></r>",
        )
        .unwrap();
    }
    let (code, out, err) = invoke(dir.path(), None);
    assert_eq!(code, 0, "{err}");
    let a = out
        .lines()
        .find_map(|line| line.strip_prefix("a.xml:1: "))
        .unwrap();
    let b = out
        .lines()
        .find_map(|line| line.strip_prefix("b.xml:1: "))
        .unwrap();
    assert_eq!(a, b);
}
