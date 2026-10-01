//! 有类型 IR 与规范容器的组合边界。 / Composition boundary for typed IR and canonical containers.

use crate::codec::{
    SectionRef, UNIT_CONTAINER_HEADER, decode_container_ref, finish_unit_container,
    section_digest_parts, write_unit_directory,
};
use crate::wire::{append_unit_projection, semantic_projection_matches, unit_projection_len};
use crate::*;
use core::fmt;
use std::sync::Arc;

/// 持久化边界错误。 / Persistent-boundary error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistError {
    Decode(DecodeError),
    Invalid(ValidationError),
    MissingSection(u32),
    KindMismatch,
    SemanticMismatch,
    InvalidSections,
    /// `.psdbg` typed payload schema 不受支持。 / Unsupported `.psdbg` typed-payload schema.
    UnsupportedBundleVersion(Version),
    /// Bundle 声明了此 reader 不理解的 feature。 / Bundle declares features unknown to this reader.
    UnsupportedFeatures(u64),
    /// 一个可重算的内容身份与 payload 不符。 / A recomputable content identity disagrees with its payload.
    IdentityMismatch(&'static str),
    /// Bundle 集合并非唯一且严格排序。 / A bundle collection is not unique and strictly sorted.
    NonCanonicalBundle(&'static str),
    /// Source record 所引用的 exact blob 缺失。 / An exact blob referenced by a source record is absent.
    MissingSourceBlob,
    /// Qualified object/local ID 或 link edge 悬空。 / A qualified object/local ID or link edge dangles.
    DanglingBundleReference(&'static str),
    /// 产品映射 origin 无法到达静态 source span。 / A mapped artifact origin cannot reach a static source span.
    UntraceableArtifactOrigin,
}
impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "persistent IR error: {self:?}")
    }
}
impl std::error::Error for PersistError {}
impl From<DecodeError> for PersistError {
    fn from(value: DecodeError) -> Self {
        Self::Decode(value)
    }
}
impl From<ValidationError> for PersistError {
    fn from(value: ValidationError) -> Self {
        Self::Invalid(value)
    }
}

/// Immutable borrowed proof that a complete unit passed structural validation.
/// The borrow prevents mutation between normalization passes; no unchecked constructor exists.
#[derive(Clone, Copy)]
pub struct VerifiedUnitEncoding<'a> {
    /// Borrowed arenas that cannot change while this structural proof exists.
    unit: &'a RelocatableUnitIr,
}
impl<'a> VerifiedUnitEncoding<'a> {
    /// Validates a unit once before one or more canonical metadata projections.
    pub fn new(unit: &'a RelocatableUnitIr) -> Result<Self, PersistError> {
        unit.validate()?;
        Ok(Self { unit })
    }
    /// Encodes one checked borrowed metadata projection without revalidating arenas.
    pub fn encode(&self, overrides: UnitEncodingOverrides<'_>) -> Result<Vec<u8>, PersistError> {
        encode_verified_unit(self.unit, overrides)
    }
    /// Streams the exact semantic identity of one checked metadata projection.
    pub fn semantic_digest(
        &self,
        overrides: UnitEncodingOverrides<'_>,
    ) -> Result<SemanticUnitDigest, PersistError> {
        digest_verified_unit(self.unit, overrides)
    }
    /// Streams the exact full container identity without serializing either payload.
    /// Section checksums and object hashing are separate bounded traversals of borrowed IR.
    pub fn object_digest(
        &self,
        overrides: UnitEncodingOverrides<'_>,
    ) -> Result<ObjectDigest, PersistError> {
        object_digest_verified_unit(self.unit, overrides)
    }
    /// Streams the exact source/debug partition of a checked canonical unit projection.
    pub fn debug_digest(
        &self,
        overrides: UnitEncodingOverrides<'_>,
    ) -> Result<DebugDigest, PersistError> {
        validate_overrides(self.unit, overrides)?;
        let length = unit_projection_len(self.unit, overrides, false);
        let mut hash = crate::digest::DomainHasher::new("debug");
        hash.update(&1u16.to_le_bytes());
        hash.update(&(container_kind(self.unit) as u16).to_le_bytes());
        hash.update(&SECTION_DEBUG.to_le_bytes());
        hash.update(&(length as u64).to_le_bytes());
        Ok(DebugDigest(
            crate::wire::update_unit_projection(hash, self.unit, overrides, false).finish(),
        ))
    }
}

