//! Actual public compiler API mechanisms, with fixture construction and oracles outside samples.

use crate::support;
use squish_ir::{
    DocumentItem, ImportBinding, PackageInstanceId, RelocatableUnitIr, ResolutionSnapshot,
    SourceKey, UnitEncodingOverrides, UnitRevision, Validate, decode_container,
    decode_unit_container, encode_expansion_trace, encode_linked_document, encode_unit_container,
    semantic_unit_digest_with,
};
use squish_link::{
    Budgets, InstantiateOutput, Instantiator, LinkOutput, MiddleEnd, PreparedUnit,
    PreparedUnitClosure, SharedUnitClosure, StaticLinker, UnitClosure, encode_archive_directives,
};
use squish_source::{
    LogicalPath, PackageId, SnapshotBuilder, SourceBlob, SourceId, SourceLocator, SourceProvider,
};
use squish_xml_front::{FrontendSourceContext, compile};
use std::{collections::BTreeMap, hint::black_box, io, sync::Arc};

/// Selects two separately compiled executables; allocation instrumentation never measures latency.
#[derive(Clone, Copy)]
#[allow(dead_code)] // Each executable intentionally constructs only its own measurement mode.
pub enum Mode {
    /// Ordinary System allocator; only duration observations are emitted.
    Latency,
    /// Counting allocator; duration observations are deliberately not emitted.
    Allocations,
}

