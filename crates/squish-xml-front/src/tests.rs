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

#[test]
fn pack_lowers_assets_and_entry_only_include_with_portable_defaults() {
    let output = compile(&blob(wrap("pack", r#"<xs:import src="macros.xml"/><xs:asset path="static/icon.bin"/><xs:include path="entries/hello.xml"/>"#)), &context()).unwrap();
    assert_eq!(output.unit.kind(), squish_ir::UnitKind::Pack);
    let header = output.unit.header();
    assert_eq!(header.imports[1].expected_kind, squish_ir::UnitKind::Entry);
    assert!(output.unit.ops().iter().any(|record| matches!(record.op, Op::Asset {name,..} if header.semantic_strings[name.0 as usize] == "static/icon.bin")));
    assert!(output.unit.ops().iter().any(|record| matches!(record.op, Op::Include {import,name,..} if import.0 == 1 && header.semantic_strings[name.0 as usize] == "hello.prompt")));
    let bytes = squish_ir::encode_unit_container(&output.unit).unwrap();
    assert_eq!(
        squish_ir::decode_unit_container(&bytes).unwrap(),
        output.unit
    );
}

#[test]
fn sopack_preserves_asset_definition_ownership_and_public_macros() {
    let output = compile(&blob(wrap("sopack", r#"<xs:import src="macros.xml"/><xs:asset path="README.txt"/><xs:macro name="m:files"><xs:asset path="static/icon.bin" name="icons/icon.bin"/></xs:macro><xs:expand ref="m:files"/>"#)), &context()).unwrap();
    assert_eq!(output.unit.kind(), squish_ir::UnitKind::Sopack);
    assert_eq!(output.unit.definitions().len(), 1);
    assert!(output.unit.root_region().is_some());
    assert_eq!(
        output
            .unit
            .ops()
            .iter()
            .filter(|record| matches!(record.op, Op::Asset { .. }))
            .count(),
        2
    );
    let bytes = squish_ir::encode_unit_container(&output.unit).unwrap();
    assert_eq!(
        squish_ir::decode_unit_container(&bytes).unwrap(),
        output.unit
    );
}

#[test]
fn sopack_rejects_include_even_in_uncalled_macros() {
    let error = compile(
        &blob(wrap(
            "sopack",
            r#"<xs:macro name="m:hidden"><xs:include path="entry.xml"/></xs:macro>"#,
        )),
        &context(),
    )
    .unwrap_err();
    assert_eq!(error.code, "XS1112");
}

#[test]
fn package_archive_names_reject_traversal_and_platform_paths() {
    for name in [
        "../escape",
        "/absolute",
        "a//b",
        "a/./b",
        "C:/drive",
        "a\\b",
    ] {
        let source = blob(wrap(
            "pack",
            &format!(r#"<xs:asset path="file.bin" name="{name}"/>"#),
        ));
        assert_eq!(
            compile(&source, &context()).unwrap_err().code,
            "XS1311",
            "{name}"
        );
    }
}

#[test]
fn module_macro_asset_remains_a_defining_unit_relative_reference() {
    let output = compile(&blob(wrap("module",r#"<xs:macro name="m:resource"><xs:asset path="../shared/image.bin" name="image.bin"/></xs:macro>"#)),&context()).unwrap();
    assert!(matches!(output.unit.ops()[0].op, Op::Asset { .. }));
    assert_eq!(output.unit.definitions().len(), 1);
    assert_eq!(output.unit.kind(), squish_ir::UnitKind::Module);
}

#[test]
fn package_container_kind_cannot_disagree_with_typed_root() {
    for (root, wrong_kind) in [
        ("pack", squish_ir::ContainerKind::Entry),
        ("sopack", squish_ir::ContainerKind::Module),
    ] {
        let output = compile(&blob(wrap(root, "")), &context()).unwrap();
        let bytes = squish_ir::encode_unit_container(&output.unit).unwrap();
        let container = squish_ir::decode_container(&bytes).unwrap();
        let forged = squish_ir::Container::v1(wrong_kind, container.sections().to_vec()).unwrap();
        assert!(matches!(
            squish_ir::decode_unit_container(&squish_ir::encode_container(&forged)),
            Err(squish_ir::PersistError::KindMismatch)
        ));
    }
}

#[test]
fn include_import_kind_is_cross_validated_after_wire_decode() {
    let mut output = compile(
        &blob(wrap("pack", r#"<xs:include path="hello.xml"/>"#)),
        &context(),
    )
    .unwrap();
    output.unit.header_mut().imports[0].expected_kind = squish_ir::UnitKind::Module;
    let bytes = squish_ir::encode_relocatable_unit(&output.unit);
    assert!(squish_ir::decode_relocatable_unit(&bytes).is_err());
}

#[test]
fn source_authority_is_an_exact_resolver_binding_not_a_package_name_prefix() {
    let mut snapshot = SnapshotBuilder::new(Memory(wrap("pack", "").into_bytes()));
    let authority = PackageId::new("library.abcdef").unwrap();
    let source = snapshot
        .load(
            SourceId::new(authority.clone(), LogicalPath::new("src/root.xml").unwrap()),
            SourceLocator::file("unused"),
        )
        .unwrap();
    let resolved = PackageInstanceId {
        source_kind: 1,
        canonical_source: "workspace:library-instance".into(),
        package_name: "library".into(),
        exact_revision: "manifest:library@1".into(),
    };
    assert_eq!(
        compile(&source, &FrontendSourceContext::new(resolved.clone()))
            .unwrap_err()
            .code,
        "XS1700"
    );
    let bound = FrontendSourceContext::new_with_source_package(resolved.clone(), authority);
    let unit = compile(&source, &bound).unwrap().unit;
    assert!(
        matches!(&unit.header().source, squish_ir::SourceKey::Project {package, ..} if package == &resolved)
    );
    for incorrect in [
        "library",
        "library.abc",
        "library.abcdef.extra",
        "unrelated",
    ] {
        let context = FrontendSourceContext::new_with_source_package(
            resolved.clone(),
            PackageId::new(incorrect).unwrap(),
        );
        assert_eq!(
            compile(&source, &context).unwrap_err().code,
            "XS1700",
            "{incorrect}"
        );
    }
}