/// An immutable validated unit with internally derived persistence identities.
/// No constructor accepts caller-asserted digests. Shared ownership prevents mutation while
/// the proof is retained; clones share arenas instead of copying them.
///
/// # Example
///
/// ```
/// use squish_ir::{RelocatableUnitIr, ValidatedUnit, PersistError};
/// use std::sync::Arc;
/// fn freeze(unit: RelocatableUnitIr) -> Result<(ValidatedUnit, Vec<u8>), PersistError> {
///     let frozen = ValidatedUnit::new(Arc::new(unit))?;
///     let bytes = frozen.encode()?;
///     let restored = ValidatedUnit::decode(&bytes)?;
///     assert_eq!(restored.revision(), frozen.revision());
///     Ok((restored, bytes))
/// }
/// ```
#[derive(Clone, Debug)]
pub struct ValidatedUnit {
    /// Shared immutable arenas covered by this structural and identity proof.
    unit: Arc<RelocatableUnitIr>,
    /// Actual kind and persistence identities derived internally, never caller assertions.
    revision: UnitRevision,
    /// Full source/debug partition identity, including accepted optional extensions.
    debug: DebugDigest,
    /// Preserves the accepted container minor version for exact decoded re-encoding.
    minor: u16,
    /// Optional debug sections retained by decoded objects; absent on the ordinary writer path.
    extensions: Option<Arc<[Section]>>,
}
impl ValidatedUnit {
    /// Validates immutable raw IR once and derives the canonical writer's identities.
    pub fn new(unit: Arc<RelocatableUnitIr>) -> Result<Self, PersistError> {
        let proof = VerifiedUnitEncoding::new(&unit)?;
        let overrides = UnitEncodingOverrides::default();
        let revision = UnitRevision {
            kind: unit.kind(),
            semantic: proof.semantic_digest(overrides)?,
            object: proof.object_digest(overrides)?,
        };
        let debug = proof.debug_digest(overrides)?;
        Ok(Self {
            unit,
            revision,
            debug,
            minor: 0,
            extensions: None,
        })
    }
    /// Validates actual container bytes and constructs a shared proof in one decode pass.
    /// Optional noncritical debug sections remain accepted and retain their byte identity.
    pub fn decode(bytes: &[u8]) -> Result<Self, PersistError> {
        let container = decode_container_ref(bytes)?;
        let unit = Arc::new(decode_checked_unit(&container)?);
        let revision = UnitRevision {
            kind: unit.kind(),
            semantic: SemanticUnitDigest(crate::codec::borrowed_partition_digest(
                &container, true, "unit",
            )),
            object: ObjectDigest::of(bytes),
        };
        let debug = DebugDigest(crate::codec::borrowed_partition_digest(
            &container, false, "debug",
        ));
        let extensions: Vec<Section> = container
            .sections
            .iter()
            .filter(|s| !matches!(s.tag, SECTION_DESCRIPTOR | SECTION_UNIT | SECTION_DEBUG))
            .map(|s| Section {
                tag: s.tag,
                flags: s.flags,
                payload: s.payload.to_vec(),
            })
            .collect();
        let extensions = (!extensions.is_empty()).then(|| Arc::<[Section]>::from(extensions));
        Ok(Self {
            unit,
            revision,
            debug,
            minor: container.minor,
            extensions,
        })
    }
    /// Returns the shared immutable arenas covered by this proof.
    pub fn unit(&self) -> &Arc<RelocatableUnitIr> {
        &self.unit
    }
    /// Returns actual internally derived kind, semantic and complete-object identities.
    pub fn revision(&self) -> &UnitRevision {
        &self.revision
    }
    /// Returns the complete validated source/debug partition identity.
    pub fn debug_digest(&self) -> DebugDigest {
        self.debug
    }
    /// Encodes without repeating structural validation, retaining decoded extension identity.
    pub fn encode(&self) -> Result<Vec<u8>, PersistError> {
        if self.minor == 0 && self.extensions.is_none() {
            return encode_verified_unit(&self.unit, UnitEncodingOverrides::default());
        }
        encode_extended_validated_unit(self)
    }
}

/// 编码完整 unit；语义投影与 source/debug attachment 进入不同 sections。
/// Encodes a complete unit with semantic projection and source/debug attachment in separate sections.
pub fn encode_unit_container(unit: &RelocatableUnitIr) -> Result<Vec<u8>, PersistError> {
    encode_unit_container_with(unit, UnitEncodingOverrides::default())
}

