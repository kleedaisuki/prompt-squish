//! AST 到可持久 IR 的确定性 lowering。 / Deterministic AST-to-persistent-IR lowering.

use crate::{FrontendSourceContext, ast::*};
use squish_ir::*;
use squish_source::SourceBlob;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn lower(
    source: &SourceBlob,
    context: &FrontendSourceContext,
    mut unit: Unit,
) -> Result<RelocatableUnitIr, String> {
    let mut package_body = None;
    let mut is_pack = false;
    let mut is_sopack = false;
    unit.root = match unit.root {
        Root::Pack { body } => {
            is_pack = true;
            Root::Entry {
                params: Vec::new(),
                param_origins: Vec::new(),
                body,
            }
        }
        Root::Sopack { definitions, body } => {
            is_sopack = true;
            package_body = Some(body);
            Root::Module { definitions }
        }
        root => root,
    };
    let mut pool = Pools::default();
    if let Some(body) = &package_body {
        pool.nodes(body);
    }

    match &unit.root {
        Root::Entry { body, .. } => pool.nodes(body),
        Root::Pack { .. } | Root::Sopack { .. } => unreachable!("normalized package root"),
        Root::Module { definitions } => {
            for d in definitions {
                pool.nodes(&d.body)
            }
        }
    }
    let strings: Vec<_> = pool.strings.into_iter().collect();
    let qnames: Vec<_> = pool.qnames.into_iter().collect();
    let string_ids: BTreeMap<_, _> = strings
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), StringId(i as u32)))
        .collect();
    let qname_ids: BTreeMap<_, _> = qnames
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), QNameId(i as u32)))
        .collect();
    let mut regexes: Vec<_> = pool
        .regexes
        .into_iter()
        .map(|(pattern, named_captures)| RegexPattern {
            pattern: string_ids[&pattern],
            named_captures,
        })
        .collect();
    regexes.sort();
    let regex_ids: BTreeMap<_, _> = regexes
        .iter()
        .enumerate()
        .map(|(i, r)| {
            (
                (
                    strings[r.pattern.0 as usize].clone(),
                    r.named_captures.clone(),
                ),
                RegexId(i as u32),
            )
        })
        .collect();
    let source_key = source_key(source, context);
    let header = UnitHeader {
        ir_schema: Version { major: 1, minor: 0 },
        language_abi: AbiId("xmlsquish.dsl/0.4".into()),
        frontend_abi: AbiId(crate::FRONTEND_ABI.into()),
        regex_abi: AbiId("rust-regex/1.13.1".into()),
        source: source_key.clone(),
        imports: unit
            .imports
            .iter()
            .enumerate()
            .map(|(i, x)| ImportDecl {
                local_id: ImportId(i as u32),
                spec: x.spec.clone(),
                expected_kind: x.expected_kind,
            })
            .collect(),
        semantic_strings: strings,
        qnames,
        regexes,
        feature_bits: FeatureBits(0),
    };
    let attachment = UnitSourceAttachment {
        source: source_key.clone(),
        source_digest: SourceDigest::of(source.bytes()),
        source_record: SourceRef(0),
    };
    let sources = source_archive(source, source_key);
    // Root provenance covers the owning XML document, even when its body is empty.
    // Spans address the UTF-8 payload rather than including its optional BOM.
    let root_loc = Loc {
        start: 0,
        end: source.bytes().len() - usize::from(sources.records[0].bom_len),
    };
    let producer = Producer {
        tool_version: env!("CARGO_PKG_VERSION").into(),
        build_fingerprint: "squish-xml-front/xml-2".into(),
    };
    let mut l = Lower {
        strings: string_ids,
        qnames: qname_ids,
        regexes: regex_ids,
        regions: Vec::new(),
        ops: Vec::new(),
        origins: origin_prefix(&unit),
        decoded: decoded_prefix(&unit),
    };
    let result = match unit.root {
        Root::Entry { params, body, .. } => {
            let root = l.root_region(body, root_loc)?;
            let external = l.external_symbols(BTreeSet::new());
            let origins = l.finish_origins();
            let object = EntryObject {
                header,
                required_params: params,
                root_region: root,
                external_symbols: external,
                regions: l.regions,
                ops: l.ops,
                origins,
                sources,
                attachment,
                producer,
            };
            if is_pack {
                RelocatableUnitIr::Pack(object)
            } else {
                RelocatableUnitIr::Entry(object)
            }
        }
        Root::Module { definitions } => {
            let package_root = package_body
                .map(|body| l.root_region(body, root_loc))
                .transpose()?;
            let local: BTreeSet<_> = definitions.iter().map(|d| d.symbol.clone()).collect();
            let mut defs = Vec::new();
            for (index, d) in definitions.into_iter().enumerate() {
                let body = l.region(d.body)?;
                defs.push(MacroDef {
                    id: LocalDefId(index as u32),
                    symbol: d.symbol,
                    signature: Signature {
                        params: d.params,
                        slots: d
                            .slots
                            .into_iter()
                            .map(|(name, required)| SlotDecl { name, required })
                            .collect(),
                    },
                    body,
                })
            }
            let external = l.external_symbols(local);
            let interface = InterfaceSummary {
                definitions: defs
                    .iter()
                    .map(|d| InterfaceDef {
                        id: d.id,
                        symbol: d.symbol.clone(),
                        signature: d.signature.clone(),
                    })
                    .collect(),
            };
            let origins = l.finish_origins();
            let module = ModuleObject {
                header,
                definitions: defs,
                external_symbols: external,
                interface,
                regions: l.regions,
                ops: l.ops,
                origins,
                sources,
                attachment,
                producer,
            };
            if is_sopack {
                RelocatableUnitIr::Sopack(SopackObject {
                    module,
                    root_region: package_root.expect("sopack root"),
                })
            } else {
                RelocatableUnitIr::Module(module)
            }
        }
        Root::Pack { .. } | Root::Sopack { .. } => unreachable!("normalized package root"),
    };
    Ok(result)
}

