//! 链接与求值跨阶段回归。 / Cross-phase linker and evaluator regressions.

use super::*;
use squish_ir::*;
use std::collections::BTreeMap;

fn source(name: &str) -> SourceKey {
    SourceKey::AdHoc {
        uri: format!("file:///workspace/{name}"),
    }
}
fn revision(kind: UnitKind, name: &str) -> UnitRevision {
    UnitRevision {
        kind,
        semantic: SemanticUnitDigest::of(name.as_bytes()),
        object: ObjectDigest::of(name.as_bytes()),
    }
}
fn header(
    source: SourceKey,
    imports: Vec<ImportDecl>,
    strings: Vec<String>,
    regexes: Vec<RegexPattern>,
) -> UnitHeader {
    UnitHeader {
        ir_schema: Version { major: 1, minor: 0 },
        language_abi: AbiId("dsl-0007".into()),
        frontend_abi: AbiId("xml-v1".into()),
        regex_abi: AbiId("regex-1.12".into()),
        source,
        imports,
        semantic_strings: strings,
        qnames: vec![],
        regexes,
        feature_bits: FeatureBits(0),
    }
}
fn debug(
    source: SourceKey,
    ops: usize,
    regions: usize,
    definitions: usize,
) -> (OriginTable, SourceArchive, UnitSourceAttachment) {
    let digest = SourceDigest::of(b"0123456789");
    let record = SourceRecord {
        key: source.clone(),
        digest,
        bom_len: 0,
        exact_bytes: BlobRef {
            digest: Digest::sha256("blob", b"0123456789"),
            byte_len: 10,
        },
        line_start_offsets: vec![0],
    };
    let mut entries = Vec::new();
    let origin = || Origin {
        source: SourceRef(0),
        span: Span { start: 0, end: 1 },
        lexical_qname: None,
        syntax_kind: SyntaxKind(1),
    };
    for local_id in 0..regions {
        entries.push(OriginEntry {
            entity_kind: EntityKind::Region,
            local_id: local_id as u32,
            origin: origin(),
        });
    }
    for local_id in 0..definitions {
        entries.push(OriginEntry {
            entity_kind: EntityKind::Definition,
            local_id: local_id as u32,
            origin: origin(),
        });
    }
    for local_id in 0..ops {
        entries.push(OriginEntry {
            entity_kind: EntityKind::Operation,
            local_id: local_id as u32,
            origin: origin(),
        });
    }
    (
        OriginTable {
            entries,
            debug_strings: vec![],
            decoded_values: vec![],
        },
        SourceArchive {
            records: vec![record],
        },
        UnitSourceAttachment {
            source,
            source_digest: digest,
            source_record: SourceRef(0),
        },
    )
}
fn import(id: u32, uri: &str) -> ImportDecl {
    ImportDecl {
        local_id: ImportId(id),
        spec: ImportSpec::RelativeUri(uri.into()),
        expected_kind: UnitKind::Module,
    }
}
fn producer() -> Producer {
    Producer {
        tool_version: "test".into(),
        build_fingerprint: "test".into(),
    }
}

fn entry(entry: SourceKey, symbol: SymbolKey) -> EntryObject {
    let (origins, sources, attachment) = debug(entry.clone(), 4, 4, 0);
    let mut unit_header = header(
        entry,
        vec![import(0, "m.xml")],
        vec!["X".into(), "ab".into()],
        vec![],
    );
    unit_header.qnames = vec![
        ExpandedName {
            namespace_uri: "".into(),
            local_name: "child".into(),
        },
        ExpandedName {
            namespace_uri: "".into(),
            local_name: "root".into(),
        },
    ];
    EntryObject {
        header: unit_header,
        required_params: vec![],
        root_region: RegionId(0),
        external_symbols: vec![symbol.clone()],
        regions: vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(1)],
            },
            Region {
                id: RegionId(2),
                ops: vec![OpId(2)],
            },
            Region {
                id: RegionId(3),
                ops: vec![OpId(3)],
            },
        ],
        ops: vec![
            OpRecord {
                id: OpId(0),
                op: Op::EmitElement {
                    name: QNameId(1),
                    attributes: vec![],
                    children: RegionId(1),
                },
            },
            OpRecord {
                id: OpId(1),
                op: Op::Call {
                    target: symbol,
                    args: vec![Argument {
                        name: "s".into(),
                        value: ScalarExpr::Literal(StringId(1)),
                    }],
                    fills: vec![Fill {
                        name: "content".into(),
                        body: RegionId(2),
                    }],
                },
            },
            OpRecord {
                id: OpId(2),
                op: Op::EmitElement {
                    name: QNameId(0),
                    attributes: vec![],
                    children: RegionId(3),
                },
            },
            OpRecord {
                id: OpId(3),
                op: Op::EmitText { value: StringId(0) },
            },
        ],
        origins,
        sources,
        attachment,
        producer: producer(),
    }
}

