//! `.psdbg` 调试 bundle 的规范持久格式。 / Canonical persistent `.psdbg` debug-bundle format.
//!
//! 外层复用 XSIR 的 checksum 分节容器；每个必要成员占一个独立 section。这样损坏在
//! typed decode 前被发现，而未知的非关键 debug section 仍可由旧 reader 安全跳过。
//! The outer checksum-protected XSIR container is reused, with one section per required member.
//! Corruption is therefore detected before typed decoding, while old readers can safely skip
//! unknown non-critical debug sections.

use crate::*;
use std::collections::{BTreeMap, BTreeSet};

const BUNDLE_MAJOR: u16 = 1;
const BUNDLE_MINOR: u16 = 0;
const SUPPORTED_FEATURES: u64 = 0;
const DIALECT: &str = "xmlsquish.psdbg";

/// 将完整 debug bundle 编码为确定性、分节的 `.psdbg` 字节。 / Encodes a complete debug bundle into deterministic sectioned `.psdbg` bytes.
pub fn encode_debug_bundle(bundle: &DebugBundle) -> Result<Vec<u8>, PersistError> {
    validate_bundle(bundle)?;
    let sections = vec![
        Section {
            tag: SECTION_DESCRIPTOR,
            flags: SectionFlags::SEMANTIC,
            payload: descriptor(bundle).encode(),
        },
        section(SECTION_BUNDLE_METADATA, encode_metadata(bundle)),
        section(SECTION_DOCUMENT, encode_linked_document(&bundle.document)),
        section(
            SECTION_BUNDLE_TRACE,
            encode_expansion_trace(&bundle.expansion_trace),
        ),
        section(
            SECTION_BUNDLE_LINK_TRACE,
            encode_link_trace(&bundle.link_trace),
        ),
        section(
            SECTION_BUNDLE_ARTIFACT_MAP,
            encode_map(&bundle.artifact_map),
        ),
        section(SECTION_BUNDLE_SOURCES, encode_sources(bundle)),
    ];
    Ok(encode_container(&Container::v1(
        ContainerKind::DebugBundle,
        sections,
    )?))
}

/// 解码 `.psdbg`，验证版本、feature、身份以及所有跨 section 引用。 / Decodes `.psdbg`, validating versions, features, identities, and every cross-section reference.
pub fn decode_debug_bundle(bytes: &[u8]) -> Result<DebugBundle, PersistError> {
    let container = decode_container(bytes)?;
    if container.kind() != ContainerKind::DebugBundle {
        return Err(PersistError::KindMismatch);
    }
    let descriptor = SemanticDescriptor::decode(required(&container, SECTION_DESCRIPTOR)?)?;
    if descriptor.dialect != DIALECT {
        return Err(PersistError::KindMismatch);
    }
    if descriptor.feature_bits & !SUPPORTED_FEATURES != 0 {
        return Err(PersistError::UnsupportedFeatures(descriptor.feature_bits));
    }
    validate_sections(&container)?;
    let metadata = decode_metadata(required(&container, SECTION_BUNDLE_METADATA)?)?;
    if metadata.schema.major != BUNDLE_MAJOR || metadata.schema.minor > BUNDLE_MINOR {
        return Err(PersistError::UnsupportedBundleVersion(metadata.schema));
    }
    if descriptor.semantic_epoch != u64::from(metadata.schema.major)
        || metadata.feature_bits.0 != descriptor.feature_bits
    {
        return Err(PersistError::SemanticMismatch);
    }
    let (source_archives, source_blobs) =
        decode_sources(required(&container, SECTION_BUNDLE_SOURCES)?)?;
    let bundle = DebugBundle {
        schema: metadata.schema,
        feature_bits: metadata.feature_bits,
        artifact: metadata.artifact,
        document_digest: metadata.document_digest,
        linked_image: metadata.linked_image,
        expansion_trace_digest: metadata.expansion_trace_digest,
        link_trace_digest: metadata.link_trace_digest,
        document: decode_linked_document(required(&container, SECTION_DOCUMENT)?)?,
        expansion_trace: decode_expansion_trace(required(&container, SECTION_BUNDLE_TRACE)?)?,
        link_trace: decode_link_trace(required(&container, SECTION_BUNDLE_LINK_TRACE)?)?,
        artifact_map: decode_map(required(&container, SECTION_BUNDLE_ARTIFACT_MAP)?)?,
        source_archives,
        source_blobs,
    };
    validate_bundle(&bundle)?;
    Ok(bundle)
}

fn descriptor(bundle: &DebugBundle) -> SemanticDescriptor {
    SemanticDescriptor {
        dialect: DIALECT.into(),
        semantic_epoch: u64::from(bundle.schema.major),
        feature_bits: bundle.feature_bits.0,
    }
}

fn section(tag: u32, payload: Vec<u8>) -> Section {
    Section {
        tag,
        flags: SectionFlags::DEBUG,
        payload,
    }
}

fn required(container: &Container, tag: u32) -> Result<&[u8], PersistError> {
    container
        .sections()
        .iter()
        .find(|s| s.tag == tag)
        .map(|s| s.payload.as_slice())
        .ok_or(PersistError::MissingSection(tag))
}

fn validate_sections(container: &Container) -> Result<(), PersistError> {
    let required_tags = [
        SECTION_DESCRIPTOR,
        SECTION_DOCUMENT,
        SECTION_BUNDLE_METADATA,
        SECTION_BUNDLE_TRACE,
        SECTION_BUNDLE_LINK_TRACE,
        SECTION_BUNDLE_ARTIFACT_MAP,
        SECTION_BUNDLE_SOURCES,
    ];
    for tag in required_tags {
        required(container, tag)?;
    }
    for section in container.sections() {
        let expected = if section.tag == SECTION_DESCRIPTOR {
            SectionFlags::SEMANTIC
        } else {
            SectionFlags::DEBUG
        };
        if required_tags.contains(&section.tag) && section.flags != expected {
            return Err(PersistError::InvalidSections);
        }
        if !required_tags.contains(&section.tag) && section.flags != SectionFlags::DEBUG {
            return Err(PersistError::InvalidSections);
        }
        if !required_tags.contains(&section.tag)
            && matches!(
                section.tag,
                SECTION_UNIT | SECTION_LINKED_IMAGE | SECTION_DEBUG
            )
        {
            return Err(PersistError::InvalidSections);
        }
    }
    Ok(())
}

