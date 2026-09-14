//! Expansion trace 的规范二进制 payload。 / Canonical binary payload for expansion traces.
//!
//! Arena 顺序属于格式契约：所有 ID 和长度采用最小 unsigned LEB128，摘要算法、范围和
//! frame 深度保持固定宽度。枚举 discriminant 是稳定协议值，未知值必须拒绝。
//! Arena order is part of the format contract: all IDs and lengths use minimal unsigned LEB128,
//! while digest algorithms, ranges, and frame depths remain fixed-width. Enum discriminants are
//! stable protocol values and unknown values are rejected.

use crate::*;

const FRAME_ENTRY: u8 = 0;
const FRAME_MACRO: u8 = 1;

const ORIGIN_SOURCE_SPAN: u8 = 0;
const ORIGIN_DECODED_SEGMENT: u8 = 1;
const ORIGIN_EXTERNAL_ARGUMENT: u8 = 2;
const ORIGIN_EXPANSION: u8 = 3;
const ORIGIN_IMPORT: u8 = 4;
const ORIGIN_REGEX_CAPTURE: u8 = 5;
const ORIGIN_CONCAT: u8 = 6;
const ORIGIN_BACKEND_TRANSFORM: u8 = 7;
const ORIGIN_FUSED: u8 = 8;
const ORIGIN_SYNTHETIC: u8 = 9;
const ORIGIN_UNKNOWN: u8 = 10;

/// 将完整动态来源图编码为规范 typed payload。 / Encodes a complete dynamic provenance graph as a canonical typed payload.
///
/// 编码保留 arena 和 substitution chain 的已有顺序；调用者应在持久化前验证构造值。
/// Encoding preserves existing arena and substitution-chain order; callers should validate
/// constructed values before persistence.
#[must_use]
pub fn encode_expansion_trace(trace: &ExpansionTrace) -> Vec<u8> {
    let mut w = Writer::default();
    w.list(&trace.frames, |w, frame| w.frame(frame));
    w.list(&trace.scalar_values, |w, value| w.string(value));
    w.list(&trace.sequences, |w, sequence| {
        w.list(sequence, |w, item| w.id(item.0));
    });
    w.list(&trace.debug_strings, |w, value| w.string(value));
    w.list(&trace.origins, |w, origin| w.origin(origin));
    w.list(&trace.document_items, |w, trace_ref| w.trace_ref(trace_ref));
    w.bytes
}

