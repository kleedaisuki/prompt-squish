//! prompt-squish 的纯后端领域。 / Pure backend domain for prompt-squish.
//!
//! 后端只消费已验证的文档 IR 与显式选项；项目发现、XML 解析、终端和文件系统均由
//! manager 持有。 / A backend consumes only validated document IR and explicit options;
//! project discovery, XML parsing, terminals, and filesystems remain manager concerns.

#![forbid(unsafe_code)]

use std::{error::Error, fmt};

use squish_ir::{
    AbiId, ArtifactByteMap, ArtifactMapEntry, DebugStringId, DocumentItem, ExpansionTrace,
    FeatureBits, LinkedDocumentIr, OriginEdge, OriginNode, OriginNodeId, OriginRole, Span,
    Validate,
};
use squish_protocol::ArtifactKind;

/// `squish` 后端的稳定插件 ID。 / Stable plugin ID of the `squish` backend.
pub const SQUISH_BACKEND_ID: &str = "xmlsquish.squish";
/// 改变产品字节语义时必须改变的 ABI。 / ABI changed whenever product-byte semantics change.
pub const SQUISH_BACKEND_ABI: &str = "xmlsquish.squish.v1";
/// 当前后端接受的文档 ABI。 / Document ABI accepted by the current backend.
pub const DOCUMENT_ABI: &str = "xmlsquish.document.v1";

/// 后端的缓存身份。 / Cache identity of a backend invocation.
///
/// Manager 必须将全部字段纳入 action key，而不能只使用文档摘要。
/// The manager must include every field in its action key, not only the document digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCacheIdentity {
    /// 稳定后端 ID。 / Stable backend ID.
    pub backend_id: &'static str,
    /// 字节语义 ABI/版本。 / Product-byte semantic ABI/version.
    pub backend_version: &'static str,
    /// 规范选项字节。 / Canonical option bytes.
    pub canonical_options: Vec<u8>,
}

/// 后端提供的静态能力。 / Static capabilities offered by a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    /// 接受的文档 schema 主版本。 / Accepted document schema major.
    pub document_schema_major: u16,
    /// 接受的文档 ABI。 / Accepted document ABI.
    pub document_abi: AbiId,
    /// 支持的 feature 位；请求不得含额外位。 / Supported feature bits; requests may not add bits.
    pub feature_bits: FeatureBits,
    /// 产物类型。 / Artifact kind.
    pub artifact_kind: ArtifactKind,
    /// 用户产物的文件后缀。 / User-product file suffix.
    pub file_suffix: &'static str,
    /// 产品媒体类型。 / Product media type.
    pub media_type: &'static str,
}

/// `squish` 发射的显式、可缓存选项。 / Explicit cacheable options for `squish` emission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SquishOptions {
    /// 允许的最大产品字节数。 / Maximum permitted product byte length.
    pub max_output_bytes: u64,
}