fn validate_bundle(bundle: &DebugBundle) -> Result<(), PersistError> {
    if bundle.schema.major != BUNDLE_MAJOR || bundle.schema.minor > BUNDLE_MINOR {
        return Err(PersistError::UnsupportedBundleVersion(bundle.schema));
    }
    if bundle.feature_bits.0 & !SUPPORTED_FEATURES != 0 {
        return Err(PersistError::UnsupportedFeatures(bundle.feature_bits.0));
    }
    bundle.document.validate()?;
    bundle
        .expansion_trace
        .validate_against_document(&bundle.document)?;
    bundle
        .artifact_map
        .validate_against(&bundle.expansion_trace, bundle.artifact.byte_len)?;
    for digest in [
        bundle.artifact.digest.0,
        bundle.document_digest.0,
        bundle.linked_image.0,
        bundle.expansion_trace_digest.0,
        bundle.link_trace_digest.0,
    ] {
        if digest.algorithm != DigestAlgorithm::Sha256 {
            return Err(PersistError::Decode(DecodeError::UnsupportedDigest));
        }
    }
    if DocumentDigest::of(&encode_linked_document(&bundle.document)) != bundle.document_digest {
        return Err(PersistError::IdentityMismatch("document"));
    }
    if DebugDigest::of(&encode_expansion_trace(&bundle.expansion_trace))
        != bundle.expansion_trace_digest
    {
        return Err(PersistError::IdentityMismatch("expansion trace"));
    }
    if DebugDigest::of(&encode_link_trace(&bundle.link_trace)) != bundle.link_trace_digest {
        return Err(PersistError::IdentityMismatch("link trace"));
    }
    if !bundle
        .source_archives
        .windows(2)
        .all(|w| w[0].object < w[1].object)
    {
        return Err(PersistError::NonCanonicalBundle("source archives"));
    }
    if !bundle
        .source_blobs
        .windows(2)
        .all(|w| blob_key(&w[0]) < blob_key(&w[1]))
    {
        return Err(PersistError::NonCanonicalBundle("source blobs"));
    }
    let blobs: BTreeMap<_, _> = bundle
        .source_blobs
        .iter()
        .map(|b| ((b.reference.digest, b.reference.byte_len), b))
        .collect();
    if blobs.len() != bundle.source_blobs.len() {
        return Err(PersistError::NonCanonicalBundle("duplicate source blob"));
    }
    let mut objects = BTreeSet::new();
    let mut referenced_blobs = BTreeSet::new();
    for member in &bundle.source_archives {
        if !objects.insert(member.object) || member.archive.records.is_empty() {
            return Err(PersistError::NonCanonicalBundle("source archive"));
        }
        if member.object.0.algorithm != DigestAlgorithm::Sha256
            || member.debug_digest.0.algorithm != DigestAlgorithm::Sha256
            || !member
                .archive
                .records
                .windows(2)
                .all(|records| records[0].key < records[1].key)
        {
            return Err(PersistError::NonCanonicalBundle("source archive records"));
        }
        for record in &member.archive.records {
            if record.digest.0 != record.exact_bytes.digest
                || record.digest.0.algorithm != DigestAlgorithm::Sha256
                || !matches!(record.bom_len, 0 | 3)
                || record.exact_bytes.byte_len < u64::from(record.bom_len)
                || record.line_start_offsets.first() != Some(&0)
                || !record.line_start_offsets.windows(2).all(|x| x[0] < x[1])
                || record
                    .line_start_offsets
                    .last()
                    .is_some_and(|offset| *offset > record.exact_bytes.byte_len)
            {
                return Err(PersistError::NonCanonicalBundle("source record"));
            }
            let blob = blobs
                .get(&(record.exact_bytes.digest, record.exact_bytes.byte_len))
                .ok_or(PersistError::MissingSourceBlob)?;
            referenced_blobs.insert((record.exact_bytes.digest, record.exact_bytes.byte_len));
            if blob.bytes.len() as u64 != record.exact_bytes.byte_len
                || SourceDigest::of(&blob.bytes) != record.digest
            {
                return Err(PersistError::IdentityMismatch("source blob"));
            }
        }
    }
    if referenced_blobs.len() != blobs.len() {
        return Err(PersistError::NonCanonicalBundle("unreferenced source blob"));
    }
    validate_provenance(bundle)?;
    Ok(())
}