fn module(module: SourceKey, symbol: SymbolKey) -> ModuleObject {
    let (origins, sources, attachment) = debug(module.clone(), 7, 3, 1);
    let signature = Signature {
        params: vec!["s".into()],
        slots: vec![SlotDecl {
            name: "content".into(),
            required: true,
        }],
    };
    ModuleObject {
        header: header(
            module,
            vec![import(0, "entry-cycle.xml")],
            vec!["(?P<x>a).*".into(), "(?P<y>ab)".into()],
            vec![
                RegexPattern {
                    pattern: StringId(0),
                    named_captures: vec!["x".into()],
                },
                RegexPattern {
                    pattern: StringId(1),
                    named_captures: vec!["y".into()],
                },
            ],
        ),
        definitions: vec![MacroDef {
            id: LocalDefId(0),
            symbol: symbol.clone(),
            signature: signature.clone(),
            body: RegionId(0),
        }],
        external_symbols: vec![],
        interface: InterfaceSummary {
            definitions: vec![InterfaceDef {
                id: LocalDefId(0),
                symbol,
                signature,
            }],
        },
        regions: vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0), OpId(1), OpId(3), OpId(4)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(2)],
            },
            Region {
                id: RegionId(2),
                ops: vec![OpId(5), OpId(6)],
            },
        ],
        ops: vec![
            OpRecord {
                id: OpId(0),
                op: Op::InsertScalar {
                    value: BindingRef::Arg("s".into()),
                },
            },
            OpRecord {
                id: OpId(1),
                op: Op::MatchRegex {
                    input: MatchInput::ReadBinding(BindingRef::Arg("s".into())),
                    pattern: RegexId(1),
                    captures: vec!["y".into()],
                    matched: RegionId(1),
                },
            },
            OpRecord {
                id: OpId(2),
                op: Op::MatchRegex {
                    input: MatchInput::ReadBinding(BindingRef::Arg("s".into())),
                    pattern: RegexId(0),
                    captures: vec!["x".into()],
                    matched: RegionId(2),
                },
            },
            OpRecord {
                id: OpId(3),
                op: Op::ReadSlot {
                    name: "content".into(),
                },
            },
            OpRecord {
                id: OpId(4),
                op: Op::InsertScalar {
                    value: BindingRef::File(FileBinding::Name),
                },
            },
            OpRecord {
                id: OpId(5),
                op: Op::InsertScalar {
                    value: BindingRef::Match("y".into()),
                },
            },
            OpRecord {
                id: OpId(6),
                op: Op::InsertScalar {
                    value: BindingRef::Match("x".into()),
                },
            },
        ],
        origins,
        sources,
        attachment,
        producer: producer(),
    }
}

