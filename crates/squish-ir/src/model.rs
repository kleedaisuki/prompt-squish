//! 前端、链接器、求值器和后端之间的强类型协议。 / Typed contracts between stages.

use crate::{
    ArtifactDigest, DebugDigest, Digest, DocumentDigest, LinkedImageDigest, ObjectDigest,
    SemanticUnitDigest, SourceDigest,
};

macro_rules! id { ($($n:ident),+$(,)?)=>{$(
    #[doc="固定宽度 arena 标识；只在所属对象内有效。 / Fixed-width arena ID, local to its owner."]
    #[derive(Clone,Copy,Debug,Default,Eq,Hash,Ord,PartialEq,PartialOrd)] pub struct $n(pub u32);
)+}; }
id!(
    StringId,
    QNameId,
    RegexId,
    DebugStringId,
    SourceRef,
    ImportId,
    LocalDefId,
    RegionId,
    OpId,
    OriginId,
    DecodedValueMapId,
    FrameId,
    ScalarValueId,
    SequenceValueId,
    OriginNodeId,
    DocumentItemId,
    DocumentRegionId,
    LinkImportRef
);

/// 半开 UTF-8 字节区间。 / Half-open UTF-8 byte range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}
/// 协议版本。 / Protocol version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}
/// 不透明 ABI 标识。 / Opaque ABI identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AbiId(pub String);
/// 特性位；wire 编码固定为 u64。 / Feature bits, fixed-width u64 on wire.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureBits(pub u64);

/// 精确解析器包实例。 / Exact resolver package instance.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageInstanceId {
    pub source_kind: u16,
    pub canonical_source: String,
    pub package_name: String,
    pub exact_revision: String,
}
/// 与物理 checkout 无关的源身份。 / Source identity independent of physical checkout.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SourceKey {
    Project {
        package: PackageInstanceId,
        path: Vec<String>,
    },
    AdHoc {
        uri: String,
    },
}
/// XML 展开名，也是符号身份。 / XML expanded name and symbol identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExpandedName {
    pub namespace_uri: String,
    pub local_name: String,
}
/// 宏符号键。 / Macro symbol key.
pub type SymbolKey = ExpandedName;
/// 本地 XML 名。 / Local XML name.
pub type LocalName = String;

/// 外置精确源 blob。 / Externally stored exact source blob.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobRef {
    pub digest: Digest,
    pub byte_len: u64,
}
/// 精确源记录。 / Exact source record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRecord {
    pub key: SourceKey,
    pub digest: SourceDigest,
    pub bom_len: u8,
    pub exact_bytes: BlobRef,
    pub line_start_offsets: Vec<u64>,
}
/// 对象调试附件中的源集合。 / Sources attached to object debug data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceArchive {
    pub records: Vec<SourceRecord>,
}
/// 语法种类的稳定数字标识。 / Stable numeric syntax-kind identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyntaxKind(pub u16);
/// 静态源 origin。 / Static source origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Origin {
    pub source: SourceRef,
    pub span: Span,
    pub lexical_qname: Option<DebugStringId>,
    pub syntax_kind: SyntaxKind,
}
/// 语义实体种类。 / Semantic entity kind.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EntityKind {
    Import,
    Definition,
    Region,
    Operation,
    Parameter,
    Slot,
    ExternalSymbol,
}
/// 静态 origin 表项。 / Static origin table entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OriginEntry {
    pub entity_kind: EntityKind,
    pub local_id: u32,
    pub origin: Origin,
}
/// 单元局部 origin 表。 / Unit-local origin table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OriginTable {
    pub entries: Vec<OriginEntry>,
    pub debug_strings: Vec<String>,
    pub decoded_values: Vec<DecodedValueMap>,
}
/// 跨对象 origin 引用。 / Cross-object origin reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualifiedOriginRef {
    pub object: ObjectDigest,
    pub local: OriginId,
}
/// 解码字符串段来源。 / Source of a decoded string segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodedSyntax {
    LiteralText,
    CharacterReference,
    EntityReference,
    CData,
}
/// 解码字段身份。 / Decoded field identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedOwner {
    pub entity_kind: EntityKind,
    pub local_id: u32,
    pub field: String,
}
/// 精确值到源的段映射。 / Exact decoded-value segment map.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedValueMap {
    pub owner: DecodedOwner,
    pub segments: Vec<DecodedSegment>,
}
/// 一个解码段。 / One decoded segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedSegment {
    pub value_utf8_range: Span,
    pub source_span: Span,
    pub syntax: DecodedSyntax,
}
/// 跨对象解码映射引用。 / Qualified decoded-value map reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualifiedDecodedValueMapRef {
    pub object: ObjectDigest,
    pub local: DecodedValueMapId,
}