#[derive(Default)]
struct Pools {
    strings: BTreeSet<String>,
    qnames: BTreeSet<ExpandedName>,
    regexes: BTreeSet<(String, Vec<String>)>,
}
impl Pools {
    fn value(&mut self, v: &Value) {
        match v {
            Value::Literal(s) => {
                self.strings.insert(s.clone());
            }
            Value::Get(_) => {}
            Value::Body(n) => self.nodes(n),
        }
    }
    fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.node(n))
        }
    }
    fn node(&mut self, n: &Node) {
        match &n.kind {
            Kind::Asset { path, name } | Kind::Include { path, name, .. } => {
                self.strings.insert(path.clone());
                self.strings.insert(name.clone());
            }
            Kind::Text(s) | Kind::Comment(s) => {
                self.strings.insert(s.clone());
            }
            Kind::Pi { target, data } => {
                self.strings.insert(target.clone());
                self.strings.insert(data.clone());
            }
            Kind::Element {
                name,
                attrs,
                children,
            } => {
                self.qnames.insert(name.clone());
                for (name, value) in attrs {
                    self.qnames.insert(name.clone());
                    self.strings.insert(value.clone());
                }
                self.nodes(children)
            }
            Kind::Insert(_) | Kind::Slot { .. } => {}
            Kind::If {
                input,
                pattern,
                captures,
                body,
            } => {
                self.value(input);
                self.strings.insert(pattern.clone());
                self.regexes.insert((pattern.clone(), captures.clone()));
                self.nodes(body)
            }
            Kind::Invoke { args, fills, .. } => {
                for a in args {
                    self.value(&a.value)
                }
                for f in fills {
                    self.nodes(&f.body)
                }
            }
        }
    }
}

struct Lower {
    strings: BTreeMap<String, StringId>,
    qnames: BTreeMap<ExpandedName, QNameId>,
    regexes: BTreeMap<(String, Vec<String>), RegexId>,
    regions: Vec<Region>,
    ops: Vec<OpRecord>,
    origins: Vec<(EntityKind, u32, Loc, Option<String>)>,
    decoded: Vec<DecodedValueMap>,
}
impl Lower {
    /// Attaches an owning source origin independently of the root's emitted operations.
    /// Declaration-only Sopacks and empty packs remain instantiable and diagnosable.
    fn root_region(&mut self, nodes: Vec<Node>, loc: Loc) -> Result<RegionId, String> {
        let id = self.region(nodes)?;
        self.origins.push((EntityKind::Region, id.0, loc, None));
        Ok(id)
    }

