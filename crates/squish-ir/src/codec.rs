//! 规范 little-endian 分节容器。 / Canonical little-endian sectioned container.

use crate::{DebugDigest, Digest, DigestAlgorithm, ObjectDigest, SemanticUnitDigest};
use core::fmt;

const MAGIC: &[u8; 8] = b"XSQIR\r\n\x1a";
const HEADER: usize = 20;
const DIRECTORY: usize = 58;
/// v1 必识别的语义描述 section。 / Required v1 semantic descriptor section.
pub const SECTION_DESCRIPTOR: u32 = 1;
/// 可重定位单元语义 section。 / Relocatable unit semantic section.
pub const SECTION_UNIT: u32 = 2;
/// 链接镜像语义 section。 / Linked-image semantic section.
pub const SECTION_LINKED_IMAGE: u32 = 3;
/// 文档语义 section。 / Linked-document semantic section.
pub const SECTION_DOCUMENT: u32 = 4;
/// 静态 source/origin 调试 section。 / Static source/origin debug section.
pub const SECTION_DEBUG: u32 = 0x8000_0001;
/// Debug bundle 元数据 section。 / Debug-bundle metadata section.
pub const SECTION_BUNDLE_METADATA: u32 = 0x8000_0010;
/// Debug bundle expansion trace section。 / Debug-bundle expansion-trace section.
pub const SECTION_BUNDLE_TRACE: u32 = 0x8000_0011;
/// Debug bundle artifact byte-map section。 / Debug-bundle artifact byte-map section.
pub const SECTION_BUNDLE_ARTIFACT_MAP: u32 = 0x8000_0012;
/// Debug bundle source archives section。 / Debug-bundle source-archive section.
pub const SECTION_BUNDLE_SOURCES: u32 = 0x8000_0013;
/// Debug bundle link-trace section。 / Debug-bundle link-trace section.
pub const SECTION_BUNDLE_LINK_TRACE: u32 = 0x8000_0014;

/// 容器对象种类。 / Container object kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ContainerKind {
    Module = 1,
    Entry = 2,
    LinkedImage = 3,
    LinkedDocument = 4,
    ExpansionTrace = 5,
    ArtifactMap = 6,
    DebugBundle = 7,
}
impl ContainerKind {
    fn from_raw(v: u16) -> Result<Self, DecodeError> {
        Ok(match v {
            1 => Self::Module,
            2 => Self::Entry,
            3 => Self::LinkedImage,
            4 => Self::LinkedDocument,
            5 => Self::ExpansionTrace,
            6 => Self::ArtifactMap,
            7 => Self::DebugBundle,
            _ => return Err(DecodeError::UnknownKind(v)),
        })
    }
}

/// Section 分类 flags。 / Section classification flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionFlags(pub u32);
impl SectionFlags {
    pub const SEMANTIC: Self = Self(0b101);
    pub const DEBUG: Self = Self(0b010);
    /// 是否影响语义摘要。 / Whether this section affects semantic identity.
    #[must_use]
    pub fn is_semantic(self) -> bool {
        self.0 & 1 != 0
    }
    /// 未知 section 是否必须拒绝。 / Whether an unknown section must be rejected.
    #[must_use]
    pub fn is_critical(self) -> bool {
        self.0 & 4 != 0
    }
    fn valid(self) -> bool {
        self.0 & !7 == 0
            && ((self.0 & 1 != 0) ^ (self.0 & 2 != 0))
            && (!self.is_semantic() || self.is_critical())
    }
}
/// 一个已校验 section。 / One validated section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    pub tag: u32,
    pub flags: SectionFlags,
    pub payload: Vec<u8>,
}
/// 完整 canonical object。 / Complete canonical object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Container {
    pub(crate) major: u16,
    pub(crate) minor: u16,
    pub(crate) kind: ContainerKind,
    pub(crate) sections: Vec<Section>,
}