/// 静态导入规格。 / Static import specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportSpec {
    RelativeUri(String),
    AbsoluteFileUri(String),
    PackageExport {
        dependency_alias: String,
        export: String,
    },
}
/// 导入预期种类。 / Expected imported unit kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnitKind {
    Entry,
    Module,
}
/// 可重定位导入。 / Relocatable import.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportDecl {
    pub local_id: ImportId,
    pub spec: ImportSpec,
    pub expected_kind: UnitKind,
}
/// 单元公共头。 / Shared unit header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitHeader {
    pub ir_schema: Version,
    pub language_abi: AbiId,
    pub frontend_abi: AbiId,
    pub regex_abi: AbiId,
    pub source: SourceKey,
    pub imports: Vec<ImportDecl>,
    pub semantic_strings: Vec<String>,
    /// 按展开名排序去重的 QName 池。 / Expanded-name-sorted unique QName pool.
    pub qnames: Vec<ExpandedName>,
    /// 按 canonical pattern 排序去重的 regex 池；不含编译器状态。 / Canonical regex pool without compiled state.
    pub regexes: Vec<RegexPattern>,
    pub feature_bits: FeatureBits,
}
/// 可移植 regex 描述；执行语义由 `regex_abi` 固定。 / Portable regex descriptor governed by `regex_abi`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegexPattern {
    pub pattern: StringId,
    pub named_captures: Vec<LocalName>,
}
/// 调试源绑定。 / Debug source binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitSourceAttachment {
    pub source: SourceKey,
    pub source_digest: SourceDigest,
    pub source_record: SourceRef,
}
/// 非语义 producer 信息。 / Non-semantic producer information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Producer {
    pub tool_version: String,
    pub build_fingerprint: String,
}
/// 参数/slot 签名。 / Parameter/slot signature.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Signature {
    pub params: Vec<LocalName>,
    pub slots: Vec<SlotDecl>,
}
/// Slot 声明。 / Slot declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlotDecl {
    pub name: LocalName,
    pub required: bool,
}
/// 宏定义。 / Macro definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroDef {
    pub id: LocalDefId,
    pub symbol: SymbolKey,
    pub signature: Signature,
    pub body: RegionId,
}
/// 链接所需接口摘要。 / Link-facing interface summary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InterfaceSummary {
    pub definitions: Vec<InterfaceDef>,
}
/// 一个公开宏签名。 / One published macro signature.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceDef {
    pub id: LocalDefId,
    pub symbol: SymbolKey,
    pub signature: Signature,
}

/// 平坦 arena 中的 region。 / Region in the flat arena.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Region {
    pub id: RegionId,
    pub ops: Vec<OpId>,
}
/// 属性保留作者顺序。 / Attribute preserving author order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribute {
    pub name: QNameId,
    pub value: StringId,
}
/// 绑定读取。 / Binding lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindingRef {
    File(FileBinding),
    Arg(LocalName),
    Match(LocalName),
}
/// file.* 绑定。 / file.* binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileBinding {
    Uri,
    Dir,
    Name,
}
/// Regex 输入。 / Regex input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatchInput {
    Literal(StringId),
    ReadBinding(BindingRef),
}
/// 标量表达式。 / Scalar expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScalarExpr {
    Literal(StringId),
    ReadBinding(BindingRef),
    RenderText(RegionId),
}
/// 有名实参。 / Named argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Argument {
    pub name: LocalName,
    pub value: ScalarExpr,
}
/// Slot fill。 / Slot fill.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fill {
    pub name: LocalName,
    pub body: RegionId,
}
/// 当前 DSL 的闭合操作代数。 / Closed operation algebra for the current DSL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
    EmitText {
        value: StringId,
    },
    EmitComment {
        value: StringId,
    },
    EmitPi {
        target: StringId,
        data: StringId,
    },
    EmitElement {
        name: QNameId,
        attributes: Vec<Attribute>,
        children: RegionId,
    },
    InsertScalar {
        value: BindingRef,
    },
    MatchRegex {
        input: MatchInput,
        pattern: RegexId,
        captures: Vec<LocalName>,
        matched: RegionId,
    },
    ReadSlot {
        name: LocalName,
    },
    Call {
        target: SymbolKey,
        args: Vec<Argument>,
        fills: Vec<Fill>,
    },
}
/// ID 和 payload 分离的 operation 表项。 / ID/payload separated operation entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpRecord {
    pub id: OpId,
    pub op: Op,
}
/// 模块对象。 / Relocatable module object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleObject {
    pub header: UnitHeader,
    pub definitions: Vec<MacroDef>,
    pub external_symbols: Vec<SymbolKey>,
    pub interface: InterfaceSummary,
    pub regions: Vec<Region>,
    pub ops: Vec<OpRecord>,
    pub origins: OriginTable,
    pub sources: SourceArchive,
    pub attachment: UnitSourceAttachment,
    pub producer: Producer,
}
/// Entry 链接根对象。 / Entry link-root object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryObject {
    pub header: UnitHeader,
    pub required_params: Vec<LocalName>,
    pub root_region: RegionId,
    pub external_symbols: Vec<SymbolKey>,
    pub regions: Vec<Region>,
    pub ops: Vec<OpRecord>,
    pub origins: OriginTable,
    pub sources: SourceArchive,
    pub attachment: UnitSourceAttachment,
    pub producer: Producer,
}
/// 任一可重定位单元。 / Any relocatable unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelocatableUnitIr {
    Module(ModuleObject),
    Entry(EntryObject),
}