impl Default for SquishOptions {
    fn default() -> Self {
        Self {
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

impl SquishOptions {
    fn canonical_bytes(self) -> Vec<u8> {
        let mut bytes = b"squish-options-v1\0".to_vec();
        bytes.extend_from_slice(&self.max_output_bytes.to_le_bytes());
        bytes
    }
}

/// 一次纯后端请求。 / One pure backend request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendRequest {
    /// 完全展开、结构化的文档。 / Fully expanded structured document.
    pub document: LinkedDocumentIr,
    /// 与文档 item 对齐的完整动态来源。 / Complete dynamic provenance aligned with document items.
    pub trace: ExpansionTrace,
    /// 发射预算和语义选项。 / Emission budget and semantic options.
    pub options: SquishOptions,
}

/// 可持久化的后端度量。 / Persistable backend metrics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendMetrics {
    /// 发射的 UTF-8 字节数。 / Emitted UTF-8 byte count.
    pub output_bytes: u64,
    /// 识别到的 XML 空白字节数。 / Recognized XML-whitespace byte count.
    pub whitespace_recognized: u64,
    /// 删除的空白字节数。 / Removed whitespace byte count.
    pub whitespace_removed: u64,
    /// 插入的 ASCII 分隔空格数。 / Inserted ASCII separator count.
    pub whitespace_inserted: u64,
}

/// 成功的后端结果。 / Successful backend result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendOutput {
    /// 可直接发布为 `*.prompt` 的 UTF-8 字节。 / UTF-8 bytes publishable as `*.prompt`.
    pub bytes: Vec<u8>,
    /// 精确覆盖产品每个字节的来源映射。 / Provenance map exactly covering every product byte.
    pub byte_map: ArtifactByteMap,
    /// 添加 backend transform/synthetic 节点后的 trace。 / Trace extended with backend transform/synthetic nodes.
    pub trace: ExpansionTrace,
    /// 进入 action cache key 的后端身份。 / Backend identity included in the action cache key.
    pub cache_identity: BackendCacheIdentity,
    /// 有界发射的度量证书。 / Metrics certificate for bounded emission.
    pub metrics: BackendMetrics,
}

/// 纯后端接口。 / Pure backend interface.
pub trait Backend {
    /// 返回稳定缓存身份。 / Returns the stable cache identity.
    fn cache_identity(&self, options: SquishOptions) -> BackendCacheIdentity;
    /// 返回可协商能力。 / Returns negotiable capabilities.
    fn capabilities(&self) -> BackendCapabilities;
    /// 在执行前协商文档 dialect 与 feature。 / Negotiates document dialect and features before execution.
    fn negotiate(&self, document: &LinkedDocumentIr) -> Result<(), BackendError>;
    /// 验证并发射产品。 / Validates and emits a product.
    fn emit(&self, request: BackendRequest) -> Result<BackendOutput, BackendError>;
}

/// 后端失败类别。 / Backend failure category.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BackendErrorKind {
    /// 输入 IR 不满足结构不变量。 / Input IR violates structural invariants.
    InvalidIr,
    /// dialect 或 feature 不受支持。 / Unsupported dialect or feature.
    IncompatibleDocument,
    /// 当前后端要求单一 XML 根。 / This backend requires one XML root.
    InvalidRoot,
    /// 名称或 XML 数据无法序列化。 / A name or XML data item cannot be serialized.
    InvalidXmlData,
    /// 产品超过发射预算。 / Product exceeds its emission budget.
    OutputBudgetExceeded,
}

/// 可由 manager 关联到 item 的结构化后端错误。 / Structured backend error joinable to an item by the manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    /// 稳定错误类别。 / Stable error category.
    pub kind: BackendErrorKind,
    /// 相关 document item（若有）。 / Related document item, when available.
    pub item: Option<u32>,
    /// 非本地化细节。 / Non-localized detail.
    pub detail: String,
}

impl BackendError {
    fn new(kind: BackendErrorKind, item: Option<usize>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            item: item.and_then(|value| u32::try_from(value).ok()),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(item) = self.item {
            write!(f, "backend error at document item {item}: {}", self.detail)
        } else {
            write!(f, "backend error: {}", self.detail)
        }
    }
}

impl Error for BackendError {}

/// 删除属性与 namespace、使用 local-name 并执行最终空白压缩的首个后端。
/// First backend: drops attributes/namespaces, uses local names, then applies final squishing.
#[derive(Clone, Copy, Debug, Default)]
pub struct SquishBackend;

