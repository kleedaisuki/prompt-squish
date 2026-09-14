//! XML 前端契约测试。 / XML frontend contract tests.

use super::*;
use squish_ir::{DecodedSyntax, ImportSpec, Op, PackageInstanceId, RelocatableUnitIr};
use squish_source::{
    LogicalPath, PackageId, SnapshotBuilder, SourceId, SourceLocator, SourceProvider,
};
use std::{io, sync::Arc};

#[derive(Clone)]
struct Memory(Vec<u8>);
impl SourceProvider for Memory {
    fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}
fn blob(text: impl Into<Vec<u8>>) -> Arc<SourceBlob> {
    let bytes = text.into();
    let mut builder = SnapshotBuilder::new(Memory(bytes));
    builder
        .load(
            SourceId::new(
                PackageId::new("fixture").unwrap(),
                LogicalPath::new("src/main.xml").unwrap(),
            ),
            SourceLocator::file("unused"),
        )
        .unwrap()
}
fn wrap(kind: &str, body: &str) -> String {
    format!(r#"<xs:{kind} xmlns:xs="{DSL_NAMESPACE}" xmlns:m="urn:macro">{body}</xs:{kind}>"#)
}
fn context() -> FrontendSourceContext {
    FrontendSourceContext::new(PackageInstanceId {
        source_kind: 1,
        canonical_source: "workspace:fixture".into(),
        package_name: "fixture".into(),
        exact_revision: "manifest:fixture@1".into(),
    })
}

#[test]
fn entry_lowers_all_operation_families_and_round_trips() {
    let source = blob(wrap(
        "entry",
        r#"<xs:param name="p"/><R a="v">text<!--c--><?go now?><xs:insert get="arg.p"/><xs:ifr str="abc" pattern="(?P&lt;x&gt;a)"><xs:expand ref="m:f"><xs:arg name="v" get="match.x"/><xs:fill name="body">z</xs:fill></xs:expand></xs:ifr></R>"#,
    ));
    let output = compile(&source, &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!("expected entry")
    };
    assert_eq!(entry.required_params, ["p"]);
    assert!(
        entry
            .ops
            .iter()
            .any(|o| matches!(o.op, Op::MatchRegex { .. }))
    );
    assert!(entry.ops.iter().any(|o| matches!(o.op, Op::Call { .. })));
    assert_eq!(entry.external_symbols.len(), 1);
}

#[test]
fn module_preserves_unresolved_import_specs_and_public_interface() {
    let source = blob(wrap(
        "module",
        r#"<xs:import src="parts/a.xml"/><xs:import src="pkg:common/main"/><xs:macro name="m:f"><xs:param name="p"/><xs:slot name="content" required="true"/></xs:macro>"#,
    ));
    let output = compile(&source, &context()).unwrap();
    let RelocatableUnitIr::Module(module) = output.unit else {
        panic!("expected module")
    };
    assert!(
        matches!(module.header.imports[0].spec,ImportSpec::RelativeUri(ref s) if s=="parts/a.xml")
    );
    assert!(
        matches!(module.header.imports[1].spec,ImportSpec::PackageExport{ref dependency_alias,ref export} if dependency_alias=="common"&&export=="main")
    );
    assert_eq!(
        module.interface.definitions,
        module
            .definitions
            .iter()
            .map(|d| squish_ir::InterfaceDef {
                id: d.id,
                symbol: d.symbol.clone(),
                signature: d.signature.clone()
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn pools_are_canonical_and_independent_of_first_use() {
    let a = blob(wrap(
        "entry",
        r#"<z:B xmlns:z="urn:z" q="z"/><a:A xmlns:a="urn:a" q="a"/>"#,
    ));
    let output = compile(&a, &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    assert!(
        entry
            .header
            .semantic_strings
            .windows(2)
            .all(|w| w[0] < w[1])
    );
    assert!(entry.header.qnames.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn syntax_namespace_and_regex_failures_are_structured() {
    for body in [
        r#"<xs:macro name="f"/>"#,
        r#"<xs:ifr str="x" pattern="(x)"/>"#,
        r#"<xs:unknown/>"#,
    ] {
        let error = compile(&blob(wrap("entry", body)), &context()).unwrap_err();
        assert_eq!(error.severity, Severity::Error);
        assert!(error.code.starts_with("XS"));
        assert!(error.primary.is_some());
    }
}

#[test]
fn utf8_bom_is_preserved_while_origins_address_payload() {
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend(wrap("entry", "<R/>").bytes());
    let output = compile(&blob(bytes), &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    assert_eq!(entry.sources.records[0].bom_len, 3);
    assert!(
        entry
            .origins
            .entries
            .iter()
            .all(|o| o.origin.span.end <= entry.sources.records[0].exact_bytes.byte_len - 3)
    );
}

#[test]
fn deeply_nested_xml_is_not_a_language_error() {
    let depth = 2048;
    let body = format!("{}x{}", "<R>".repeat(depth), "</R>".repeat(depth));
    let output = compile(&blob(wrap("entry", &body)), &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    assert_eq!(entry.regions.len(), depth + 1);
}

#[test]
fn malformed_input_diagnostics_use_exact_byte_offsets() {
    let invalid_utf8 = blob(vec![0xef, 0xbb, 0xbf, 0xff]);
    let error = compile(&invalid_utf8, &context()).unwrap_err();
    assert_eq!(error.primary.as_ref().unwrap().bytes(), 3..4);

    let malformed_text = wrap("entry", "中<R></Q>");
    let expected = malformed_text.find("</Q>").unwrap() as u64;
    let malformed = blob(malformed_text);
    let error = compile(&malformed, &context()).unwrap_err();
    let span = error.primary.as_ref().unwrap().bytes();
    assert!(span.start >= expected && span.start <= expected + 4);
}

#[test]
fn resolved_package_context_cannot_be_silently_substituted() {
    let wrong = FrontendSourceContext::new(PackageInstanceId {
        source_kind: 1,
        canonical_source: "workspace:other".into(),
        package_name: "other".into(),
        exact_revision: "manifest:other@1".into(),
    });
    let error = compile(&blob(wrap("entry", "<R/>")), &wrong).unwrap_err();
    assert_eq!(error.code, "XS1700");
}

#[test]
fn decoded_text_retains_literal_reference_and_cdata_provenance() {
    let output = compile(
        &blob(wrap("entry", "<R a=\"D&#x45;\">A&#x42;<![CDATA[C]]></R>")),
        &context(),
    )
    .unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    let syntaxes: Vec<_> = entry
        .origins
        .decoded_values
        .iter()
        .flat_map(|map| map.segments.iter().map(|segment| segment.syntax))
        .collect();
    assert!(syntaxes.contains(&DecodedSyntax::LiteralText));
    assert!(syntaxes.contains(&DecodedSyntax::CharacterReference));
    assert!(syntaxes.contains(&DecodedSyntax::CData));
    assert!(entry.origins.decoded_values.iter().any(|map| {
        map.owner.field == "attributes.0.value"
            && map
                .segments
                .iter()
                .any(|segment| segment.syntax == DecodedSyntax::CharacterReference)
    }));
    for map in &entry.origins.decoded_values {
        assert!(
            map.segments
                .windows(2)
                .all(|pair| pair[0].value_utf8_range.end == pair[1].value_utf8_range.start)
        );
    }
}

#[test]
fn long_literal_text_coalesces_to_one_linear_mapping_segment() {
    let text = "x".repeat(100_000);
    let output = compile(&blob(wrap("entry", &text)), &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    assert_eq!(entry.origins.decoded_values.len(), 1);
    assert_eq!(entry.origins.decoded_values[0].segments.len(), 1);
}

#[test]
fn lossy_lexical_forms_keep_individual_source_boundaries() {
    let output = compile(&blob(wrap("entry", "&#65;&#x42;A\r\nB")), &context()).unwrap();
    let RelocatableUnitIr::Entry(entry) = output.unit else {
        panic!()
    };
    let segments = &entry.origins.decoded_values[0].segments;
    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0].syntax, DecodedSyntax::CharacterReference);
    assert_eq!(segments[1].syntax, DecodedSyntax::CharacterReference);
    assert_eq!(
        segments[2].source_span.end - segments[2].source_span.start,
        1
    );
    assert_eq!(
        segments[3].source_span.end - segments[3].source_span.start,
        2
    );
    assert_eq!(
        segments[3].value_utf8_range.end - segments[3].value_utf8_range.start,
        1
    );
    assert_eq!(
        segments[4].source_span.end - segments[4].source_span.start,
        1
    );
}