/// 便携定义地址。 / Portable definition address.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DefAddr {
    pub unit_slot: u32,
    pub local_def: LocalDefId,
}
/// 链接后的 region。 / Linked region reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkedRegionRef {
    pub unit_slot: u32,
    pub region: RegionId,
}
/// 链接后的 operation。 / Linked operation reference.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LinkedOpRef {
    pub unit_slot: u32,
    pub op: OpId,
}
/// 静态绑定映射；可独立重建 session 索引。 / Static binding map; session indexes are reconstructable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StaticLinkMap {
    pub symbols: Vec<(SymbolKey, DefAddr)>,
    pub relocations: Vec<(LinkedOpRef, DefAddr)>,
}
/// 一次解析快照中的精确 unit revision。 / Exact unit revision in a resolution snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnitRevision {
    pub kind: UnitKind,
    pub semantic: SemanticUnitDigest,
    pub object: ObjectDigest,
}
/// 解析快照中的导入边。 / Import edge in a resolution snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBinding {
    pub importer: SourceKey,
    pub import: ImportId,
    pub target: SourceKey,
}
/// 可移植、确定顺序的 resolver 输出。 / Portable, deterministically ordered resolver output.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolutionSnapshot {
    pub units: Vec<(SourceKey, UnitRevision)>,
    pub imports: Vec<ImportBinding>,
}
/// 镜像的单元记录。 / Unit record in an image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedUnit {
    pub kind: UnitKind,
    pub source: SourceKey,
    pub semantic_digest: SemanticUnitDigest,
}
/// 链接宏。 / Linked macro definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedMacroDef {
    pub addr: DefAddr,
    pub symbol: SymbolKey,
    pub signature: Signature,
    pub body: LinkedRegionRef,
}
/// 链接入口。 / Linked entry metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedEntry {
    pub source: SourceKey,
    pub semantic_digest: SemanticUnitDigest,
    pub required_params: Vec<LocalName>,
    pub root_region: LinkedRegionRef,
}
/// 可由语义 blob 和静态映射重建执行 session 的镜像。 / Session-reconstructable linked image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedImage {
    pub schema: Version,
    pub language_abi: AbiId,
    pub entry: LinkedEntry,
    pub units: Vec<LinkedUnit>,
    pub definitions: Vec<LinkedMacroDef>,
    pub link_map: StaticLinkMap,
    pub feature_bits: FeatureBits,
}
/// 完整 import resolution 证据。 / Complete import-resolution evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkImportRecord {
    pub importer: SourceKey,
    pub import_id: ImportId,
    pub spec: ImportSpec,
    pub resolved_source: SourceKey,
    pub resolved_object: ObjectDigest,
    pub origin: QualifiedOriginRef,
}
/// 完整 symbol relocation 证据。 / Complete symbol-relocation evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkSymbolRecord {
    pub reference_origin: QualifiedOriginRef,
    pub symbol: SymbolKey,
    pub definition: DefAddr,
    pub definition_origin: QualifiedOriginRef,
}
/// 可机器查询的链接 trace。 / Machine-queryable link trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkTrace {
    pub entry_object: ObjectDigest,
    pub resolution_snapshot: Digest,
    pub imports: Vec<LinkImportRecord>,
    pub symbols: Vec<LinkSymbolRecord>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