impl Backend for SquishBackend {
    fn cache_identity(&self, options: SquishOptions) -> BackendCacheIdentity {
        BackendCacheIdentity {
            backend_id: SQUISH_BACKEND_ID,
            backend_version: SQUISH_BACKEND_ABI,
            canonical_options: options.canonical_bytes(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            document_schema_major: 1,
            document_abi: AbiId(DOCUMENT_ABI.to_owned()),
            feature_bits: FeatureBits(0),
            artifact_kind: ArtifactKind::Prompt,
            file_suffix: ".prompt",
            media_type: "application/xml; charset=utf-8",
        }
    }

    fn negotiate(&self, document: &LinkedDocumentIr) -> Result<(), BackendError> {
        let offered = self.capabilities();
        if document.schema.major != offered.document_schema_major
            || document.document_abi != offered.document_abi
            || document.feature_bits.0 & !offered.feature_bits.0 != 0
        {
            return Err(BackendError::new(
                BackendErrorKind::IncompatibleDocument,
                None,
                format!(
                    "document schema {}.{}, ABI `{}`, or feature bits {:#x} are unsupported",
                    document.schema.major,
                    document.schema.minor,
                    document.document_abi.0,
                    document.feature_bits.0
                ),
            ));
        }
        Ok(())
    }

    fn emit(&self, mut request: BackendRequest) -> Result<BackendOutput, BackendError> {
        request.document.validate().map_err(|error| {
            BackendError::new(BackendErrorKind::InvalidIr, None, error.to_string())
        })?;
        request.trace.validate().map_err(|error| {
            BackendError::new(BackendErrorKind::InvalidIr, None, error.to_string())
        })?;
        if request.trace.document_items.len() != request.document.items.len() {
            return Err(BackendError::new(
                BackendErrorKind::InvalidIr,
                None,
                "trace/document item counts differ",
            ));
        }
        self.negotiate(&request.document)?;
        validate_product_xml(&request.document)?;

        let cache_identity = self.cache_identity(request.options);
        let mut emitter = Emitter::new(request.options.max_output_bytes, &mut request.trace);
        let mut element_names = Vec::new();
        for (index, item) in request.document.items.iter().enumerate() {
            let serialized = match item {
                DocumentItem::ElementStart { expanded_name, .. } => {
                    let name = &request.document.qnames[expanded_name.0 as usize].local_name;
                    element_names.push(name.as_str());
                    format!("<{name}>")
                }
                DocumentItem::ElementEnd => {
                    let name = element_names.pop().ok_or_else(|| {
                        BackendError::new(
                            BackendErrorKind::InvalidIr,
                            Some(index),
                            "unmatched element end",
                        )
                    })?;
                    format!("</{name}>")
                }
                _ => serialize_data_item(&request.document, item),
            };
            emitter.push_serialized(index, serialized.as_bytes())?;
        }
        let (bytes, byte_map, metrics) = emitter.finish()?;
        request.trace.validate().map_err(|error| {
            BackendError::new(BackendErrorKind::InvalidIr, None, error.to_string())
        })?;
        byte_map
            .validate_against(&request.trace, bytes.len() as u64)
            .map_err(|error| {
                BackendError::new(BackendErrorKind::InvalidIr, None, error.to_string())
            })?;
        Ok(BackendOutput {
            bytes,
            byte_map,
            trace: request.trace,
            cache_identity,
            metrics,
        })
    }
}

fn validate_product_xml(document: &LinkedDocumentIr) -> Result<(), BackendError> {
    let mut depth = 0usize;
    let mut roots = 0usize;
    for (index, item) in document.items.iter().enumerate() {
        match item {
            DocumentItem::ElementStart { expanded_name, .. } => {
                let name = &document.qnames[expanded_name.0 as usize].local_name;
                if !is_xml_ncname(name) {
                    return Err(BackendError::new(
                        BackendErrorKind::InvalidXmlData,
                        Some(index),
                        format!("local element name `{name}` is not serializable"),
                    ));
                }
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            DocumentItem::ElementEnd => depth -= 1,
            DocumentItem::Text { value } if depth == 0 => {
                let value = &document.strings[value.0 as usize];
                validate_xml_chars(value, index, "character data")?;
                if !value.bytes().all(is_xml_space) {
                    return Err(BackendError::new(
                        BackendErrorKind::InvalidRoot,
                        Some(index),
                        "character data is not allowed outside the product root",
                    ));
                }
            }
            DocumentItem::Comment { value } => {
                let value = &document.strings[value.0 as usize];
                validate_xml_chars(value, index, "comment")?;
                if value.contains("--") || value.ends_with('-') {
                    return Err(BackendError::new(
                        BackendErrorKind::InvalidXmlData,
                        Some(index),
                        "XML comment contains an illegal `--` or trailing `-`",
                    ));
                }
            }
            DocumentItem::ProcessingInstruction { target, data } => {
                let target = &document.strings[target.0 as usize];
                let data = &document.strings[data.0 as usize];
                validate_xml_chars(data, index, "processing instruction")?;
                if !is_xml_name(target) || target.eq_ignore_ascii_case("xml") || data.contains("?>")
                {
                    return Err(BackendError::new(
                        BackendErrorKind::InvalidXmlData,
                        Some(index),
                        "processing instruction is not XML-serializable",
                    ));
                }
            }
            DocumentItem::Text { value } => {
                validate_xml_chars(&document.strings[value.0 as usize], index, "character data")?;
            }
        }
    }
    if roots != 1 {
        return Err(BackendError::new(
            BackendErrorKind::InvalidRoot,
            None,
            format!("squish output requires exactly one root element, found {roots}"),
        ));
    }
    Ok(())
}

fn validate_xml_chars(value: &str, item: usize, kind: &str) -> Result<(), BackendError> {
    if value.chars().all(is_xml_char) {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorKind::InvalidXmlData,
            Some(item),
            format!("{kind} contains a character forbidden by XML 1.0"),
        ))
    }
}

