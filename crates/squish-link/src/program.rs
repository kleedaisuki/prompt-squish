//! 会话内链接程序视图。 / Session-local linked-program view.

use crate::LinkError;
use squish_ir::{LinkedImage, ObjectDigest, RelocatableUnitIr, SourceKey, UnitKind, Validate};
use std::collections::BTreeMap;

/// 可执行的会话视图；可由持久化 `LinkedImage` 与语义单元重建。
/// Executable session view reconstructed from a persistent `LinkedImage` and semantic units.
///
/// 该类型故意不是新的 wire artifact：它只把链接映射和已缓存单元组合成高效访问视图。
/// It deliberately is not another wire artifact; it only combines the link map and cached units.
#[derive(Clone, Debug)]
pub struct LinkedProgram {
    image: LinkedImage,
    units: Vec<RelocatableUnitIr>,
    objects: Vec<ObjectDigest>,
}

impl LinkedProgram {
    /// 重建并验证会话视图。 / Reconstructs and validates a session view.
    pub fn reconstruct(
        image: LinkedImage,
        units: BTreeMap<SourceKey, RelocatableUnitIr>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        image
            .validate()
            .map_err(|e| LinkError::new("LNK001", format!("invalid linked image: {e}")))?;
        let mut ordered = Vec::with_capacity(image.units.len());
        let mut ordered_objects = Vec::with_capacity(image.units.len());
        for linked in &image.units {
            let unit = units.get(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            unit.validate()
                .map_err(|e| LinkError::new("LNK003", format!("invalid unit: {e}")))?;
            let kind = match unit {
                RelocatableUnitIr::Entry(_) => UnitKind::Entry,
                RelocatableUnitIr::Module(_) => UnitKind::Module,
            };
            if kind != linked.kind {
                return Err(
                    LinkError::new("LNK004", "linked unit kind differs from payload")
                        .at_source(linked.source.clone()),
                );
            }
            ordered.push(unit.clone());
            ordered_objects.push(*objects.get(&linked.source).ok_or_else(|| {
                LinkError::new("LNK005", "linked unit object digest is missing")
                    .at_source(linked.source.clone())
            })?);
        }
        Ok(Self {
            image,
            units: ordered,
            objects: ordered_objects,
        })
    }

    /// 返回持久化链接镜像。 / Returns the persistent linked image.
    #[must_use]
    pub fn image(&self) -> &LinkedImage {
        &self.image
    }
    pub(crate) fn unit(&self, slot: u32) -> Option<&RelocatableUnitIr> {
        self.units.get(slot as usize)
    }
    pub(crate) fn object(&self, slot: u32) -> Option<ObjectDigest> {
        self.objects.get(slot as usize).copied()
    }
}
