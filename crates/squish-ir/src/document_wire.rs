//! 链接文档 IR 的规范二进制 payload。 / Canonical binary payload for linked-document IR.
//!
//! 该格式是容器 `SECTION_DOCUMENT` 的有类型内容，而不是另一个容器层。所有长度和
//! arena ID 使用最小 unsigned LEB128；schema 与 feature bits 保持固定宽度，以便工具
//! 无需遍历 payload 即可辨认协议边界。 / This format is the typed content of the container's
//! `SECTION_DOCUMENT`, not another container layer. All lengths and arena IDs use minimal unsigned
//! LEB128; schema and feature bits remain fixed-width so tools can identify protocol boundaries
//! without walking the payload.

use crate::{
    AbiId, Attribute, DecodeError, DocumentItem, DocumentRegion, DocumentRegionId, ExpandedName,
    FeatureBits, LinkedDocumentIr, QNameId, StringId, Validate, Version,
};

const ITEM_TEXT: u8 = 0;
const ITEM_COMMENT: u8 = 1;
const ITEM_PI: u8 = 2;
const ITEM_ELEMENT_START: u8 = 3;
const ITEM_ELEMENT_END: u8 = 4;

/// 将已构造的链接文档编码为规范 payload。 / Encodes a constructed linked document as a canonical payload.
///
/// 调用者应在进入持久化边界前执行 [`Validate::validate`]。解码路径始终执行完整验证，
/// 因而不可信字节无法构造无效的 IR。 / Callers should run [`Validate::validate`] before the
/// persistence boundary. Decoding always performs full validation, so untrusted bytes cannot
/// construct invalid IR.
#[must_use]
pub fn encode_linked_document(document: &LinkedDocumentIr) -> Vec<u8> {
    let mut w = Writer::default();
    w.u16(document.schema.major);
    w.u16(document.schema.minor);
    w.string(&document.document_abi.0);
    w.var(u64::from(document.root.0));

    w.len(document.strings.len());
    for value in &document.strings {
        w.string(value);
    }

    w.len(document.qnames.len());
    for name in &document.qnames {
        w.string(&name.namespace_uri);
        w.string(&name.local_name);
    }

    w.len(document.items.len());
    for item in &document.items {
        encode_item(&mut w, item);
    }

    w.len(document.regions.len());
    for region in &document.regions {
        w.var(u64::from(region.id.0));
        w.var(u64::from(region.start));
        w.var(u64::from(region.end));
    }

    w.u64(document.feature_bits.0);
    w.bytes
}

/// 解码规范链接文档 payload，并在返回前验证全部 arena 不变量。 / Decodes a canonical linked-document payload and validates all arena invariants before returning.
pub fn decode_linked_document(bytes: &[u8]) -> Result<LinkedDocumentIr, DecodeError> {
    let mut r = Reader::new(bytes);
    let schema = Version {
        major: r.u16()?,
        minor: r.u16()?,
    };
    let document_abi = AbiId(r.string()?);
    let root = DocumentRegionId(r.id()?);

    let strings = r.vec(|r| r.string())?;
    let qnames = r.vec(|r| {
        Ok(ExpandedName {
            namespace_uri: r.string()?,
            local_name: r.string()?,
        })
    })?;
    let items = r.vec(decode_item)?;
    let regions = r.vec(|r| {
        Ok(DocumentRegion {
            id: DocumentRegionId(r.id()?),
            start: r.id()?,
            end: r.id()?,
        })
    })?;
    let feature_bits = FeatureBits(r.u64()?);
    r.finish()?;

    let document = LinkedDocumentIr {
        schema,
        document_abi,
        root,
        regions,
        items,
        strings,
        qnames,
        feature_bits,
    };
    document
        .validate()
        .map_err(|_| DecodeError::NonCanonical("linked document invariants"))?;
    Ok(document)
}

fn encode_item(w: &mut Writer, item: &DocumentItem) {
    match item {
        DocumentItem::Text { value } => {
            w.byte(ITEM_TEXT);
            w.var(u64::from(value.0));
        }
        DocumentItem::Comment { value } => {
            w.byte(ITEM_COMMENT);
            w.var(u64::from(value.0));
        }
        DocumentItem::ProcessingInstruction { target, data } => {
            w.byte(ITEM_PI);
            w.var(u64::from(target.0));
            w.var(u64::from(data.0));
        }
        DocumentItem::ElementStart {
            expanded_name,
            attributes,
            children,
        } => {
            w.byte(ITEM_ELEMENT_START);
            w.var(u64::from(expanded_name.0));
            w.len(attributes.len());
            for attribute in attributes {
                w.var(u64::from(attribute.name.0));
                w.var(u64::from(attribute.value.0));
            }
            w.var(u64::from(children.0));
        }
        DocumentItem::ElementEnd => w.byte(ITEM_ELEMENT_END),
    }
}

