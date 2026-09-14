//! 反序列化后的结构验证。 / Post-decode structural validation.

use crate::*;
use core::fmt;
use std::collections::BTreeSet;

/// 所有持久 IR 在使用前实施此契约。 / Contract applied to every persistent IR before use.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}
/// 带稳定字段路径的验证错误。 / Validation error with a stable field path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    pub path: String,
    pub message: &'static str,
}
impl ValidationError {
    fn at(path: impl Into<String>, message: &'static str) -> Self {
        Self {
            path: path.into(),
            message,
        }
    }
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}
impl std::error::Error for ValidationError {}

impl Validate for ModuleObject {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_header(&self.header)?;
        let roots: Vec<_> = self
            .definitions
            .iter()
            .map(|definition| definition.body)
            .collect();
        validate_arena(
            &self.regions,
            &self.ops,
            self.header.semantic_strings.len(),
            self.header.qnames.len(),
            self.header.regexes.len(),
            &roots,
        )?;
        validate_sorted_unique(&self.external_symbols, "external_symbols")?;
        dense(self.definitions.iter().map(|d| d.id.0), "definitions")?;
        for (d, i) in self.definitions.iter().zip(&self.interface.definitions) {
            if d.id != i.id || d.symbol != i.symbol || d.signature != i.signature {
                return Err(ValidationError::at(
                    "interface",
                    "does not match definitions",
                ));
            }
            validate_signature(&d.signature)?;
            region(d.body, self.regions.len(), "definitions.body")?
        }
        if self.definitions.len() != self.interface.definitions.len() {
            return Err(ValidationError::at("interface", "wrong definition count"));
        }
        validate_attachment(&self.header, &self.attachment, &self.sources)?;
        validate_origins(&self.origins, &self.sources, self.ops.len())
    }
}
impl Validate for EntryObject {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_header(&self.header)?;
        validate_arena(
            &self.regions,
            &self.ops,
            self.header.semantic_strings.len(),
            self.header.qnames.len(),
            self.header.regexes.len(),
            &[self.root_region],
        )?;
        validate_sorted_unique(&self.external_symbols, "external_symbols")?;
        unique(self.required_params.iter(), "required_params")?;
        region(self.root_region, self.regions.len(), "root_region")?;
        if self.ops.iter().any(|o| matches!(o.op, Op::ReadSlot { .. })) {
            return Err(ValidationError::at("ops", "entry contains ReadSlot"));
        }
        validate_attachment(&self.header, &self.attachment, &self.sources)?;
        validate_origins(&self.origins, &self.sources, self.ops.len())
    }
}
impl Validate for RelocatableUnitIr {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Module(v) => v.validate(),
            Self::Entry(v) => v.validate(),
        }
    }
}

