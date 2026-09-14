//! prompt-squish 的可复用中间表示领域。 / Reusable intermediate-representation domain.
//!
//! 该 crate 刻意只包含可移植值；文件系统、正则表达式执行器和缓存实现属于适配器。
//! This crate deliberately contains portable values only; filesystems, regex engines, and
//! cache implementations belong in adapters.

#![forbid(unsafe_code)]

mod codec;
mod digest;
mod document_wire;
mod inspect;
mod link_wire;
mod model;
mod persistent;
mod trace_wire;
mod validate;
mod wire;

pub use codec::{
    Container, ContainerKind, DecodeError, SECTION_DEBUG, SECTION_DESCRIPTOR, SECTION_DOCUMENT,
    SECTION_LINKED_IMAGE, SECTION_UNIT, Section, SectionFlags, SemanticDescriptor,
    decode_container, encode_container,
};
pub use digest::{
    ArtifactDigest, DebugDigest, Digest, DigestAlgorithm, DocumentDigest, LinkedImageDigest,
    ObjectDigest, SemanticUnitDigest, SourceDigest,
};
pub use document_wire::{decode_linked_document, encode_linked_document};
pub use inspect::{Inspect, JsonProjection};
pub use link_wire::{
    decode_linked_image, decode_static_link_map, encode_linked_image, encode_static_link_map,
};
pub use model::*;
pub use persistent::{PersistError, decode_unit_container, encode_unit_container};
pub use trace_wire::{decode_expansion_trace, encode_expansion_trace};
pub use validate::{Validate, ValidationError};
pub use wire::{decode_relocatable_unit, encode_relocatable_unit};
