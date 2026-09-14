//! 可重定位单元的规范类型化 payload。 / Canonical typed payload for relocatable units.

use crate::*;

/// 编码完整的 module/entry 值；顺序是 IR 的一部分且不会被隐式重排。
/// Encodes a complete module/entry value; IR order is preserved rather than normalized implicitly.
#[must_use]
pub fn encode_relocatable_unit(unit: &RelocatableUnitIr) -> Vec<u8> {
    let mut w = W(Vec::new());
    match unit {
        RelocatableUnitIr::Module(v) => {
            w.tag(1);
            w.module(v);
        }
        RelocatableUnitIr::Entry(v) => {
            w.tag(2);
            w.entry(v);
        }
    }
    w.0
}

/// 有界解码完整单元，拒绝未知 discriminant、非最小 LEB128、非法 UTF-8 和结构无效值。
/// Decodes a complete unit with canonical scalar and post-decode structural validation.
pub fn decode_relocatable_unit(bytes: &[u8]) -> Result<RelocatableUnitIr, DecodeError> {
    let mut r = R { b: bytes, p: 0 };
    let v = match r.tag()? {
        1 => RelocatableUnitIr::Module(r.module()?),
        2 => RelocatableUnitIr::Entry(r.entry()?),
        _ => return Err(DecodeError::NonCanonical("relocatable unit discriminant")),
    };
    r.done()?;
    v.validate()
        .map_err(|_| DecodeError::NonCanonical("invalid relocatable unit"))?;
    Ok(v)
}