fn validate_header(h: &UnitHeader) -> Result<(), ValidationError> {
    validate_schema(h.ir_schema, h.feature_bits, "header")?;
    validate_sorted_unique(&h.semantic_strings, "semantic_strings")?;
    validate_sorted_unique(&h.qnames, "qnames")?;
    validate_sorted_unique(&h.regexes, "regexes")?;
    for regex in &h.regexes {
        index(regex.pattern.0, h.semantic_strings.len(), "regexes.pattern")?;
        unique(regex.named_captures.iter(), "regexes.named_captures")?;
    }
    dense(h.imports.iter().map(|i| i.local_id.0), "imports")?;
    for i in &h.imports {
        if i.expected_kind != UnitKind::Module {
            return Err(ValidationError::at(
                "imports.expected_kind",
                "imports must target modules",
            ));
        }
        match &i.spec {
            ImportSpec::RelativeUri(s) | ImportSpec::AbsoluteFileUri(s)
                if s.is_empty() || s.chars().any(char::is_control) =>
            {
                return Err(ValidationError::at(
                    "imports.spec",
                    "empty or control-bearing URI",
                ));
            }
            ImportSpec::PackageExport {
                dependency_alias,
                export,
            } if dependency_alias.is_empty() || export.is_empty() => {
                return Err(ValidationError::at(
                    "imports.spec",
                    "empty package alias/export",
                ));
            }
            _ => {}
        }
    }
    validate_source_key(&h.source)
}
fn validate_source_key(k: &SourceKey) -> Result<(), ValidationError> {
    match k {
        SourceKey::Project { path, package } => {
            if path.is_empty()
                || path.iter().any(|s| {
                    s.is_empty() || s.contains('/') || s.contains('\0') || s == "." || s == ".."
                })
            {
                return Err(ValidationError::at("source.path", "invalid logical path"));
            }
            if package.package_name.is_empty() || package.exact_revision.is_empty() {
                return Err(ValidationError::at(
                    "source.package",
                    "incomplete package identity",
                ));
            }
        }
        SourceKey::AdHoc { uri } => {
            if !uri.starts_with("file:") || uri.chars().any(char::is_control) {
                return Err(ValidationError::at("source.uri", "noncanonical file URI"));
            }
        }
    }
    Ok(())
}
fn validate_signature(s: &Signature) -> Result<(), ValidationError> {
    unique(s.params.iter(), "signature.params")?;
    unique(s.slots.iter().map(|x| &x.name), "signature.slots")
}
fn validate_arena(
    regions: &[Region],
    ops: &[OpRecord],
    strings: usize,
    qnames: usize,
    regexes: usize,
    roots: &[RegionId],
) -> Result<(), ValidationError> {
    dense(regions.iter().map(|x| x.id.0), "regions")?;
    dense(ops.iter().map(|x| x.id.0), "ops")?;
    let mut owners = vec![0u8; ops.len()];
    for r in regions {
        for op in &r.ops {
            index(op.0, ops.len(), "regions.ops")?;
            owners[op.0 as usize] = owners[op.0 as usize].saturating_add(1);
        }
    }
    if owners.iter().any(|count| *count != 1) {
        return Err(ValidationError::at(
            "regions.ops",
            "every operation must have exactly one owning region",
        ));
    }
    let mut region_owners = vec![0u8; regions.len()];
    for root in roots {
        index(root.0, regions.len(), "regions.root")?;
        region_owners[root.0 as usize] = region_owners[root.0 as usize].saturating_add(1);
    }
    for rec in ops {
        let sid = |x: StringId, p| index(x.0, strings, p);
        match &rec.op {
            Op::EmitText { value } | Op::EmitComment { value } => sid(*value, "ops.value")?,
            Op::EmitPi { target, data } => {
                sid(*target, "ops.target")?;
                sid(*data, "ops.data")?
            }
            Op::EmitElement {
                name,
                attributes,
                children,
            } => {
                index(name.0, qnames, "ops.name")?;
                region(*children, regions.len(), "ops.children")?;
                for a in attributes {
                    index(a.name.0, qnames, "ops.attributes.name")?;
                    sid(a.value, "ops.attributes.value")?
                }
            }
            Op::InsertScalar { value } => validate_binding(value)?,
            Op::MatchRegex {
                input,
                pattern,
                captures,
                matched,
            } => {
                index(pattern.0, regexes, "ops.pattern")?;
                region(*matched, regions.len(), "ops.matched")?;
                unique(captures.iter(), "ops.captures")?;
                if let MatchInput::Literal(x) = input {
                    sid(*x, "ops.input")?
                } else if let MatchInput::ReadBinding(x) = input {
                    validate_binding(x)?
                }
            }
            Op::ReadSlot { name } => {
                if name.is_empty() {
                    return Err(ValidationError::at("ops.slot", "empty name"));
                }
            }
            Op::Call { args, fills, .. } => {
                unique(args.iter().map(|x| &x.name), "ops.args")?;
                unique(fills.iter().map(|x| &x.name), "ops.fills")?;
                for a in args {
                    match &a.value {
                        ScalarExpr::Literal(x) => sid(*x, "ops.args.literal")?,
                        ScalarExpr::ReadBinding(x) => validate_binding(x)?,
                        ScalarExpr::RenderText(x) => region(*x, regions.len(), "ops.args.region")?,
                    }
                }
                for f in fills {
                    region(f.body, regions.len(), "ops.fills.body")?
                }
            }
        }
    }
    // 结构 region 使用 canonical preorder，因此每条结构边只能指向更晚的 region。
    // Structural regions use canonical preorder, so every structural edge points forward.
    for owner in regions {
        for op in &owner.ops {
            for child in child_regions(&ops[op.0 as usize].op) {
                if child.0 <= owner.id.0 {
                    return Err(ValidationError::at(
                        "regions",
                        "structural region cycle or noncanonical preorder",
                    ));
                }
                region_owners[child.0 as usize] = region_owners[child.0 as usize].saturating_add(1);
            }
        }
    }
    if region_owners.iter().any(|count| *count != 1) {
        return Err(ValidationError::at(
            "regions",
            "every region must have exactly one root or structural owner",
        ));
    }
    Ok(())
}