fn validate_provenance(bundle: &DebugBundle) -> Result<(), PersistError> {
    let archives: BTreeMap<_, _> = bundle
        .source_archives
        .iter()
        .map(|archive| (archive.object, archive))
        .collect();
    let origin = |reference: &QualifiedOriginRef| {
        archives
            .get(&reference.object)
            .and_then(|archive| archive.origins.entries.get(reference.local.0 as usize))
            .ok_or(PersistError::DanglingBundleReference("qualified origin"))
    };
    let decoded = |reference: &QualifiedDecodedValueMapRef| {
        archives
            .get(&reference.object)
            .and_then(|archive| {
                archive
                    .origins
                    .decoded_values
                    .get(reference.local.0 as usize)
            })
            .ok_or(PersistError::DanglingBundleReference(
                "qualified decoded map",
            ))
    };

    for archive in archives.values() {
        for entry in &archive.origins.entries {
            let record = archive
                .archive
                .records
                .get(entry.origin.source.0 as usize)
                .ok_or(PersistError::DanglingBundleReference("origin source"))?;
            if entry
                .origin
                .lexical_qname
                .is_some_and(|id| id.0 as usize >= archive.origins.debug_strings.len())
                || entry.origin.span.start >= entry.origin.span.end
                || entry.origin.span.end > record.exact_bytes.byte_len - u64::from(record.bom_len)
            {
                return Err(PersistError::DanglingBundleReference("origin table"));
            }
        }
        for map in &archive.origins.decoded_values {
            let owner = archive.origins.entries.iter().find(|entry| {
                entry.entity_kind == map.owner.entity_kind && entry.local_id == map.owner.local_id
            });
            let owner = owner.ok_or(PersistError::DanglingBundleReference("decoded owner"))?;
            let record = &archive.archive.records[owner.origin.source.0 as usize];
            let mut value_end = 0;
            for segment in &map.segments {
                if segment.value_utf8_range.start != value_end
                    || segment.value_utf8_range.start > segment.value_utf8_range.end
                    || segment.source_span.start > segment.source_span.end
                    || segment.source_span.end
                        > record.exact_bytes.byte_len - u64::from(record.bom_len)
                {
                    return Err(PersistError::DanglingBundleReference("decoded segment"));
                }
                value_end = segment.value_utf8_range.end;
            }
        }
    }

    let check_origin = |reference: &QualifiedOriginRef| origin(reference).map(|_| ());
    for frame in &bundle.expansion_trace.frames {
        check_origin(&frame.definition_origin)?;
        if let Some(reference) = &frame.call_origin {
            check_origin(reference)?;
        }
    }
    for trace in &bundle.expansion_trace.document_items {
        check_origin(&trace.definition_origin)?;
        if let Some(reference) = &trace.call_origin {
            check_origin(reference)?;
        }
        for step in &trace.substitution_chain {
            check_origin(&step.origin)?;
        }
    }
    for node in &bundle.expansion_trace.origins {
        match node {
            OriginNode::SourceSpan { origin: reference } => check_origin(reference)?,
            OriginNode::DecodedSegment { map, segment_index } => {
                if *segment_index as usize >= decoded(map)?.segments.len() {
                    return Err(PersistError::DanglingBundleReference("decoded segment"));
                }
            }
            OriginNode::Import { edge, .. }
                if edge.0 as usize >= bundle.link_trace.imports.len() =>
            {
                return Err(PersistError::DanglingBundleReference("link import edge"));
            }
            _ => {}
        }
    }

    if !archives.contains_key(&bundle.link_trace.entry_object) {
        return Err(PersistError::DanglingBundleReference("link entry object"));
    }
    for import in &bundle.link_trace.imports {
        check_origin(&import.origin)?;
        let resolved = archives
            .get(&import.resolved_object)
            .ok_or(PersistError::DanglingBundleReference("resolved object"))?;
        let importer_exists = archives.values().any(|archive| {
            archive
                .archive
                .records
                .iter()
                .any(|record| record.key == import.importer)
        });
        if !importer_exists
            || !resolved
                .archive
                .records
                .iter()
                .any(|record| record.key == import.resolved_source)
        {
            return Err(PersistError::DanglingBundleReference("link import source"));
        }
    }
    for symbol in &bundle.link_trace.symbols {
        check_origin(&symbol.reference_origin)?;
        check_origin(&symbol.definition_origin)?;
    }
    for diagnostic in &bundle.link_trace.diagnostics {
        check_origin(&diagnostic.primary_origin)?;
        for (_, reference) in &diagnostic.related_origins {
            check_origin(reference)?;
        }
        if diagnostic
            .frame_chain
            .iter()
            .any(|frame| frame.0 as usize >= bundle.expansion_trace.frames.len())
        {
            return Err(PersistError::DanglingBundleReference("diagnostic frame"));
        }
    }

    for entry in &bundle.artifact_map.entries {
        if !origin_reaches_source(entry.origin, &bundle.expansion_trace, &archives) {
            return Err(PersistError::UntraceableArtifactOrigin);
        }
    }
    Ok(())
}

fn origin_reaches_source(
    id: OriginNodeId,
    trace: &ExpansionTrace,
    archives: &BTreeMap<ObjectDigest, &SourceArchiveReference>,
) -> bool {
    match &trace.origins[id.0 as usize] {
        OriginNode::SourceSpan { origin } => archives.get(&origin.object).is_some_and(|archive| {
            archive
                .origins
                .entries
                .get(origin.local.0 as usize)
                .is_some()
        }),
        OriginNode::DecodedSegment { map, segment_index } => archives
            .get(&map.object)
            .and_then(|archive| archive.origins.decoded_values.get(map.local.0 as usize))
            .is_some_and(|map| map.segments.get(*segment_index as usize).is_some()),
        OriginNode::Import { child, .. } => origin_reaches_source(*child, trace, archives),
        OriginNode::RegexCapture { input, .. } => origin_reaches_source(*input, trace, archives),
        OriginNode::Concat { ordered_inputs } => ordered_inputs
            .iter()
            .any(|input| origin_reaches_source(*input, trace, archives)),
        OriginNode::BackendTransform { inputs, .. } | OriginNode::Fused { inputs, .. } => inputs
            .iter()
            .any(|input| origin_reaches_source(input.parent, trace, archives)),
        OriginNode::Synthetic {
            nearest: Some(input),
            ..
        } => origin_reaches_source(*input, trace, archives),
        _ => false,
    }
}

fn blob_key(blob: &BundledSourceBlob) -> (Digest, u64) {
    (blob.reference.digest, blob.reference.byte_len)
}

struct Metadata {
    schema: Version,
    feature_bits: FeatureBits,
    artifact: ArtifactIdentity,
    document_digest: DocumentDigest,
    linked_image: LinkedImageDigest,
    expansion_trace_digest: DebugDigest,
    link_trace_digest: DebugDigest,
}

