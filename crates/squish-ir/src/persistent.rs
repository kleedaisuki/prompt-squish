//! 有类型 IR 与规范容器的组合边界。 / Composition boundary for typed IR and canonical containers.

use crate::*;
use core::fmt;

/// 持久化边界错误。 / Persistent-boundary error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistError {
    Decode(DecodeError),
    Invalid(ValidationError),
    MissingSection(u32),
    KindMismatch,
    SemanticMismatch,
    InvalidSections,
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

/// 编码完整 unit；语义投影与 source/debug attachment 进入不同 sections。
/// Encodes a complete unit with semantic projection and source/debug attachment in separate sections.
pub fn encode_unit_container(unit: &RelocatableUnitIr) -> Result<Vec<u8>, PersistError> {
    unit.validate()?;
    let header = match unit {
        RelocatableUnitIr::Module(v) => &v.header,
        RelocatableUnitIr::Entry(v) => &v.header,
    };
    let descriptor = SemanticDescriptor {
        dialect: header.language_abi.0.clone(),
        semantic_epoch: u64::from(header.ir_schema.major),
        feature_bits: header.feature_bits.0,
    };
    let kind = match unit {
        RelocatableUnitIr::Module(_) => ContainerKind::Module,
        RelocatableUnitIr::Entry(_) => ContainerKind::Entry,
    };
    let semantic = encode_relocatable_unit(&without_debug(unit.clone()));
    let complete = encode_relocatable_unit(unit);
    let container = Container::v1(
        kind,
        vec![
            Section {
                tag: SECTION_DESCRIPTOR,
                flags: SectionFlags::SEMANTIC,
                payload: descriptor.encode(),
            },
            Section {
                tag: SECTION_UNIT,
                flags: SectionFlags::SEMANTIC,
                payload: semantic,
            },
            Section {
                tag: SECTION_DEBUG,
                flags: SectionFlags::DEBUG,
                payload: complete,
            },
        ],
    )?;
    Ok(encode_container(&container))
}

/// 解码容器、重建完整 unit，并证明语义投影与语义 section 相同。
/// Decodes a container, reconstructs the complete unit, and proves its semantic projection matches.
pub fn decode_unit_container(bytes: &[u8]) -> Result<RelocatableUnitIr, PersistError> {
    let container = decode_container(bytes)?;
    if !matches!(
        container.kind(),
        ContainerKind::Module | ContainerKind::Entry
    ) {
        return Err(PersistError::KindMismatch);
    }
    validate_unit_sections(container.sections())?;
    let semantic = container
        .sections()
        .iter()
        .find(|s| s.tag == SECTION_UNIT)
        .ok_or(PersistError::MissingSection(SECTION_UNIT))?;
    let debug = container
        .sections()
        .iter()
        .find(|s| s.tag == SECTION_DEBUG)
        .ok_or(PersistError::MissingSection(SECTION_DEBUG))?;
    let unit = decode_relocatable_unit(&debug.payload)?;
    let header = match &unit {
        RelocatableUnitIr::Module(v) => &v.header,
        RelocatableUnitIr::Entry(v) => &v.header,
    };
    let descriptor_section = container
        .sections()
        .iter()
        .find(|s| s.tag == SECTION_DESCRIPTOR)
        .ok_or(PersistError::MissingSection(SECTION_DESCRIPTOR))?;
    let descriptor = SemanticDescriptor::decode(&descriptor_section.payload)?;
    if descriptor.dialect != header.language_abi.0
        || descriptor.semantic_epoch != u64::from(header.ir_schema.major)
        || descriptor.feature_bits != header.feature_bits.0
    {
        return Err(PersistError::SemanticMismatch);
    }
    let kind = match unit {
        RelocatableUnitIr::Module(_) => ContainerKind::Module,
        RelocatableUnitIr::Entry(_) => ContainerKind::Entry,
    };
    if kind != container.kind() {
        return Err(PersistError::KindMismatch);
    }
    if encode_relocatable_unit(&without_debug(unit.clone())) != semantic.payload {
        return Err(PersistError::SemanticMismatch);
    }
    Ok(unit)
}

fn validate_unit_sections(sections: &[Section]) -> Result<(), PersistError> {
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

fn without_debug(mut unit: RelocatableUnitIr) -> RelocatableUnitIr {
    let strip = |origins: &mut OriginTable,
                 sources: &mut SourceArchive,
                 attachment: &mut UnitSourceAttachment,
                 producer: &mut Producer| {
        *origins = OriginTable::default();
        *sources = SourceArchive::default();
        attachment.source_record = SourceRef(0);
        attachment.source_digest = SourceDigest::of(&[]);
        producer.tool_version.clear();
        producer.build_fingerprint.clear();
    };
    match &mut unit {
        RelocatableUnitIr::Module(v) => strip(
            &mut v.origins,
            &mut v.sources,
            &mut v.attachment,
            &mut v.producer,
        ),
        RelocatableUnitIr::Entry(v) => strip(
            &mut v.origins,
            &mut v.sources,
            &mut v.attachment,
            &mut v.producer,
        ),
    }
    unit
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
        assert_eq!(validate_unit_sections(&sections), Ok(()));
    }
}