fn validate_schema(
    version: Version,
    features: FeatureBits,
    path: &str,
) -> Result<(), ValidationError> {
    // v1 注册了低两位；其余位必须通过新 schema 或显式 upgrader 引入。
    // v1 registers the low two bits; all others require a new schema or explicit upgrader.
    const SUPPORTED_V1_FEATURES: u64 = 0b11;
    if version.major != 1 {
        return Err(ValidationError::at(path, "unsupported schema major"));
    }
    if features.0 & !SUPPORTED_V1_FEATURES != 0 {
        return Err(ValidationError::at(path, "unknown required feature bits"));
    }
    Ok(())
}
fn child_regions(op: &Op) -> Vec<RegionId> {
    match op {
        Op::EmitElement { children, .. } => vec![*children],
        Op::MatchRegex { matched, .. } => vec![*matched],
        Op::Call { args, fills, .. } => args
            .iter()
            .filter_map(|arg| match &arg.value {
                ScalarExpr::RenderText(id) => Some(*id),
                _ => None,
            })
            .chain(fills.iter().map(|fill| fill.body))
            .collect(),
        _ => Vec::new(),
    }
}
fn validate_binding(b: &BindingRef) -> Result<(), ValidationError> {
    match b {
        BindingRef::Arg(s) | BindingRef::Match(s) if s.is_empty() => {
            Err(ValidationError::at("binding", "empty name"))
        }
        _ => Ok(()),
    }
}
fn validate_origins(
    o: &OriginTable,
    s: &SourceArchive,
    op_count: usize,
) -> Result<(), ValidationError> {
    validate_sorted_unique(&o.debug_strings, "origins.debug_strings")?;
    for record in &s.records {
        if record.digest.0.algorithm != DigestAlgorithm::Sha256
            || record.exact_bytes.digest.algorithm != DigestAlgorithm::Sha256
        {
            return Err(ValidationError::at(
                "sources.digest",
                "IR v1 requires SHA-256",
            ));
        }
        if !matches!(record.bom_len, 0 | 3)
            || record.exact_bytes.byte_len < u64::from(record.bom_len)
        {
            return Err(ValidationError::at("sources", "invalid UTF-8 BOM envelope"));
        }
        if record.line_start_offsets.first() != Some(&0)
            || !record.line_start_offsets.windows(2).all(|w| w[0] < w[1])
        {
            return Err(ValidationError::at(
                "sources.line_starts",
                "must begin at zero and increase",
            ));
        }
    }
    let mut op_origins = BTreeSet::new();
    for e in &o.entries {
        index(e.origin.source.0, s.records.len(), "origins.source")?;
        if let Some(name) = e.origin.lexical_qname {
            index(name.0, o.debug_strings.len(), "origins.lexical_qname")?;
        }
        let source = &s.records[e.origin.source.0 as usize];
        let payload_len = source.exact_bytes.byte_len - u64::from(source.bom_len);
        if e.origin.span.start >= e.origin.span.end {
            return Err(ValidationError::at(
                "origins.span",
                "origin must be nonempty",
            ));
        }
        if e.origin.span.end > payload_len {
            return Err(ValidationError::at(
                "origins.span",
                "span exceeds exact source envelope",
            ));
        }
        if e.entity_kind == EntityKind::Operation {
            index(e.local_id, op_count, "origins.operation")?;
            op_origins.insert(e.local_id);
        }
    }
    if op_origins.len() != op_count {
        return Err(ValidationError::at(
            "origins",
            "every operation needs one origin",
        ));
    }
    for map in &o.decoded_values {
        let mut end = 0;
        for segment in &map.segments {
            if segment.value_utf8_range.start != end
                || segment.value_utf8_range.start > segment.value_utf8_range.end
            {
                return Err(ValidationError::at(
                    "origins.decoded_values",
                    "decoded ranges must be ordered and contiguous",
                ));
            }
            end = segment.value_utf8_range.end;
        }
    }
    Ok(())
}