fn encode_metadata(bundle: &DebugBundle) -> Vec<u8> {
    let mut w = Writer::default();
    w.u16(bundle.schema.major);
    w.u16(bundle.schema.minor);
    w.u64(bundle.feature_bits.0);
    w.digest(bundle.artifact.digest.0);
    w.u64(bundle.artifact.byte_len);
    w.digest(bundle.document_digest.0);
    w.digest(bundle.linked_image.0);
    w.digest(bundle.expansion_trace_digest.0);
    w.digest(bundle.link_trace_digest.0);
    w.bytes
}

fn decode_metadata(bytes: &[u8]) -> Result<Metadata, DecodeError> {
    let mut r = Reader::new(bytes);
    let value = Metadata {
        schema: Version {
            major: r.u16()?,
            minor: r.u16()?,
        },
        feature_bits: FeatureBits(r.u64()?),
        artifact: ArtifactIdentity {
            digest: ArtifactDigest(r.digest()?),
            byte_len: r.u64()?,
        },
        document_digest: DocumentDigest(r.digest()?),
        linked_image: LinkedImageDigest(r.digest()?),
        expansion_trace_digest: DebugDigest(r.digest()?),
        link_trace_digest: DebugDigest(r.digest()?),
    };
    r.finish()?;
    Ok(value)
}

fn encode_map(map: &ArtifactByteMap) -> Vec<u8> {
    let mut w = Writer::default();
    w.len(map.entries.len());
    for entry in &map.entries {
        w.u64(entry.output_range.start);
        w.u64(entry.output_range.end);
        w.var(u64::from(entry.origin.0));
    }
    w.bytes
}
fn decode_map(bytes: &[u8]) -> Result<ArtifactByteMap, DecodeError> {
    let mut r = Reader::new(bytes);
    let count = r.count(17)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ArtifactMapEntry {
            output_range: Span {
                start: r.u64()?,
                end: r.u64()?,
            },
            origin: OriginNodeId(r.id()?),
        });
    }
    r.finish()?;
    Ok(ArtifactByteMap { entries })
}

fn encode_sources(bundle: &DebugBundle) -> Vec<u8> {
    let mut w = Writer::default();
    w.len(bundle.source_archives.len());
    for a in &bundle.source_archives {
        w.digest(a.object.0);
        w.digest(a.debug_digest.0);
        w.len(a.archive.records.len());
        for r in &a.archive.records {
            w.source(&r.key);
            w.digest(r.digest.0);
            w.byte(r.bom_len);
            w.digest(r.exact_bytes.digest);
            w.u64(r.exact_bytes.byte_len);
            w.len(r.line_start_offsets.len());
            for x in &r.line_start_offsets {
                w.u64(*x);
            }
        }
        w.origin_table(&a.origins);
    }
    w.len(bundle.source_blobs.len());
    for b in &bundle.source_blobs {
        w.digest(b.reference.digest);
        w.u64(b.reference.byte_len);
        w.raw(&b.bytes);
    }
    w.bytes
}

fn decode_sources(
    bytes: &[u8],
) -> Result<(Vec<SourceArchiveReference>, Vec<BundledSourceBlob>), DecodeError> {
    let mut r = Reader::new(bytes);
    let ac = r.count(69)?;
    let mut archives = Vec::with_capacity(ac);
    for _ in 0..ac {
        let object = ObjectDigest(r.digest()?);
        let debug_digest = DebugDigest(r.digest()?);
        let rc = r.count(80)?;
        let mut records = Vec::with_capacity(rc);
        for _ in 0..rc {
            let key = r.source()?;
            let digest = SourceDigest(r.digest()?);
            let bom_len = r.byte()?;
            let exact_bytes = BlobRef {
                digest: r.digest()?,
                byte_len: r.u64()?,
            };
            let lc = r.count(8)?;
            let mut lines = Vec::with_capacity(lc);
            for _ in 0..lc {
                lines.push(r.u64()?);
            }
            records.push(SourceRecord {
                key,
                digest,
                bom_len,
                exact_bytes,
                line_start_offsets: lines,
            });
        }
        archives.push(SourceArchiveReference {
            object,
            debug_digest,
            archive: SourceArchive { records },
            origins: r.origin_table()?,
        });
    }
    let bc = r.count(43)?;
    let mut blobs = Vec::with_capacity(bc);
    for _ in 0..bc {
        let digest = r.digest()?;
        let byte_len = r.u64()?;
        let data = r.raw()?;
        blobs.push(BundledSourceBlob {
            reference: BlobRef { digest, byte_len },
            bytes: data,
        });
    }
    r.finish()?;
    Ok((archives, blobs))
}

fn encode_link_trace(trace: &LinkTrace) -> Vec<u8> {
    let mut w = Writer::default();
    w.digest(trace.entry_object.0);
    w.digest(trace.resolution_snapshot);
    w.len(trace.imports.len());
    for import in &trace.imports {
        w.source(&import.importer);
        w.var(u64::from(import.import_id.0));
        match &import.spec {
            ImportSpec::RelativeUri(value) => {
                w.byte(1);
                w.string(value);
            }
            ImportSpec::AbsoluteFileUri(value) => {
                w.byte(2);
                w.string(value);
            }
            ImportSpec::PackageExport {
                dependency_alias,
                export,
            } => {
                w.byte(3);
                w.string(dependency_alias);
                w.string(export);
            }
        }
        w.source(&import.resolved_source);
        w.digest(import.resolved_object.0);
        w.qualified_origin(&import.origin);
    }
    w.len(trace.symbols.len());
    for symbol in &trace.symbols {
        w.qualified_origin(&symbol.reference_origin);
        w.string(&symbol.symbol.namespace_uri);
        w.string(&symbol.symbol.local_name);
        w.var(u64::from(symbol.definition.unit_slot));
        w.var(u64::from(symbol.definition.local_def.0));
        w.qualified_origin(&symbol.definition_origin);
    }
    w.len(trace.diagnostics.len());
    for diagnostic in &trace.diagnostics {
        w.string(&diagnostic.code);
        w.byte(match diagnostic.severity {
            DiagnosticSeverity::Error => 1,
            DiagnosticSeverity::Warning => 2,
            DiagnosticSeverity::Note => 3,
        });
        w.len(diagnostic.message_args.len());
        for (name, value) in &diagnostic.message_args {
            w.var(u64::from(name.0));
            match value {
                DiagnosticValue::String(value) => {
                    w.byte(1);
                    w.string(value);
                }
                DiagnosticValue::Unsigned(value) => {
                    w.byte(2);
                    w.u64(*value);
                }
                DiagnosticValue::Symbol(value) => {
                    w.byte(3);
                    w.string(&value.namespace_uri);
                    w.string(&value.local_name);
                }
                DiagnosticValue::Source(value) => {
                    w.byte(4);
                    w.source(value);
                }
            }
        }
        w.qualified_origin(&diagnostic.primary_origin);
        w.len(diagnostic.related_origins.len());
        for (label, origin) in &diagnostic.related_origins {
            w.var(u64::from(label.0));
            w.qualified_origin(origin);
        }
        w.len(diagnostic.frame_chain.len());
        for frame in &diagnostic.frame_chain {
            w.var(u64::from(frame.0));
        }
    }
    w.bytes
}