struct W(Vec<u8>);
impl W {
    fn tag(&mut self, v: u8) {
        self.0.push(v)
    }
    fn var(&mut self, mut v: u64) {
        loop {
            let mut b = (v & 127) as u8;
            v >>= 7;
            if v != 0 {
                b |= 128
            }
            self.0.push(b);
            if v == 0 {
                break;
            }
        }
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes())
    }
    fn u32(&mut self, v: u32) {
        self.var(v as u64)
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes())
    }
    fn bool(&mut self, v: bool) {
        self.tag(v as u8)
    }
    fn str(&mut self, v: &str) {
        self.var(v.len() as u64);
        self.0.extend(v.as_bytes())
    }
    fn list<T>(&mut self, v: &[T], f: impl Fn(&mut Self, &T)) {
        self.var(v.len() as u64);
        for x in v {
            f(self, x)
        }
    }
    fn opt<T>(&mut self, v: &Option<T>, f: impl Fn(&mut Self, &T)) {
        match v {
            None => self.tag(0),
            Some(x) => {
                self.tag(1);
                f(self, x)
            }
        }
    }
    fn digest(&mut self, v: &Digest) {
        self.u16(v.algorithm as u16);
        self.0.extend(v.bytes)
    }
    fn source(&mut self, v: &SourceKey) {
        match v {
            SourceKey::Project { package, path } => {
                self.tag(1);
                self.u16(package.source_kind);
                self.str(&package.canonical_source);
                self.str(&package.package_name);
                self.str(&package.exact_revision);
                self.list(path, |w, x| w.str(x))
            }
            SourceKey::AdHoc { uri } => {
                self.tag(2);
                self.str(uri)
            }
        }
    }
    fn name(&mut self, v: &ExpandedName) {
        self.str(&v.namespace_uri);
        self.str(&v.local_name)
    }
    fn import_spec(&mut self, v: &ImportSpec) {
        match v {
            ImportSpec::RelativeUri(x) => {
                self.tag(1);
                self.str(x)
            }
            ImportSpec::AbsoluteFileUri(x) => {
                self.tag(2);
                self.str(x)
            }
            ImportSpec::PackageExport {
                dependency_alias,
                export,
            } => {
                self.tag(3);
                self.str(dependency_alias);
                self.str(export)
            }
        }
    }
    fn sig(&mut self, v: &Signature) {
        self.list(&v.params, |w, x| w.str(x));
        self.list(&v.slots, |w, x| {
            w.str(&x.name);
            w.bool(x.required)
        })
    }
    fn header(&mut self, v: &UnitHeader) {
        self.u16(v.ir_schema.major);
        self.u16(v.ir_schema.minor);
        self.str(&v.language_abi.0);
        self.str(&v.frontend_abi.0);
        self.str(&v.regex_abi.0);
        self.source(&v.source);
        self.list(&v.imports, |w, x| {
            w.u32(x.local_id.0);
            w.import_spec(&x.spec);
            w.tag(match x.expected_kind {
                UnitKind::Entry => 1,
                UnitKind::Module => 2,
            })
        });
        self.list(&v.semantic_strings, |w, x| w.str(x));
        self.list(&v.qnames, |w, x| w.name(x));
        self.list(&v.regexes, |w, x| {
            w.u32(x.pattern.0);
            w.list(&x.named_captures, |w, n| w.str(n))
        });
        self.u64(v.feature_bits.0)
    }
    fn binding(&mut self, v: &BindingRef) {
        match v {
            BindingRef::File(x) => {
                self.tag(1);
                self.tag(match x {
                    FileBinding::Uri => 1,
                    FileBinding::Dir => 2,
                    FileBinding::Name => 3,
                })
            }
            BindingRef::Arg(x) => {
                self.tag(2);
                self.str(x)
            }
            BindingRef::Match(x) => {
                self.tag(3);
                self.str(x)
            }
        }
    }
    fn scalar(&mut self, v: &ScalarExpr) {
        match v {
            ScalarExpr::Literal(x) => {
                self.tag(1);
                self.u32(x.0)
            }
            ScalarExpr::ReadBinding(x) => {
                self.tag(2);
                self.binding(x)
            }
            ScalarExpr::RenderText(x) => {
                self.tag(3);
                self.u32(x.0)
            }
        }
    }
    fn op(&mut self, v: &Op) {
        match v {
            Op::EmitText { value } => {
                self.tag(1);
                self.u32(value.0)
            }
            Op::EmitComment { value } => {
                self.tag(2);
                self.u32(value.0)
            }
            Op::EmitPi { target, data } => {
                self.tag(3);
                self.u32(target.0);
                self.u32(data.0)
            }
            Op::EmitElement {
                name,
                attributes,
                children,
            } => {
                self.tag(4);
                self.u32(name.0);
                self.list(attributes, |w, x| {
                    w.u32(x.name.0);
                    w.u32(x.value.0)
                });
                self.u32(children.0)
            }
            Op::InsertScalar { value } => {
                self.tag(5);
                self.binding(value)
            }
            Op::MatchRegex {
                input,
                pattern,
                captures,
                matched,
            } => {
                self.tag(6);
                match input {
                    MatchInput::Literal(x) => {
                        self.tag(1);
                        self.u32(x.0)
                    }
                    MatchInput::ReadBinding(x) => {
                        self.tag(2);
                        self.binding(x)
                    }
                }
                self.u32(pattern.0);
                self.list(captures, |w, x| w.str(x));
                self.u32(matched.0)
            }
            Op::ReadSlot { name } => {
                self.tag(7);
                self.str(name)
            }
            Op::Call {
                target,
                args,
                fills,
            } => {
                self.tag(8);
                self.name(target);
                self.list(args, |w, x| {
                    w.str(&x.name);
                    w.scalar(&x.value)
                });
                self.list(fills, |w, x| {
                    w.str(&x.name);
                    w.u32(x.body.0)
                })
            }
        }
    }
    fn common(&mut self, regions: &[Region], ops: &[OpRecord]) {
        self.list(regions, |w, x| {
            w.u32(x.id.0);
            w.list(&x.ops, |w, id| w.u32(id.0))
        });
        self.list(ops, |w, x| {
            w.u32(x.id.0);
            w.op(&x.op)
        })
    }
    fn origins(&mut self, v: &OriginTable) {
        self.list(&v.entries, |w, x| {
            w.tag(match x.entity_kind {
                EntityKind::Import => 1,
                EntityKind::Definition => 2,
                EntityKind::Region => 3,
                EntityKind::Operation => 4,
                EntityKind::Parameter => 5,
                EntityKind::Slot => 6,
                EntityKind::ExternalSymbol => 7,
            });
            w.u32(x.local_id);
            w.u32(x.origin.source.0);
            w.u64(x.origin.span.start);
            w.u64(x.origin.span.end);
            w.opt(&x.origin.lexical_qname, |w, x| w.u32(x.0));
            w.u16(x.origin.syntax_kind.0)
        });
        self.list(&v.debug_strings, |w, x| w.str(x));
        self.list(&v.decoded_values, |w, x| {
            w.tag(match x.owner.entity_kind {
                EntityKind::Import => 1,
                EntityKind::Definition => 2,
                EntityKind::Region => 3,
                EntityKind::Operation => 4,
                EntityKind::Parameter => 5,
                EntityKind::Slot => 6,
                EntityKind::ExternalSymbol => 7,
            });
            w.u32(x.owner.local_id);
            w.str(&x.owner.field);
            w.list(&x.segments, |w, s| {
                w.u64(s.value_utf8_range.start);
                w.u64(s.value_utf8_range.end);
                w.u64(s.source_span.start);
                w.u64(s.source_span.end);
                w.tag(match s.syntax {
                    DecodedSyntax::LiteralText => 1,
                    DecodedSyntax::CharacterReference => 2,
                    DecodedSyntax::EntityReference => 3,
                    DecodedSyntax::CData => 4,
                })
            })
        })
    }
    fn archive(&mut self, v: &SourceArchive) {
        self.list(&v.records, |w, x| {
            w.source(&x.key);
            w.digest(&x.digest.0);
            w.tag(x.bom_len);
            w.digest(&x.exact_bytes.digest);
            w.u64(x.exact_bytes.byte_len);
            w.list(&x.line_start_offsets, |w, n| w.u64(*n))
        })
    }
    fn attach(&mut self, v: &UnitSourceAttachment) {
        self.source(&v.source);
        self.digest(&v.source_digest.0);
        self.u32(v.source_record.0)
    }
    fn producer(&mut self, v: &Producer) {
        self.str(&v.tool_version);
        self.str(&v.build_fingerprint)
    }
    fn module(&mut self, v: &ModuleObject) {
        self.header(&v.header);
        self.list(&v.definitions, |w, x| {
            w.u32(x.id.0);
            w.name(&x.symbol);
            w.sig(&x.signature);
            w.u32(x.body.0)
        });
        self.list(&v.external_symbols, |w, x| w.name(x));
        self.list(&v.interface.definitions, |w, x| {
            w.u32(x.id.0);
            w.name(&x.symbol);
            w.sig(&x.signature)
        });
        self.common(&v.regions, &v.ops);
        self.origins(&v.origins);
        self.archive(&v.sources);
        self.attach(&v.attachment);
        self.producer(&v.producer)
    }
    fn entry(&mut self, v: &EntryObject) {
        self.header(&v.header);
        self.list(&v.required_params, |w, x| w.str(x));
        self.u32(v.root_region.0);
        self.list(&v.external_symbols, |w, x| w.name(x));
        self.common(&v.regions, &v.ops);
        self.origins(&v.origins);
        self.archive(&v.sources);
        self.attach(&v.attachment);
        self.producer(&v.producer)
    }
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> R<'a> {
    fn tag(&mut self) -> Result<u8, DecodeError> {
        let x = *self.b.get(self.p).ok_or(DecodeError::Truncated)?;
        self.p += 1;
        Ok(x)
    }
    fn var(&mut self) -> Result<u64, DecodeError> {
        let start = self.p;
        let mut n = 0u64;
        for sh in (0..=63).step_by(7) {
            let b = self.tag()?;
            if sh == 63 && b > 1 {
                return Err(DecodeError::LengthOverflow);
            }
            n |= ((b & 127) as u64) << sh;
            if b & 128 == 0 {
                if self.p - start > 1 && b == 0 {
                    return Err(DecodeError::NonCanonical("LEB128"));
                }
                return Ok(n);
            }
        }
        Err(DecodeError::LengthOverflow)
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let e = self.p.checked_add(2).ok_or(DecodeError::LengthOverflow)?;
        let a: self::ResultBytes<2> = self
            .b
            .get(self.p..e)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap();
        self.p = e;
        Ok(u16::from_le_bytes(a))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        u32::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        let e = self.p.checked_add(8).ok_or(DecodeError::LengthOverflow)?;
        let a: [u8; 8] = self
            .b
            .get(self.p..e)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap();
        self.p = e;
        Ok(u64::from_le_bytes(a))
    }
    fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.tag()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::NonCanonical("boolean")),
        }
    }
    fn str(&mut self) -> Result<String, DecodeError> {
        let n = usize::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)?;
        let e = self.p.checked_add(n).ok_or(DecodeError::LengthOverflow)?;
        let s = core::str::from_utf8(self.b.get(self.p..e).ok_or(DecodeError::Truncated)?)
            .map_err(|_| DecodeError::InvalidUtf8)?;
        self.p = e;
        Ok(s.into())
    }
    fn list<T>(
        &mut self,
        mut f: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let n = usize::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)?;
        if n > self.b.len().saturating_sub(self.p) {
            return Err(DecodeError::Truncated);
        }
        let mut v = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            v.push(f(self)?)
        }
        Ok(v)
    }
    fn opt<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        match self.tag()? {
            0 => Ok(None),
            1 => Ok(Some(f(self)?)),
            _ => Err(DecodeError::NonCanonical("option")),
        }
    }
    fn digest(&mut self) -> Result<Digest, DecodeError> {
        let algorithm = match self.u16()? {
            1 => DigestAlgorithm::Sha256,
            2 => DigestAlgorithm::Blake3,
            _ => return Err(DecodeError::UnsupportedDigest),
        };
        let e = self.p.checked_add(32).ok_or(DecodeError::LengthOverflow)?;
        let bytes = self
            .b
            .get(self.p..e)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap();
        self.p = e;
        Ok(Digest { algorithm, bytes })
    }
    fn source(&mut self) -> Result<SourceKey, DecodeError> {
        match self.tag()? {
            1 => Ok(SourceKey::Project {
                package: PackageInstanceId {
                    source_kind: self.u16()?,
                    canonical_source: self.str()?,
                    package_name: self.str()?,
                    exact_revision: self.str()?,
                },
                path: self.list(|r| r.str())?,
            }),
            2 => Ok(SourceKey::AdHoc { uri: self.str()? }),
            _ => Err(DecodeError::NonCanonical("source key")),
        }
    }
    fn name(&mut self) -> Result<ExpandedName, DecodeError> {
        Ok(ExpandedName {
            namespace_uri: self.str()?,
            local_name: self.str()?,
        })
    }
    fn import_spec(&mut self) -> Result<ImportSpec, DecodeError> {
        match self.tag()? {
            1 => Ok(ImportSpec::RelativeUri(self.str()?)),
            2 => Ok(ImportSpec::AbsoluteFileUri(self.str()?)),
            3 => Ok(ImportSpec::PackageExport {
                dependency_alias: self.str()?,
                export: self.str()?,
            }),
            _ => Err(DecodeError::NonCanonical("import spec")),
        }
    }
    fn sig(&mut self) -> Result<Signature, DecodeError> {
        Ok(Signature {
            params: self.list(|r| r.str())?,
            slots: self.list(|r| {
                Ok(SlotDecl {
                    name: r.str()?,
                    required: r.bool()?,
                })
            })?,
        })
    }
    fn header(&mut self) -> Result<UnitHeader, DecodeError> {
        Ok(UnitHeader {
            ir_schema: Version {
                major: self.u16()?,
                minor: self.u16()?,
            },
            language_abi: AbiId(self.str()?),
            frontend_abi: AbiId(self.str()?),
            regex_abi: AbiId(self.str()?),
            source: self.source()?,
            imports: self.list(|r| {
                Ok(ImportDecl {
                    local_id: ImportId(r.u32()?),
                    spec: r.import_spec()?,
                    expected_kind: match r.tag()? {
                        1 => UnitKind::Entry,
                        2 => UnitKind::Module,
                        _ => return Err(DecodeError::NonCanonical("unit kind")),
                    },
                })
            })?,
            semantic_strings: self.list(|r| r.str())?,
            qnames: self.list(|r| r.name())?,
            regexes: self.list(|r| {
                Ok(RegexPattern {
                    pattern: StringId(r.u32()?),
                    named_captures: r.list(|r| r.str())?,
                })
            })?,
            feature_bits: FeatureBits(self.u64()?),
        })
    }
    fn binding(&mut self) -> Result<BindingRef, DecodeError> {
        match self.tag()? {
            1 => Ok(BindingRef::File(match self.tag()? {
                1 => FileBinding::Uri,
                2 => FileBinding::Dir,
                3 => FileBinding::Name,
                _ => return Err(DecodeError::NonCanonical("file binding")),
            })),
            2 => Ok(BindingRef::Arg(self.str()?)),
            3 => Ok(BindingRef::Match(self.str()?)),
            _ => Err(DecodeError::NonCanonical("binding")),
        }
    }
    fn scalar(&mut self) -> Result<ScalarExpr, DecodeError> {
        match self.tag()? {
            1 => Ok(ScalarExpr::Literal(StringId(self.u32()?))),
            2 => Ok(ScalarExpr::ReadBinding(self.binding()?)),
            3 => Ok(ScalarExpr::RenderText(RegionId(self.u32()?))),
            _ => Err(DecodeError::NonCanonical("scalar expression")),
        }
    }
    fn op(&mut self) -> Result<Op, DecodeError> {
        Ok(match self.tag()? {
            1 => Op::EmitText {
                value: StringId(self.u32()?),
            },
            2 => Op::EmitComment {
                value: StringId(self.u32()?),
            },
            3 => Op::EmitPi {
                target: StringId(self.u32()?),
                data: StringId(self.u32()?),
            },
            4 => Op::EmitElement {
                name: QNameId(self.u32()?),
                attributes: self.list(|r| {
                    Ok(Attribute {
                        name: QNameId(r.u32()?),
                        value: StringId(r.u32()?),
                    })
                })?,
                children: RegionId(self.u32()?),
            },
            5 => Op::InsertScalar {
                value: self.binding()?,
            },
            6 => Op::MatchRegex {
                input: match self.tag()? {
                    1 => MatchInput::Literal(StringId(self.u32()?)),
                    2 => MatchInput::ReadBinding(self.binding()?),
                    _ => return Err(DecodeError::NonCanonical("match input")),
                },
                pattern: RegexId(self.u32()?),
                captures: self.list(|r| r.str())?,
                matched: RegionId(self.u32()?),
            },
            7 => Op::ReadSlot { name: self.str()? },
            8 => Op::Call {
                target: self.name()?,
                args: self.list(|r| {
                    Ok(Argument {
                        name: r.str()?,
                        value: r.scalar()?,
                    })
                })?,
                fills: self.list(|r| {
                    Ok(Fill {
                        name: r.str()?,
                        body: RegionId(r.u32()?),
                    })
                })?,
            },
            _ => return Err(DecodeError::NonCanonical("operation")),
        })
    }
    fn common(&mut self) -> Result<(Vec<Region>, Vec<OpRecord>), DecodeError> {
        Ok((
            self.list(|r| {
                Ok(Region {
                    id: RegionId(r.u32()?),
                    ops: r.list(|r| Ok(OpId(r.u32()?)))?,
                })
            })?,
            self.list(|r| {
                Ok(OpRecord {
                    id: OpId(r.u32()?),
                    op: r.op()?,
                })
            })?,
        ))
    }
    fn entity(&mut self) -> Result<EntityKind, DecodeError> {
        Ok(match self.tag()? {
            1 => EntityKind::Import,
            2 => EntityKind::Definition,
            3 => EntityKind::Region,
            4 => EntityKind::Operation,
            5 => EntityKind::Parameter,
            6 => EntityKind::Slot,
            7 => EntityKind::ExternalSymbol,
            _ => return Err(DecodeError::NonCanonical("entity kind")),
        })
    }
    fn origins(&mut self) -> Result<OriginTable, DecodeError> {
        Ok(OriginTable {
            entries: self.list(|r| {
                Ok(OriginEntry {
                    entity_kind: r.entity()?,
                    local_id: r.u32()?,
                    origin: Origin {
                        source: SourceRef(r.u32()?),
                        span: Span {
                            start: r.u64()?,
                            end: r.u64()?,
                        },
                        lexical_qname: r.opt(|r| Ok(DebugStringId(r.u32()?)))?,
                        syntax_kind: SyntaxKind(r.u16()?),
                    },
                })
            })?,
            debug_strings: self.list(|r| r.str())?,
            decoded_values: self.list(|r| {
                Ok(DecodedValueMap {
                    owner: DecodedOwner {
                        entity_kind: r.entity()?,
                        local_id: r.u32()?,
                        field: r.str()?,
                    },
                    segments: r.list(|r| {
                        Ok(DecodedSegment {
                            value_utf8_range: Span {
                                start: r.u64()?,
                                end: r.u64()?,
                            },
                            source_span: Span {
                                start: r.u64()?,
                                end: r.u64()?,
                            },
                            syntax: match r.tag()? {
                                1 => DecodedSyntax::LiteralText,
                                2 => DecodedSyntax::CharacterReference,
                                3 => DecodedSyntax::EntityReference,
                                4 => DecodedSyntax::CData,
                                _ => return Err(DecodeError::NonCanonical("decoded syntax")),
                            },
                        })
                    })?,
                })
            })?,
        })
    }
    fn archive(&mut self) -> Result<SourceArchive, DecodeError> {
        Ok(SourceArchive {
            records: self.list(|r| {
                Ok(SourceRecord {
                    key: r.source()?,
                    digest: SourceDigest(r.digest()?),
                    bom_len: r.tag()?,
                    exact_bytes: BlobRef {
                        digest: r.digest()?,
                        byte_len: r.u64()?,
                    },
                    line_start_offsets: r.list(|r| r.u64())?,
                })
            })?,
        })
    }
    fn attach(&mut self) -> Result<UnitSourceAttachment, DecodeError> {
        Ok(UnitSourceAttachment {
            source: self.source()?,
            source_digest: SourceDigest(self.digest()?),
            source_record: SourceRef(self.u32()?),
        })
    }
    fn producer(&mut self) -> Result<Producer, DecodeError> {
        Ok(Producer {
            tool_version: self.str()?,
            build_fingerprint: self.str()?,
        })
    }
    fn module(&mut self) -> Result<ModuleObject, DecodeError> {
        let header = self.header()?;
        let definitions = self.list(|r| {
            Ok(MacroDef {
                id: LocalDefId(r.u32()?),
                symbol: r.name()?,
                signature: r.sig()?,
                body: RegionId(r.u32()?),
            })
        })?;
        let external_symbols = self.list(|r| r.name())?;
        let interface = InterfaceSummary {
            definitions: self.list(|r| {
                Ok(InterfaceDef {
                    id: LocalDefId(r.u32()?),
                    symbol: r.name()?,
                    signature: r.sig()?,
                })
            })?,
        };
        let (regions, ops) = self.common()?;
        Ok(ModuleObject {
            header,
            definitions,
            external_symbols,
            interface,
            regions,
            ops,
            origins: self.origins()?,
            sources: self.archive()?,
            attachment: self.attach()?,
            producer: self.producer()?,
        })
    }
    fn entry(&mut self) -> Result<EntryObject, DecodeError> {
        let header = self.header()?;
        let required_params = self.list(|r| r.str())?;
        let root_region = RegionId(self.u32()?);
        let external_symbols = self.list(|r| r.name())?;
        let (regions, ops) = self.common()?;
        Ok(EntryObject {
            header,
            required_params,
            root_region,
            external_symbols,
            regions,
            ops,
            origins: self.origins()?,
            sources: self.archive()?,
            attachment: self.attach()?,
            producer: self.producer()?,
        })
    }
    fn done(self) -> Result<(), DecodeError> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}
