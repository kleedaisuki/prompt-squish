//! 链接镜像与静态链接表的规范类型化 payload。 / Canonical typed payloads for linked images and static link maps.
//!
//! 容器层负责分节、校验和版本协商；本模块只定义链接领域值的唯一字节表示。
//! 长度和 arena ID 使用最小 unsigned LEB128，协议边界则保持定宽。 / The container layer
//! owns sections, checksums, and version negotiation; this module defines only the unique byte
//! representation of link-domain values. Lengths and arena IDs use minimal unsigned LEB128 while
//! protocol boundaries remain fixed-width.

use crate::*;

const SOURCE_PROJECT: u8 = 1;
const SOURCE_AD_HOC: u8 = 2;
const UNIT_ENTRY: u8 = 1;
const UNIT_MODULE: u8 = 2;

/// 将静态链接表编码为规范 payload。 / Encodes a static link map as a canonical payload.
#[must_use]
pub fn encode_static_link_map(link_map: &StaticLinkMap) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.link_map(link_map);
    writer.bytes
}

/// 解码并验证静态链接表。 / Decodes and validates a static link map.
pub fn decode_static_link_map(bytes: &[u8]) -> Result<StaticLinkMap, DecodeError> {
    let mut reader = Reader::new(bytes);
    let link_map = reader.link_map()?;
    reader.finish()?;
    link_map
        .validate()
        .map_err(|_| DecodeError::NonCanonical("static link map invariants"))?;
    Ok(link_map)
}

/// 将完整链接镜像编码为规范 payload。 / Encodes a complete linked image as a canonical payload.
#[must_use]
pub fn encode_linked_image(image: &LinkedImage) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.u16(image.schema.major);
    writer.u16(image.schema.minor);
    writer.string(&image.language_abi.0);
    writer.entry(&image.entry);
    writer.list(&image.units, |w, unit| w.unit(unit));
    writer.list(&image.definitions, |w, definition| w.definition(definition));
    writer.link_map(&image.link_map);
    writer.u64(image.feature_bits.0);
    writer.bytes
}