fn validate_attachment(
    header: &UnitHeader,
    attachment: &UnitSourceAttachment,
    sources: &SourceArchive,
) -> Result<(), ValidationError> {
    if attachment.source != header.source {
        return Err(ValidationError::at(
            "attachment.source",
            "must match unit source",
        ));
    }
    index(
        attachment.source_record.0,
        sources.records.len(),
        "attachment.source_record",
    )?;
    let record = &sources.records[attachment.source_record.0 as usize];
    if record.key != attachment.source || record.digest != attachment.source_digest {
        return Err(ValidationError::at(
            "attachment",
            "source record identity/digest mismatch",
        ));
    }
    Ok(())
}

impl Validate for StaticLinkMap {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_sorted_unique_by(&self.symbols, "symbols", |x| &x.0)?;
        validate_sorted_unique_by(&self.relocations, "relocations", |x| &x.0)
    }
}
impl Validate for ResolutionSnapshot {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_sorted_unique_by(&self.units, "resolution.units", |unit| &unit.0)?;
        if !self
            .imports
            .windows(2)
            .all(|w| (&w[0].importer, w[0].import) < (&w[1].importer, w[1].import))
            && self.imports.len() > 1
        {
            return Err(ValidationError::at(
                "resolution.imports",
                "must be sorted and unique",
            ));
        }
        Ok(())
    }
}
impl Validate for LinkedImage {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema(self.schema, self.feature_bits, "linked_image")?;
        self.link_map.validate()?;
        validate_sorted_unique_by(&self.units, "units", |u| &u.source)?;
        for d in &self.definitions {
            index(d.addr.unit_slot, self.units.len(), "definitions.addr")?;
            index(d.body.unit_slot, self.units.len(), "definitions.body")?;
            validate_signature(&d.signature)?
        }
        index(
            self.entry.root_region.unit_slot,
            self.units.len(),
            "entry.root_region",
        )?;
        Ok(())
    }
}
impl Validate for ExpansionTrace {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_sorted_unique(&self.scalar_values, "trace.scalar_values")?;
        validate_sorted_unique(&self.debug_strings, "trace.debug_strings")?;
        dense(self.frames.iter().map(|f| f.id.0), "trace.frames")?;
        let roots = self
            .frames
            .iter()
            .filter(|frame| frame.parent.is_none())
            .count();
        if !self.frames.is_empty()
            && (roots != 1
                || self.frames[0].identity != FrameIdentity::Entry
                || self.frames[0].depth != 1)
        {
            return Err(ValidationError::at(
                "trace.frames",
                "frames must form one entry-rooted tree",
            ));
        }
        for f in &self.frames {
            if let Some(p) = f.parent
                && p.0 >= f.id.0
            {
                return Err(ValidationError::at(
                    "trace.frames.parent",
                    "parent must be earlier",
                ));
            }
            if let Some(parent) = f.parent
                && self.frames[parent.0 as usize].depth.checked_add(1) != Some(f.depth)
            {
                return Err(ValidationError::at(
                    "trace.frames.depth",
                    "child depth must be parent depth plus one",
                ));
            }
            for (_, value) in &f.args {
                index(value.0, self.scalar_values.len(), "trace.frames.args")?;
            }
            for (_, sequence) in &f.fills {
                index(sequence.0, self.sequences.len(), "trace.frames.fills")?;
            }
        }
        for (i, n) in self.origins.iter().enumerate() {
            let prior = |x: OriginNodeId| {
                if (x.0 as usize) < i {
                    Ok(())
                } else {
                    Err(ValidationError::at(
                        "trace.origins",
                        "edge must point earlier",
                    ))
                }
            };
            match n {
                OriginNode::ExternalArgument { value, .. } => {
                    index(value.0, self.scalar_values.len(), "trace.origins.scalar")?
                }
                OriginNode::Expansion { frame, .. } => {
                    index(frame.0, self.frames.len(), "trace.origins.frame")?
                }
                OriginNode::Import { child, .. } => prior(*child)?,
                OriginNode::RegexCapture {
                    input,
                    matched_range,
                    ..
                } => {
                    prior(*input)?;
                    if matched_range.start > matched_range.end {
                        return Err(ValidationError::at("trace.range", "reversed range"));
                    }
                }
                OriginNode::Concat { ordered_inputs } => {
                    for x in ordered_inputs {
                        prior(*x)?
                    }
                }
                OriginNode::BackendTransform {
                    backend_step,
                    inputs,
                } => {
                    index(
                        backend_step.0,
                        self.debug_strings.len(),
                        "trace.origins.string",
                    )?;
                    for x in inputs {
                        prior(x.parent)?
                    }
                }
                OriginNode::Fused { role, inputs } => {
                    index(role.0, self.debug_strings.len(), "trace.origins.string")?;
                    for x in inputs {
                        prior(x.parent)?
                    }
                }
                OriginNode::Synthetic {
                    reason,
                    nearest: Some(x),
                } => {
                    index(reason.0, self.debug_strings.len(), "trace.origins.string")?;
                    prior(*x)?
                }
                OriginNode::Synthetic {
                    reason,
                    nearest: None,
                }
                | OriginNode::Unknown { reason } => {
                    index(reason.0, self.debug_strings.len(), "trace.origins.string")?
                }
                _ => {}
            }
        }
        for trace in &self.document_items {
            index(
                trace.frame.0,
                self.frames.len(),
                "trace.document_items.frame",
            )?;
        }
        for sequence in &self.sequences {
            for occurrence in sequence {
                index(
                    occurrence.0,
                    self.document_items.len(),
                    "trace.sequences.document_item",
                )?;
            }
        }
        Ok(())
    }
}
impl ExpansionTrace {
    /// 将运行 trace 与实际文档联合验证，防止 occurrence ID 在独立解码后悬空。
    /// Jointly validates a runtime trace against the actual document so occurrence IDs cannot dangle.
    pub fn validate_against_document(
        &self,
        document: &LinkedDocumentIr,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        document.validate()?;
        if self.document_items.len() != document.items.len() {
            return Err(ValidationError::at(
                "trace.document_items",
                "must contain exactly one trace reference per document occurrence",
            ));
        }
        for sequence in &self.sequences {
            for occurrence in sequence {
                index(
                    occurrence.0,
                    document.items.len(),
                    "trace.sequences.document_item",
                )?;
            }
        }
        Ok(())
    }
}
impl Validate for LinkedDocumentIr {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_schema(self.schema, self.feature_bits, "document")?;
        validate_sorted_unique(&self.strings, "document.strings")?;
        validate_sorted_unique(&self.qnames, "document.qnames")?;
        dense(self.regions.iter().map(|r| r.id.0), "document.regions")?;
        index(self.root.0, self.regions.len(), "document.root")?;
        for r in &self.regions {
            if r.start > r.end || r.end as usize > self.items.len() {
                return Err(ValidationError::at(
                    "document.regions",
                    "invalid item range",
                ));
            }
        }
        let root = &self.regions[self.root.0 as usize];
        if root.start != 0 || root.end as usize != self.items.len() {
            return Err(ValidationError::at(
                "document.root",
                "root region must cover the complete event tape",
            ));
        }
        let mut stack: Vec<(usize, DocumentRegionId)> = Vec::new();
        let mut child_owners = vec![0u8; self.regions.len()];
        for (position, item) in self.items.iter().enumerate() {
            match item {
                DocumentItem::ElementStart {
                    expanded_name,
                    attributes,
                    children,
                } => {
                    index(expanded_name.0, self.qnames.len(), "document.qname")?;
                    index(children.0, self.regions.len(), "document.children")?;
                    child_owners[children.0 as usize] =
                        child_owners[children.0 as usize].saturating_add(1);
                    stack.push((position, *children));
                    for a in attributes {
                        index(a.name.0, self.qnames.len(), "document.attribute.name")?;
                        index(a.value.0, self.strings.len(), "document.attribute")?
                    }
                }
                DocumentItem::ElementEnd => {
                    let (start, children) = stack
                        .pop()
                        .ok_or_else(|| ValidationError::at("document.items", "unbalanced end"))?;
                    let region = &self.regions[children.0 as usize];
                    if region.start as usize != start + 1 || region.end as usize != position {
                        return Err(ValidationError::at(
                            "document.children",
                            "child region does not describe the enclosed event range",
                        ));
                    }
                }
                DocumentItem::Text { value } | DocumentItem::Comment { value } => {
                    index(value.0, self.strings.len(), "document.value")?
                }
                DocumentItem::ProcessingInstruction { target, data } => {
                    index(target.0, self.strings.len(), "document.target")?;
                    index(data.0, self.strings.len(), "document.data")?
                }
            }
        }
        if !stack.is_empty() {
            return Err(ValidationError::at("document.items", "unbalanced start"));
        }
        for (index, owners) in child_owners.into_iter().enumerate() {
            let expected = u8::from(index != self.root.0 as usize);
            if owners != expected {
                return Err(ValidationError::at(
                    "document.regions",
                    "every non-root region must own exactly one element child range",
                ));
            }
        }
        Ok(())
    }
}
impl Validate for ArtifactByteMap {
    fn validate(&self) -> Result<(), ValidationError> {
        let mut end = 0;
        for e in &self.entries {
            if e.output_range.start != end || e.output_range.start >= e.output_range.end {
                return Err(ValidationError::at(
                    "artifact_map",
                    "ranges must exactly cover output",
                ));
            }
            end = e.output_range.end
        }
        Ok(())
    }
}
impl ArtifactByteMap {
    /// 联合 trace 和实际产品长度验证映射。 / Validates the map against its trace and actual product length.
    pub fn validate_against(
        &self,
        trace: &ExpansionTrace,
        output_len: u64,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        let covered = self
            .entries
            .last()
            .map_or(0, |entry| entry.output_range.end);
        if covered != output_len {
            return Err(ValidationError::at(
                "artifact_map",
                "mapping must cover every product byte",
            ));
        }
        for entry in &self.entries {
            index(entry.origin.0, trace.origins.len(), "artifact_map.origin")?;
        }
        Ok(())
    }
}