fn linked() -> LinkOutput {
    let e = source("entry.xml");
    let m = source("m.xml");
    let symbol = ExpandedName {
        namespace_uri: "urn:test".into(),
        local_name: "render".into(),
    };
    let entry_unit = RelocatableUnitIr::Entry(entry(e.clone(), symbol.clone()));
    let module_unit = RelocatableUnitIr::Module(module(m.clone(), symbol));
    let snapshot = ResolutionSnapshot {
        units: vec![
            (e.clone(), revision(UnitKind::Entry, "entry")),
            (m.clone(), revision(UnitKind::Module, "module")),
        ],
        imports: vec![
            ImportBinding {
                importer: e.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
            ImportBinding {
                importer: m.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
        ],
    };
    StaticLinker
        .link(
            &e,
            UnitClosure {
                snapshot,
                units: BTreeMap::from([(e.clone(), entry_unit), (m, module_unit)]),
            },
        )
        .unwrap()
}

fn linked_non_tail_reverse() -> LinkOutput {
    let entry_source = source("reverse-entry.xml");
    let module_source = source("reverse-module.xml");
    let reverse = ExpandedName {
        namespace_uri: "urn:reverse".into(),
        local_name: "reverse".into(),
    };
    let emit = ExpandedName {
        namespace_uri: "urn:reverse".into(),
        local_name: "emit".into(),
    };

    let (entry_origins, entry_sources, entry_attachment) = debug(entry_source.clone(), 2, 2, 0);
    let mut entry_header = header(
        entry_source.clone(),
        vec![import(0, "reverse-module.xml")],
        vec!["abcd".into()],
        vec![],
    );
    entry_header.qnames = vec![ExpandedName {
        namespace_uri: "".into(),
        local_name: "Result".into(),
    }];
    let entry = EntryObject {
        header: entry_header,
        required_params: vec![],
        root_region: RegionId(0),
        external_symbols: vec![reverse.clone()],
        regions: vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(1)],
            },
        ],
        ops: vec![
            OpRecord {
                id: OpId(0),
                op: Op::EmitElement {
                    name: QNameId(0),
                    attributes: vec![],
                    children: RegionId(1),
                },
            },
            OpRecord {
                id: OpId(1),
                op: Op::Call {
                    target: reverse.clone(),
                    args: vec![Argument {
                        name: "s".into(),
                        value: ScalarExpr::Literal(StringId(0)),
                    }],
                    fills: vec![],
                },
            },
        ],
        origins: entry_origins,
        sources: entry_sources,
        attachment: entry_attachment,
        producer: producer(),
    };

    let (module_origins, module_sources, module_attachment) = debug(module_source.clone(), 6, 5, 2);
    let reverse_signature = Signature {
        params: vec!["s".into()],
        slots: vec![],
    };
    let emit_signature = Signature {
        params: vec!["value".into()],
        slots: vec![],
    };
    let module = ModuleObject {
        header: header(
            module_source.clone(),
            vec![],
            vec!["^$".into(), "^(?P<head>.)(?P<tail>.*)$".into()],
            vec![
                RegexPattern {
                    pattern: StringId(0),
                    named_captures: vec![],
                },
                RegexPattern {
                    pattern: StringId(1),
                    named_captures: vec!["head".into(), "tail".into()],
                },
            ],
        ),
        definitions: vec![
            MacroDef {
                id: LocalDefId(0),
                symbol: reverse.clone(),
                signature: reverse_signature.clone(),
                body: RegionId(0),
            },
            MacroDef {
                id: LocalDefId(1),
                symbol: emit.clone(),
                signature: emit_signature.clone(),
                body: RegionId(1),
            },
        ],
        external_symbols: vec![],
        interface: InterfaceSummary {
            definitions: vec![
                InterfaceDef {
                    id: LocalDefId(0),
                    symbol: reverse.clone(),
                    signature: reverse_signature,
                },
                InterfaceDef {
                    id: LocalDefId(1),
                    symbol: emit.clone(),
                    signature: emit_signature,
                },
            ],
        },
        regions: vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0), OpId(1)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(2)],
            },
            Region {
                id: RegionId(2),
                ops: vec![],
            },
            Region {
                id: RegionId(3),
                ops: vec![OpId(3)],
            },
            Region {
                id: RegionId(4),
                // 递归结果必须先返回，再追加当前字符；这是非尾组合的关键顺序。
                // The recursive result must return before the current character is appended.
                ops: vec![OpId(4), OpId(5)],
            },
        ],
        ops: vec![
            OpRecord {
                id: OpId(0),
                op: Op::MatchRegex {
                    input: MatchInput::ReadBinding(BindingRef::Arg("s".into())),
                    pattern: RegexId(0),
                    captures: vec![],
                    matched: RegionId(2),
                },
            },
            OpRecord {
                id: OpId(1),
                op: Op::MatchRegex {
                    input: MatchInput::ReadBinding(BindingRef::Arg("s".into())),
                    pattern: RegexId(1),
                    captures: vec!["head".into(), "tail".into()],
                    matched: RegionId(3),
                },
            },
            OpRecord {
                id: OpId(2),
                op: Op::InsertScalar {
                    value: BindingRef::Arg("value".into()),
                },
            },
            OpRecord {
                id: OpId(3),
                op: Op::Call {
                    target: emit,
                    args: vec![Argument {
                        name: "value".into(),
                        value: ScalarExpr::RenderText(RegionId(4)),
                    }],
                    fills: vec![],
                },
            },
            OpRecord {
                id: OpId(4),
                op: Op::Call {
                    target: reverse,
                    args: vec![Argument {
                        name: "s".into(),
                        value: ScalarExpr::ReadBinding(BindingRef::Match("tail".into())),
                    }],
                    fills: vec![],
                },
            },
            OpRecord {
                id: OpId(5),
                op: Op::InsertScalar {
                    value: BindingRef::Match("head".into()),
                },
            },
        ],
        origins: module_origins,
        sources: module_sources,
        attachment: module_attachment,
        producer: producer(),
    };
    let snapshot = ResolutionSnapshot {
        units: vec![
            (
                entry_source.clone(),
                revision(UnitKind::Entry, "reverse-entry"),
            ),
            (
                module_source.clone(),
                revision(UnitKind::Module, "reverse-module"),
            ),
        ],
        imports: vec![ImportBinding {
            importer: entry_source.clone(),
            import: ImportId(0),
            target: module_source.clone(),
        }],
    };
    StaticLinker
        .link(
            &entry_source,
            UnitClosure {
                snapshot,
                units: BTreeMap::from([
                    (entry_source.clone(), RelocatableUnitIr::Entry(entry)),
                    (module_source, RelocatableUnitIr::Module(module)),
                ]),
            },
        )
        .unwrap()
}

