//! AST 到可持久 IR 的确定性 lowering。 / Deterministic AST-to-persistent-IR lowering.

use crate::{FrontendSourceContext, ast::*};
use squish_ir::*;
use squish_source::SourceBlob;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn lower(
    source: &SourceBlob,
    context: &FrontendSourceContext,
    unit: Unit,
) -> Result<RelocatableUnitIr, String> {
    let mut pool = Pools::default();
    match &unit.root {
        Root::Entry { body, .. } => pool.nodes(body),
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
        language_abi: AbiId("xmlsquish.dsl/0.3".into()),
        frontend_abi: AbiId("xmlsquish.xml/1".into()),
        regex_abi: AbiId("rust-regex/1.13.1".into()),
        source: source_key.clone(),
        imports: unit
            .imports
            .iter()
            .enumerate()
            .map(|(i, x)| ImportDecl {
                local_id: ImportId(i as u32),
                spec: x.spec.clone(),
                expected_kind: UnitKind::Module,
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
    let producer = Producer {
        tool_version: env!("CARGO_PKG_VERSION").into(),
        build_fingerprint: "squish-xml-front/xml-1".into(),
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
            let root = l.region(body)?;
            let external = l.external_symbols(BTreeSet::new());
            let origins = l.finish_origins();
            RelocatableUnitIr::Entry(EntryObject {
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
            })
        }
        Root::Module { definitions } => {
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
            RelocatableUnitIr::Module(ModuleObject {
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
            })
        }
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