/// 有界解码链接镜像，并在返回前执行全部结构验证。
/// Decodes a linked image with bounded allocation and validates all structural invariants.
pub fn decode_linked_image(bytes: &[u8]) -> Result<LinkedImage, DecodeError> {
    let mut reader = Reader::new(bytes);
    let image = LinkedImage {
        schema: Version {
            major: reader.u16()?,
            minor: reader.u16()?,
        },
        language_abi: AbiId(reader.string()?),
        entry: reader.entry()?,
        units: reader.list(|r| r.unit())?,
        definitions: reader.list(|r| r.definition())?,
        link_map: reader.link_map()?,
        feature_bits: FeatureBits(reader.u64()?),
    };
    reader.finish()?;
    image
        .validate()
        .map_err(|_| DecodeError::NonCanonical("linked image invariants"))?;
    Ok(image)
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn var(&mut self, mut value: u64) {
        loop {
            let low = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.byte(low);
                return;
            }
            self.byte(low | 0x80);
        }
    }

    fn string(&mut self, value: &str) {
        self.var(value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    fn list<T>(&mut self, values: &[T], mut encode: impl FnMut(&mut Self, &T)) {
        self.var(values.len() as u64);
        for value in values {
            encode(self, value);
        }
    }

    fn digest(&mut self, digest: Digest) {
        self.u16(digest.algorithm as u16);
        self.bytes.extend_from_slice(&digest.bytes);
    }

    fn source(&mut self, source: &SourceKey) {
        match source {
            SourceKey::Project { package, path } => {
                self.byte(SOURCE_PROJECT);
                self.u16(package.source_kind);
                self.string(&package.canonical_source);
                self.string(&package.package_name);
                self.string(&package.exact_revision);
                self.list(path, |w, component| w.string(component));
            }
            SourceKey::AdHoc { uri } => {
                self.byte(SOURCE_AD_HOC);
                self.string(uri);
            }
        }
    }

    fn symbol(&mut self, symbol: &SymbolKey) {
        self.string(&symbol.namespace_uri);
        self.string(&symbol.local_name);
    }

    fn kind(&mut self, kind: UnitKind) {
        self.byte(match kind {
            UnitKind::Entry => UNIT_ENTRY,
            UnitKind::Module => UNIT_MODULE,
        });
    }

    fn signature(&mut self, signature: &Signature) {
        self.list(&signature.params, |w, name| w.string(name));
        self.list(&signature.slots, |w, slot| {
            w.string(&slot.name);
            w.byte(u8::from(slot.required));
        });
    }

    fn def_addr(&mut self, address: DefAddr) {
        self.var(u64::from(address.unit_slot));
        self.var(u64::from(address.local_def.0));
    }

    fn region(&mut self, region: LinkedRegionRef) {
        self.var(u64::from(region.unit_slot));
        self.var(u64::from(region.region.0));
    }

    fn op_ref(&mut self, operation: LinkedOpRef) {
        self.var(u64::from(operation.unit_slot));
        self.var(u64::from(operation.op.0));
    }

    fn entry(&mut self, entry: &LinkedEntry) {
        self.source(&entry.source);
        self.digest(entry.semantic_digest.0);
        self.list(&entry.required_params, |w, name| w.string(name));
        self.region(entry.root_region);
    }

    fn unit(&mut self, unit: &LinkedUnit) {
        self.kind(unit.kind);
        self.source(&unit.source);
        self.digest(unit.semantic_digest.0);
    }

    fn definition(&mut self, definition: &LinkedMacroDef) {
        self.def_addr(definition.addr);
        self.symbol(&definition.symbol);
        self.signature(&definition.signature);
        self.region(definition.body);
    }

    fn link_map(&mut self, link_map: &StaticLinkMap) {
        self.list(&link_map.symbols, |w, (symbol, address)| {
            w.symbol(symbol);
            w.def_addr(*address);
        });
        self.list(&link_map.relocations, |w, (operation, address)| {
            w.op_ref(*operation);
            w.def_addr(*address);
        });
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn byte(&mut self) -> Result<u8, DecodeError> {
        let value = *self
            .bytes
            .get(self.position)
            .ok_or(DecodeError::Truncated)?;
        self.position += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("fixed-size slice"),
        ))
    }

    /// 读取最小 unsigned LEB128，拒绝溢出与等价长形。 / Reads minimal unsigned LEB128, rejecting overflow and long forms.
    fn var(&mut self) -> Result<u64, DecodeError> {
        let mut value = 0u64;
        for index in 0..10 {
            let byte = self.byte()?;
            let payload = u64::from(byte & 0x7f);
            if index == 9 && payload > 1 {
                return Err(DecodeError::LengthOverflow);
            }
            value |= payload << (index * 7);
            if byte & 0x80 == 0 {
                if index > 0 && payload == 0 {
                    return Err(DecodeError::NonCanonical("LEB128"));
                }
                return Ok(value);
            }
        }
        Err(DecodeError::LengthOverflow)
    }

    fn id(&mut self) -> Result<u32, DecodeError> {
        self.var()?
            .try_into()
            .map_err(|_| DecodeError::LengthOverflow)
    }

    fn length(&mut self) -> Result<usize, DecodeError> {
        self.var()?
            .try_into()
            .map_err(|_| DecodeError::LengthOverflow)
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        let length = self.length()?;
        let bytes = self.take(length)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }

    fn list<T>(
        &mut self,
        mut decode: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let count = self.length()?;
        // 每个编码元素至少占一字节；在分配前用剩余 payload 设定严格上界。
        // Every encoded element occupies at least one byte; bound allocation by the remaining payload.
        if count > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(decode(self)?);
        }
        Ok(values)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(DecodeError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(DecodeError::Truncated)?;
        self.position = end;
        Ok(value)
    }

    fn digest(&mut self) -> Result<Digest, DecodeError> {
        let algorithm = match self.u16()? {
            1 => DigestAlgorithm::Sha256,
            2 => DigestAlgorithm::Blake3,
            _ => return Err(DecodeError::UnsupportedDigest),
        };
        let bytes = self.take(32)?;
        Ok(Digest {
            algorithm,
            bytes: bytes.try_into().expect("fixed-size slice"),
        })
    }

    fn source(&mut self) -> Result<SourceKey, DecodeError> {
        match self.byte()? {
            SOURCE_PROJECT => Ok(SourceKey::Project {
                package: PackageInstanceId {
                    source_kind: self.u16()?,
                    canonical_source: self.string()?,
                    package_name: self.string()?,
                    exact_revision: self.string()?,
                },
                path: self.list(|r| r.string())?,
            }),
            SOURCE_AD_HOC => Ok(SourceKey::AdHoc {
                uri: self.string()?,
            }),
            _ => Err(DecodeError::NonCanonical("source key discriminant")),
        }
    }

    fn symbol(&mut self) -> Result<SymbolKey, DecodeError> {
        Ok(ExpandedName {
            namespace_uri: self.string()?,
            local_name: self.string()?,
        })
    }

    fn kind(&mut self) -> Result<UnitKind, DecodeError> {
        match self.byte()? {
            UNIT_ENTRY => Ok(UnitKind::Entry),
            UNIT_MODULE => Ok(UnitKind::Module),
            _ => Err(DecodeError::NonCanonical("unit kind discriminant")),
        }
    }

    fn boolean(&mut self) -> Result<bool, DecodeError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::NonCanonical("boolean discriminant")),
        }
    }

    fn signature(&mut self) -> Result<Signature, DecodeError> {
        Ok(Signature {
            params: self.list(|r| r.string())?,
            slots: self.list(|r| {
                Ok(SlotDecl {
                    name: r.string()?,
                    required: r.boolean()?,
                })
            })?,
        })
    }

    fn def_addr(&mut self) -> Result<DefAddr, DecodeError> {
        Ok(DefAddr {
            unit_slot: self.id()?,
            local_def: LocalDefId(self.id()?),
        })
    }

    fn region(&mut self) -> Result<LinkedRegionRef, DecodeError> {
        Ok(LinkedRegionRef {
            unit_slot: self.id()?,
            region: RegionId(self.id()?),
        })
    }

    fn op_ref(&mut self) -> Result<LinkedOpRef, DecodeError> {
        Ok(LinkedOpRef {
            unit_slot: self.id()?,
            op: OpId(self.id()?),
        })
    }

    fn entry(&mut self) -> Result<LinkedEntry, DecodeError> {
        Ok(LinkedEntry {
            source: self.source()?,
            semantic_digest: SemanticUnitDigest(self.digest()?),
            required_params: self.list(|r| r.string())?,
            root_region: self.region()?,
        })
    }

    fn unit(&mut self) -> Result<LinkedUnit, DecodeError> {
        Ok(LinkedUnit {
            kind: self.kind()?,
            source: self.source()?,
            semantic_digest: SemanticUnitDigest(self.digest()?),
        })
    }

    fn definition(&mut self) -> Result<LinkedMacroDef, DecodeError> {
        Ok(LinkedMacroDef {
            addr: self.def_addr()?,
            symbol: self.symbol()?,
            signature: self.signature()?,
            body: self.region()?,
        })
    }

    fn link_map(&mut self) -> Result<StaticLinkMap, DecodeError> {
        Ok(StaticLinkMap {
            symbols: self.list(|r| Ok((r.symbol()?, r.def_addr()?)))?,
            relocations: self.list(|r| Ok((r.op_ref()?, r.def_addr()?)))?,
        })
    }

    fn finish(self) -> Result<(), DecodeError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(uri: &str) -> SourceKey {
        SourceKey::AdHoc { uri: uri.into() }
    }

    fn digest(value: u8) -> SemanticUnitDigest {
        SemanticUnitDigest(Digest {
            algorithm: DigestAlgorithm::Sha256,
            bytes: [value; 32],
        })
    }

    fn symbol(local_name: &str) -> SymbolKey {
        ExpandedName {
            namespace_uri: "urn:test".into(),
            local_name: local_name.into(),
        }
    }

    fn link_map() -> StaticLinkMap {
        StaticLinkMap {
            symbols: vec![
                (
                    symbol("a"),
                    DefAddr {
                        unit_slot: 0,
                        local_def: LocalDefId(0),
                    },
                ),
                (
                    symbol("b"),
                    DefAddr {
                        unit_slot: 1,
                        local_def: LocalDefId(2),
                    },
                ),
            ],
            relocations: vec![(
                LinkedOpRef {
                    unit_slot: 0,
                    op: OpId(3),
                },
                DefAddr {
                    unit_slot: 1,
                    local_def: LocalDefId(2),
                },
            )],
        }
    }

    fn image() -> LinkedImage {
        let project = SourceKey::Project {
            package: PackageInstanceId {
                source_kind: 7,
                canonical_source: "registry+https://example.invalid/index".into(),
                package_name: "fixture".into(),
                exact_revision: "1.2.3".into(),
            },
            path: vec!["src".into(), "main.xml".into()],
        };
        LinkedImage {
            schema: Version { major: 1, minor: 0 },
            language_abi: AbiId("xmlsquish.core/1".into()),
            entry: LinkedEntry {
                source: project.clone(),
                semantic_digest: digest(1),
                required_params: vec!["locale".into()],
                root_region: LinkedRegionRef {
                    unit_slot: 0,
                    region: RegionId(4),
                },
            },
            units: vec![
                LinkedUnit {
                    kind: UnitKind::Entry,
                    source: project,
                    semantic_digest: digest(1),
                },
                LinkedUnit {
                    kind: UnitKind::Module,
                    source: source("memory:module"),
                    semantic_digest: digest(2),
                },
            ],
            definitions: vec![LinkedMacroDef {
                addr: DefAddr {
                    unit_slot: 1,
                    local_def: LocalDefId(2),
                },
                symbol: symbol("b"),
                signature: Signature {
                    params: vec!["name".into()],
                    slots: vec![SlotDecl {
                        name: "body".into(),
                        required: true,
                    }],
                },
                body: LinkedRegionRef {
                    unit_slot: 1,
                    region: RegionId(5),
                },
            }],
            link_map: link_map(),
            feature_bits: FeatureBits(0b11),
        }
    }

    #[test]
    fn linked_image_roundtrip_covers_sources_digests_signatures_and_addresses() {
        let image = image();
        image.validate().unwrap();
        let encoded = encode_linked_image(&image);
        assert_eq!(decode_linked_image(&encoded).unwrap(), image);
        assert_eq!(encode_linked_image(&image), encoded);
    }

    #[test]
    fn static_link_map_roundtrip_is_deterministic() {
        let link_map = link_map();
        link_map.validate().unwrap();
        let encoded = encode_static_link_map(&link_map);
        assert_eq!(decode_static_link_map(&encoded).unwrap(), link_map);
        assert_eq!(encode_static_link_map(&link_map), encoded);
    }

    #[test]
    fn decoder_rejects_non_minimal_lengths_and_unbounded_counts() {
        assert!(matches!(
            decode_static_link_map(&[0x80, 0x00, 0x00]),
            Err(DecodeError::NonCanonical("LEB128"))
        ));
        assert!(matches!(
            decode_static_link_map(&[0x7f]),
            Err(DecodeError::Truncated)
        ));
    }

    #[test]
    fn decoder_rejects_invalid_utf8_unknown_discriminants_and_trailing_bytes() {
        // One symbol whose namespace is a one-byte invalid UTF-8 sequence. / 一个符号，其命名空间含单字节非法 UTF-8。
        assert!(matches!(
            decode_static_link_map(&[1, 1, 0xff]),
            Err(DecodeError::InvalidUtf8)
        ));

        let mut encoded = encode_linked_image(&image());
        let source_discriminant = 4 + 1 + "xmlsquish.core/1".len();
        encoded[source_discriminant] = 0xff;
        assert!(matches!(
            decode_linked_image(&encoded),
            Err(DecodeError::NonCanonical("source key discriminant"))
        ));

        let mut encoded = encode_static_link_map(&link_map());
        encoded.push(0);
        assert!(matches!(
            decode_static_link_map(&encoded),
            Err(DecodeError::TrailingBytes)
        ));
    }

    #[test]
    fn decoder_validates_the_reconstructed_value() {
        let mut invalid = link_map();
        invalid.symbols.reverse();
        let encoded = encode_static_link_map(&invalid);
        assert!(matches!(
            decode_static_link_map(&encoded),
            Err(DecodeError::NonCanonical("static link map invariants"))
        ));

        let mut invalid = image();
        invalid.entry.root_region.unit_slot = 99;
        let encoded = encode_linked_image(&invalid);
        assert!(matches!(
            decode_linked_image(&encoded),
            Err(DecodeError::NonCanonical("linked image invariants"))
        ));
    }
}