/// 动态 expansion frame。 / Dynamic expansion frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameRecord {
    pub id: FrameId,
    pub parent: Option<FrameId>,
    pub identity: FrameIdentity,
    pub call_origin: Option<QualifiedOriginRef>,
    pub definition_origin: QualifiedOriginRef,
    pub depth: u64,
    pub args: Vec<(LocalName, ScalarValueId)>,
    pub fills: Vec<(LocalName, SequenceValueId)>,
}
/// 一步替换的稳定 kind。 / Stable kind of one substitution step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubstitutionKind {
    SlotFill,
    ScalarBody,
    InsertScalar,
}
/// 输出 occurrence 的一次替换。 / One substitution contributing to an output occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubstitutionStep {
    pub kind: SubstitutionKind,
    pub origin: QualifiedOriginRef,
}
/// 文档 occurrence 与静态、动态来源的连接。 / Join from a document occurrence to static/dynamic provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceRef {
    pub producer_op: LinkedOpRef,
    pub frame: FrameId,
    pub definition_origin: QualifiedOriginRef,
    pub call_origin: Option<QualifiedOriginRef>,
    pub substitution_chain: Vec<SubstitutionStep>,
}
/// Frame 身份。 / Frame identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameIdentity {
    Entry,
    Macro(DefAddr),
}
/// Origin DAG 的有类型边。 / Typed edge in the origin DAG.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OriginEdge {
    pub role: OriginRole,
    pub parent: OriginNodeId,
}
/// Origin 边角色。 / Origin edge role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginRole {
    Input,
    Caller,
    Definition,
    Substitution,
    Transform,
}
/// 完整静态+动态来源节点。 / Complete static/dynamic provenance node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OriginNode {
    SourceSpan {
        origin: QualifiedOriginRef,
    },
    DecodedSegment {
        map: QualifiedDecodedValueMapRef,
        segment_index: u32,
    },
    ExternalArgument {
        name: LocalName,
        value: ScalarValueId,
    },
    Expansion {
        frame: FrameId,
        producer: LinkedOpRef,
    },
    Import {
        edge: LinkImportRef,
        child: OriginNodeId,
    },
    RegexCapture {
        input: OriginNodeId,
        capture: LocalName,
        matched_range: Span,
    },
    Concat {
        ordered_inputs: Vec<OriginNodeId>,
    },
    BackendTransform {
        backend_step: DebugStringId,
        inputs: Vec<OriginEdge>,
    },
    Fused {
        role: DebugStringId,
        inputs: Vec<OriginEdge>,
    },
    Synthetic {
        reason: DebugStringId,
        nearest: Option<OriginNodeId>,
    },
    Unknown {
        reason: DebugStringId,
    },
}
/// 动态来源 DAG。 / Dynamic provenance DAG.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExpansionTrace {
    pub frames: Vec<FrameRecord>,
    pub scalar_values: Vec<String>,
    pub sequences: Vec<Vec<DocumentItemId>>,
    pub debug_strings: Vec<String>,
    pub origins: Vec<OriginNode>,
    pub document_items: Vec<TraceRef>,
}

/// 诊断级别。 / Diagnostic severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}
/// 稳定诊断值。 / Stable diagnostic argument value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticValue {
    String(String),
    Unsigned(u64),
    Symbol(SymbolKey),
    Source(SourceKey),
}
/// 本地化之前的结构化诊断。 / Structured diagnostic before localization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message_args: Vec<(StringId, DiagnosticValue)>,
    pub primary_origin: QualifiedOriginRef,
    pub related_origins: Vec<(DebugStringId, QualifiedOriginRef)>,
    pub frame_chain: Vec<FrameId>,
}

/// 后端中立文档 item。 / Backend-neutral document item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentItem {
    Text {
        value: StringId,
    },
    Comment {
        value: StringId,
    },
    ProcessingInstruction {
        target: StringId,
        data: StringId,
    },
    ElementStart {
        expanded_name: QNameId,
        attributes: Vec<Attribute>,
        children: DocumentRegionId,
    },
    ElementEnd,
}
/// 平坦平衡事件带上的 region。 / Region over the flat balanced event tape.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentRegion {
    pub id: DocumentRegionId,
    pub start: u32,
    pub end: u32,
}
/// 完全展开的后端输入。 / Fully expanded backend input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedDocumentIr {
    pub schema: Version,
    pub document_abi: AbiId,
    pub root: DocumentRegionId,
    pub regions: Vec<DocumentRegion>,
    pub items: Vec<DocumentItem>,
    pub strings: Vec<String>,
    pub qnames: Vec<ExpandedName>,
    pub feature_bits: FeatureBits,
}
/// 文档与本次 provenance 的关联。 / Association between document and current provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentBundle {
    pub document: DocumentDigest,
    pub linked_image: LinkedImageDigest,
    pub runtime_transcript: Digest,
    pub expansion_trace: DebugDigest,
}
/// 产品字节来源映射。 / Product-byte provenance mapping.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMapEntry {
    pub output_range: Span,
    pub origin: OriginNodeId,
}
/// 完整产品映射。 / Complete product mapping.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ArtifactByteMap {
    pub entries: Vec<ArtifactMapEntry>,
}