impl Container {
    /// 容器 schema。 / Container schema.
    #[must_use]
    pub fn schema(&self) -> (u16, u16) {
        (self.major, self.minor)
    }
    /// 对象种类。 / Object kind.
    #[must_use]
    pub fn kind(&self) -> ContainerKind {
        self.kind
    }
    /// 已按 tag 排序的 sections。 / Sections sorted by tag.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }
    /// 创建 v1 容器并排序 section。 / Creates a v1 container and sorts sections.
    pub fn v1(kind: ContainerKind, mut sections: Vec<Section>) -> Result<Self, DecodeError> {
        u32::try_from(sections.len()).map_err(|_| DecodeError::LengthOverflow)?;
        let directory = DIRECTORY
            .checked_mul(sections.len())
            .and_then(|length| HEADER.checked_add(length))
            .ok_or(DecodeError::LengthOverflow)?;
        sections.iter().try_fold(directory, |length, section| {
            length
                .checked_add(section.payload.len())
                .ok_or(DecodeError::LengthOverflow)
        })?;
        sections.sort_by_key(|s| s.tag);
        let c = Self {
            major: 1,
            minor: 0,
            kind,
            sections,
        };
        validate_sections(&c.sections)?;
        Ok(c)
    }
    /// 完整对象摘要。 / Complete-object digest.
    #[must_use]
    pub fn object_digest(&self) -> ObjectDigest {
        ObjectDigest::of(&encode_container(self))
    }
    /// 仅语义 section 的摘要。 / Semantic-only digest.
    #[must_use]
    pub fn semantic_digest(&self) -> SemanticUnitDigest {
        let mut p = Vec::new();
        p.extend_from_slice(&self.major.to_le_bytes());
        p.extend_from_slice(&(self.kind as u16).to_le_bytes());
        for s in self.sections.iter().filter(|s| s.flags.is_semantic()) {
            p.extend_from_slice(&s.tag.to_le_bytes());
            p.extend_from_slice(&(s.payload.len() as u64).to_le_bytes());
            p.extend_from_slice(&s.payload)
        }
        SemanticUnitDigest::of(&p)
    }
    /// 仅调试/source section 的摘要。 / Debug/source-only digest.
    #[must_use]
    pub fn debug_digest(&self) -> DebugDigest {
        let mut p = Vec::new();
        p.extend_from_slice(&self.major.to_le_bytes());
        p.extend_from_slice(&(self.kind as u16).to_le_bytes());
        for s in self.sections.iter().filter(|s| !s.flags.is_semantic()) {
            p.extend_from_slice(&s.tag.to_le_bytes());
            p.extend_from_slice(&(s.payload.len() as u64).to_le_bytes());
            p.extend_from_slice(&s.payload)
        }
        DebugDigest::of(&p)
    }
}

/// 语义 descriptor，明确 dialect、epoch 和 feature。 / Semantic descriptor naming dialect, epoch and features.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticDescriptor {
    pub dialect: String,
    pub semantic_epoch: u64,
    pub feature_bits: u64,
}
impl SemanticDescriptor {
    /// 编为最小 LEB128 + UTF-8。 / Encodes as minimal LEB128 plus UTF-8.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.string(&self.dialect);
        w.var(self.semantic_epoch);
        w.u64(self.feature_bits);
        w.0
    }
    /// 解码且拒绝非 canonical 输入。 / Decodes and rejects non-canonical input.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let v = Self {
            dialect: r.string()?,
            semantic_epoch: r.var()?,
            feature_bits: r.u64()?,
        };
        r.finish()?;
        Ok(v)
    }
}