fn decode_item(r: &mut Reader<'_>) -> Result<DocumentItem, DecodeError> {
    Ok(match r.byte()? {
        ITEM_TEXT => DocumentItem::Text {
            value: StringId(r.id()?),
        },
        ITEM_COMMENT => DocumentItem::Comment {
            value: StringId(r.id()?),
        },
        ITEM_PI => DocumentItem::ProcessingInstruction {
            target: StringId(r.id()?),
            data: StringId(r.id()?),
        },
        ITEM_ELEMENT_START => {
            let expanded_name = QNameId(r.id()?);
            let attributes = r.vec(|r| {
                Ok(Attribute {
                    name: QNameId(r.id()?),
                    value: StringId(r.id()?),
                })
            })?;
            let children = DocumentRegionId(r.id()?);
            DocumentItem::ElementStart {
                expanded_name,
                attributes,
                children,
            }
        }
        ITEM_ELEMENT_END => DocumentItem::ElementEnd,
        _ => return Err(DecodeError::NonCanonical("document item discriminant")),
    })
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

    fn len(&mut self, value: usize) {
        self.var(value as u64);
    }

    fn string(&mut self, value: &str) {
        self.len(value.len());
        self.bytes.extend_from_slice(value.as_bytes());
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

    /// 读取 unsigned LEB128，并拒绝溢出和任何更长的等价形式。 / Reads unsigned LEB128, rejecting overflow and every longer equivalent form.
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

    fn string(&mut self) -> Result<String, DecodeError> {
        let length = self.length()?;
        let bytes = self.take(length)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }

    fn vec<T>(
        &mut self,
        mut decode: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let count = self.length()?;
        // 每个编码元素至少占一个字节；在分配之前用剩余 payload 给出严格上界。 / Every
        // encoded element occupies at least one byte; bound allocation by the remaining payload.
        if count > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(decode(self)?);
        }
        Ok(values)
    }

    fn length(&mut self) -> Result<usize, DecodeError> {
        self.var()?
            .try_into()
            .map_err(|_| DecodeError::LengthOverflow)
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

    fn fixture() -> LinkedDocumentIr {
        LinkedDocumentIr {
            schema: Version { major: 1, minor: 0 },
            document_abi: AbiId("xmlsquish.document.v1".into()),
            root: DocumentRegionId(0),
            regions: vec![
                DocumentRegion {
                    id: DocumentRegionId(0),
                    start: 0,
                    end: 5,
                },
                DocumentRegion {
                    id: DocumentRegionId(1),
                    start: 1,
                    end: 4,
                },
            ],
            items: vec![
                DocumentItem::ElementStart {
                    expanded_name: QNameId(0),
                    attributes: vec![Attribute {
                        name: QNameId(1),
                        value: StringId(0),
                    }],
                    children: DocumentRegionId(1),
                },
                DocumentItem::Text { value: StringId(1) },
                DocumentItem::Comment { value: StringId(2) },
                DocumentItem::ProcessingInstruction {
                    target: StringId(4),
                    data: StringId(3),
                },
                DocumentItem::ElementEnd,
            ],
            strings: vec![
                "a".into(),
                "body".into(),
                "comment".into(),
                "data".into(),
                "target".into(),
            ],
            qnames: vec![
                ExpandedName {
                    namespace_uri: "".into(),
                    local_name: "root".into(),
                },
                ExpandedName {
                    namespace_uri: "urn:test".into(),
                    local_name: "attribute".into(),
                },
            ],
            feature_bits: FeatureBits(0b11),
        }
    }

    #[test]
    fn deterministic_roundtrip_covers_every_item_kind() {
        let document = fixture();
        document.validate().unwrap();
        let encoded = encode_linked_document(&document);
        assert_eq!(decode_linked_document(&encoded).unwrap(), document);
        assert_eq!(encode_linked_document(&document), encoded);
    }

    #[test]
    fn rejects_non_minimal_leb128() {
        let document = fixture();
        let mut encoded = encode_linked_document(&document);
        // schema 后的 ABI 长度是单字节；替换为等值的二字节表示。 / The ABI length after
        // schema is one byte; replace it with an equivalent two-byte representation.
        let length = encoded[4];
        encoded.splice(4..5, [length | 0x80, 0]);
        assert!(matches!(
            decode_linked_document(&encoded),
            Err(DecodeError::NonCanonical("LEB128"))
        ));
    }

    #[test]
    fn rejects_unknown_discriminant_and_trailing_bytes() {
        let mut unknown = LinkedDocumentIr {
            items: vec![DocumentItem::ElementEnd],
            regions: vec![DocumentRegion {
                id: DocumentRegionId(0),
                start: 0,
                end: 1,
            }],
            strings: Vec::new(),
            qnames: Vec::new(),
            ..fixture()
        };
        unknown.root = DocumentRegionId(0);
        let mut bytes = encode_linked_document(&unknown);
        // 4 schema + ABI + root + empty strings + empty qnames + item count. / Skip the fixed
        // schema, ABI, root, empty pools, and item count to reach the discriminant.
        let item = 4 + 1 + unknown.document_abi.0.len() + 1 + 1 + 1 + 1;
        bytes[item] = 0xff;
        assert!(matches!(
            decode_linked_document(&bytes),
            Err(DecodeError::NonCanonical("document item discriminant"))
        ));

        let mut trailing = encode_linked_document(&fixture());
        trailing.push(0);
        assert!(matches!(
            decode_linked_document(&trailing),
            Err(DecodeError::TrailingBytes)
        ));
    }

    #[test]
    fn decode_runs_semantic_validation() {
        let mut document = fixture();
        document.items[1] = DocumentItem::Text {
            value: StringId(99),
        };
        assert!(matches!(
            decode_linked_document(&encode_linked_document(&document)),
            Err(DecodeError::NonCanonical("linked document invariants"))
        ));
    }

    #[test]
    fn rejects_truncation_and_invalid_utf8() {
        let bytes = encode_linked_document(&fixture());
        assert!(matches!(
            decode_linked_document(&bytes[..bytes.len() - 1]),
            Err(DecodeError::Truncated)
        ));

        let mut invalid = bytes;
        // ABI 是第一个 string，其首字节从 offset 5 开始。 / ABI is the first string and
        // its first byte starts at offset five.
        invalid[5] = 0xff;
        assert!(matches!(
            decode_linked_document(&invalid),
            Err(DecodeError::InvalidUtf8)
        ));
    }
}