/// Encodes immutable IR with borrowed relocation metadata into one final allocation.
/// Overrides must preserve arena shape and defining-source attachment consistency.
/// The original unit remains unchanged; the v1 wire format and digests are unchanged.
///
/// # Example
///
/// ```
/// use squish_ir::{RelocatableUnitIr, SourceKey, UnitEncodingOverrides, PersistError,
///     encode_unit_container_with};
/// use std::collections::BTreeMap;
/// fn portable(unit: &RelocatableUnitIr, keys: &BTreeMap<SourceKey, SourceKey>)
///     -> Result<Vec<u8>, PersistError>
/// {
///     encode_unit_container_with(unit, UnitEncodingOverrides {
///         source_keys: Some(keys), ..Default::default()
///     })
/// }
/// ```
pub fn encode_unit_container_with(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<Vec<u8>, PersistError> {
    VerifiedUnitEncoding::new(unit)?.encode(overrides)
}

fn encode_verified_unit(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<Vec<u8>, PersistError> {
    validate_overrides(unit, overrides)?;
    let header = unit.header();
    let descriptor = SemanticDescriptor {
        dialect: header.language_abi.0.clone(),
        semantic_epoch: u64::from(header.ir_schema.major),
        feature_bits: header.feature_bits.0,
    }
    .encode();
    let semantic_len = unit_projection_len(unit, overrides, true);
    let complete_len = unit_projection_len(unit, overrides, false);
    let length = UNIT_CONTAINER_HEADER
        .checked_add(descriptor.len())
        .and_then(|v| v.checked_add(semantic_len))
        .and_then(|v| v.checked_add(complete_len))
        .ok_or(DecodeError::LengthOverflow)?;
    let mut out = Vec::with_capacity(length);
    out.resize(UNIT_CONTAINER_HEADER, 0);
    out.extend_from_slice(&descriptor);
    append_unit_projection(&mut out, unit, overrides, true);
    append_unit_projection(&mut out, unit, overrides, false);
    finish_unit_container(
        &mut out,
        container_kind(unit),
        &[
            (SECTION_DESCRIPTOR, SectionFlags::SEMANTIC, descriptor.len()),
            (SECTION_UNIT, SectionFlags::SEMANTIC, semantic_len),
            (SECTION_DEBUG, SectionFlags::DEBUG, complete_len),
        ],
    );
    Ok(out)
}

/// Computes the exact semantic container identity without allocating container payloads.
/// This is the digest returned by `decode_container(encode_unit_container_with(...)).semantic_digest()`.
pub fn semantic_unit_digest_with(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<SemanticUnitDigest, PersistError> {
    VerifiedUnitEncoding::new(unit)?.semantic_digest(overrides)
}

fn digest_verified_unit(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<SemanticUnitDigest, PersistError> {
    validate_overrides(unit, overrides)?;
    let header = unit.header();
    let descriptor = SemanticDescriptor {
        dialect: header.language_abi.0.clone(),
        semantic_epoch: u64::from(header.ir_schema.major),
        feature_bits: header.feature_bits.0,
    }
    .encode();
    let length = unit_projection_len(unit, overrides, true);
    let mut hash = crate::digest::DomainHasher::new("unit");
    hash.update(&1u16.to_le_bytes());
    hash.update(&(container_kind(unit) as u16).to_le_bytes());
    hash.update(&SECTION_DESCRIPTOR.to_le_bytes());
    hash.update(&(descriptor.len() as u64).to_le_bytes());
    hash.update(&descriptor);
    hash.update(&SECTION_UNIT.to_le_bytes());
    hash.update(&(length as u64).to_le_bytes());
    Ok(SemanticUnitDigest(crate::wire::hash_unit_projection(
        hash, unit, overrides,
    )))
}

/// Streams section checksums and the exact canonical object framing with fixed-size metadata.
fn object_digest_verified_unit(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<ObjectDigest, PersistError> {
    validate_overrides(unit, overrides)?;
    let header = unit.header();
    let descriptor = SemanticDescriptor {
        dialect: header.language_abi.0.clone(),
        semantic_epoch: u64::from(header.ir_schema.major),
        feature_bits: header.feature_bits.0,
    }
    .encode();
    let semantic_len = unit_projection_len(unit, overrides, true);
    let complete_len = unit_projection_len(unit, overrides, false);
    // Keep overflow rejection identical to the encoder before writing canonical offsets.
    UNIT_CONTAINER_HEADER
        .checked_add(descriptor.len())
        .and_then(|v| v.checked_add(semantic_len))
        .and_then(|v| v.checked_add(complete_len))
        .ok_or(DecodeError::LengthOverflow)?;
    let checksum = |tag: u32, length: usize, semantic: bool| {
        let mut hash = crate::digest::DomainHasher::new("section");
        hash.update(&tag.to_le_bytes());
        hash.update(&(length as u64).to_le_bytes());
        crate::wire::update_unit_projection(hash, unit, overrides, semantic).finish()
    };
    let metadata = [
        (
            SECTION_DESCRIPTOR,
            SectionFlags::SEMANTIC,
            descriptor.len(),
            section_digest_parts(SECTION_DESCRIPTOR, &descriptor),
        ),
        (
            SECTION_UNIT,
            SectionFlags::SEMANTIC,
            semantic_len,
            checksum(SECTION_UNIT, semantic_len, true),
        ),
        (
            SECTION_DEBUG,
            SectionFlags::DEBUG,
            complete_len,
            checksum(SECTION_DEBUG, complete_len, false),
        ),
    ];
    let mut directory = [0; UNIT_CONTAINER_HEADER];
    write_unit_directory(&mut directory, container_kind(unit), &metadata);
    let mut hash = crate::digest::DomainHasher::new("object");
    hash.update(&directory);
    hash.update(&descriptor);
    let hash = crate::wire::update_unit_projection(hash, unit, overrides, true);
    let hash = crate::wire::update_unit_projection(hash, unit, overrides, false);
    Ok(ObjectDigest(hash.finish()))
}

/// Validates substitutions without cloning semantic arenas or source line maps.
fn validate_overrides(
    unit: &RelocatableUnitIr,
    overrides: UnitEncodingOverrides<'_>,
) -> Result<(), PersistError> {
    let original = &unit.header().source;
    let mapped = overrides
        .source_keys
        .and_then(|keys| keys.get(original))
        .unwrap_or(original);
    let source = overrides.source.unwrap_or(mapped);
    if source != mapped {
        return Err(PersistError::Invalid(ValidationError {
            path: "encoding_override.source".into(),
            message: "must agree with the attachment source mapping",
        }));
    }
    if overrides.source.is_some() || overrides.source_keys.is_some() {
        crate::validate::validate_source_key(source)?;
    }
    if let Some(keys) = overrides.source_keys {
        // Check only identities this unit consumes, not every provider in the global map.
        for record in &unit.sources().records {
            if let Some(key) = keys.get(&record.key) {
                crate::validate::validate_source_key(key)?;
            }
        }
    }
    if let Some(imports) = overrides.imports {
        if imports.len() != unit.header().imports.len()
            || imports
                .iter()
                .zip(&unit.header().imports)
                .any(|(new, old)| {
                    new.local_id != old.local_id || new.expected_kind != old.expected_kind
                })
        {
            return Err(PersistError::Invalid(ValidationError {
                path: "encoding_override.imports".into(),
                message: "must preserve import IDs and expected kinds",
            }));
        }
        for import in imports {
            let invalid = match &import.spec {
                ImportSpec::RelativeUri(s) | ImportSpec::AbsoluteFileUri(s) => {
                    s.is_empty() || s.chars().any(char::is_control)
                }
                ImportSpec::PackageExport {
                    dependency_alias,
                    export,
                } => dependency_alias.is_empty() || export.is_empty(),
            };
            if invalid {
                return Err(PersistError::Invalid(ValidationError {
                    path: "encoding_override.imports.spec".into(),
                    message: "empty or invalid import specification",
                }));
            }
        }
    }
    Ok(())
}

/// 解码容器、重建完整 unit，并证明语义投影与语义 section 相同。
/// Decodes a container, reconstructs the complete unit, and proves its semantic projection matches.
pub fn decode_unit_container(bytes: &[u8]) -> Result<RelocatableUnitIr, PersistError> {
    let container = decode_container_ref(bytes)?;
    decode_checked_unit(&container)
}

/// Completes typed validation over an already checksum-validated immutable container view.
fn decode_checked_unit(
    container: &crate::codec::ContainerRef<'_>,
) -> Result<RelocatableUnitIr, PersistError> {
    if !matches!(
        container.kind,
        ContainerKind::Module | ContainerKind::Entry | ContainerKind::Pack | ContainerKind::Sopack
    ) {
        return Err(PersistError::KindMismatch);
    }
    validate_unit_sections(&container.sections)?;
    let semantic = container
        .sections
        .iter()
        .find(|s| s.tag == SECTION_UNIT)
        .ok_or(PersistError::MissingSection(SECTION_UNIT))?;
    let debug = container
        .sections
        .iter()
        .find(|s| s.tag == SECTION_DEBUG)
        .ok_or(PersistError::MissingSection(SECTION_DEBUG))?;
    let unit = decode_relocatable_unit(debug.payload)?;
    let header = unit.header();
    let descriptor_section = container
        .sections
        .iter()
        .find(|s| s.tag == SECTION_DESCRIPTOR)
        .ok_or(PersistError::MissingSection(SECTION_DESCRIPTOR))?;
    let descriptor = SemanticDescriptor::decode(descriptor_section.payload)?;
    if descriptor.dialect != header.language_abi.0
        || descriptor.semantic_epoch != u64::from(header.ir_schema.major)
        || descriptor.feature_bits != header.feature_bits.0
    {
        return Err(PersistError::SemanticMismatch);
    }
    if container_kind(&unit) != container.kind {
        return Err(PersistError::KindMismatch);
    }
    if !semantic_projection_matches(&unit, semantic.payload) {
        return Err(PersistError::SemanticMismatch);
    }
    Ok(unit)
}

/// Preserves optional debug sections without ever materializing known unit payload buffers.
fn encode_extended_validated_unit(value: &ValidatedUnit) -> Result<Vec<u8>, PersistError> {
    let unit = &value.unit;
    let extensions = value.extensions.as_deref().unwrap_or(&[]);
    let overrides = UnitEncodingOverrides::default();
    let h = unit.header();
    let descriptor = SemanticDescriptor {
        dialect: h.language_abi.0.clone(),
        semantic_epoch: u64::from(h.ir_schema.major),
        feature_bits: h.feature_bits.0,
    }
    .encode();
    let semantic_len = unit_projection_len(unit, overrides, true);
    let complete_len = unit_projection_len(unit, overrides, false);
    let count = 3usize
        .checked_add(extensions.len())
        .ok_or(DecodeError::LengthOverflow)?;
    let envelope = crate::codec::container_header_len(count)?;
    let mut layout = Vec::with_capacity(count);
    layout.push((SECTION_DESCRIPTOR, SectionFlags::SEMANTIC, descriptor.len()));
    layout.push((SECTION_UNIT, SectionFlags::SEMANTIC, semantic_len));
    layout.push((SECTION_DEBUG, SectionFlags::DEBUG, complete_len));
    layout.extend(extensions.iter().map(|s| (s.tag, s.flags, s.payload.len())));
    layout.sort_by_key(|s| s.0);
    let length = layout.iter().try_fold(envelope, |n, s| {
        n.checked_add(s.2).ok_or(DecodeError::LengthOverflow)
    })?;
    let mut out = Vec::with_capacity(length);
    out.resize(envelope, 0);
    let mut metadata = Vec::with_capacity(count);
    for (tag, flags, length) in layout {
        let offset = out.len();
        match tag {
            SECTION_DESCRIPTOR => out.extend_from_slice(&descriptor),
            SECTION_UNIT => append_unit_projection(&mut out, unit, overrides, true),
            SECTION_DEBUG => append_unit_projection(&mut out, unit, overrides, false),
            _ => {
                let index = extensions
                    .binary_search_by_key(&tag, |s| s.tag)
                    .expect("validated extension directory");
                out.extend_from_slice(&extensions[index].payload);
            }
        }
        let checksum = section_digest_parts(tag, &out[offset..]);
        metadata.push((tag, flags, length, checksum));
    }
    crate::codec::write_container_directory(
        &mut out,
        1,
        value.minor,
        container_kind(unit),
        &metadata,
    );
    Ok(out)
}

/// 将领域单元种类映射到容器标签；只在持久化边界执行此策略。
/// Maps the domain unit kind to its container tag only at the persistence boundary.
fn container_kind(unit: &RelocatableUnitIr) -> ContainerKind {
    match unit.kind() {
        UnitKind::Module => ContainerKind::Module,
        UnitKind::Entry => ContainerKind::Entry,
        UnitKind::Pack => ContainerKind::Pack,
        UnitKind::Sopack => ContainerKind::Sopack,
    }
}

fn validate_unit_sections(sections: &[SectionRef<'_>]) -> Result<(), PersistError> {
    for (tag, flags) in [
        (SECTION_DESCRIPTOR, SectionFlags::SEMANTIC),
        (SECTION_UNIT, SectionFlags::SEMANTIC),
        (SECTION_DEBUG, SectionFlags::DEBUG),
    ] {
        let section = sections
            .iter()
            .find(|section| section.tag == tag)
            .ok_or(PersistError::MissingSection(tag))?;
        if section.flags != flags {
            return Err(PersistError::InvalidSections);
        }
    }
    if sections.iter().any(|section| {
        !matches!(
            section.tag,
            SECTION_DESCRIPTOR | SECTION_UNIT | SECTION_DEBUG
        ) && (section.flags != SectionFlags::DEBUG
            || matches!(section.tag, SECTION_LINKED_IMAGE | SECTION_DOCUMENT))
    }) {
        return Err(PersistError::InvalidSections);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Section {
        Section {
            tag: SECTION_DESCRIPTOR,
            flags: SectionFlags::SEMANTIC,
            payload: SemanticDescriptor {
                dialect: "xmlsquish.core".into(),
                semantic_epoch: 1,
                feature_bits: 0,
            }
            .encode(),
        }
    }

    #[test]
    fn unit_container_requires_exact_section_contract() {
        let wrong = Container::v1(
            ContainerKind::Module,
            vec![
                descriptor(),
                Section {
                    tag: SECTION_UNIT,
                    flags: SectionFlags::DEBUG,
                    payload: vec![],
                },
                Section {
                    tag: SECTION_DEBUG,
                    flags: SectionFlags::SEMANTIC,
                    payload: vec![],
                },
            ],
        )
        .unwrap();
        assert_eq!(
            decode_unit_container(&encode_container(&wrong)),
            Err(PersistError::InvalidSections)
        );

        let wrong_kind = Container::v1(ContainerKind::LinkedImage, vec![descriptor()]).unwrap();
        assert_eq!(
            decode_unit_container(&encode_container(&wrong_kind)),
            Err(PersistError::KindMismatch)
        );
    }

    #[test]
    fn unit_sections_allow_unknown_optional_debug_extensions() {
        let sections = vec![
            descriptor(),
            Section {
                tag: SECTION_UNIT,
                flags: SectionFlags::SEMANTIC,
                payload: vec![],
            },
            Section {
                tag: SECTION_DEBUG,
                flags: SectionFlags::DEBUG,
                payload: vec![],
            },
            Section {
                tag: 0x9000_0000,
                flags: SectionFlags::DEBUG,
                payload: b"future".to_vec(),
            },
        ];
        let refs: Vec<_> = sections
            .iter()
            .map(|s| SectionRef {
                tag: s.tag,
                flags: s.flags,
                payload: &s.payload,
            })
            .collect();
        assert_eq!(validate_unit_sections(&refs), Ok(()));
    }
    /// Compatibility oracle: the former owned debug-stripping projection, confined to tests.
    fn legacy_container(unit: &RelocatableUnitIr) -> Vec<u8> {
        let mut stripped = unit.clone();
        stripped.attachment_mut().source_record = SourceRef(0);
        stripped.attachment_mut().source_digest = SourceDigest::of(&[]);
        let strip =
            |origins: &mut OriginTable, sources: &mut SourceArchive, producer: &mut Producer| {
                *origins = OriginTable::default();
                *sources = SourceArchive::default();
                producer.tool_version.clear();
                producer.build_fingerprint.clear();
            };
        match &mut stripped {
            RelocatableUnitIr::Module(v) => strip(&mut v.origins, &mut v.sources, &mut v.producer),
            RelocatableUnitIr::Entry(v) | RelocatableUnitIr::Pack(v) => {
                strip(&mut v.origins, &mut v.sources, &mut v.producer)
            }
            RelocatableUnitIr::Sopack(v) => strip(
                &mut v.module.origins,
                &mut v.module.sources,
                &mut v.module.producer,
            ),
        }
        let h = unit.header();
        let descriptor = SemanticDescriptor {
            dialect: h.language_abi.0.clone(),
            semantic_epoch: u64::from(h.ir_schema.major),
            feature_bits: h.feature_bits.0,
        }
        .encode();
        encode_container(
            &Container::v1(
                container_kind(unit),
                vec![
                    Section {
                        tag: SECTION_DESCRIPTOR,
                        flags: SectionFlags::SEMANTIC,
                        payload: descriptor,
                    },
                    Section {
                        tag: SECTION_UNIT,
                        flags: SectionFlags::SEMANTIC,
                        payload: encode_relocatable_unit(&stripped),
                    },
                    Section {
                        tag: SECTION_DEBUG,
                        flags: SectionFlags::DEBUG,
                        payload: encode_relocatable_unit(unit),
                    },
                ],
            )
            .unwrap(),
        )
    }
    #[test]
    fn borrowed_projections_preserve_all_four_kinds_and_large_literal_bytes() {
        let module = crate::wire::tests::module();
        let entry = crate::wire::tests::entry();
        let RelocatableUnitIr::Module(mut m) = module.clone() else {
            unreachable!()
        };
        let RelocatableUnitIr::Entry(e) = entry.clone() else {
            unreachable!()
        };
        let sopack_root = RegionId(m.regions.len() as u32);
        m.regions.push(Region {
            id: sopack_root,
            ops: vec![],
        });
        for mut unit in [
            module,
            entry,
            RelocatableUnitIr::Pack(e),
            RelocatableUnitIr::Sopack(SopackObject {
                module: m,
                root_region: sopack_root,
            }),
        ] {
            // Keep the sorted pool valid while exercising multi-megabyte borrowed slices.
            unit.header_mut().semantic_strings[2] =
                format!("z{}", "可\u{1f338}".repeat(4 * 1024 * 1024 / 7));
            let bytes = encode_unit_container(&unit).unwrap();
            assert_eq!(bytes, legacy_container(&unit));
            assert_eq!(decode_unit_container(&bytes).unwrap(), unit);
            let container = decode_container(&bytes).unwrap();
            assert_eq!(container.object_digest(), ObjectDigest::of(&bytes));
            let shared = Arc::new(unit.clone());
            let witness = ValidatedUnit::new(shared.clone()).unwrap();
            assert!(Arc::ptr_eq(witness.unit(), &shared));
            assert_eq!(witness.revision().kind, unit.kind());
            assert_eq!(witness.revision().semantic, container.semantic_digest());
            assert_eq!(witness.revision().object, container.object_digest());
            assert_eq!(witness.debug_digest(), container.debug_digest());
            assert_eq!(witness.encode().unwrap(), bytes);
            let decoded = ValidatedUnit::decode(&bytes).unwrap();
            assert_eq!(decoded.unit().as_ref(), &unit);
            assert_eq!(decoded.revision(), witness.revision());
            assert_eq!(decoded.debug_digest(), witness.debug_digest());
            assert_eq!(decoded.encode().unwrap(), bytes);

            assert_eq!(
                VerifiedUnitEncoding::new(&unit)
                    .unwrap()
                    .object_digest(UnitEncodingOverrides::default())
                    .unwrap(),
                container.object_digest()
            );
            for (semantic, domain, digest) in [
                (true, "unit", container.semantic_digest().0),
                (false, "debug", container.debug_digest().0),
            ] {
                let mut legacy = Vec::new();
                legacy.extend_from_slice(&container.schema().0.to_le_bytes());
                legacy.extend_from_slice(&(container.kind() as u16).to_le_bytes());
                for section in container
                    .sections()
                    .iter()
                    .filter(|s| s.flags.is_semantic() == semantic)
                {
                    legacy.extend_from_slice(&section.tag.to_le_bytes());
                    legacy.extend_from_slice(&(section.payload.len() as u64).to_le_bytes());
                    legacy.extend_from_slice(&section.payload);
                }
                assert_eq!(digest, Digest::sha256(domain, &legacy));
            }
            assert_eq!(
                semantic_unit_digest_with(&unit, UnitEncodingOverrides::default()).unwrap(),
                container.semantic_digest()
            );
        }
    }
    #[test]
    fn borrowed_relocation_matches_owned_metadata_rewrite() {
        let unit = crate::wire::tests::module();
        let relocated = SourceKey::AdHoc {
            uri: "file:///relocated.xml".into(),
        };
        let map =
            std::collections::BTreeMap::from([(unit.header().source.clone(), relocated.clone())]);
        let producer = Producer {
            tool_version: "portable".into(),
            build_fingerprint: "fixed".into(),
        };
        let mut imports = unit.header().imports.clone();
        imports[0].spec = ImportSpec::RelativeUri("sopack:module".into());
        let overrides = UnitEncodingOverrides {
            source: Some(&relocated),
            imports: Some(&imports),
            source_keys: Some(&map),
            producer: Some(&producer),
        };
        let mut owned = unit.clone();
        owned.header_mut().source = relocated.clone();
        owned.header_mut().imports = imports.clone();
        owned.attachment_mut().source = relocated.clone();
        owned.sources_mut().records[0].key = relocated.clone();
        let RelocatableUnitIr::Module(module) = &mut owned else {
            unreachable!()
        };
        module.producer = producer.clone();
        let bytes = encode_unit_container_with(&unit, overrides).unwrap();
        assert_eq!(bytes, encode_unit_container(&owned).unwrap());
        assert_eq!(
            semantic_unit_digest_with(&unit, overrides).unwrap(),
            decode_container(&bytes).unwrap().semantic_digest()
        );
        assert_eq!(decode_unit_container(&bytes).unwrap(), owned);
        assert_eq!(
            VerifiedUnitEncoding::new(&unit)
                .unwrap()
                .debug_digest(overrides)
                .unwrap(),
            decode_container(&bytes).unwrap().debug_digest()
        );

        assert_eq!(
            VerifiedUnitEncoding::new(&unit)
                .unwrap()
                .object_digest(overrides)
                .unwrap(),
            decode_container(&bytes).unwrap().object_digest()
        );
    }
    #[test]
    fn checked_debug_payload_cannot_disagree_with_semantic_section() {
        let unit = crate::wire::tests::module();
        let mut container = decode_container(&encode_unit_container(&unit).unwrap()).unwrap();
        let debug = container
            .sections
            .iter_mut()
            .find(|s| s.tag == SECTION_DEBUG)
            .unwrap();
        let mut different = decode_relocatable_unit(&debug.payload).unwrap();
        different.header_mut().semantic_strings[2] = "different".into();
        debug.payload = encode_relocatable_unit(&different);
        assert_eq!(
            decode_unit_container(&encode_container(&container)),
            Err(PersistError::SemanticMismatch)
        );
        assert!(matches!(
            ValidatedUnit::decode(&encode_container(&container)),
            Err(PersistError::SemanticMismatch)
        ));
        let semantic = container
            .sections
            .iter_mut()
            .find(|s| s.tag == SECTION_UNIT)
            .unwrap();
        semantic.payload.push(0);
        assert_eq!(
            decode_unit_container(&encode_container(&container)),
            Err(PersistError::SemanticMismatch)
        );
    }
    #[test]
    fn verified_handle_and_projection_reject_invalid_inputs() {
        let mut invalid = crate::wire::tests::module();
        invalid.header_mut().semantic_strings[2] = "a".into();
        assert!(VerifiedUnitEncoding::new(&invalid).is_err());
        let unit = crate::wire::tests::module();
        let verified = VerifiedUnitEncoding::new(&unit).unwrap();
        let wrong = SourceKey::AdHoc {
            uri: "file:///different.xml".into(),
        };
        assert!(
            verified
                .encode(UnitEncodingOverrides {
                    source: Some(&wrong),
                    ..Default::default()
                })
                .is_err()
        );
        let mut imports = unit.header().imports.clone();
        imports[0].expected_kind = UnitKind::Entry;
        assert!(
            verified
                .encode(UnitEncodingOverrides {
                    imports: Some(&imports),
                    ..Default::default()
                })
                .is_err()
        );
        imports[0].expected_kind = UnitKind::Module;
        imports[0].spec = ImportSpec::RelativeUri("\n".into());
        assert!(
            verified
                .semantic_digest(UnitEncodingOverrides {
                    imports: Some(&imports),
                    ..Default::default()
                })
                .is_err()
        );
        let keys = std::collections::BTreeMap::from([(
            unit.header().source.clone(),
            SourceKey::AdHoc {
                uri: "https://not-a-file".into(),
            },
        )]);
        assert!(
            verified
                .encode(UnitEncodingOverrides {
                    source_keys: Some(&keys),
                    ..Default::default()
                })
                .is_err()
        );
    }
    #[test]
    fn validated_decode_preserves_minor_and_optional_debug_extension_identity() {
        let unit = crate::wire::tests::entry();
        let mut container = decode_container(&encode_unit_container(&unit).unwrap()).unwrap();
        container.minor = 7;
        container.sections.push(Section {
            tag: 5,
            flags: SectionFlags::DEBUG,
            payload: b"before-main-debug".to_vec(),
        });
        container.sections.push(Section {
            tag: 0x9000_0000,
            flags: SectionFlags::DEBUG,
            payload: b"future-debug".to_vec(),
        });
        container.sections.sort_by_key(|s| s.tag);
        let bytes = encode_container(&container);
        assert_eq!(decode_unit_container(&bytes).unwrap(), unit);
        let witness = ValidatedUnit::decode(&bytes).unwrap();
        assert_eq!(witness.revision().semantic, container.semantic_digest());
        assert_eq!(witness.revision().object, container.object_digest());
        assert_eq!(witness.debug_digest(), container.debug_digest());
        assert_eq!(witness.encode().unwrap(), bytes);
        assert_eq!(witness.clone().encode().unwrap(), bytes);
    }
    #[test]
    fn validated_witness_rejects_corrupt_bytes_and_tracks_debug_only_changes() {
        let unit = crate::wire::tests::module();
        let witness = ValidatedUnit::new(Arc::new(unit.clone())).unwrap();
        let mut bytes = witness.encode().unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(matches!(
            ValidatedUnit::decode(&bytes),
            Err(PersistError::Decode(DecodeError::Checksum(_)))
        ));
        let mut different = unit;
        let RelocatableUnitIr::Module(m) = &mut different else {
            unreachable!()
        };
        m.producer.tool_version.push_str("-other");
        let changed = ValidatedUnit::new(Arc::new(different)).unwrap();
        assert_eq!(witness.revision().semantic, changed.revision().semantic);
        assert_ne!(witness.revision().object, changed.revision().object);
        assert_ne!(witness.debug_digest(), changed.debug_digest());
        let mut alias = witness.unit().clone();
        assert!(Arc::get_mut(&mut alias).is_none());
    }
}