fn index(i: u32, n: usize, p: &str) -> Result<(), ValidationError> {
    if (i as usize) < n {
        Ok(())
    } else {
        Err(ValidationError::at(p, "index out of bounds"))
    }
}
fn region(i: RegionId, n: usize, p: &str) -> Result<(), ValidationError> {
    index(i.0, n, p)
}
fn dense(xs: impl IntoIterator<Item = u32>, p: &str) -> Result<(), ValidationError> {
    for (i, x) in xs.into_iter().enumerate() {
        if x != i as u32 {
            return Err(ValidationError::at(p, "IDs must be dense and ordered"));
        }
    }
    Ok(())
}
fn unique<'a, T: Ord + 'a>(
    xs: impl IntoIterator<Item = &'a T>,
    p: &str,
) -> Result<(), ValidationError> {
    let mut set = BTreeSet::new();
    for x in xs {
        if !set.insert(x) {
            return Err(ValidationError::at(p, "duplicate value"));
        }
    }
    Ok(())
}
fn validate_sorted_unique<T: Ord>(x: &[T], p: &str) -> Result<(), ValidationError> {
    if x.len() < 2 || x.windows(2).all(|w| w[0] < w[1]) {
        Ok(())
    } else {
        Err(ValidationError::at(p, "must be sorted and unique"))
    }
}
fn validate_sorted_unique_by<T, K: Ord>(
    x: &[T],
    p: &str,
    key: impl Fn(&T) -> &K,
) -> Result<(), ValidationError> {
    if x.windows(2).all(|w| key(&w[0]) < key(&w[1])) || x.len() < 2 {
        Ok(())
    } else {
        Err(ValidationError::at(p, "must be sorted and unique"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provenance_rejects_forward_edges() {
        let t = ExpansionTrace {
            origins: vec![OriginNode::Concat {
                ordered_inputs: vec![OriginNodeId(0)],
            }],
            ..Default::default()
        };
        assert!(t.validate().is_err())
    }
    #[test]
    fn map_requires_coverage() {
        let m = ArtifactByteMap {
            entries: vec![ArtifactMapEntry {
                output_range: Span { start: 1, end: 2 },
                origin: OriginNodeId(0),
            }],
        };
        assert!(m.validate().is_err())
    }

    #[test]
    fn map_is_checked_against_product_and_trace() {
        let m = ArtifactByteMap {
            entries: vec![ArtifactMapEntry {
                output_range: Span { start: 0, end: 2 },
                origin: OriginNodeId(0),
            }],
        };
        let trace = ExpansionTrace {
            origins: vec![OriginNode::Unknown {
                reason: DebugStringId(0),
            }],
            ..Default::default()
        };
        assert!(m.validate_against(&trace, 2).is_ok());
        assert!(m.validate_against(&trace, 3).is_err());
    }

    #[test]
    fn structural_region_cycle_is_not_macro_recursion() {
        let regions = vec![Region {
            id: RegionId(0),
            ops: vec![OpId(0)],
        }];
        let ops = vec![OpRecord {
            id: OpId(0),
            op: Op::EmitElement {
                name: QNameId(0),
                attributes: vec![],
                children: RegionId(0),
            },
        }];
        assert!(validate_arena(&regions, &ops, 0, 1, 0, &[RegionId(0)]).is_err());
    }

    #[test]
    fn every_region_has_exactly_one_owner() {
        let regions = vec![Region {
            id: RegionId(0),
            ops: vec![],
        }];
        assert!(validate_arena(&regions, &[], 0, 0, 0, &[RegionId(0), RegionId(0)]).is_err());
    }

    #[test]
    fn schema_and_feature_contract_is_closed() {
        let document = |major, features| LinkedDocumentIr {
            schema: Version { major, minor: 0 },
            document_abi: AbiId("document-v1".into()),
            root: DocumentRegionId(0),
            regions: vec![DocumentRegion {
                id: DocumentRegionId(0),
                start: 0,
                end: 0,
            }],
            items: vec![],
            strings: vec![],
            qnames: vec![],
            feature_bits: FeatureBits(features),
        };
        assert!(document(2, 0).validate().is_err());
        assert!(document(1, 4).validate().is_err());
        assert!(document(1, 0).validate().is_ok());
    }

    #[test]
    fn trace_sequences_are_joined_to_actual_document() {
        let document = LinkedDocumentIr {
            schema: Version { major: 1, minor: 0 },
            document_abi: AbiId("document-v1".into()),
            root: DocumentRegionId(0),
            regions: vec![DocumentRegion {
                id: DocumentRegionId(0),
                start: 0,
                end: 0,
            }],
            items: vec![],
            strings: vec![],
            qnames: vec![],
            feature_bits: FeatureBits(0),
        };
        let trace = ExpansionTrace {
            sequences: vec![vec![DocumentItemId(0)]],
            ..Default::default()
        };
        assert!(trace.validate().is_err());
        assert!(trace.validate_against_document(&document).is_err());
    }
}