type ResultBytes<const N: usize> = [u8; N];

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceKey {
        SourceKey::AdHoc {
            uri: "file:///unit.xml".into(),
        }
    }
    fn archive() -> SourceArchive {
        let bytes = Digest::sha256("fixture", b"0123456789");
        SourceArchive {
            records: vec![SourceRecord {
                key: source(),
                digest: SourceDigest(bytes),
                bom_len: 0,
                exact_bytes: BlobRef {
                    digest: bytes,
                    byte_len: 10,
                },
                line_start_offsets: vec![0],
            }],
        }
    }
    fn header() -> UnitHeader {
        UnitHeader {
            ir_schema: Version { major: 1, minor: 0 },
            language_abi: AbiId("lang/1".into()),
            frontend_abi: AbiId("xml/1".into()),
            regex_abi: AbiId("regex/1".into()),
            source: source(),
            imports: vec![ImportDecl {
                local_id: ImportId(0),
                spec: ImportSpec::PackageExport {
                    dependency_alias: "dep".into(),
                    export: "main".into(),
                },
                expected_kind: UnitKind::Module,
            }],
            semantic_strings: vec!["a".into(), "b".into(), "c".into()],
            qnames: vec![ExpandedName {
                namespace_uri: "urn:x".into(),
                local_name: "x".into(),
            }],
            regexes: vec![RegexPattern {
                pattern: StringId(0),
                named_captures: vec!["cap".into()],
            }],
            feature_bits: FeatureBits(3),
        }
    }
    fn origins(n: u32) -> OriginTable {
        OriginTable {
            entries: (0..n)
                .map(|i| OriginEntry {
                    entity_kind: EntityKind::Operation,
                    local_id: i,
                    origin: Origin {
                        source: SourceRef(0),
                        span: Span {
                            start: i as u64 + 1,
                            end: i as u64 + 2,
                        },
                        lexical_qname: None,
                        syntax_kind: SyntaxKind(1),
                    },
                })
                .collect(),
            debug_strings: vec![],
            decoded_values: vec![],
        }
    }
    fn module() -> RelocatableUnitIr {
        let ops = vec![
            Op::EmitText { value: StringId(0) },
            Op::EmitComment { value: StringId(1) },
            Op::EmitPi {
                target: StringId(0),
                data: StringId(2),
            },
            Op::EmitElement {
                name: QNameId(0),
                attributes: vec![Attribute {
                    name: QNameId(0),
                    value: StringId(0),
                }],
                children: RegionId(1),
            },
            Op::InsertScalar {
                value: BindingRef::File(FileBinding::Uri),
            },
            Op::MatchRegex {
                input: MatchInput::ReadBinding(BindingRef::Arg("arg".into())),
                pattern: RegexId(0),
                captures: vec!["cap".into()],
                matched: RegionId(2),
            },
            Op::ReadSlot {
                name: "slot".into(),
            },
            Op::Call {
                target: ExpandedName {
                    namespace_uri: "urn:z".into(),
                    local_name: "z".into(),
                },
                args: vec![Argument {
                    name: "a".into(),
                    value: ScalarExpr::RenderText(RegionId(3)),
                }],
                fills: vec![Fill {
                    name: "s".into(),
                    body: RegionId(4),
                }],
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(i, op)| OpRecord {
            id: OpId(i as u32),
            op,
        })
        .collect::<Vec<_>>();
        let definition = MacroDef {
            id: LocalDefId(0),
            symbol: ExpandedName {
                namespace_uri: "urn:def".into(),
                local_name: "root".into(),
            },
            signature: Signature::default(),
            body: RegionId(0),
        };
        RelocatableUnitIr::Module(ModuleObject {
            header: header(),
            definitions: vec![definition.clone()],
            external_symbols: vec![ExpandedName {
                namespace_uri: "urn:z".into(),
                local_name: "z".into(),
            }],
            interface: InterfaceSummary {
                definitions: vec![InterfaceDef {
                    id: definition.id,
                    symbol: definition.symbol,
                    signature: definition.signature,
                }],
            },
            regions: vec![
                Region {
                    id: RegionId(0),
                    ops: (0..8).map(OpId).collect(),
                },
                Region {
                    id: RegionId(1),
                    ops: vec![],
                },
                Region {
                    id: RegionId(2),
                    ops: vec![],
                },
                Region {
                    id: RegionId(3),
                    ops: vec![],
                },
                Region {
                    id: RegionId(4),
                    ops: vec![],
                },
            ],
            ops,
            origins: origins(8),
            sources: archive(),
            attachment: UnitSourceAttachment {
                source: source(),
                source_digest: archive().records[0].digest,
                source_record: SourceRef(0),
            },
            producer: Producer {
                tool_version: "test".into(),
                build_fingerprint: "fixed".into(),
            },
        })
    }
    fn entry() -> RelocatableUnitIr {
        RelocatableUnitIr::Entry(EntryObject {
            header: header(),
            required_params: vec!["arg".into()],
            root_region: RegionId(0),
            external_symbols: vec![],
            regions: vec![Region {
                id: RegionId(0),
                ops: vec![],
            }],
            ops: vec![],
            origins: origins(0),
            sources: archive(),
            attachment: UnitSourceAttachment {
                source: source(),
                source_digest: archive().records[0].digest,
                source_record: SourceRef(0),
            },
            producer: Producer {
                tool_version: "test".into(),
                build_fingerprint: "fixed".into(),
            },
        })
    }

    #[test]
    fn module_all_ops_roundtrip_and_is_deterministic() {
        let u = module();
        let a = encode_relocatable_unit(&u);
        assert_eq!(a, encode_relocatable_unit(&u));
        assert_eq!(decode_relocatable_unit(&a), Ok(u));
    }
    #[test]
    fn entry_roundtrip() {
        let u = entry();
        assert_eq!(decode_relocatable_unit(&encode_relocatable_unit(&u)), Ok(u));
    }

    #[test]
    fn unit_decoder_rejects_unknown_schema_and_features() {
        let mut value = module();
        if let RelocatableUnitIr::Module(unit) = &mut value {
            unit.header.ir_schema.major = 2;
        }
        assert!(decode_relocatable_unit(&encode_relocatable_unit(&value)).is_err());
        if let RelocatableUnitIr::Module(unit) = &mut value {
            unit.header.ir_schema.major = 1;
            unit.header.feature_bits = FeatureBits(1 << 63);
        }
        assert!(decode_relocatable_unit(&encode_relocatable_unit(&value)).is_err());
    }
    #[test]
    fn bad_discriminants_are_rejected() {
        assert!(matches!(
            decode_relocatable_unit(&[99]),
            Err(DecodeError::NonCanonical("relocatable unit discriminant"))
        ));
    }
    #[test]
    fn nonminimal_lengths_and_bad_utf8_are_rejected() {
        assert_eq!(
            decode_relocatable_unit(&[1, 1, 0, 0, 0, 0x80, 0]),
            Err(DecodeError::NonCanonical("LEB128"))
        );
        let mut b = encode_relocatable_unit(&entry());
        b[9] = 0xff;
        assert_eq!(decode_relocatable_unit(&b), Err(DecodeError::InvalidUtf8));
    }
}