/// 编码已经验证的容器。 / Encodes a validated container.
#[must_use]
pub fn encode_container(c: &Container) -> Vec<u8> {
    let count = c.sections.len();
    let payload_start = HEADER + DIRECTORY * count;
    let mut out = Vec::with_capacity(
        payload_start + c.sections.iter().map(|s| s.payload.len()).sum::<usize>(),
    );
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&c.major.to_le_bytes());
    out.extend_from_slice(&c.minor.to_le_bytes());
    out.extend_from_slice(&(c.kind as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    let mut off = payload_start as u64;
    for s in &c.sections {
        out.extend_from_slice(&s.tag.to_le_bytes());
        out.extend_from_slice(&s.flags.0.to_le_bytes());
        out.extend_from_slice(&off.to_le_bytes());
        out.extend_from_slice(&(s.payload.len() as u64).to_le_bytes());
        out.extend_from_slice(&(DigestAlgorithm::Sha256 as u16).to_le_bytes());
        out.extend_from_slice(&section_digest(s).bytes);
        off += s.payload.len() as u64
    }
    for s in &c.sections {
        out.extend_from_slice(&s.payload)
    }
    out
}

/// 有界解码、校验 checksum，然后才返回值。 / Bounded decode that validates checksums before returning.
pub fn decode_container(bytes: &[u8]) -> Result<Container, DecodeError> {
    if bytes.len() < HEADER {
        return Err(DecodeError::Truncated);
    }
    if &bytes[..8] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let major = u16le(bytes, 8)?;
    let minor = u16le(bytes, 10)?;
    if major != 1 {
        return Err(DecodeError::UnsupportedMajor(major));
    }
    let kind = ContainerKind::from_raw(u16le(bytes, 12)?)?;
    if u16le(bytes, 14)? != 0 {
        return Err(DecodeError::NonCanonical("container flags"));
    }
    let count = u32le(bytes, 16)? as usize;
    let dir_len = DIRECTORY
        .checked_mul(count)
        .and_then(|n| HEADER.checked_add(n))
        .ok_or(DecodeError::LengthOverflow)?;
    if dir_len > bytes.len() {
        return Err(DecodeError::Truncated);
    }
    let mut sections = Vec::with_capacity(count.min(1024));
    let mut expected = dir_len as u64;
    let mut last = None;
    for i in 0..count {
        let p = HEADER + i * DIRECTORY;
        let tag = u32le(bytes, p)?;
        if last.is_some_and(|x| tag <= x) {
            return Err(DecodeError::NonCanonical("section tag order"));
        }
        last = Some(tag);
        let flags = SectionFlags(u32le(bytes, p + 4)?);
        if !flags.valid() {
            return Err(DecodeError::BadSectionFlags(tag));
        }
        if flags.is_critical() && !known_tag(tag) {
            return Err(DecodeError::UnknownCriticalSection(tag));
        }
        let off = u64le(bytes, p + 8)?;
        let len = u64le(bytes, p + 16)?;
        if off != expected {
            return Err(DecodeError::NonCanonical("section offset"));
        }
        let end = off.checked_add(len).ok_or(DecodeError::LengthOverflow)?;
        if end > bytes.len() as u64 {
            return Err(DecodeError::Truncated);
        }
        if u16le(bytes, p + 24)? != DigestAlgorithm::Sha256 as u16 {
            return Err(DecodeError::UnsupportedDigest);
        }
        let payload = bytes[off as usize..end as usize].to_vec();
        let s = Section {
            tag,
            flags,
            payload,
        };
        if section_digest(&s).bytes != bytes[p + 26..p + 58] {
            return Err(DecodeError::Checksum(tag));
        }
        expected = end;
        sections.push(s)
    }
    if expected != bytes.len() as u64 {
        return Err(DecodeError::TrailingBytes);
    }
    validate_sections(&sections)?;
    Ok(Container {
        major,
        minor,
        kind,
        sections,
    })
}

fn known_tag(t: u32) -> bool {
    matches!(
        t,
        SECTION_DESCRIPTOR
            | SECTION_UNIT
            | SECTION_LINKED_IMAGE
            | SECTION_DOCUMENT
            | SECTION_DEBUG
            | SECTION_BUNDLE_METADATA
            | SECTION_BUNDLE_TRACE
            | SECTION_BUNDLE_ARTIFACT_MAP
            | SECTION_BUNDLE_SOURCES
            | SECTION_BUNDLE_LINK_TRACE
    )
}
fn validate_sections(s: &[Section]) -> Result<(), DecodeError> {
    let mut last = None;
    for x in s {
        if last.is_some_and(|v| x.tag <= v) {
            return Err(DecodeError::NonCanonical("section tag order"));
        }
        if !x.flags.valid() {
            return Err(DecodeError::BadSectionFlags(x.tag));
        }
        last = Some(x.tag)
    }
    let descriptor = s
        .iter()
        .find(|section| section.tag == SECTION_DESCRIPTOR)
        .ok_or(DecodeError::MissingDescriptor)?;
    if !descriptor.flags.is_semantic() {
        return Err(DecodeError::NonCanonical("descriptor classification"));
    }
    SemanticDescriptor::decode(&descriptor.payload)?;
    Ok(())
}
fn section_digest(s: &Section) -> Digest {
    let mut p = Vec::with_capacity(12 + s.payload.len());
    p.extend_from_slice(&s.tag.to_le_bytes());
    p.extend_from_slice(&(s.payload.len() as u64).to_le_bytes());
    p.extend_from_slice(&s.payload);
    Digest::sha256("section", &p)
}
fn u16le(b: &[u8], p: usize) -> Result<u16, DecodeError> {
    Ok(u16::from_le_bytes(
        b.get(p..p + 2)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}
fn u32le(b: &[u8], p: usize) -> Result<u32, DecodeError> {
    Ok(u32::from_le_bytes(
        b.get(p..p + 4)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}
fn u64le(b: &[u8], p: usize) -> Result<u64, DecodeError> {
    Ok(u64::from_le_bytes(
        b.get(p..p + 8)
            .ok_or(DecodeError::Truncated)?
            .try_into()
            .unwrap(),
    ))
}

#[derive(Default)]
pub(crate) struct Writer(pub Vec<u8>);
impl Writer {
    pub fn var(&mut self, mut n: u64) {
        loop {
            let mut b = (n & 127) as u8;
            n >>= 7;
            if n != 0 {
                b |= 128
            }
            self.0.push(b);
            if n == 0 {
                break;
            }
        }
    }
    pub fn u64(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_le_bytes())
    }
    pub fn string(&mut self, s: &str) {
        self.var(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes())
    }
}
pub(crate) struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn var(&mut self) -> Result<u64, DecodeError> {
        let start = self.p;
        let mut n = 0u64;
        for shift in (0..=63).step_by(7) {
            let b = *self.b.get(self.p).ok_or(DecodeError::Truncated)?;
            self.p += 1;
            if shift == 63 && b > 1 {
                return Err(DecodeError::LengthOverflow);
            }
            n |= ((b & 127) as u64) << shift;
            if b & 128 == 0 {
                if self.p - start > 1 && b == 0 {
                    return Err(DecodeError::NonCanonical("LEB128"));
                }
                return Ok(n);
            }
        }
        Err(DecodeError::LengthOverflow)
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        let n = u64le(self.b, self.p)?;
        self.p += 8;
        Ok(n)
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let n = usize::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)?;
        let e = self.p.checked_add(n).ok_or(DecodeError::LengthOverflow)?;
        let s = core::str::from_utf8(self.b.get(self.p..e).ok_or(DecodeError::Truncated)?)
            .map_err(|_| DecodeError::InvalidUtf8)?;
        self.p = e;
        Ok(s.into())
    }
    fn finish(self) -> Result<(), DecodeError> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// 任何错误均发生在对象发布之前。 / Every failure occurs before object publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    Truncated,
    BadMagic,
    UnsupportedMajor(u16),
    UnknownKind(u16),
    BadSectionFlags(u32),
    UnknownCriticalSection(u32),
    UnsupportedDigest,
    Checksum(u32),
    LengthOverflow,
    InvalidUtf8,
    TrailingBytes,
    NonCanonical(&'static str),
    MissingDescriptor,
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IR decode error: {self:?}")
    }
}
impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Container {
        Container::v1(
            ContainerKind::Module,
            vec![
                Section {
                    tag: SECTION_DESCRIPTOR,
                    flags: SectionFlags::SEMANTIC,
                    payload: SemanticDescriptor {
                        dialect: "xmlsquish.core".into(),
                        semantic_epoch: 1,
                        feature_bits: 0,
                    }
                    .encode(),
                },
                Section {
                    tag: 99,
                    flags: SectionFlags::DEBUG,
                    payload: b"future".to_vec(),
                },
            ],
        )
        .unwrap()
    }
    #[test]
    fn deterministic_roundtrip() {
        let c = fixture();
        let a = encode_container(&c);
        let golden = a.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            golden,
            concat!(
                "58535149520d0a1a0100000001000000020000000100000005000000880000000000000018000000000000000100",
                "2cdafd71a10a901e558b06dd7ec3b57be111dcde5ab340feab90b7730d3310b66300000002000000a000000000000000",
                "060000000000000001008f4d136e4c4a9194f41259b7e882ddf98d48c67ecf4e3ffb68e714aa0a3c4b950e786d6c7371",
                "756973682e636f7265010000000000000000667574757265"
            )
        );
        assert_eq!(a, encode_container(&c));
        assert_eq!(decode_container(&a).unwrap(), c)
    }
    #[test]
    fn corruption_is_rejected() {
        let mut b = encode_container(&fixture());
        *b.last_mut().unwrap() ^= 1;
        assert!(matches!(
            decode_container(&b),
            Err(DecodeError::Checksum(99))
        ))
    }

    #[test]
    fn truncation_and_schema_major_are_rejected() {
        let mut bytes = encode_container(&fixture());
        assert_eq!(decode_container(&bytes[..19]), Err(DecodeError::Truncated));
        bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_container(&bytes),
            Err(DecodeError::UnsupportedMajor(2))
        );
    }
    #[test]
    fn unknown_optional_skips_but_required_rejects() {
        let c = fixture();
        assert!(decode_container(&encode_container(&c)).is_ok());
        let mut bad = c;
        bad.sections[1].flags = SectionFlags::SEMANTIC;
        assert!(matches!(
            decode_container(&encode_container(&bad)),
            Err(DecodeError::UnknownCriticalSection(99))
        ))
    }
    #[test]
    fn semantic_and_artifact_identity_are_separate() {
        let a = fixture();
        let mut b = a.clone();
        b.sections[1].payload.push(0);
        assert_eq!(a.semantic_digest(), b.semantic_digest());
        assert_ne!(a.object_digest(), b.object_digest())
    }
    #[test]
    fn descriptor_rejects_nonminimal_leb() {
        assert!(matches!(
            SemanticDescriptor::decode(&[0x80, 0]),
            Err(DecodeError::NonCanonical("LEB128"))
        ))
    }
}