/// 解码动态来源 payload，并在暴露值之前验证 frame tree、arena 和 origin DAG。
/// Decodes a dynamic provenance payload and validates its frame tree, arenas, and origin DAG
/// before exposing the value.
pub fn decode_expansion_trace(bytes: &[u8]) -> Result<ExpansionTrace, DecodeError> {
    let mut r = Reader::new(bytes);
    let trace = ExpansionTrace {
        frames: r.list(|r| r.frame())?,
        scalar_values: r.list(|r| r.string())?,
        sequences: r.list(|r| r.list(|r| Ok(DocumentItemId(r.id()?))))?,
        debug_strings: r.list(|r| r.string())?,
        origins: r.list(|r| r.origin())?,
        document_items: r.list(|r| r.trace_ref())?,
    };
    r.finish()?;
    trace
        .validate()
        .map_err(|_| DecodeError::NonCanonical("expansion trace invariants"))?;
    Ok(trace)
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

    fn id(&mut self, value: u32) {
        self.var(u64::from(value));
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

    fn option<T>(&mut self, value: &Option<T>, encode: impl FnOnce(&mut Self, &T)) {
        match value {
            None => self.byte(0),
            Some(value) => {
                self.byte(1);
                encode(self, value);
            }
        }
    }

    fn digest(&mut self, value: &Digest) {
        self.u16(value.algorithm as u16);
        self.bytes.extend_from_slice(&value.bytes);
    }

    fn qualified_origin(&mut self, value: &QualifiedOriginRef) {
        self.digest(&value.object.0);
        self.id(value.local.0);
    }

    fn qualified_map(&mut self, value: &QualifiedDecodedValueMapRef) {
        self.digest(&value.object.0);
        self.id(value.local.0);
    }

    fn def_addr(&mut self, value: &DefAddr) {
        self.id(value.unit_slot);
        self.id(value.local_def.0);
    }

    fn linked_op(&mut self, value: &LinkedOpRef) {
        self.id(value.unit_slot);
        self.id(value.op.0);
    }

    fn frame(&mut self, value: &FrameRecord) {
        self.id(value.id.0);
        self.option(&value.parent, |w, id| w.id(id.0));
        match &value.identity {
            FrameIdentity::Entry => self.byte(FRAME_ENTRY),
            FrameIdentity::Macro(addr) => {
                self.byte(FRAME_MACRO);
                self.def_addr(addr);
            }
        }
        self.option(&value.call_origin, |w, origin| w.qualified_origin(origin));
        self.qualified_origin(&value.definition_origin);
        self.u64(value.depth);
        self.list(&value.args, |w, (name, scalar)| {
            w.string(name);
            w.id(scalar.0);
        });
        self.list(&value.fills, |w, (name, sequence)| {
            w.string(name);
            w.id(sequence.0);
        });
    }

    fn role(&mut self, value: OriginRole) {
        self.byte(match value {
            OriginRole::Input => 0,
            OriginRole::Caller => 1,
            OriginRole::Definition => 2,
            OriginRole::Substitution => 3,
            OriginRole::Transform => 4,
        });
    }

    fn edge(&mut self, value: &OriginEdge) {
        self.role(value.role);
        self.id(value.parent.0);
    }

    fn origin(&mut self, value: &OriginNode) {
        match value {
            OriginNode::SourceSpan { origin } => {
                self.byte(ORIGIN_SOURCE_SPAN);
                self.qualified_origin(origin);
            }
            OriginNode::DecodedSegment { map, segment_index } => {
                self.byte(ORIGIN_DECODED_SEGMENT);
                self.qualified_map(map);
                self.id(*segment_index);
            }
            OriginNode::ExternalArgument { name, value } => {
                self.byte(ORIGIN_EXTERNAL_ARGUMENT);
                self.string(name);
                self.id(value.0);
            }
            OriginNode::Expansion { frame, producer } => {
                self.byte(ORIGIN_EXPANSION);
                self.id(frame.0);
                self.linked_op(producer);
            }
            OriginNode::Import { edge, child } => {
                self.byte(ORIGIN_IMPORT);
                self.id(edge.0);
                self.id(child.0);
            }
            OriginNode::RegexCapture {
                input,
                capture,
                matched_range,
            } => {
                self.byte(ORIGIN_REGEX_CAPTURE);
                self.id(input.0);
                self.string(capture);
                self.u64(matched_range.start);
                self.u64(matched_range.end);
            }
            OriginNode::Concat { ordered_inputs } => {
                self.byte(ORIGIN_CONCAT);
                self.list(ordered_inputs, |w, input| w.id(input.0));
            }
            OriginNode::BackendTransform {
                backend_step,
                inputs,
            } => {
                self.byte(ORIGIN_BACKEND_TRANSFORM);
                self.id(backend_step.0);
                self.list(inputs, |w, edge| w.edge(edge));
            }
            OriginNode::Fused { role, inputs } => {
                self.byte(ORIGIN_FUSED);
                self.id(role.0);
                self.list(inputs, |w, edge| w.edge(edge));
            }
            OriginNode::Synthetic { reason, nearest } => {
                self.byte(ORIGIN_SYNTHETIC);
                self.id(reason.0);
                self.option(nearest, |w, node| w.id(node.0));
            }
            OriginNode::Unknown { reason } => {
                self.byte(ORIGIN_UNKNOWN);
                self.id(reason.0);
            }
        }
    }

    fn substitution(&mut self, value: &SubstitutionStep) {
        self.byte(match value.kind {
            SubstitutionKind::SlotFill => 0,
            SubstitutionKind::ScalarBody => 1,
            SubstitutionKind::InsertScalar => 2,
        });
        self.qualified_origin(&value.origin);
    }

    fn trace_ref(&mut self, value: &TraceRef) {
        self.linked_op(&value.producer_op);
        self.id(value.frame.0);
        self.qualified_origin(&value.definition_origin);
        self.option(&value.call_origin, |w, origin| w.qualified_origin(origin));
        self.list(&value.substitution_chain, |w, step| w.substitution(step));
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

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("fixed-size slice"),
        ))
    }

    /// 读取 unsigned LEB128，并拒绝溢出及非最小编码。 / Reads unsigned LEB128, rejecting overflow and non-minimal encodings.
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
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }

    fn length(&mut self) -> Result<usize, DecodeError> {
        self.var()?
            .try_into()
            .map_err(|_| DecodeError::LengthOverflow)
    }

    fn list<T>(
        &mut self,
        mut decode: impl FnMut(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let count = self.length()?;
        // 每个元素至少有一个编码字节；先约束分配再进入递归解码。 / Every element has at
        // least one encoded byte; constrain allocation before recursively decoding it.
        if count > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(decode(self)?);
        }
        Ok(values)
    }

    fn option<T>(
        &mut self,
        decode: impl FnOnce(&mut Self) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        match self.byte()? {
            0 => Ok(None),
            1 => decode(self).map(Some),
            _ => Err(DecodeError::NonCanonical("option discriminant")),
        }
    }

    fn digest(&mut self) -> Result<Digest, DecodeError> {
        let algorithm = match self.u16()? {
            1 => DigestAlgorithm::Sha256,
            2 => DigestAlgorithm::Blake3,
            _ => return Err(DecodeError::UnsupportedDigest),
        };
        let bytes = self.take(32)?.try_into().expect("fixed-size slice");
        Ok(Digest { algorithm, bytes })
    }

    fn qualified_origin(&mut self) -> Result<QualifiedOriginRef, DecodeError> {
        Ok(QualifiedOriginRef {
            object: ObjectDigest(self.digest()?),
            local: OriginId(self.id()?),
        })
    }

    fn qualified_map(&mut self) -> Result<QualifiedDecodedValueMapRef, DecodeError> {
        Ok(QualifiedDecodedValueMapRef {
            object: ObjectDigest(self.digest()?),
            local: DecodedValueMapId(self.id()?),
        })
    }

    fn def_addr(&mut self) -> Result<DefAddr, DecodeError> {
        Ok(DefAddr {
            unit_slot: self.id()?,
            local_def: LocalDefId(self.id()?),
        })
    }

    fn linked_op(&mut self) -> Result<LinkedOpRef, DecodeError> {
        Ok(LinkedOpRef {
            unit_slot: self.id()?,
            op: OpId(self.id()?),
        })
    }

    fn frame(&mut self) -> Result<FrameRecord, DecodeError> {
        let id = FrameId(self.id()?);
        let parent = self.option(|r| Ok(FrameId(r.id()?)))?;
        let identity = match self.byte()? {
            FRAME_ENTRY => FrameIdentity::Entry,
            FRAME_MACRO => FrameIdentity::Macro(self.def_addr()?),
            _ => return Err(DecodeError::NonCanonical("frame identity discriminant")),
        };
        let call_origin = self.option(|r| r.qualified_origin())?;
        let definition_origin = self.qualified_origin()?;
        let depth = self.u64()?;
        let args = self.list(|r| Ok((r.string()?, ScalarValueId(r.id()?))))?;
        let fills = self.list(|r| Ok((r.string()?, SequenceValueId(r.id()?))))?;
        Ok(FrameRecord {
            id,
            parent,
            identity,
            call_origin,
            definition_origin,
            depth,
            args,
            fills,
        })
    }

    fn role(&mut self) -> Result<OriginRole, DecodeError> {
        match self.byte()? {
            0 => Ok(OriginRole::Input),
            1 => Ok(OriginRole::Caller),
            2 => Ok(OriginRole::Definition),
            3 => Ok(OriginRole::Substitution),
            4 => Ok(OriginRole::Transform),
            _ => Err(DecodeError::NonCanonical("origin role discriminant")),
        }
    }

    fn edge(&mut self) -> Result<OriginEdge, DecodeError> {
        Ok(OriginEdge {
            role: self.role()?,
            parent: OriginNodeId(self.id()?),
        })
    }

    fn origin(&mut self) -> Result<OriginNode, DecodeError> {
        Ok(match self.byte()? {
            ORIGIN_SOURCE_SPAN => OriginNode::SourceSpan {
                origin: self.qualified_origin()?,
            },
            ORIGIN_DECODED_SEGMENT => OriginNode::DecodedSegment {
                map: self.qualified_map()?,
                segment_index: self.id()?,
            },
            ORIGIN_EXTERNAL_ARGUMENT => OriginNode::ExternalArgument {
                name: self.string()?,
                value: ScalarValueId(self.id()?),
            },
            ORIGIN_EXPANSION => OriginNode::Expansion {
                frame: FrameId(self.id()?),
                producer: self.linked_op()?,
            },
            ORIGIN_IMPORT => OriginNode::Import {
                edge: LinkImportRef(self.id()?),
                child: OriginNodeId(self.id()?),
            },
            ORIGIN_REGEX_CAPTURE => OriginNode::RegexCapture {
                input: OriginNodeId(self.id()?),
                capture: self.string()?,
                matched_range: Span {
                    start: self.u64()?,
                    end: self.u64()?,
                },
            },
            ORIGIN_CONCAT => OriginNode::Concat {
                ordered_inputs: self.list(|r| Ok(OriginNodeId(r.id()?)))?,
            },
            ORIGIN_BACKEND_TRANSFORM => OriginNode::BackendTransform {
                backend_step: DebugStringId(self.id()?),
                inputs: self.list(|r| r.edge())?,
            },
            ORIGIN_FUSED => OriginNode::Fused {
                role: DebugStringId(self.id()?),
                inputs: self.list(|r| r.edge())?,
            },
            ORIGIN_SYNTHETIC => OriginNode::Synthetic {
                reason: DebugStringId(self.id()?),
                nearest: self.option(|r| Ok(OriginNodeId(r.id()?)))?,
            },
            ORIGIN_UNKNOWN => OriginNode::Unknown {
                reason: DebugStringId(self.id()?),
            },
            _ => return Err(DecodeError::NonCanonical("origin node discriminant")),
        })
    }

    fn substitution(&mut self) -> Result<SubstitutionStep, DecodeError> {
        let kind = match self.byte()? {
            0 => SubstitutionKind::SlotFill,
            1 => SubstitutionKind::ScalarBody,
            2 => SubstitutionKind::InsertScalar,
            _ => return Err(DecodeError::NonCanonical("substitution kind discriminant")),
        };
        Ok(SubstitutionStep {
            kind,
            origin: self.qualified_origin()?,
        })
    }

    fn trace_ref(&mut self) -> Result<TraceRef, DecodeError> {
        Ok(TraceRef {
            producer_op: self.linked_op()?,
            frame: FrameId(self.id()?),
            definition_origin: self.qualified_origin()?,
            call_origin: self.option(|r| r.qualified_origin())?,
            substitution_chain: self.list(|r| r.substitution())?,
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

    fn object(seed: u8) -> ObjectDigest {
        ObjectDigest(Digest {
            algorithm: DigestAlgorithm::Sha256,
            bytes: [seed; 32],
        })
    }

    fn qorigin(seed: u8, local: u32) -> QualifiedOriginRef {
        QualifiedOriginRef {
            object: object(seed),
            local: OriginId(local),
        }
    }

    fn edge(role: OriginRole, parent: u32) -> OriginEdge {
        OriginEdge {
            role,
            parent: OriginNodeId(parent),
        }
    }

    fn fixture() -> ExpansionTrace {
        ExpansionTrace {
            frames: vec![
                FrameRecord {
                    id: FrameId(0),
                    parent: None,
                    identity: FrameIdentity::Entry,
                    call_origin: None,
                    definition_origin: qorigin(1, 0),
                    depth: 1,
                    args: vec![("arg".into(), ScalarValueId(0))],
                    fills: vec![("slot".into(), SequenceValueId(0))],
                },
                FrameRecord {
                    id: FrameId(1),
                    parent: Some(FrameId(0)),
                    identity: FrameIdentity::Macro(DefAddr {
                        unit_slot: 3,
                        local_def: LocalDefId(4),
                    }),
                    call_origin: Some(qorigin(2, 1)),
                    definition_origin: qorigin(3, 2),
                    depth: 2,
                    args: vec![],
                    fills: vec![],
                },
            ],
            scalar_values: vec!["scalar".into()],
            sequences: vec![vec![DocumentItemId(0)]],
            debug_strings: vec![
                "backend".into(),
                "fused".into(),
                "reason".into(),
                "unknown".into(),
            ],
            origins: vec![
                OriginNode::SourceSpan {
                    origin: qorigin(1, 0),
                },
                OriginNode::DecodedSegment {
                    map: QualifiedDecodedValueMapRef {
                        object: object(2),
                        local: DecodedValueMapId(7),
                    },
                    segment_index: 2,
                },
                OriginNode::ExternalArgument {
                    name: "arg".into(),
                    value: ScalarValueId(0),
                },
                OriginNode::Expansion {
                    frame: FrameId(1),
                    producer: LinkedOpRef {
                        unit_slot: 3,
                        op: OpId(9),
                    },
                },
                OriginNode::Import {
                    edge: LinkImportRef(4),
                    child: OriginNodeId(0),
                },
                OriginNode::RegexCapture {
                    input: OriginNodeId(2),
                    capture: "capture".into(),
                    matched_range: Span { start: 5, end: 8 },
                },
                OriginNode::Concat {
                    ordered_inputs: vec![OriginNodeId(0), OriginNodeId(5)],
                },
                OriginNode::BackendTransform {
                    backend_step: DebugStringId(0),
                    inputs: vec![edge(OriginRole::Input, 6), edge(OriginRole::Transform, 3)],
                },
                OriginNode::Fused {
                    role: DebugStringId(1),
                    inputs: vec![
                        edge(OriginRole::Caller, 3),
                        edge(OriginRole::Definition, 0),
                        edge(OriginRole::Substitution, 5),
                    ],
                },
                OriginNode::Synthetic {
                    reason: DebugStringId(2),
                    nearest: Some(OriginNodeId(8)),
                },
                OriginNode::Unknown {
                    reason: DebugStringId(3),
                },
            ],
            document_items: vec![TraceRef {
                producer_op: LinkedOpRef {
                    unit_slot: 3,
                    op: OpId(9),
                },
                frame: FrameId(1),
                definition_origin: qorigin(3, 2),
                call_origin: Some(qorigin(2, 1)),
                substitution_chain: vec![
                    SubstitutionStep {
                        kind: SubstitutionKind::SlotFill,
                        origin: qorigin(4, 0),
                    },
                    SubstitutionStep {
                        kind: SubstitutionKind::ScalarBody,
                        origin: qorigin(4, 1),
                    },
                    SubstitutionStep {
                        kind: SubstitutionKind::InsertScalar,
                        origin: qorigin(4, 2),
                    },
                ],
            }],
        }
    }

    #[test]
    fn deterministic_roundtrip_covers_every_origin_kind() {
        let trace = fixture();
        trace.validate().unwrap();
        let encoded = encode_expansion_trace(&trace);
        assert_eq!(decode_expansion_trace(&encoded).unwrap(), trace);
        assert_eq!(encode_expansion_trace(&trace), encoded);
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_unknown_discriminants() {
        let encoded = encode_expansion_trace(&fixture());
        assert_eq!(
            decode_expansion_trace(&encoded[..encoded.len() - 1]),
            Err(DecodeError::Truncated)
        );
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            decode_expansion_trace(&trailing),
            Err(DecodeError::TrailingBytes)
        );

        // 此 fixture 的尾部固定为 origin tag、reason ID、空 document arena。 / This
        // fixture has a fixed tail: origin tag, reason ID, and empty document arena.
        let mut minimal = ExpansionTrace::default();
        minimal.debug_strings.push("reason".into());
        minimal.origins.push(OriginNode::Unknown {
            reason: DebugStringId(0),
        });
        let mut bad_tag = encode_expansion_trace(&minimal);
        let origin_tag = bad_tag.len() - 3;
        bad_tag[origin_tag] = 0xff;
        assert_eq!(
            decode_expansion_trace(&bad_tag),
            Err(DecodeError::NonCanonical("origin node discriminant"))
        );
    }

    #[test]
    fn rejects_nonminimal_leb128_invalid_utf8_and_oversized_counts() {
        let mut nonminimal = encode_expansion_trace(&ExpansionTrace::default());
        nonminimal.splice(0..1, [0x80, 0]);
        assert_eq!(
            decode_expansion_trace(&nonminimal),
            Err(DecodeError::NonCanonical("LEB128"))
        );

        // frame count=0, scalar count=1, string length=1, invalid byte. / One scalar string with invalid UTF-8.
        assert_eq!(
            decode_expansion_trace(&[0, 1, 1, 0xff, 0, 0, 0, 0]),
            Err(DecodeError::InvalidUtf8)
        );
        assert_eq!(decode_expansion_trace(&[0x7f]), Err(DecodeError::Truncated));
    }

    #[test]
    fn rejects_decoded_values_that_violate_structural_invariants() {
        let mut trace = fixture();
        trace.frames[1].parent = Some(FrameId(1));
        let encoded = encode_expansion_trace(&trace);
        assert_eq!(
            decode_expansion_trace(&encoded),
            Err(DecodeError::NonCanonical("expansion trace invariants"))
        );
    }

    #[test]
    fn decoder_rejects_sequence_occurrence_out_of_bounds() {
        let mut trace = fixture();
        trace.sequences[0][0] = DocumentItemId(u32::MAX);
        assert!(decode_expansion_trace(&encode_expansion_trace(&trace)).is_err());
    }
}