/// 按 XML 1.0 Fifth Edition `Name` production 验证 PI target。
/// Validates a PI target against the XML 1.0 Fifth Edition `Name` production.
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_xml_name_start(first, true) && chars.all(|character| is_xml_name_char(character, true))
}

/// 按 XML Namespaces 的 `NCName` 规则验证降级后的 local-name。
/// Validates a lowered local name against the XML Namespaces `NCName` rule.
fn is_xml_ncname(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_xml_name_start(first, false) && chars.all(|character| is_xml_name_char(character, false))
}

fn is_xml_name_start(character: char, colon: bool) -> bool {
    let code = character as u32;
    (colon && character == ':')
        || character == '_'
        || character.is_ascii_alphabetic()
        || matches!(code,
            0x00c0..=0x00d6 | 0x00d8..=0x00f6 | 0x00f8..=0x02ff |
            0x0370..=0x037d | 0x037f..=0x1fff | 0x200c..=0x200d |
            0x2070..=0x218f | 0x2c00..=0x2fef | 0x3001..=0xd7ff |
            0xf900..=0xfdcf | 0xfdf0..=0xfffd | 0x10000..=0xeffff)
}

fn is_xml_name_char(character: char, colon: bool) -> bool {
    is_xml_name_start(character, colon)
        || character == '-'
        || character == '.'
        || character.is_ascii_digit()
        || matches!(character as u32, 0x00b7 | 0x0300..=0x036f | 0x203f..=0x2040)
}

/// XML 1.0 Fifth Edition 的 `Char` production。 / XML 1.0 Fifth Edition `Char` production.
const fn is_xml_char(character: char) -> bool {
    matches!(character as u32,
        0x09 | 0x0a | 0x0d | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

/// 序列化非 element 事件；element 由显式栈处理。 / Serializes non-element events; an explicit stack handles elements.
fn serialize_data_item(document: &LinkedDocumentIr, item: &DocumentItem) -> String {
    match item {
        DocumentItem::Text { value } => escape_text(&document.strings[value.0 as usize]),
        DocumentItem::Comment { value } => format!("<!--{}-->", document.strings[value.0 as usize]),
        DocumentItem::ProcessingInstruction { target, data } => {
            let target = &document.strings[target.0 as usize];
            let data = &document.strings[data.0 as usize];
            if data.is_empty() {
                format!("<?{target}?>")
            } else {
                format!("<?{target} {data}?>")
            }
        }
        DocumentItem::ElementStart { .. } | DocumentItem::ElementEnd => {
            unreachable!("element events use the explicit name stack")
        }
    }
}

/// 使用当前 DSL 的字符数据转义规则。 / Applies the current DSL character-data escaping rules.
fn escape_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(character),
        }
    }
    output
}