/// Memory-only XML source adapter; reading fixtures is never part of a core sample.
struct Memory(Vec<u8>);
impl SourceProvider for Memory {
    fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

/// Fully verified immutable fixtures and counters produced before measurements begin.
struct Fixture {
    /// Stable workload label, never a physical checkout location.
    name: String,
    /// Unbound portable units and exact frozen revision/import evidence.
    closure: UnitClosure,
    /// Verified optimized executable used by isolated instantiation measurements.
    linked: LinkOutput,
    /// Immutable entry arguments; cloning is explicit preparation, not hidden setup.
    args: BTreeMap<String, String>,
    /// Real Entry or Module selected to contain the mechanism being investigated.
    sample_unit: SourceKey,
    /// Exact memory-frozen source belonging to the selected compilation unit.
    source: Arc<SourceBlob>,
    /// Authoritative frontend package identity shared by all fixture units.
    context: FrontendSourceContext,
    /// Production canonical unit container, built and round-trip checked before timing.
    encoded_unit: Vec<u8>,
    /// Complete checked result for isolated document/provenance codec measurements.
    result: InstantiateOutput,
    /// Input geometry and directly observed production result counters.
    dimensions: Vec<(&'static str, u64)>,
}

/// Generate a frozen production frontend result, not a simplified benchmark IR interpreter.
fn compile_source(
    path: &str,
    text: String,
    context: &FrontendSourceContext,
) -> (Arc<SourceBlob>, RelocatableUnitIr) {
    let mut snapshot = SnapshotBuilder::new(Memory(text.into_bytes()));
    let source = snapshot
        .load(
            SourceId::new(
                PackageId::new("bench").unwrap(),
                LogicalPath::new(path).unwrap(),
            ),
            SourceLocator::file("unused"),
        )
        .unwrap();
    let unit = compile(&source, context).unwrap().unit;
    (source, unit)
}

/// Wraps source fixtures with exactly the same XML namespace and root framing.
fn xml(root: &str, body: &str) -> String {
    format!(
        r#"<xs:{root} xmlns:xs="{}" xmlns:m="urn:bench">{body}</xs:{root}>"#,
        squish_xml_front::DSL_NAMESPACE
    )
}

/// Builds a real frozen closure, validates persistence, and checks output and full provenance.
fn fixture(
    name: String,
    entry_body: String,
    module_body: Option<String>,
    args: BTreeMap<String, String>,
    expected_text: String,
    mut axes: Vec<(&'static str, u64)>,
) -> Fixture {
    let context = FrontendSourceContext::new(PackageInstanceId {
        source_kind: 1,
        canonical_source: "workspace:bench".into(),
        package_name: "bench".into(),
        exact_revision: "frozen-fixture-v1".into(),
    });
    let entry_body = if module_body.is_some() {
        format!(r#"<xs:import src="module.xml"/>{entry_body}"#)
    } else {
        entry_body
    };
    let (entry_source, entry_unit) =
        compile_source("entry.xml", xml("entry", &entry_body), &context);
    let entry = entry_unit.header().source.clone();
    let mut units = BTreeMap::from([(entry.clone(), entry_unit)]);
    let (mut source, mut sample_unit) = match module_body {
        Some(body) => {
            let (source, unit) = compile_source("module.xml", xml("module", &body), &context);
            let key = unit.header().source.clone();
            units.insert(key.clone(), unit);
            (source, key)
        }
        None => (entry_source.clone(), entry.clone()),
    };
    let mut revisions = Vec::new();
    let mut encoded_unit = Vec::new();
    for (key, unit) in &units {
        unit.validate().unwrap();
        let bytes = encode_unit_container(unit).unwrap();
        assert_eq!(decode_unit_container(&bytes).unwrap(), *unit);
        let container = decode_container(&bytes).unwrap();
        revisions.push((
            key.clone(),
            UnitRevision {
                kind: unit.kind(),
                semantic: container.semantic_digest(),
                object: container.object_digest(),
            },
        ));
        if key == &sample_unit {
            encoded_unit = bytes;
        }
    }
    let source_bytes = entry_source.bytes().len() as u64
        + if units.len() == 2 {
            source.bytes().len() as u64
        } else {
            0
        };
    let imports = if units.len() == 2 {
        vec![ImportBinding {
            importer: entry.clone(),
            import: units[&entry].header().imports[0].local_id,
            target: sample_unit.clone(),
        }]
    } else {
        Vec::new()
    };
    // Scalar argument regions/values live in the Entry, not the macro's Module.
    // Select the relevant real compilation unit rather than timing an unrelated pool.
    if name.starts_with("scalar") {
        source = entry_source.clone();
        sample_unit = entry.clone();
        encoded_unit = encode_unit_container(&units[&entry]).unwrap();
    }
    let closure = UnitClosure {
        snapshot: ResolutionSnapshot {
            units: revisions,
            imports,
        },
        units,
    };
    closure.snapshot.validate().unwrap();
    let linked = StaticLinker.link(&entry, closure.clone()).unwrap();
    let result = Instantiator
        .instantiate(&linked.program, args.clone(), Budgets::default())
        .unwrap();
    result.document.validate().unwrap();
    result
        .trace
        .validate_against_document(&result.document)
        .unwrap();
    assert!(result.directives.is_empty());
    let text: String = result
        .document
        .items
        .iter()
        .filter_map(|item| match item {
            DocumentItem::Text { value } => {
                Some(result.document.strings[value.0 as usize].as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(text, expected_text, "fixture oracle {name}");
    axes.extend([
        ("units", closure.units.len() as u64),
        (
            "actual_definitions",
            closure
                .units
                .values()
                .map(|unit| unit.definitions().len() as u64)
                .sum(),
        ),
        ("source_bytes", source_bytes),
        (
            "unit_ops",
            closure.units.values().map(|u| u.ops().len() as u64).sum(),
        ),
        (
            "regions",
            closure
                .units
                .values()
                .map(|u| u.regions().len() as u64)
                .sum(),
        ),
        (
            "origins",
            closure
                .units
                .values()
                .map(|u| u.origins().entries.len() as u64)
                .sum(),
        ),
        (
            "semantic_string_bytes",
            closure
                .units
                .values()
                .map(|u| {
                    u.header()
                        .semantic_strings
                        .iter()
                        .map(|s| s.len() as u64)
                        .sum::<u64>()
                })
                .sum(),
        ),
        ("persisted_unit_bytes", encoded_unit.len() as u64),
        ("frames", result.trace.frames.len() as u64),
        ("document_items", result.document.items.len() as u64),
        ("trace_origins", result.trace.origins.len() as u64),
        ("scalar_values", result.trace.scalar_values.len() as u64),
        // Fixed provenance-derived proxy for the released linear lookup's comparison floor.
        // After indexing, keep the same paired geometry; it is not candidate comparisons.
        (
            "origin_scan_comparison_floor",
            result
                .trace
                .frames
                .iter()
                .map(|frame| {
                    u64::from(frame.definition_origin.local.0)
                        + 1
                        + frame
                            .call_origin
                            .as_ref()
                            .map_or(0, |origin| u64::from(origin.local.0) + 1)
                })
                .sum::<u64>()
                + result
                    .trace
                    .origins
                    .iter()
                    .filter_map(|node| match node {
                        squish_ir::OriginNode::SourceSpan { origin } => {
                            Some(u64::from(origin.local.0) + 1)
                        }
                        _ => None,
                    })
                    .sum::<u64>(),
        ),
        (
            "static_scalars",
            linked.program.optimization_stats().static_scalars as u64,
        ),
        (
            "compiled_regexes",
            linked.program.optimization_stats().compiled_regexes as u64,
        ),
    ]);
    Fixture {
        name,
        closure,
        linked,
        args,
        sample_unit,
        source,
        context,
        encoded_unit,
        result,
        dimensions: axes,
    }
}

/// Fixed-width names and identical bodies make first/last definition probes output-equivalent.
fn macros(definitions: usize, expansions: usize, call_last: bool, payload: usize) -> Fixture {
    let mut module = String::new();
    for index in 0..definitions {
        module.push_str(&format!(r#"<xs:macro name="m:d{index:05}">x</xs:macro>"#));
    }
    if payload > 0 {
        module.push_str(&format!(
            r#"<xs:macro name="m:padding">{}</xs:macro>"#,
            "p".repeat(payload)
        ));
    }
    let called = if call_last { definitions - 1 } else { 0 };
    let call = format!(r#"<xs:expand ref="m:d{called:05}"/>"#);
    fixture(
        format!(
            "macro-d{definitions}-e{expansions}-last{}-payload{payload}",
            u8::from(call_last)
        ),
        format!("<root>{}</root>", call.repeat(expansions)),
        Some(module),
        BTreeMap::new(),
        "x".repeat(expansions),
        vec![
            ("definitions", definitions as u64),
            ("expansions", expansions as u64),
            ("called_last", u64::from(call_last)),
            ("unused_payload_bytes", payload as u64),
        ],
    )
}

/// Holds operations/regions/output/arguments fixed while changing which RenderText regions fold.
fn scalars(expansions: usize, static_regions: usize, bytes: usize) -> Fixture {
    let payload = "x".repeat(bytes);
    let mut body = r#"<xs:param name="payload"/><root>"#.to_string();
    for index in 0..expansions {
        let scalar = if index < static_regions {
            payload.clone()
        } else {
            r#"<xs:insert get="arg.payload"/>"#.into()
        };
        body.push_str(&format!(
            r#"<xs:expand ref="m:echo"><xs:arg name="text">{scalar}</xs:arg></xs:expand>"#
        ));
    }
    body.push_str("</root>");
    let module =
        r#"<xs:macro name="m:echo"><xs:param name="text"/><xs:insert get="arg.text"/></xs:macro>"#
            .into();
    fixture(
        format!("scalar-e{expansions}-static{static_regions}-bytes{bytes}"),
        body,
        Some(module),
        BTreeMap::from([("payload".into(), payload.clone())]),
        payload.repeat(expansions),
        vec![
            ("definitions", 1),
            ("expansions", expansions as u64),
            ("selected_static_regions", static_regions as u64),
            ("scalar_bytes", bytes as u64),
        ],
    )
}

/// Reuses one compiled RenderText fact K times rather than compiling K separate regions.
/// Both alternatives have the same signature, frame count, arguments and output text.
fn reused_scalar(expansions: usize, bytes: usize, is_static: bool) -> Fixture {
    let payload = "x".repeat(bytes);
    let scalar = if is_static {
        payload.clone()
    } else {
        r#"<xs:insert get="arg.payload"/>"#.into()
    };
    let module = format!(
        r#"<xs:macro name="m:repeat"><xs:param name="payload"/><xs:expand ref="m:echo"><xs:arg name="text">{scalar}</xs:arg></xs:expand></xs:macro><xs:macro name="m:echo"><xs:param name="text"/><xs:insert get="arg.text"/></xs:macro>"#
    );
    let call =
        r#"<xs:expand ref="m:repeat"><xs:arg name="payload" get="arg.payload"/></xs:expand>"#;
    fixture(
        format!(
            "reused-scalar-e{expansions}-bytes{bytes}-static{}",
            u8::from(is_static)
        ),
        format!(
            r#"<xs:param name="payload"/><root>{}</root>"#,
            call.repeat(expansions)
        ),
        Some(module),
        BTreeMap::from([("payload".into(), payload.clone())]),
        payload.repeat(expansions),
        vec![
            ("definitions", 2),
            ("expansions", expansions as u64),
            ("shared_scalar_regions", 1),
            ("scalar_is_static", u64::from(is_static)),
            ("scalar_bytes", bytes as u64),
        ],
    )
}

/// Eager middle-end pattern compilation is measured even when the public entry never uses them.
fn unused_regexes(patterns: usize) -> Fixture {
    let mut module = r#"<xs:macro name="m:main">x</xs:macro>"#.to_string();
    for index in 0..patterns {
        module.push_str(&format!(r#"<xs:macro name="m:unused{index:05}"><xs:ifr str="value{index:05}" pattern="^value{index:05}$">x</xs:ifr></xs:macro>"#));
    }
    fixture(
        format!("unused-regex-pool-{patterns}"),
        r#"<root><xs:expand ref="m:main"/></root>"#.into(),
        Some(module),
        BTreeMap::new(),
        "x".into(),
        vec![
            ("definitions", patterns as u64 + 1),
            ("expansions", 1),
            ("unused_patterns", patterns as u64),
        ],
    )
}

/// Unique scalar values exercise the actual production interning/canonicalization paths.
fn scalar_values(expansions: usize, unique: bool, bytes: usize) -> Fixture {
    let mut entry = String::from("<root>");
    for index in 0..expansions {
        let value = format!(
            "{}{:08}",
            "v".repeat(bytes - 8),
            if unique { index } else { 0 }
        );
        entry.push_str(&format!(
            r#"<xs:expand ref="m:echo"><xs:arg name="text" value="{value}"/></xs:expand>"#
        ));
    }
    entry.push_str("</root>");
    fixture(
        format!(
            "scalar-values-e{expansions}-unique{}-bytes{bytes}",
            u8::from(unique)
        ),
        entry,
        Some(r#"<xs:macro name="m:echo"><xs:param name="text"/>x</xs:macro>"#.into()),
        BTreeMap::new(),
        "x".repeat(expansions),
        vec![
            ("definitions", 1),
            ("expansions", expansions as u64),
            ("unique_values", if unique { expansions as u64 } else { 1 }),
            ("scalar_bytes", bytes as u64),
        ],
    )
}

/// Uses the actual public API with per-operation input preparation outside latency/counting.
fn measure<I, T>(
    mode: Mode,
    fixture: &Fixture,
    mechanism: &str,
    prepare: impl FnMut() -> I,
    operation: impl FnMut(I) -> T,
) {
    match mode {
        Mode::Latency => support::latency_prepared(
            "core",
            &fixture.name,
            mechanism,
            &fixture.dimensions,
            prepare,
            operation,
        ),
        Mode::Allocations => support::allocations_prepared(
            "core",
            &fixture.name,
            mechanism,
            &fixture.dimensions,
            prepare,
            operation,
        ),
    }
}

/// Emits independent canonical result evidence outside every observation interval.
/// Generated workload/dimension labels contain only trusted ASCII benchmark identifiers.
fn oracle(fixture: &Fixture) {
    let document = squish_ir::Digest::sha256(
        "mechanism-oracle-document",
        &encode_linked_document(&fixture.result.document),
    )
    .hex();
    let trace = squish_ir::Digest::sha256(
        "mechanism-oracle-trace",
        &encode_expansion_trace(&fixture.result.trace),
    )
    .hex();
    let directives = squish_ir::Digest::sha256(
        "mechanism-oracle-directives",
        &encode_archive_directives(&fixture.result.directives),
    )
    .hex();
    let dimensions = fixture
        .dimensions
        .iter()
        .map(|(key, value)| format!("\"{key}\":{value}"))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"schema\":\"xmlsquish.mechanism.oracle.v1\",\"suite\":\"core\",\"workload\":\"{}\",\"dimensions\":{{{dimensions}}},\"document_sha256\":\"{document}\",\"trace_sha256\":\"{trace}\",\"directives_sha256\":\"{directives}\"}}",
        fixture.name
    );
}

/// All stage outputs are dropped after measurement; owned input destruction remains API work.
fn stages(mode: Mode, fixture: &Fixture) {
    oracle(fixture);
    let entry = &fixture.linked.image.entry.source;
    let unit = &fixture.closure.units[&fixture.sample_unit];
    // Keep raw owned Clone controls as historical counterfactuals, not production work.
    // The evaluator now borrows immutable facts; the explicit controls quantify avoided copies.
    let optimized = MiddleEnd.optimize(unit.clone()).unwrap();
    let shared_closure = SharedUnitClosure {
        snapshot: Arc::new(fixture.closure.snapshot.clone()),
        units: fixture
            .closure
            .units
            .iter()
            .map(|(key, unit)| (key.clone(), Arc::new(unit.clone())))
            .collect(),
    };
    let shared_unit = &shared_closure.units[&fixture.sample_unit];
    let shared_middle = MiddleEnd.optimize_shared(shared_unit.clone()).unwrap();
    assert!(Arc::ptr_eq(&shared_middle.unit, shared_unit));
    assert_eq!(shared_middle.static_scalars, optimized.static_scalars);
    assert_eq!(shared_middle.static_matches, optimized.static_matches);
    assert_eq!(shared_middle.stats, optimized.stats);
    let shared_linked = StaticLinker
        .link_shared(entry, shared_closure.clone())
        .unwrap();
    assert_eq!(shared_linked.image, fixture.linked.image);
    assert_eq!(shared_linked.map, fixture.linked.map);
    assert_eq!(
        Instantiator
            .instantiate(
                &shared_linked.program,
                fixture.args.clone(),
                Budgets::default()
            )
            .unwrap(),
        fixture.result
    );
    assert_eq!(
        semantic_unit_digest_with(unit, UnitEncodingOverrides::default()).unwrap(),
        decode_container(&fixture.encoded_unit)
            .unwrap()
            .semantic_digest()
    );
    let prepared_closure = PreparedUnitClosure {
        snapshot: shared_closure.snapshot.clone(),
        units: shared_closure
            .units
            .iter()
            .map(|(key, unit)| {
                let prepared = PreparedUnit::new(unit.clone()).unwrap();
                let expected = &shared_closure
                    .snapshot
                    .units
                    .iter()
                    .find(|(source, _)| source == key)
                    .unwrap()
                    .1;
                assert_eq!(prepared.revision(), expected);
                (key.clone(), prepared)
            })
            .collect(),
    };
    let prepared_linked = StaticLinker
        .link_prepared(entry, prepared_closure.clone())
        .unwrap();
    assert_eq!(prepared_linked.image, fixture.linked.image);
    assert_eq!(prepared_linked.map, fixture.linked.map);
    assert_eq!(
        Instantiator
            .instantiate(
                &prepared_linked.program,
                fixture.args.clone(),
                Budgets::default()
            )
            .unwrap(),
        fixture.result
    );
    let prepared_cached = squish_link::LinkedProgram::reconstruct_prepared(
        prepared_linked.image.clone(),
        prepared_closure.units.clone(),
    )
    .unwrap();
    assert_eq!(
        Instantiator
            .instantiate(&prepared_cached, fixture.args.clone(), Budgets::default())
            .unwrap(),
        fixture.result
    );
    let program_clone = fixture.linked.program.clone();
    assert!(std::ptr::eq(
        program_clone.image(),
        fixture.linked.program.image()
    ));
    if let Some(fact) = optimized.static_scalars.values().next() {
        measure(
            mode,
            fixture,
            "borrowed_static_scalar_control",
            || (),
            |()| black_box(fact),
        );
        measure(
            mode,
            fixture,
            "deep_clone_static_scalar",
            || (),
            |()| black_box(fact).clone(),
        );
    }
    measure(
        mode,
        fixture,
        "borrowed_closure_control",
        || (),
        |()| black_box(&fixture.closure),
    );
    measure(
        mode,
        fixture,
        "deep_clone_closure",
        || (),
        |()| black_box(&fixture.closure).clone(),
    );
    // Stable historical label: this measures the real public Clone in each version.
    // It is a deep clone only in the pinned baseline; the candidate shares Arc storage.
    measure(
        mode,
        fixture,
        "deep_clone_linked_program",
        || (),
        |()| black_box(&fixture.linked.program).clone(),
    );
    measure(
        mode,
        fixture,
        "validate_unit",
        || (),
        |()| black_box(unit).validate().unwrap(),
    );
    measure(
        mode,
        fixture,
        "middle_prepared_input",
        || unit.clone(),
        |input| MiddleEnd.optimize(input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "middle_clone_plus_call",
        || (),
        |()| MiddleEnd.optimize(black_box(unit).clone()).unwrap(),
    );
    measure(
        mode,
        fixture,
        "link_prepared_input",
        || fixture.closure.clone(),
        |input| StaticLinker.link(black_box(entry), input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "link_clone_plus_call",
        || (),
        |()| {
            StaticLinker
                .link(black_box(entry), black_box(&fixture.closure).clone())
                .unwrap()
        },
    );
    measure(
        mode,
        fixture,
        "clone_shared_closure",
        || (),
        |()| black_box(&shared_closure).clone(),
    );
    measure(
        mode,
        fixture,
        "middle_shared_prepared_input",
        || shared_unit.clone(),
        |input| MiddleEnd.optimize_shared(input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "link_shared_prepared_input",
        || shared_closure.clone(),
        |input| StaticLinker.link_shared(black_box(entry), input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "link_shared_clone_plus_call",
        || (),
        |()| {
            StaticLinker
                .link_shared(black_box(entry), black_box(&shared_closure).clone())
                .unwrap()
        },
    );
    measure(
        mode,
        fixture,
        "prepare_unit_shared",
        || shared_unit.clone(),
        |input| PreparedUnit::new(input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "prepare_and_specialize_unit_shared",
        || shared_unit.clone(),
        |input| {
            let prepared = PreparedUnit::new(input).unwrap();
            prepared.specialize().unwrap();
            prepared
        },
    );
    measure(
        mode,
        fixture,
        "specialize_prepared_unit_cold",
        || PreparedUnit::new(shared_unit.clone()).unwrap(),
        |prepared| {
            prepared.specialize().unwrap();
            // Return the populated handle: owned fact/regex destruction is outside timing,
            // matching the prepared-input middle-end API boundary used by the baseline.
            prepared
        },
    );
    measure(
        mode,
        fixture,
        "link_prepared_facts",
        || prepared_closure.clone(),
        |input| StaticLinker.link_prepared(black_box(entry), input).unwrap(),
    );
    measure(
        mode,
        fixture,
        "link_prepared_clone_plus_call",
        || (),
        |()| {
            StaticLinker
                .link_prepared(black_box(entry), black_box(&prepared_closure).clone())
                .unwrap()
        },
    );
    measure(
        mode,
        fixture,
        "reconstruct_prepared_image",
        || {
            (
                prepared_linked.image.clone(),
                prepared_closure.units.clone(),
            )
        },
        |(image, units)| squish_link::LinkedProgram::reconstruct_prepared(image, units).unwrap(),
    );
    measure(
        mode,
        fixture,
        "semantic_unit_digest_projection",
        || (),
        |()| semantic_unit_digest_with(black_box(unit), UnitEncodingOverrides::default()).unwrap(),
    );
    measure(
        mode,
        fixture,
        "instantiate_complete_provenance",
        || fixture.args.clone(),
        |args| {
            Instantiator
                .instantiate(black_box(&fixture.linked.program), args, Budgets::default())
                .unwrap()
        },
    );
    measure(
        mode,
        fixture,
        "validate_document",
        || (),
        |()| black_box(&fixture.result.document).validate().unwrap(),
    );
    measure(
        mode,
        fixture,
        "validate_complete_trace",
        || (),
        |()| {
            black_box(&fixture.result.trace)
                .validate_against_document(black_box(&fixture.result.document))
                .unwrap()
        },
    );
    measure(
        mode,
        fixture,
        "encode_complete_trace",
        || (),
        |()| encode_expansion_trace(black_box(&fixture.result.trace)),
    );
    measure(
        mode,
        fixture,
        "persist_encode_unit",
        || (),
        |()| encode_unit_container(black_box(unit)).unwrap(),
    );
    measure(
        mode,
        fixture,
        "persist_decode_unit",
        || (),
        |()| decode_unit_container(black_box(&fixture.encoded_unit)).unwrap(),
    );
    measure(
        mode,
        fixture,
        "frontend_compile_frozen_source",
        || (),
        |()| compile(black_box(&fixture.source), black_box(&fixture.context)).unwrap(),
    );
}

/// Bounded scaling sweeps; Cargo test executes only smoke fixtures rather than full measurements.
pub fn run(mode: Mode) {
    support::verify_allocation_accounting();
    if support::smoke_requested() {
        stages(mode, &macros(1, 1, false, 0));
        stages(mode, &scalars(2, 1, 8));
        return;
    }
    // Origin-only selection remains bounded. Full-repair comparisons use each version's
    // own harness because shared public APIs do not exist in the pinned baseline.
    if std::env::var_os("MECHANISM_ORIGIN_ONLY").is_some() {
        let tiny = fixture(
            "tiny-document".into(),
            "<root>x</root>".into(),
            None,
            BTreeMap::new(),
            "x".into(),
            vec![("definitions", 0), ("expansions", 0)],
        );
        stages(mode, &tiny);
        for definitions in [1, 128, 1024, 4096] {
            for call_last in [false, true] {
                stages(mode, &macros(definitions, 64, call_last, 0));
            }
        }
        stages(mode, &macros(1, 1024, false, 0));
        return;
    }
    let tiny = fixture(
        "tiny-document".into(),
        "<root>x</root>".into(),
        None,
        BTreeMap::new(),
        "x".into(),
        vec![("definitions", 0), ("expansions", 0)],
    );
    stages(mode, &tiny);
    for definitions in [1, 128, 1024, 4096] {
        for call_last in [false, true] {
            stages(mode, &macros(definitions, 64, call_last, 0));
        }
    }
    for expansions in [1, 32, 256, 1024] {
        stages(mode, &macros(1, expansions, false, 0));
    }
    for payload in [65536, 1048576, 4194304] {
        stages(mode, &macros(1, 16, false, payload));
    }
    for selected in [0, 8, 64] {
        stages(mode, &scalars(64, selected, 16));
    }
    for bytes in [4096, 65536] {
        stages(mode, &scalars(16, 16, bytes));
    }
    for expansions in [1, 16, 256] {
        for is_static in [false, true] {
            stages(mode, &reused_scalar(expansions, 4096, is_static));
        }
    }
    for is_static in [false, true] {
        stages(mode, &reused_scalar(16, 65536, is_static));
    }
    for patterns in [0, 64, 256] {
        stages(mode, &unused_regexes(patterns));
    }
    for expansions in [128, 512, 2048] {
        for unique in [false, true] {
            stages(mode, &scalar_values(expansions, unique, 8));
        }
    }
    for unique in [false, true] {
        stages(mode, &scalar_values(512, unique, 256));
    }
}
