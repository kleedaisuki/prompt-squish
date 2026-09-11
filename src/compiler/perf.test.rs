//! Deterministic full-compiler microbenchmarks, without filesystem I/O.
//! 确定性的完整编译器微基准，不包含文件系统 I/O。
use super::{CompileResult, Compiler};
use std::{hint::black_box, path::Path, time::Instant};

/// Fixed sources and logical paths keep provenance reproducible between binaries.
/// 固定源码和逻辑路径，使不同二进制的来源信息可复现。
struct Workload {
    /// Stable measurement label. / 稳定的测量名称。
    name: &'static str,
    /// Entry source. / 入口源码。
    entry: String,
    /// Imported library source. / 导入的库源码。
    library: String,
}

/// Prepare inputs outside the timed region. / 在计时区域外准备输入。
fn workload(name: &'static str, macros: &str, body: &str) -> Workload {
    let ns = r#"xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:perf""#;
    Workload {
        name,
        entry: format!(
            r#"<xs:entry {ns}><xs:import src="library.xml"/><Root>{body}</Root></xs:entry>"#
        ),
        library: format!("<xs:module {ns}>{macros}</xs:module>"),
    }
}

/// Exercise distinct sources of expansion and IR cost. / 覆盖不同的展开与 IR 开销来源。
fn workloads() -> Vec<Workload> {
    let static_macro = r#"<xs:macro name="m:static"><Item><Title>Reusable text</Title><Body>Some repeated body &amp; content.</Body></Item></xs:macro>"#;
    let mut slots = String::new();
    for i in 0..48 {
        let body = if i == 47 {
            r#"<Panel><xs:slot name="body" required="true"/></Panel>"#.to_owned()
        } else {
            format!(
                r#"<xs:expand ref="m:slots{}"><xs:fill name="body"><xs:slot name="body" required="true"/></xs:fill></xs:expand>"#,
                i + 1
            )
        };
        slots.push_str(&format!(r#"<xs:macro name="m:slots{i}">{body}</xs:macro>"#));
    }
    let fill = format!(
        r#"<xs:expand ref="m:slots0"><xs:fill name="body">{}</xs:fill></xs:expand>"#,
        "<Item><A>alpha</A><B>beta</B></Item>".repeat(24)
    );
    let recurse = r#"<xs:macro name="m:recurse"><xs:param name="x"/><xs:ifr get="arg.x" pattern="^x(?&lt;tail&gt;.*)$"><Item>x</Item><xs:expand ref="m:recurse"><xs:arg name="x" get="match.tail"/></xs:expand></xs:ifr></xs:macro>"#;
    let attributes = (0..24)
        .map(|i| format!(" a{i}=\"attribute-{i}-value\""))
        .collect::<String>();
    let attrs_macro = format!(
        r#"<xs:macro name="m:attrs"><Item{attributes}><Child{attributes}>content</Child></Item></xs:macro>"#
    );
    // Unique definitions expose preparation overhead without reuse amortization.
    // 唯一节点揭示无法由复用摊销的预处理开销。
    let unique = (0..1000)
        .map(|i| format!(r#"<Item{i} key="value-{i}">unique-{i}-雪-&amp;-&lt;</Item{i}>"#))
        .collect::<String>();
    // Reverse returns scalar text through arg bodies; intermediate strings are discarded.
    // 反转宏通过 arg 正文返回标量文本；中间字符串最终被丢弃。
    let reverse = r#"<xs:macro name="m:join"><xs:param name="left"/><xs:param name="right"/><xs:insert get="arg.left"/><xs:insert get="arg.right"/></xs:macro><xs:macro name="m:reverse"><xs:param name="text"/><xs:ifr get="arg.text" pattern="(?s)^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$"><xs:expand ref="m:join"><xs:arg name="left"><xs:expand ref="m:reverse"><xs:arg name="text" get="match.tail"/></xs:expand></xs:arg><xs:arg name="right" get="match.head"/></xs:expand></xs:ifr></xs:macro>"#;
    vec![
        workload("tiny", "", "<Message>Hello</Message>"),
        workload(
            "static_2000",
            static_macro,
            &r#"<xs:expand ref="m:static"/>"#.repeat(2000),
        ),
        workload("slots_48x24x12", &slots, &fill.repeat(12)),
        workload(
            "recursive_384",
            recurse,
            &format!(
                r#"<xs:expand ref="m:recurse"><xs:arg name="x" value="{}"/></xs:expand>"#,
                "x".repeat(384)
            ),
        ),
        workload(
            "attrs_1000x48",
            &attrs_macro,
            &r#"<xs:expand ref="m:attrs"/>"#.repeat(1000),
        ),
        workload("unique_1000", "", &unique),
        workload(
            "scalar_reverse_128",
            reverse,
            &format!(
                r#"<xs:expand ref="m:reverse"><xs:arg name="text" value="{}"/></xs:expand>"#,
                "雪&lt;&amp;🙂".repeat(32)
            ),
        ),
    ]
}

/// Stable FNV-1a fingerprint; not a cryptographic integrity claim.
/// 稳定的 FNV-1a 指纹，不作为密码学完整性证明。
fn fingerprint(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

/// Use the real parse/link/expand/serialize path with an in-memory loader.
/// 使用真实的解析、链接、展开、序列化管线以及内存加载器。
fn compile(compiler: &Compiler, input: &Workload) -> CompileResult {
    compiler
        .compile(
            Path::new("/xmlsquish-perf/entry.xml"),
            black_box(&input.entry),
            |_| Ok(input.library.clone()),
        )
        .unwrap()
}

/// Run alone in release mode; emit individual samples and stable output fingerprints.
/// 在 release 模式下单独运行；输出独立样本与稳定产物指纹。
/// Example / 示例: cargo test --release compiler::perf_tests::compiler_baseline -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "manual release-mode performance measurement"]
fn compiler_baseline() {
    let samples = std::env::var("XMLSQUISH_PERF_SAMPLES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(11)
        .max(1);
    let compiler = Compiler::default();
    for input in workloads() {
        let expected = compile(&compiler, &input);
        for _ in 0..3 {
            black_box(compile(&compiler, &input));
        }
        let batch = if input.name == "tiny" { 100 } else { 1 };
        let mut times = Vec::with_capacity(samples);
        for _ in 0..samples {
            let start = Instant::now();
            for _ in 0..batch {
                black_box(compile(&compiler, &input));
            }
            times.push(start.elapsed().as_nanos() / batch);
            let observed = compile(&compiler, &input);
            assert_eq!(observed.output, expected.output);
            assert_eq!(observed.intermediate, expected.intermediate);
        }
        println!(
            "PERF {{\"name\":\"{}\",\"batch\":{},\"samples_ns\":{:?},\"output_bytes\":{},\"intermediate_bytes\":{},\"output_fnv\":\"{:016x}\",\"intermediate_fnv\":\"{:016x}\"}}",
            input.name,
            batch,
            times,
            expected.output.len(),
            expected.intermediate.len(),
            fingerprint(&expected.output),
            fingerprint(&expected.intermediate)
        );
        let path = std::env::current_dir().unwrap().join("perf-entry.xml");
        let discover =
            || super::discover(&path, &input.entry, &mut |_| Ok(input.library.clone())).unwrap();
        let program = discover();
        let options = super::CompileOptions::default();
        for phase in ["discover", "expand"] {
            let run = || {
                if phase == "discover" {
                    black_box(discover());
                } else {
                    black_box(super::runtime::expand(&program, &options).unwrap());
                }
            };
            for _ in 0..3 {
                run();
            }
            let mut times = Vec::with_capacity(samples);
            for _ in 0..samples {
                let start = Instant::now();
                for _ in 0..batch {
                    run();
                }
                times.push(start.elapsed().as_nanos() / batch);
            }
            println!(
                "PERF_PHASE {{\"name\":\"{}\",\"phase\":\"{}\",\"batch\":{},\"samples_ns\":{:?}}}",
                input.name, phase, batch, times
            );
        }
    }
}