#[derive(Clone, Copy)]
struct Atom {
    start: usize,
    end: usize,
    origin: OriginNodeId,
    kind: AtomKind,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AtomKind {
    Text,
    Markup,
}

/// 两阶段迭代 emitter：先记录词法 atom，再线性规范空白并构造 byte map。
/// Two-pass iterative emitter: records lexical atoms, then linearly normalizes whitespace and builds a byte map.
struct Emitter<'a> {
    max: u64,
    trace: &'a mut ExpansionTrace,
    serialized: Vec<u8>,
    atoms: Vec<Atom>,
    recognized: u64,
}

impl<'a> Emitter<'a> {
    fn new(max: u64, trace: &'a mut ExpansionTrace) -> Self {
        Self {
            max,
            trace,
            serialized: Vec::new(),
            atoms: Vec::new(),
            recognized: 0,
        }
    }

    fn push_serialized(&mut self, item: usize, bytes: &[u8]) -> Result<(), BackendError> {
        let origin = self.transform_origin(item)?;
        let base = self.serialized.len();
        self.serialized.extend_from_slice(bytes);
        if bytes.first() == Some(&b'<') {
            self.atoms.push(Atom {
                start: base,
                end: base + bytes.len(),
                origin,
                kind: AtomKind::Markup,
            });
            return Ok(());
        }
        let mut cursor = 0;
        while cursor < bytes.len() {
            if is_xml_space(bytes[cursor]) {
                self.recognized += 1;
                cursor += 1;
                continue;
            }
            let start = cursor;
            while cursor < bytes.len() && !is_xml_space(bytes[cursor]) {
                cursor += 1;
            }
            self.atoms.push(Atom {
                start: base + start,
                end: base + cursor,
                origin,
                kind: AtomKind::Text,
            });
        }
        Ok(())
    }

    fn transform_origin(&mut self, item: usize) -> Result<OriginNodeId, BackendError> {
        let trace = &self.trace.document_items[item];
        let source_id = push_origin(
            &mut self.trace.origins,
            OriginNode::Expansion {
                frame: trace.frame,
                producer: trace.producer_op,
            },
        )?;
        let step = intern_debug_string(self.trace, "squish.serialize")?;
        push_origin(
            &mut self.trace.origins,
            OriginNode::BackendTransform {
                backend_step: step,
                inputs: vec![OriginEdge {
                    role: OriginRole::Transform,
                    parent: source_id,
                }],
            },
        )
    }

    fn finish(self) -> Result<(Vec<u8>, ArtifactByteMap, BackendMetrics), BackendError> {
        let mut output = Vec::new();
        let mut map = ArtifactByteMap::default();
        let mut inserted = 0u64;
        for atom_index in 0..self.atoms.len() {
            let atom = self.atoms[atom_index];
            if atom_index != 0 {
                let prior = self.atoms[atom_index - 1];
                let gap = &self.serialized[prior.end..atom.start];
                let continuous_text =
                    gap.is_empty() && prior.kind == AtomKind::Text && atom.kind == AtomKind::Text;
                if !continuous_text {
                    let origin = if gap.is_empty() {
                        inserted += 1;
                        let reason = intern_debug_string(self.trace, "squish.separator")?;
                        push_origin(
                            &mut self.trace.origins,
                            OriginNode::Synthetic {
                                reason,
                                nearest: Some(prior.origin),
                            },
                        )?
                    } else {
                        atom.origin
                    };
                    append(&mut output, &mut map, b" ", origin, self.max)?;
                }
            }
            append(
                &mut output,
                &mut map,
                &self.serialized[atom.start..atom.end],
                atom.origin,
                self.max,
            )?;
        }
        let retained_separators = self
            .atoms
            .windows(2)
            .filter(|pair| !self.serialized[pair[0].end..pair[1].start].is_empty())
            .count() as u64;
        let removed = self.recognized.saturating_sub(retained_separators);
        let metrics = BackendMetrics {
            output_bytes: output.len() as u64,
            whitespace_recognized: self.recognized,
            whitespace_removed: removed,
            whitespace_inserted: inserted,
        };
        Ok((output, map, metrics))
    }
}