/// 最终产品的稳定身份；产品字节本身不进入 debug bundle。 / Stable final-artifact identity; product bytes are not embedded in the debug bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactIdentity {
    /// 领域分离的产品内容摘要。 / Domain-separated artifact-content digest.
    pub digest: ArtifactDigest,
    /// 产品的精确字节长度。 / Exact artifact byte length.
    pub byte_len: u64,
}

/// 一个完整 unit 的 source/debug archive 引用。 / Reference to one complete unit's source/debug archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceArchiveReference {
    /// 产生该 archive 的完整 unit 对象身份。 / Complete unit-object identity that produced this archive.
    pub object: ObjectDigest,
    /// 原 unit 对象的 debug projection 身份。 / Debug-projection identity of the original unit object.
    pub debug_digest: DebugDigest,
    /// 自包含的规范源记录。 / Self-contained canonical source records.
    pub archive: SourceArchive,
    /// Qualified trace 引用所需的 unit-local 静态 provenance 表。 / Unit-local static
    /// provenance table required to resolve qualified trace references.
    pub origins: OriginTable,
}

/// 可移植 bundle 中按内容寻址的精确源 blob。 / Content-addressed exact-source blob carried by a portable bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledSourceBlob {
    /// 精确源字节的内容地址。 / Content address of the exact source bytes.
    pub reference: BlobRef,
    /// 可移植调试所需的精确源字节。 / Exact source bytes required for portable debugging.
    pub bytes: Vec<u8>,
}

/// `.psdbg` 的强类型、跨进程持久值。 / Strongly typed, cross-process persistent `.psdbg` value.
///
/// `document`、`expansion_trace` 与 `artifact_map` 在解码时联合校验。Archive 只通过
/// `BlobRef` 指向 `source_blobs`，从而保持 unit object 的原始编码不变。
/// `document`, `expansion_trace`, and `artifact_map` are jointly validated on decode. Archives
/// address `source_blobs` through `BlobRef`, preserving the original unit-object encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugBundle {
    /// `.psdbg` typed payload schema。 / `.psdbg` typed-payload schema.
    pub schema: Version,
    /// 此 schema 已启用的 feature 位。 / Feature bits enabled for this schema.
    pub feature_bits: FeatureBits,
    /// bundle 所描述的最终产品身份。 / Final-artifact identity described by this bundle.
    pub artifact: ArtifactIdentity,
    /// `document` 的规范 wire 身份。 / Canonical wire identity of `document`.
    pub document_digest: DocumentDigest,
    /// 产生文档的链接镜像身份。 / Identity of the linked image that produced the document.
    pub linked_image: LinkedImageDigest,
    /// `expansion_trace` 的规范 wire 身份。 / Canonical wire identity of `expansion_trace`.
    pub expansion_trace_digest: DebugDigest,
    /// `link_trace` 的规范 wire 身份。 / Canonical wire identity of `link_trace`.
    pub link_trace_digest: DebugDigest,
    /// 后端消费的完全展开文档。 / Fully expanded document consumed by the backend.
    pub document: LinkedDocumentIr,
    /// 文档 occurrence 到源 frame 的动态 provenance。 / Dynamic provenance from document occurrences to source frames.
    pub expansion_trace: ExpansionTrace,
    /// Import/symbol resolution 的可查询链接证据。 / Queryable import/symbol resolution evidence.
    pub link_trace: LinkTrace,
    /// 最终产品字节到 trace origin 的覆盖映射。 / Coverage map from final-artifact bytes to trace origins.
    pub artifact_map: ArtifactByteMap,
    /// 按 unit 对象身份排序的源 archives。 / Source archives sorted by unit-object identity.
    pub source_archives: Vec<SourceArchiveReference>,
    /// 按 `(digest, byte_len)` 排序的精确源 blobs。 / Exact source blobs sorted by `(digest, byte_len)`.
    pub source_blobs: Vec<BundledSourceBlob>,
}