fn decode_link_trace(bytes: &[u8]) -> Result<LinkTrace, DecodeError> {
    let mut r = Reader::new(bytes);
    let entry_object = ObjectDigest(r.digest()?);
    let resolution_snapshot = r.digest()?;
    let count = r.count(74)?;
    let mut imports = Vec::with_capacity(count);
    for _ in 0..count {
        let importer = r.source()?;
        let import_id = ImportId(r.id()?);
        let spec = match r.byte()? {
            1 => ImportSpec::RelativeUri(r.string()?),
            2 => ImportSpec::AbsoluteFileUri(r.string()?),
            3 => ImportSpec::PackageExport {
                dependency_alias: r.string()?,
                export: r.string()?,
            },
            _ => return Err(DecodeError::NonCanonical("import spec")),
        };
        imports.push(LinkImportRecord {
            importer,
            import_id,
            spec,
            resolved_source: r.source()?,
            resolved_object: ObjectDigest(r.digest()?),
            origin: r.qualified_origin()?,
        });
    }
    let count = r.count(74)?;
    let mut symbols = Vec::with_capacity(count);
    for _ in 0..count {
        symbols.push(LinkSymbolRecord {
            reference_origin: r.qualified_origin()?,
            symbol: ExpandedName {
                namespace_uri: r.string()?,
                local_name: r.string()?,
            },
            definition: DefAddr {
                unit_slot: r.id()?,
                local_def: LocalDefId(r.id()?),
            },
            definition_origin: r.qualified_origin()?,
        });
    }
    let count = r.count(40)?;
    let mut diagnostics = Vec::with_capacity(count);
    for _ in 0..count {
        let code = r.string()?;
        let severity = match r.byte()? {
            1 => DiagnosticSeverity::Error,
            2 => DiagnosticSeverity::Warning,
            3 => DiagnosticSeverity::Note,
            _ => return Err(DecodeError::NonCanonical("diagnostic severity")),
        };
        let arg_count = r.count(3)?;
        let mut message_args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            let name = StringId(r.id()?);
            let value = match r.byte()? {
                1 => DiagnosticValue::String(r.string()?),
                2 => DiagnosticValue::Unsigned(r.u64()?),
                3 => DiagnosticValue::Symbol(ExpandedName {
                    namespace_uri: r.string()?,
                    local_name: r.string()?,
                }),
                4 => DiagnosticValue::Source(r.source()?),
                _ => return Err(DecodeError::NonCanonical("diagnostic value")),
            };
            message_args.push((name, value));
        }
        let primary_origin = r.qualified_origin()?;
        let related_count = r.count(35)?;
        let mut related_origins = Vec::with_capacity(related_count);
        for _ in 0..related_count {
            related_origins.push((DebugStringId(r.id()?), r.qualified_origin()?));
        }
        let frame_count = r.count(1)?;
        let mut frame_chain = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            frame_chain.push(FrameId(r.id()?));
        }
        diagnostics.push(DiagnosticRecord {
            code,
            severity,
            message_args,
            primary_origin,
            related_origins,
            frame_chain,
        });
    }
    r.finish()?;
    Ok(LinkTrace {
        entry_object,
        resolution_snapshot,
        imports,
        symbols,
        diagnostics,
    })
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn byte(&mut self, x: u8) {
        self.bytes.push(x)
    }
    fn u16(&mut self, x: u16) {
        self.bytes.extend(x.to_le_bytes())
    }
    fn u64(&mut self, x: u64) {
        self.bytes.extend(x.to_le_bytes())
    }
    fn var(&mut self, mut x: u64) {
        loop {
            let mut b = (x & 127) as u8;
            x >>= 7;
            if x != 0 {
                b |= 128
            }
            self.byte(b);
            if x == 0 {
                break;
            }
        }
    }
    fn len(&mut self, x: usize) {
        self.var(x as u64)
    }
    fn string(&mut self, x: &str) {
        self.len(x.len());
        self.bytes.extend(x.as_bytes())
    }
    fn raw(&mut self, x: &[u8]) {
        self.len(x.len());
        self.bytes.extend(x)
    }
    fn digest(&mut self, x: Digest) {
        self.u16(x.algorithm as u16);
        self.bytes.extend(x.bytes)
    }
    fn source(&mut self, x: &SourceKey) {
        match x {
            SourceKey::Project { package, path } => {
                self.byte(1);
                self.u16(package.source_kind);
                self.string(&package.canonical_source);
                self.string(&package.package_name);
                self.string(&package.exact_revision);
                self.len(path.len());
                for p in path {
                    self.string(p)
                }
            }
            SourceKey::AdHoc { uri } => {
                self.byte(2);
                self.string(uri)
            }
        }
    }
    fn entity(&mut self, value: EntityKind) {
        self.byte(match value {
            EntityKind::Import => 1,
            EntityKind::Definition => 2,
            EntityKind::Region => 3,
            EntityKind::Operation => 4,
            EntityKind::Parameter => 5,
            EntityKind::Slot => 6,
            EntityKind::ExternalSymbol => 7,
        });
    }
    fn span(&mut self, value: Span) {
        self.u64(value.start);
        self.u64(value.end);
    }
    fn qualified_origin(&mut self, value: &QualifiedOriginRef) {
        self.digest(value.object.0);
        self.var(u64::from(value.local.0));
    }
    fn origin_table(&mut self, table: &OriginTable) {
        self.len(table.entries.len());
        for entry in &table.entries {
            self.entity(entry.entity_kind);
            self.var(u64::from(entry.local_id));
            self.var(u64::from(entry.origin.source.0));
            self.span(entry.origin.span);
            match entry.origin.lexical_qname {
                None => self.byte(0),
                Some(value) => {
                    self.byte(1);
                    self.var(u64::from(value.0));
                }
            }
            self.u16(entry.origin.syntax_kind.0);
        }
        self.len(table.debug_strings.len());
        for value in &table.debug_strings {
            self.string(value);
        }
        self.len(table.decoded_values.len());
        for map in &table.decoded_values {
            self.entity(map.owner.entity_kind);
            self.var(u64::from(map.owner.local_id));
            self.string(&map.owner.field);
            self.len(map.segments.len());
            for segment in &map.segments {
                self.span(segment.value_utf8_range);
                self.span(segment.source_span);
                self.byte(match segment.syntax {
                    DecodedSyntax::LiteralText => 1,
                    DecodedSyntax::CharacterReference => 2,
                    DecodedSyntax::EntityReference => 3,
                    DecodedSyntax::CData => 4,
                });
            }
        }
    }
}
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn byte(&mut self) -> Result<u8, DecodeError> {
        let x = *self.b.get(self.p).ok_or(DecodeError::Truncated)?;
        self.p += 1;
        Ok(x)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let e = self.p.checked_add(n).ok_or(DecodeError::LengthOverflow)?;
        let x = self.b.get(self.p..e).ok_or(DecodeError::Truncated)?;
        self.p = e;
        Ok(x)
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn var(&mut self) -> Result<u64, DecodeError> {
        let start = self.p;
        let mut x = 0;
        for shift in (0..=63).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                return Err(DecodeError::LengthOverflow);
            }
            x |= u64::from(b & 127) << shift;
            if b & 128 == 0 {
                if self.p - start > 1 && b == 0 {
                    return Err(DecodeError::NonCanonical("LEB128"));
                }
                return Ok(x);
            }
        }
        Err(DecodeError::LengthOverflow)
    }
    fn len(&mut self) -> Result<usize, DecodeError> {
        usize::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)
    }
    /// 读取集合长度，并在分配前按每项最小 wire 大小约束它。 / Reads a collection
    /// length and bounds it by the minimum wire size per item before allocation.
    fn count(&mut self, minimum_item_bytes: usize) -> Result<usize, DecodeError> {
        let count = self.len()?;
        if count > self.b.len().saturating_sub(self.p) / minimum_item_bytes {
            return Err(DecodeError::Truncated);
        }
        Ok(count)
    }
    fn id(&mut self) -> Result<u32, DecodeError> {
        u32::try_from(self.var()?).map_err(|_| DecodeError::LengthOverflow)
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        let n = self.len()?;
        std::str::from_utf8(self.take(n)?)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8)
    }
    fn raw(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.len()?;
        Ok(self.take(n)?.to_vec())
    }
    fn digest(&mut self) -> Result<Digest, DecodeError> {
        let algorithm = match self.u16()? {
            1 => DigestAlgorithm::Sha256,
            2 => DigestAlgorithm::Blake3,
            _ => return Err(DecodeError::UnsupportedDigest),
        };
        Ok(Digest {
            algorithm,
            bytes: self.take(32)?.try_into().unwrap(),
        })
    }
    fn source(&mut self) -> Result<SourceKey, DecodeError> {
        match self.byte()? {
            1 => {
                let package = PackageInstanceId {
                    source_kind: self.u16()?,
                    canonical_source: self.string()?,
                    package_name: self.string()?,
                    exact_revision: self.string()?,
                };
                let n = self.count(1)?;
                let mut path = Vec::with_capacity(n);
                for _ in 0..n {
                    path.push(self.string()?)
                }
                Ok(SourceKey::Project { package, path })
            }
            2 => Ok(SourceKey::AdHoc {
                uri: self.string()?,
            }),
            _ => Err(DecodeError::NonCanonical("source key")),
        }
    }
    fn entity(&mut self) -> Result<EntityKind, DecodeError> {
        Ok(match self.byte()? {
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
    fn span(&mut self) -> Result<Span, DecodeError> {
        Ok(Span {
            start: self.u64()?,
            end: self.u64()?,
        })
    }
    fn qualified_origin(&mut self) -> Result<QualifiedOriginRef, DecodeError> {
        Ok(QualifiedOriginRef {
            object: ObjectDigest(self.digest()?),
            local: OriginId(self.id()?),
        })
    }
    fn origin_table(&mut self) -> Result<OriginTable, DecodeError> {
        let count = self.count(22)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let entity_kind = self.entity()?;
            let local_id = self.id()?;
            let source = SourceRef(self.id()?);
            let span = self.span()?;
            let lexical_qname = match self.byte()? {
                0 => None,
                1 => Some(DebugStringId(self.id()?)),
                _ => return Err(DecodeError::NonCanonical("option")),
            };
            let syntax_kind = SyntaxKind(self.u16()?);
            entries.push(OriginEntry {
                entity_kind,
                local_id,
                origin: Origin {
                    source,
                    span,
                    lexical_qname,
                    syntax_kind,
                },
            });
        }
        let count = self.count(1)?;
        let mut debug_strings = Vec::with_capacity(count);
        for _ in 0..count {
            debug_strings.push(self.string()?);
        }
        let count = self.count(4)?;
        let mut decoded_values = Vec::with_capacity(count);
        for _ in 0..count {
            let owner = DecodedOwner {
                entity_kind: self.entity()?,
                local_id: self.id()?,
                field: self.string()?,
            };
            let segment_count = self.count(33)?;
            let mut segments = Vec::with_capacity(segment_count);
            for _ in 0..segment_count {
                let value_utf8_range = self.span()?;
                let source_span = self.span()?;
                let syntax = match self.byte()? {
                    1 => DecodedSyntax::LiteralText,
                    2 => DecodedSyntax::CharacterReference,
                    3 => DecodedSyntax::EntityReference,
                    4 => DecodedSyntax::CData,
                    _ => return Err(DecodeError::NonCanonical("decoded syntax")),
                };
                segments.push(DecodedSegment {
                    value_utf8_range,
                    source_span,
                    syntax,
                });
            }
            decoded_values.push(DecodedValueMap { owner, segments });
        }
        Ok(OriginTable {
            entries,
            debug_strings,
            decoded_values,
        })
    }
    fn finish(self) -> Result<(), DecodeError> {
        if self.p == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> DebugBundle {
        let bytes = b"x".to_vec();
        let source_digest = SourceDigest::of(&bytes);
        let blob = BlobRef {
            digest: source_digest.0,
            byte_len: 1,
        };
        let document = LinkedDocumentIr {
            schema: Version { major: 1, minor: 0 },
            document_abi: AbiId("doc-v1".into()),
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
        let object = ObjectDigest::of(b"object");
        let expansion_trace = ExpansionTrace::default();
        let link_trace = LinkTrace {
            entry_object: object,
            resolution_snapshot: Digest::sha256("resolution", b"fixture"),
            imports: Vec::new(),
            symbols: Vec::new(),
            diagnostics: Vec::new(),
        };
        DebugBundle {
            schema: Version { major: 1, minor: 0 },
            feature_bits: FeatureBits(0),
            artifact: ArtifactIdentity {
                digest: ArtifactDigest::of(&[]),
                byte_len: 0,
            },
            document_digest: DocumentDigest::of(&encode_linked_document(&document)),
            linked_image: LinkedImageDigest::of(b"image"),
            expansion_trace_digest: DebugDigest::of(&encode_expansion_trace(&expansion_trace)),
            link_trace_digest: DebugDigest::of(&encode_link_trace(&link_trace)),
            document,
            expansion_trace,
            link_trace,
            artifact_map: ArtifactByteMap::default(),
            source_archives: vec![SourceArchiveReference {
                object,
                debug_digest: DebugDigest::of(b"source"),
                archive: SourceArchive {
                    records: vec![SourceRecord {
                        key: SourceKey::AdHoc {
                            uri: "file:///x".into(),
                        },
                        digest: source_digest,
                        bom_len: 0,
                        exact_bytes: blob.clone(),
                        line_start_offsets: vec![0],
                    }],
                },
                origins: OriginTable::default(),
            }],
            source_blobs: vec![BundledSourceBlob {
                reference: blob,
                bytes,
            }],
        }
    }

    fn provenance_fixture() -> DebugBundle {
        let mut bundle = fixture();
        let object = bundle.source_archives[0].object;
        bundle.source_archives[0].origins.entries.push(OriginEntry {
            entity_kind: EntityKind::Operation,
            local_id: 0,
            origin: Origin {
                source: SourceRef(0),
                span: Span { start: 0, end: 1 },
                lexical_qname: None,
                syntax_kind: SyntaxKind(1),
            },
        });
        bundle.expansion_trace.origins.push(OriginNode::SourceSpan {
            origin: QualifiedOriginRef {
                object,
                local: OriginId(0),
            },
        });
        bundle.expansion_trace_digest =
            DebugDigest::of(&encode_expansion_trace(&bundle.expansion_trace));
        bundle.link_trace.symbols.push(LinkSymbolRecord {
            reference_origin: QualifiedOriginRef {
                object,
                local: OriginId(0),
            },
            symbol: ExpandedName {
                namespace_uri: "urn:test".into(),
                local_name: "macro".into(),
            },
            definition: DefAddr {
                unit_slot: 0,
                local_def: LocalDefId(0),
            },
            definition_origin: QualifiedOriginRef {
                object,
                local: OriginId(0),
            },
        });
        bundle.link_trace_digest = DebugDigest::of(&encode_link_trace(&bundle.link_trace));
        bundle.artifact.byte_len = 1;
        bundle.artifact_map.entries.push(ArtifactMapEntry {
            output_range: Span { start: 0, end: 1 },
            origin: OriginNodeId(0),
        });
        bundle
    }
    #[test]
    fn deterministic_roundtrip() {
        let value = fixture();
        let a = encode_debug_bundle(&value).unwrap();
        assert_eq!(a, encode_debug_bundle(&value).unwrap());
        assert_eq!(decode_debug_bundle(&a).unwrap(), value)
    }
    #[test]
    fn corruption_is_rejected() {
        let mut bytes = encode_debug_bundle(&fixture()).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        assert!(matches!(
            decode_debug_bundle(&bytes),
            Err(PersistError::Decode(DecodeError::Checksum(_)))
        ))
    }
    #[test]
    fn unknown_features_are_rejected() {
        let mut value = fixture();
        value.feature_bits = FeatureBits(1);
        assert_eq!(
            encode_debug_bundle(&value),
            Err(PersistError::UnsupportedFeatures(1))
        )
    }
    #[test]
    fn identity_mismatch_is_rejected() {
        let mut value = fixture();
        value.document_digest = DocumentDigest::of(b"bad");
        assert_eq!(
            encode_debug_bundle(&value),
            Err(PersistError::IdentityMismatch("document"))
        )
    }
    #[test]
    fn optional_debug_sections_are_ignored() {
        let value = fixture();
        let raw = encode_debug_bundle(&value).unwrap();
        let c = decode_container(&raw).unwrap();
        let mut sections = c.sections().to_vec();
        sections.push(Section {
            tag: 0x9000_1000,
            flags: SectionFlags::DEBUG,
            payload: b"future".to_vec(),
        });
        let bytes = encode_container(&Container::v1(ContainerKind::DebugBundle, sections).unwrap());
        assert_eq!(decode_debug_bundle(&bytes).unwrap(), value)
    }

    #[test]
    fn trace_and_source_cross_references_are_checked() {
        let mut wrong_trace = fixture();
        wrong_trace.expansion_trace_digest = DebugDigest::of(b"another trace");
        assert_eq!(
            encode_debug_bundle(&wrong_trace),
            Err(PersistError::IdentityMismatch("expansion trace"))
        );

        let mut wrong_link_trace = fixture();
        wrong_link_trace.link_trace_digest = DebugDigest::of(b"another link trace");
        assert_eq!(
            encode_debug_bundle(&wrong_link_trace),
            Err(PersistError::IdentityMismatch("link trace"))
        );

        let mut missing_blob = fixture();
        missing_blob.source_blobs.clear();
        assert_eq!(
            encode_debug_bundle(&missing_blob),
            Err(PersistError::MissingSourceBlob)
        );
    }

    #[test]
    fn qualified_origins_reject_foreign_objects_and_out_of_range_ids() {
        let mut foreign = provenance_fixture();
        let OriginNode::SourceSpan { origin } = &mut foreign.expansion_trace.origins[0] else {
            unreachable!()
        };
        origin.object = ObjectDigest::of(b"not bundled");
        foreign.expansion_trace_digest =
            DebugDigest::of(&encode_expansion_trace(&foreign.expansion_trace));
        assert_eq!(
            encode_debug_bundle(&foreign),
            Err(PersistError::DanglingBundleReference("qualified origin"))
        );

        let mut out_of_range = provenance_fixture();
        let OriginNode::SourceSpan { origin } = &mut out_of_range.expansion_trace.origins[0] else {
            unreachable!()
        };
        origin.local = OriginId(1);
        out_of_range.expansion_trace_digest =
            DebugDigest::of(&encode_expansion_trace(&out_of_range.expansion_trace));
        assert_eq!(
            encode_debug_bundle(&out_of_range),
            Err(PersistError::DanglingBundleReference("qualified origin"))
        );
    }

    #[test]
    fn artifact_map_must_reach_a_static_source_span() {
        let value = provenance_fixture();
        assert_eq!(
            decode_debug_bundle(&encode_debug_bundle(&value).unwrap()).unwrap(),
            value
        );

        let mut untraceable = provenance_fixture();
        untraceable.expansion_trace.origins[0] = OriginNode::Unknown {
            reason: DebugStringId(0),
        };
        untraceable
            .expansion_trace
            .debug_strings
            .push("unknown".into());
        untraceable.expansion_trace_digest =
            DebugDigest::of(&encode_expansion_trace(&untraceable.expansion_trace));
        assert_eq!(
            encode_debug_bundle(&untraceable),
            Err(PersistError::UntraceableArtifactOrigin)
        );
    }

    #[test]
    fn import_origin_nodes_reject_out_of_range_link_edges() {
        let mut value = provenance_fixture();
        value.expansion_trace.origins.push(OriginNode::Import {
            edge: LinkImportRef(0),
            child: OriginNodeId(0),
        });
        value.artifact_map.entries[0].origin = OriginNodeId(1);
        value.expansion_trace_digest =
            DebugDigest::of(&encode_expansion_trace(&value.expansion_trace));
        assert_eq!(
            encode_debug_bundle(&value),
            Err(PersistError::DanglingBundleReference("link import edge"))
        );
    }

    #[test]
    fn duplicate_and_foreign_sections_are_rejected() {
        let raw = encode_debug_bundle(&fixture()).unwrap();
        let container = decode_container(&raw).unwrap();
        let mut duplicate = container.sections().to_vec();
        duplicate.push(duplicate[0].clone());
        assert!(matches!(
            Container::v1(ContainerKind::DebugBundle, duplicate),
            Err(DecodeError::NonCanonical("section tag order"))
        ));

        let mut foreign = container.sections().to_vec();
        foreign.push(Section {
            tag: SECTION_UNIT,
            flags: SectionFlags::DEBUG,
            payload: Vec::new(),
        });
        let encoded = encode_container(
            &Container::v1(ContainerKind::DebugBundle, foreign).expect("valid outer container"),
        );
        assert_eq!(
            decode_debug_bundle(&encoded),
            Err(PersistError::InvalidSections)
        );
    }

    #[test]
    fn collection_lengths_are_bounded_before_allocation() {
        let raw = encode_debug_bundle(&fixture()).unwrap();
        let container = decode_container(&raw).unwrap();
        let mut sections = container.sections().to_vec();
        sections
            .iter_mut()
            .find(|section| section.tag == SECTION_BUNDLE_ARTIFACT_MAP)
            .unwrap()
            .payload = vec![100];
        let encoded = encode_container(
            &Container::v1(ContainerKind::DebugBundle, sections).expect("valid outer container"),
        );
        assert_eq!(
            decode_debug_bundle(&encoded),
            Err(PersistError::Decode(DecodeError::Truncated))
        );
    }

    #[test]
    fn inspect_projection_names_artifact_and_component_identities() {
        let json = fixture().inspect_json().0;
        assert!(json.starts_with("{\"format\":\"xsir-inspect-v1\",\"kind\":\"debug-bundle\""));
        assert!(json.contains("\"artifact\":{\"digest\":"));
        assert!(json.contains("\"linkedImageDigest\":"));
        assert!(json.contains("\"expansionTraceDigest\":"));
        assert!(json.contains("\"linkTraceDigest\":"));
        assert!(json.contains("\"linkTrace\":{\"imports\":"));
    }
}