#[test]
fn import_cycles_link_and_runtime_preserves_scope_slot_regex_and_file_bindings() {
    let linked = linked();
    assert_eq!(linked.map.symbols.len(), 1);
    assert_eq!(linked.map.relocations.len(), 1);
    let output = Instantiator
        .instantiate(&linked.program, BTreeMap::new(), Budgets::default())
        .unwrap();
    assert_eq!(output.trace.frames.len(), 2);
    assert_eq!(output.document.items.len(), 9);
    let rendered: Vec<_> = output
        .document
        .items
        .iter()
        .filter_map(|item| match item {
            DocumentItem::Text { value } => {
                Some(output.document.strings[value.0 as usize].as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(rendered, ["ab", "ab", "a", "X", "m.xml"]);
    let x_index = output.document.items.iter().position(|item| matches!(item, DocumentItem::Text { value } if output.document.strings[value.0 as usize] == "X")).unwrap();
    assert!(
        output.trace.document_items[x_index]
            .substitution_chain
            .iter()
            .any(|step| step.kind == SubstitutionKind::SlotFill)
    );
    output
        .trace
        .validate_against_document(&output.document)
        .unwrap();
}

#[test]
fn static_link_checks_signatures_in_the_complete_closure() {
    let fixture = linked();
    let entry_slot = fixture.image.entry.root_region.unit_slot;
    let RelocatableUnitIr::Entry(entry) = fixture.program.unit(entry_slot).unwrap() else {
        panic!()
    };
    let mut broken = entry.clone();
    let Op::Call { args, .. } = &mut broken.ops[1].op else {
        panic!()
    };
    args[0].name = "wrong".into();
    let e = source("entry.xml");
    let m = source("m.xml");
    let RelocatableUnitIr::Module(module) = fixture.program.unit(1 - entry_slot).unwrap() else {
        panic!()
    };
    let snapshot = ResolutionSnapshot {
        units: vec![
            (e.clone(), revision(UnitKind::Entry, "entry")),
            (m.clone(), revision(UnitKind::Module, "module")),
        ],
        imports: vec![
            ImportBinding {
                importer: e.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
            ImportBinding {
                importer: m.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
        ],
    };
    let error = StaticLinker
        .link(
            &e,
            UnitClosure {
                snapshot,
                units: BTreeMap::from([
                    (e.clone(), RelocatableUnitIr::Entry(broken)),
                    (m, RelocatableUnitIr::Module(module.clone())),
                ]),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "LNK027");
}

#[test]
fn phase_keys_have_the_intended_invalidation_boundaries() {
    let linked = linked();
    let a = InstantiateKeyProjection::new(
        "runtime-v1",
        &linked.program,
        BTreeMap::new(),
        Budgets::default(),
    );
    let first = a.digest();
    let mut changed_budget = Budgets::default();
    changed_budget.max_depth += 1;
    let b = InstantiateKeyProjection::new(
        "runtime-v1",
        &linked.program,
        BTreeMap::new(),
        changed_budget,
    );
    assert_ne!(first, b.digest());
    let units: BTreeMap<_, _> = linked
        .image
        .units
        .iter()
        .enumerate()
        .map(|(slot, unit)| {
            (
                unit.source.clone(),
                linked.program.unit(slot as u32).unwrap().clone(),
            )
        })
        .collect();
    let mut objects: BTreeMap<_, _> = linked
        .image
        .units
        .iter()
        .enumerate()
        .map(|(slot, unit)| {
            (
                unit.source.clone(),
                linked.program.object(slot as u32).unwrap(),
            )
        })
        .collect();
    objects.insert(
        linked.image.units[0].source.clone(),
        ObjectDigest::of(b"different-debug-object"),
    );
    let changed_debug = LinkedProgram::reconstruct(linked.image.clone(), units, objects).unwrap();
    let c = InstantiateKeyProjection::new(
        "runtime-v1",
        &changed_debug,
        BTreeMap::new(),
        Budgets::default(),
    );
    assert_eq!(a.document_digest(), c.document_digest());
    assert_ne!(a.digest(), c.digest());
    let snapshot = ResolutionSnapshot {
        units: vec![],
        imports: vec![],
    };
    let key = LinkKeyProjection {
        linker_abi: "link-v1".into(),
        entry: source("entry.xml"),
        resolution: snapshot,
    };
    assert_eq!(key.digest(), key.clone().digest());
}

#[test]
fn recursion_uses_explicit_frames_and_reports_the_complete_budget_chain() {
    let fixture = linked();
    let entry_slot = fixture.image.entry.root_region.unit_slot;
    let RelocatableUnitIr::Entry(entry) = fixture.program.unit(entry_slot).unwrap() else {
        panic!()
    };
    let RelocatableUnitIr::Module(module) = fixture.program.unit(1 - entry_slot).unwrap() else {
        panic!()
    };
    let mut module = module.clone();
    let symbol = module.definitions[0].symbol.clone();
    module.definitions[0].signature.slots[0].required = false;
    module.interface.definitions[0].signature.slots[0].required = false;
    module.external_symbols = vec![];
    module.ops[3].op = Op::Call {
        target: symbol,
        args: vec![Argument {
            name: "s".into(),
            value: ScalarExpr::ReadBinding(BindingRef::Arg("s".into())),
        }],
        fills: vec![],
    };
    let e = source("entry.xml");
    let m = source("m.xml");
    let snapshot = ResolutionSnapshot {
        units: vec![
            (e.clone(), revision(UnitKind::Entry, "entry")),
            (m.clone(), revision(UnitKind::Module, "module")),
        ],
        imports: vec![
            ImportBinding {
                importer: e.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
            ImportBinding {
                importer: m.clone(),
                import: ImportId(0),
                target: m.clone(),
            },
        ],
    };
    let linked = StaticLinker
        .link(
            &e,
            UnitClosure {
                snapshot,
                units: BTreeMap::from([
                    (e.clone(), RelocatableUnitIr::Entry(entry.clone())),
                    (m, RelocatableUnitIr::Module(module)),
                ]),
            },
        )
        .unwrap();
    let error = Instantiator
        .instantiate(
            &linked.program,
            BTreeMap::new(),
            Budgets {
                max_depth: 64,
                max_expansions: 100,
                max_output_bytes: 1_000_000,
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "RUN012");
    assert_eq!(error.frame_chain.len(), 64);
}

#[test]
fn recursive_scalar_argument_body_composes_non_tail_results_exactly() {
    let linked = linked_non_tail_reverse();
    let output = Instantiator
        .instantiate(&linked.program, BTreeMap::new(), Budgets::default())
        .unwrap();
    let text: String = output
        .document
        .items
        .iter()
        .filter_map(|item| match item {
            DocumentItem::Text { value } => {
                Some(output.document.strings[value.0 as usize].as_str())
            }
            _ => None,
        })
        .collect();

    assert_eq!(text, "dcba");
    assert_eq!(output.document.items.len(), 3);
    assert_eq!(output.trace.frames.len(), 10);
    assert_eq!(output.trace.document_items.len(), 3);
    let text_trace = &output.trace.document_items[1];
    assert!(text_trace.call_origin.is_some());
    assert!(
        text_trace
            .substitution_chain
            .iter()
            .any(|step| step.kind == SubstitutionKind::ScalarBody)
    );
    assert!(
        text_trace
            .substitution_chain
            .iter()
            .any(|step| step.kind == SubstitutionKind::InsertScalar)
    );
    output
        .trace
        .validate_against_document(&output.document)
        .unwrap();
}

#[test]
fn output_budget_is_checked_before_a_partial_document_can_escape() {
    let linked = linked();
    let error = Instantiator
        .instantiate(
            &linked.program,
            BTreeMap::new(),
            Budgets {
                max_output_bytes: 8,
                ..Budgets::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "RUN016");
    assert!(!error.frame_chain.is_empty());
    assert!(error.origin.is_some());
}

#[test]
fn backend_neutral_budget_counts_attributes_and_expanded_names() {
    let linked = linked();
    let mut units = BTreeMap::new();
    let mut objects = BTreeMap::new();
    for (slot, linked_unit) in linked.image.units.iter().enumerate() {
        let mut unit = linked.program.unit(slot as u32).unwrap().clone();
        if let RelocatableUnitIr::Entry(entry) = &mut unit {
            entry.header.semantic_strings.push("z".repeat(2_000));
            let Op::EmitElement { attributes, .. } = &mut entry.ops[0].op else {
                panic!()
            };
            attributes.push(Attribute {
                name: QNameId(0),
                value: StringId(2),
            });
        }
        units.insert(linked_unit.source.clone(), unit);
        objects.insert(
            linked_unit.source.clone(),
            linked.program.object(slot as u32).unwrap(),
        );
    }
    let program = LinkedProgram::reconstruct(linked.image, units, objects).unwrap();
    let error = Instantiator
        .instantiate(
            &program,
            BTreeMap::new(),
            Budgets {
                max_output_bytes: 512,
                ..Budgets::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "RUN016");
}

#[test]
fn instantiated_documents_advertise_authoritative_portable_abi() {
    let fixture = linked();
    let output = Instantiator
        .instantiate(&fixture.program, BTreeMap::new(), Budgets::default())
        .unwrap();
    assert_eq!(output.document.document_abi.0, squish_ir::DOCUMENT_ABI);
    assert_eq!(
        decode_linked_document(&encode_linked_document(&output.document)).unwrap(),
        output.document
    );
}

/// Mutates only debug provenance while retaining real linked units and qualified object identities.
fn reconstruct_with_origins(
    mut mutate: impl FnMut(u32, &mut RelocatableUnitIr),
) -> Result<LinkedProgram, LinkError> {
    let fixture = linked();
    let mut units = BTreeMap::new();
    let mut objects = BTreeMap::new();
    for (slot, linked) in fixture.image.units.iter().enumerate() {
        let mut unit = fixture.program.unit(slot as u32).unwrap().clone();
        mutate(slot as u32, &mut unit);
        units.insert(linked.source.clone(), unit);
        objects.insert(
            linked.source.clone(),
            fixture.program.object(slot as u32).unwrap(),
        );
    }
    LinkedProgram::reconstruct(fixture.image, units, objects)
}

/// Accesses test provenance without changing the production unit representation.
fn test_origins_mut(unit: &mut RelocatableUnitIr) -> &mut OriginTable {
    match unit {
        RelocatableUnitIr::Entry(unit) | RelocatableUnitIr::Pack(unit) => &mut unit.origins,
        RelocatableUnitIr::Module(unit) => &mut unit.origins,
        RelocatableUnitIr::Sopack(unit) => &mut unit.module.origins,
    }
}

/// Differential oracle for the exact previous successful, object-qualified lookup contract.
fn linear_origin_oracle(
    program: &LinkedProgram,
    slot: u32,
    kind: EntityKind,
    local: u32,
) -> Option<QualifiedOriginRef> {
    let position = program
        .unit(slot)?
        .origins()
        .entries
        .iter()
        .position(|entry| entry.entity_kind == kind && entry.local_id == local)?;
    Some(QualifiedOriginRef {
        object: program.object(slot)?,
        local: OriginId(position as u32),
    })
}

#[test]
fn origin_index_matches_linear_first_position_for_duplicates_gaps_and_each_kind() {
    let program = reconstruct_with_origins(|_, unit| {
        let origins = test_origins_mut(unit);
        origins.entries.reverse();
        let mut extra = origins.entries[0].clone();
        extra.entity_kind = EntityKind::Parameter;
        extra.local_id = u32::MAX;
        origins.entries.insert(0, extra.clone());
        extra.entity_kind = EntityKind::Region;
        origins.entries.insert(1, extra.clone());
        extra.entity_kind = EntityKind::Definition;
        origins.entries.insert(2, extra);
        let duplicates = origins
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.entity_kind,
                    EntityKind::Operation | EntityKind::Region | EntityKind::Definition
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        for mut duplicate in duplicates {
            // A conflicting span makes accidental last-match selection observable.
            duplicate.origin.span = Span { start: 1, end: 2 };
            origins.entries.push(duplicate);
        }
    })
    .unwrap();
    for slot in 0..program.image().units.len() as u32 {
        let unit = program.unit(slot).unwrap();
        for (kind, count) in [
            (EntityKind::Operation, unit.ops().len()),
            (EntityKind::Region, unit.regions().len()),
            (EntityKind::Definition, unit.definitions().len()),
        ] {
            for local in 0..count as u32 {
                assert_eq!(
                    program.origin(slot, kind, local),
                    linear_origin_oracle(&program, slot, kind, local),
                    "{slot}/{kind:?}/{local}"
                );
            }
            assert_eq!(program.origin(slot, kind, u32::MAX), None);
        }
    }
    assert_eq!(program.origin(u32::MAX, EntityKind::Operation, 0), None);
    let output = Instantiator
        .instantiate(&program, BTreeMap::new(), Budgets::default())
        .unwrap();
    output
        .trace
        .validate_against_document(&output.document)
        .unwrap();
}

#[test]
fn explicit_region_origin_wins_over_earlier_operation_origin() {
    let program =
        reconstruct_with_origins(|_, unit| test_origins_mut(unit).entries.reverse()).unwrap();
    let root = program.image().entry.root_region;
    let first = program.unit(root.unit_slot).unwrap().regions()[root.region.0 as usize].ops[0];
    let region = program
        .origin(root.unit_slot, EntityKind::Region, root.region.0)
        .unwrap();
    let operation = program
        .origin(root.unit_slot, EntityKind::Operation, first.0)
        .unwrap();
    assert!(
        operation.local.0 < region.local.0,
        "fixture puts operation first in the table"
    );
    let result = Instantiator
        .instantiate(&program, BTreeMap::new(), Budgets::default())
        .unwrap();
    assert_eq!(result.trace.frames[0].definition_origin, region);
}

#[test]
fn absent_region_origin_falls_back_to_the_first_region_operation() {
    let program = reconstruct_with_origins(|_, unit| {
        if let Some(root) = unit.root_region() {
            test_origins_mut(unit).entries.retain(|entry| {
                !(entry.entity_kind == EntityKind::Region && entry.local_id == root.0)
            });
        }
    })
    .unwrap();
    let root = program.image().entry.root_region;
    assert!(
        program
            .origin(root.unit_slot, EntityKind::Region, root.region.0)
            .is_none()
    );
    let first = program.unit(root.unit_slot).unwrap().regions()[root.region.0 as usize].ops[0];
    let result = Instantiator
        .instantiate(&program, BTreeMap::new(), Budgets::default())
        .unwrap();
    assert_eq!(
        result.trace.frames[0].definition_origin,
        linear_origin_oracle(&program, root.unit_slot, EntityKind::Operation, first.0).unwrap()
    );
}

#[test]
fn fresh_cached_and_duplicate_attachment_programs_preserve_complete_results() {
    let fixture = linked();
    let expected = Instantiator
        .instantiate(&fixture.program, BTreeMap::new(), Budgets::default())
        .unwrap();
    let cached = reconstruct_with_origins(|_, _| {}).unwrap();
    assert_eq!(
        Instantiator
            .instantiate(&cached, BTreeMap::new(), Budgets::default())
            .unwrap(),
        expected
    );
    let duplicated = reconstruct_with_origins(|_, unit| {
        let table = test_origins_mut(unit);
        table.entries.extend(table.entries.clone());
    })
    .unwrap();
    assert_eq!(
        Instantiator
            .instantiate(&duplicated, BTreeMap::new(), Budgets::default())
            .unwrap(),
        expected
    );
    let first = cached.origin(0, EntityKind::Operation, 0).unwrap();
    let second = cached.origin(1, EntityKind::Operation, 0).unwrap();
    assert_ne!(first.object, second.object);
    assert_eq!(
        first,
        linear_origin_oracle(&cached, 0, EntityKind::Operation, 0).unwrap()
    );
    assert_eq!(
        second,
        linear_origin_oracle(&cached, 1, EntityKind::Operation, 0).unwrap()
    );
}

#[test]
fn missing_definition_origin_and_invalid_operation_origin_keep_failure_semantics() {
    let missing = reconstruct_with_origins(|_, unit| {
        test_origins_mut(unit)
            .entries
            .retain(|entry| entry.entity_kind != EntityKind::Definition)
    })
    .unwrap();
    let error = Instantiator
        .instantiate(&missing, BTreeMap::new(), Budgets::default())
        .unwrap_err();
    assert_eq!(error.code, "RUN014");
    assert_eq!(error.frame_chain, vec![FrameId(0)]);
    assert!(error.origin.is_some());
    let invalid = reconstruct_with_origins(|_, unit| {
        let table = test_origins_mut(unit);
        let mut extra = table.entries[0].clone();
        extra.entity_kind = EntityKind::Operation;
        extra.local_id = u32::MAX;
        table.entries.push(extra);
    })
    .unwrap_err();
    assert_eq!(
        invalid.code, "LNK030",
        "the unchanged verifier rejects invalid operation metadata before indexing"
    );
}

#[test]
fn empty_root_without_region_origin_still_reports_run003() {
    let fixture = linked();
    let root_slot = fixture.image.entry.root_region.unit_slot;
    let RelocatableUnitIr::Entry(original) = fixture.program.unit(root_slot).unwrap() else {
        panic!()
    };
    let mut entry = original.clone();
    entry.header.imports.clear();
    entry.external_symbols.clear();
    entry.required_params.clear();
    entry.regions = vec![Region {
        id: RegionId(0),
        ops: Vec::new(),
    }];
    entry.ops.clear();
    entry.root_region = RegionId(0);
    entry.origins.entries.clear();
    entry.origins.decoded_values.clear();
    let source = entry.header.source.clone();
    let semantic = SemanticUnitDigest::of(b"empty");
    let image = LinkedImage {
        schema: entry.header.ir_schema,
        language_abi: entry.header.language_abi.clone(),
        entry: LinkedEntry {
            source: source.clone(),
            semantic_digest: semantic,
            required_params: Vec::new(),
            root_region: LinkedRegionRef {
                unit_slot: 0,
                region: RegionId(0),
            },
        },
        units: vec![LinkedUnit {
            kind: UnitKind::Entry,
            source: source.clone(),
            semantic_digest: semantic,
        }],
        definitions: Vec::new(),
        link_map: StaticLinkMap::default(),
        feature_bits: FeatureBits(0),
    };
    let program = LinkedProgram::reconstruct(
        image,
        BTreeMap::from([(source.clone(), RelocatableUnitIr::Entry(entry))]),
        BTreeMap::from([(source, ObjectDigest::of(b"empty"))]),
    )
    .unwrap();
    assert_eq!(
        Instantiator
            .instantiate(&program, BTreeMap::new(), Budgets::default())
            .unwrap_err()
            .code,
        "RUN003"
    );
}