    fn region(&mut self, nodes: Vec<Node>) -> Result<RegionId, String> {
        let id = RegionId(self.regions.len() as u32);
        self.regions.push(Region {
            id,
            ops: Vec::new(),
        });
        let mut owned = Vec::with_capacity(nodes.len());
        for node in nodes {
            let op = self.operation(node)?;
            let op_id = OpId(self.ops.len() as u32);
            let (loc, lexical, payload, decoded_values) = op;
            self.ops.push(OpRecord {
                id: op_id,
                op: payload,
            });
            self.origins
                .push((EntityKind::Operation, op_id.0, loc, lexical));
            for decoded in decoded_values {
                self.decoded.push(DecodedValueMap {
                    owner: DecodedOwner {
                        entity_kind: EntityKind::Operation,
                        local_id: op_id.0,
                        field: decoded.field,
                    },
                    segments: decoded
                        .segments
                        .into_iter()
                        .map(|segment| DecodedSegment {
                            value_utf8_range: Span {
                                start: segment.value_start as u64,
                                end: segment.value_end as u64,
                            },
                            source_span: Span {
                                start: segment.source_start as u64,
                                end: segment.source_end as u64,
                            },
                            syntax: segment.syntax,
                        })
                        .collect(),
                });
            }
            owned.push(op_id)
        }
        self.regions[id.0 as usize].ops = owned;
        Ok(id)
    }
    fn operation(
        &mut self,
        node: Node,
    ) -> Result<(Loc, Option<String>, Op, Vec<RawDecodedValue>), String> {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.operation_inner(node))
    }
    fn operation_inner(
        &mut self,
        node: Node,
    ) -> Result<(Loc, Option<String>, Op, Vec<RawDecodedValue>), String> {
        let (loc, lexical, kind, decoded) = node.take();
        let op = match kind {
            Kind::Asset { path, name } => Op::Asset {
                path: self.strings[&path],
                name: self.strings[&name],
            },
            Kind::Include { import, path, name } => Op::Include {
                import: ImportId(import),
                path: self.strings[&path],
                name: self.strings[&name],
            },
            Kind::Text(value) => Op::EmitText {
                value: self.strings[&value],
            },
            Kind::Comment(value) => Op::EmitComment {
                value: self.strings[&value],
            },
            Kind::Pi { target, data } => Op::EmitPi {
                target: self.strings[&target],
                data: self.strings[&data],
            },
            Kind::Element {
                name,
                attrs,
                children,
            } => {
                let children = self.region(children)?;
                Op::EmitElement {
                    name: self.qnames[&name],
                    attributes: attrs
                        .into_iter()
                        .map(|(name, value)| Attribute {
                            name: self.qnames[&name],
                            value: self.strings[&value],
                        })
                        .collect(),
                    children,
                }
            }
            Kind::Insert(value) => Op::InsertScalar {
                value: binding(&value)?,
            },
            Kind::Slot { name } => Op::ReadSlot { name },
            Kind::If {
                input,
                pattern,
                captures,
                body,
            } => {
                let input = match input {
                    Value::Literal(v) => MatchInput::Literal(self.strings[&v]),
                    Value::Get(v) => MatchInput::ReadBinding(binding(&v)?),
                    Value::Body(_) => return Err("ifr input cannot be a body".into()),
                };
                let matched = self.region(body)?;
                Op::MatchRegex {
                    input,
                    pattern: self.regexes[&(pattern, captures.clone())],
                    captures,
                    matched,
                }
            }
            Kind::Invoke {
                target,
                args,
                fills,
            } => {
                let args = args
                    .into_iter()
                    .map(|a| {
                        Ok(squish_ir::Argument {
                            name: a.name,
                            value: match a.value {
                                Value::Literal(v) => ScalarExpr::Literal(self.strings[&v]),
                                Value::Get(v) => ScalarExpr::ReadBinding(binding(&v)?),
                                Value::Body(v) => ScalarExpr::RenderText(self.region(v)?),
                            },
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let fills = fills
                    .into_iter()
                    .map(|f| {
                        Ok(squish_ir::Fill {
                            name: f.name,
                            body: self.region(f.body)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Op::Call {
                    target,
                    args,
                    fills,
                }
            }
        };
        Ok((loc, lexical, op, decoded))
    }
    fn external_symbols(&self, locals: BTreeSet<ExpandedName>) -> Vec<ExpandedName> {
        let mut result = BTreeSet::new();
        for op in &self.ops {
            if let Op::Call { target, .. } = &op.op
                && !locals.contains(target)
            {
                result.insert(target.clone());
            }
        }
        result.into_iter().collect()
    }
    fn finish_origins(&self) -> OriginTable {
        let debug: Vec<_> = self
            .origins
            .iter()
            .filter_map(|x| x.3.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let ids: BTreeMap<_, _> = debug
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), DebugStringId(i as u32)))
            .collect();
        OriginTable {
            entries: self
                .origins
                .iter()
                .map(|(entity_kind, local_id, loc, lexical_qname)| OriginEntry {
                    entity_kind: *entity_kind,
                    local_id: *local_id,
                    origin: Origin {
                        source: SourceRef(0),
                        span: Span {
                            start: loc.start as u64,
                            end: loc.end as u64,
                        },
                        lexical_qname: lexical_qname.as_ref().map(|v| ids[v]),
                        syntax_kind: SyntaxKind(1),
                    },
                })
                .collect(),
            debug_strings: debug,
            decoded_values: self.decoded.clone(),
        }
    }
}
fn origin_prefix(unit: &Unit) -> Vec<(EntityKind, u32, Loc, Option<String>)> {
    let mut entries = Vec::new();
    for (i, import) in unit.imports.iter().enumerate() {
        entries.push((EntityKind::Import, i as u32, import.loc, None))
    }
    if let Root::Module { definitions } = &unit.root {
        let mut parameter_id = 0;
        for (i, d) in definitions.iter().enumerate() {
            entries.push((
                EntityKind::Definition,
                i as u32,
                d.loc,
                Some(d.lexical.clone()),
            ));
            for decoded in d.decoded.iter().filter(|value| value.field != "symbol") {
                entries.push((
                    EntityKind::Parameter,
                    parameter_id,
                    decoded_loc(decoded),
                    None,
                ));
                parameter_id += 1;
            }
        }
    } else if let Root::Entry { param_origins, .. } = &unit.root {
        for (id, (loc, _)) in param_origins.iter().enumerate() {
            entries.push((EntityKind::Parameter, id as u32, *loc, None));
        }
    }
    entries
}

fn decoded_loc(value: &RawDecodedValue) -> Loc {
    Loc {
        start: value
            .segments
            .first()
            .map_or(0, |segment| segment.source_start),
        end: value
            .segments
            .last()
            .map_or(1, |segment| segment.source_end),
    }
}

fn decoded_prefix(unit: &Unit) -> Vec<DecodedValueMap> {
    let mut maps = Vec::new();
    for (id, import) in unit.imports.iter().enumerate() {
        maps.push(decoded_map(
            EntityKind::Import,
            id as u32,
            import.decoded.clone(),
        ));
    }
    match &unit.root {
        Root::Pack { .. } | Root::Sopack { .. } => unreachable!("normalized package root"),
        Root::Entry { param_origins, .. } => {
            for (id, (_, decoded)) in param_origins.iter().enumerate() {
                maps.push(decoded_map(
                    EntityKind::Parameter,
                    id as u32,
                    decoded.clone(),
                ));
            }
        }
        Root::Module { definitions } => {
            let mut parameter_id = 0;
            for (id, definition) in definitions.iter().enumerate() {
                for decoded in &definition.decoded {
                    let (kind, local_id) = if decoded.field == "symbol" {
                        (EntityKind::Definition, id as u32)
                    } else {
                        let current = parameter_id;
                        parameter_id += 1;
                        (EntityKind::Parameter, current)
                    };
                    maps.push(decoded_map(kind, local_id, decoded.clone()));
                }
            }
        }
    }
    maps
}

fn decoded_map(kind: EntityKind, local_id: u32, decoded: RawDecodedValue) -> DecodedValueMap {
    DecodedValueMap {
        owner: DecodedOwner {
            entity_kind: kind,
            local_id,
            field: decoded.field,
        },
        segments: decoded
            .segments
            .into_iter()
            .map(|segment| DecodedSegment {
                value_utf8_range: Span {
                    start: segment.value_start as u64,
                    end: segment.value_end as u64,
                },
                source_span: Span {
                    start: segment.source_start as u64,
                    end: segment.source_end as u64,
                },
                syntax: segment.syntax,
            })
            .collect(),
    }
}
fn binding(value: &str) -> Result<BindingRef, String> {
    let (scope, name) = value.split_once('.').ok_or("invalid binding")?;
    Ok(match scope {
        "file" => BindingRef::File(match name {
            "uri" => FileBinding::Uri,
            "dir" => FileBinding::Dir,
            "name" => FileBinding::Name,
            _ => return Err(format!("unknown file binding {name}")),
        }),
        "arg" => BindingRef::Arg(name.into()),
        "match" => BindingRef::Match(name.into()),
        _ => return Err(format!("unknown binding scope {scope}")),
    })
}
fn source_key(source: &SourceBlob, context: &FrontendSourceContext) -> SourceKey {
    SourceKey::Project {
        package: context.package_instance().clone(),
        path: source
            .id()
            .path()
            .as_str()
            .split('/')
            .map(str::to_owned)
            .collect(),
    }
}
fn source_archive(source: &SourceBlob, key: SourceKey) -> SourceArchive {
    let bytes = source.bytes();
    let bom_len = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    let payload = &bytes[bom_len..];
    let mut lines = vec![0];
    for (i, b) in payload.iter().enumerate() {
        if *b == b'\n' && i + 1 < payload.len() {
            lines.push((i + 1) as u64)
        }
    }
    let digest = SourceDigest::of(bytes);
    SourceArchive {
        records: vec![SourceRecord {
            key,
            digest,
            bom_len: bom_len as u8,
            exact_bytes: BlobRef {
                digest: digest.0,
                byte_len: bytes.len() as u64,
            },
            line_start_offsets: lines,
        }],
    }
}

#[cfg(test)]
mod root_origin_tests {
    use super::*;
    use squish_source::{
        LogicalPath, PackageId, SnapshotBuilder, SourceId, SourceLocator, SourceProvider,
    };
    use std::io;

    struct Memory(Vec<u8>);
    impl SourceProvider for Memory {
        fn read(&self, _: &SourceLocator) -> io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    /// Exercises provenance without depending on linker first-operation fallback behavior.
    #[test]
    fn empty_and_declaration_only_roots_have_explicit_payload_origins() {
        for (root, body) in [
            ("entry", ""),
            ("pack", ""),
            ("sopack", ""),
            (
                "sopack",
                r#"<xs:import src="dep.xml"/><xs:macro name="m:hello"><hello/></xs:macro>"#,
            ),
        ] {
            for bom in [false, true] {
                let text = format!(
                    r#"<xs:{root} xmlns:xs="{}" xmlns:m="urn:m">{body}</xs:{root}>"#,
                    crate::DSL_NAMESPACE
                );
                let mut bytes = if bom {
                    vec![0xef, 0xbb, 0xbf]
                } else {
                    Vec::new()
                };
                bytes.extend_from_slice(text.as_bytes());
                let mut snapshot = SnapshotBuilder::new(Memory(bytes));
                let source = snapshot
                    .load(
                        SourceId::new(
                            PackageId::new("fixture").unwrap(),
                            LogicalPath::new("root.xml").unwrap(),
                        ),
                        SourceLocator::file("unused"),
                    )
                    .unwrap();
                let context = FrontendSourceContext::new(PackageInstanceId {
                    source_kind: 1,
                    canonical_source: "workspace:fixture".into(),
                    package_name: "fixture".into(),
                    exact_revision: "fixture@1".into(),
                });
                let unit = crate::compile(&source, &context).unwrap().unit;
                let id = unit.root_region().unwrap();
                let origins: Vec<_> = unit
                    .origins()
                    .entries
                    .iter()
                    .filter(|entry| {
                        entry.entity_kind == EntityKind::Region && entry.local_id == id.0
                    })
                    .collect();
                assert_eq!(origins.len(), 1, "{root}, BOM={bom}");
                assert_eq!(origins[0].origin.source, SourceRef(0));
                assert_eq!(
                    origins[0].origin.span,
                    Span {
                        start: 0,
                        end: text.len() as u64
                    }
                );
                assert!(unit.regions()[id.0 as usize].ops.is_empty());
            }
        }
    }

    #[test]
    fn root_origin_uses_allocated_region_id_not_an_assumed_zero() {
        let mut lower = Lower {
            strings: BTreeMap::new(),
            qnames: BTreeMap::new(),
            regexes: BTreeMap::new(),
            regions: Vec::new(),
            ops: Vec::new(),
            origins: Vec::new(),
            decoded: Vec::new(),
        };
        lower.region(Vec::new()).unwrap();
        let id = lower
            .root_region(Vec::new(), Loc { start: 7, end: 19 })
            .unwrap();
        assert_eq!(id, RegionId(1));
        assert_eq!(lower.origins[0].0, EntityKind::Region);
        assert_eq!(lower.origins[0].1, id.0);
        assert_eq!(lower.origins[0].2.start, 7);
        assert_eq!(lower.origins[0].2.end, 19);
    }
}