fn push_origin(
    origins: &mut Vec<OriginNode>,
    node: OriginNode,
) -> Result<OriginNodeId, BackendError> {
    let id = u32::try_from(origins.len()).map_err(|_| {
        BackendError::new(
            BackendErrorKind::OutputBudgetExceeded,
            None,
            "origin arena exceeds u32",
        )
    })?;
    origins.push(node);
    Ok(OriginNodeId(id))
}

/// 在排序 string arena 中 intern，并重定位既有引用。 / Interns into a sorted string arena and relocates existing references.
fn intern_debug_string(
    trace: &mut ExpansionTrace,
    value: &str,
) -> Result<DebugStringId, BackendError> {
    if let Ok(index) = trace
        .debug_strings
        .binary_search_by(|candidate| candidate.as_str().cmp(value))
    {
        return u32::try_from(index).map(DebugStringId).map_err(|_| {
            BackendError::new(
                BackendErrorKind::OutputBudgetExceeded,
                None,
                "debug string arena exceeds u32",
            )
        });
    }
    let position = trace
        .debug_strings
        .partition_point(|candidate| candidate.as_str() < value);
    let id = u32::try_from(position).map_err(|_| {
        BackendError::new(
            BackendErrorKind::OutputBudgetExceeded,
            None,
            "debug string arena exceeds u32",
        )
    })?;
    trace.debug_strings.insert(position, value.to_owned());
    for origin in &mut trace.origins {
        let string = match origin {
            OriginNode::BackendTransform { backend_step, .. } => Some(backend_step),
            OriginNode::Fused { role, .. } => Some(role),
            OriginNode::Synthetic { reason, .. } | OriginNode::Unknown { reason } => Some(reason),
            _ => None,
        };
        if let Some(string) = string
            && string.0 >= id
        {
            string.0 = string.0.checked_add(1).ok_or_else(|| {
                BackendError::new(
                    BackendErrorKind::OutputBudgetExceeded,
                    None,
                    "debug string arena exceeds u32",
                )
            })?;
        }
    }
    Ok(DebugStringId(id))
}

/// 有预算地追加一个完整 provenance 段。 / Appends one complete provenance segment under the output budget.
fn append(
    output: &mut Vec<u8>,
    map: &mut ArtifactByteMap,
    bytes: &[u8],
    origin: OriginNodeId,
    max: u64,
) -> Result<(), BackendError> {
    let end = (output.len() as u64)
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorKind::OutputBudgetExceeded,
                None,
                "output length overflow",
            )
        })?;
    if end > max {
        return Err(BackendError::new(
            BackendErrorKind::OutputBudgetExceeded,
            None,
            format!("output requires {end} bytes but budget is {max}"),
        ));
    }
    if bytes.is_empty() {
        return Ok(());
    }
    let start = output.len() as u64;
    output.extend_from_slice(bytes);
    if let Some(last) = map.entries.last_mut()
        && last.origin == origin
        && last.output_range.end == start
    {
        last.output_range.end = end;
    } else {
        map.entries.push(ArtifactMapEntry {
            output_range: Span { start, end },
            origin,
        });
    }
    Ok(())
}

const fn is_xml_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

#[cfg(test)]
mod tests;
