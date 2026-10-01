//! 会话内链接程序视图。 / Session-local linked-program view.

use crate::{LinkError, MiddleEnd, OptimizationStats, OptimizedUnit};
use squish_ir::{
    DefAddr, EntityKind, LinkedImage, LinkedMacroDef, ObjectDigest, OriginEntry, OriginId,
    QualifiedOriginRef, RelocatableUnitIr, SourceKey, Validate,
};
use std::collections::BTreeMap;

/// 可执行的会话视图；可由持久化 `LinkedImage` 与语义单元重建。
/// Executable session view reconstructed from a persistent `LinkedImage` and semantic units.
///
/// 该类型故意不是新的 wire artifact：它只把链接映射和已缓存单元组合成高效访问视图。
/// It deliberately is not another wire artifact; it only combines the link map and cached units.
#[derive(Clone, Debug)]
pub struct LinkedProgram {
    image: LinkedImage,
    units: Vec<OptimizedUnit>,
    /// Link-time indexed definition addresses replace per-call linear symbol scans.
    definition_slots: Vec<Vec<usize>>,
    objects: Vec<ObjectDigest>,
    /// One bounded dense origin index per immutable unit; source table IDs are unchanged.
    origin_indexes: Vec<OriginIndex>,
}

impl LinkedProgram {
    /// 重建并验证会话视图。 / Reconstructs and validates a session view.
    pub fn reconstruct(
        image: LinkedImage,
        mut units: BTreeMap<SourceKey, RelocatableUnitIr>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        image
            .validate()
            .map_err(|error| LinkError::new("LNK001", format!("invalid linked image: {error}")))?;
        let mut optimized = BTreeMap::new();
        for linked in &image.units {
            let unit = units.remove(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            let result = MiddleEnd.optimize(unit).map_err(|error| {
                LinkError::new("LNK030", format!("middle-end optimization failed: {error}"))
                    .at_source(linked.source.clone())
            })?;
            optimized.insert(linked.source.clone(), result);
        }
        Self::reconstruct_optimized(image, optimized, objects)
    }

    /// Builds the linked executable from already-specialized middle-end units.
    ///
    /// Fresh compilation runs the middle end before symbol binding and retains its results
    /// here. Cache hydration uses `reconstruct` to rebuild device-local regex machinery.
    pub(crate) fn reconstruct_optimized(
        image: LinkedImage,
        mut units: BTreeMap<SourceKey, OptimizedUnit>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        image
            .validate()
            .map_err(|e| LinkError::new("LNK001", format!("invalid linked image: {e}")))?;
        let mut ordered = Vec::with_capacity(image.units.len());
        let mut ordered_objects = Vec::with_capacity(image.units.len());
        for linked in &image.units {
            let unit = units.remove(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            if unit.unit.kind() != linked.kind || unit.unit.header().source != linked.source {
                return Err(LinkError::new(
                    "LNK004",
                    "linked unit kind/source differs from payload",
                )
                .at_source(linked.source.clone()));
            }
            ordered.push(unit);
            ordered_objects.push(*objects.get(&linked.source).ok_or_else(|| {
                LinkError::new("LNK005", "linked unit object digest is missing")
                    .at_source(linked.source.clone())
            })?);
        }
        let origin_indexes = ordered
            .iter()
            .map(|unit| OriginIndex::new(&unit.unit))
            .collect();
        let mut definition_slots = vec![Vec::new(); ordered.len()];
        for (index, definition) in image.definitions.iter().enumerate() {
            let unit = &mut definition_slots[definition.addr.unit_slot as usize];
            let local = definition.addr.local_def.0 as usize;
            if local != unit.len() {
                return Err(LinkError::new(
                    "LNK031",
                    "noncontiguous definition addresses",
                ));
            }
            unit.push(index);
        }
        Ok(Self {
            image,
            definition_slots,
            units: ordered,
            objects: ordered_objects,
            origin_indexes,
        })
    }

    /// 返回持久化链接镜像。 / Returns the persistent linked image.
    #[must_use]
    pub fn image(&self) -> &LinkedImage {
        &self.image
    }
    /// Aggregates deterministic middle-end counters for opt-in end-to-end telemetry.
    #[must_use]
    pub fn optimization_stats(&self) -> OptimizationStats {
        self.units
            .iter()
            .fold(OptimizationStats::default(), |mut total, unit| {
                total.operations += unit.stats.operations;
                total.compiled_regexes += unit.stats.compiled_regexes;
                total.static_matches += unit.stats.static_matches;
                total.eliminated_branches += unit.stats.eliminated_branches;
                total.static_scalars += unit.stats.static_scalars;
                total.scalar_bytes += unit.stats.scalar_bytes;
                total
            })
    }
    pub(crate) fn unit(&self, slot: u32) -> Option<&RelocatableUnitIr> {
        self.units.get(slot as usize).map(|unit| &unit.unit)
    }
    pub(crate) fn optimized(&self, slot: u32) -> Option<&OptimizedUnit> {
        self.units.get(slot as usize)
    }
    pub(crate) fn definition(&self, addr: DefAddr) -> Option<&LinkedMacroDef> {
        self.definition_slots
            .get(addr.unit_slot as usize)
            .and_then(|unit| unit.get(addr.local_def.0 as usize))
            .and_then(|index| self.image.definitions.get(*index))
    }
    /// Resolves the first source-table entry for a valid arena ID, qualified by this object.
    ///
    /// The index never renumbers origins or hashes them by semantic unit identity. Debug
    /// attachment revisions can therefore retain distinct source/provenance evidence.
    pub(crate) fn origin(
        &self,
        slot: u32,
        kind: EntityKind,
        local: u32,
    ) -> Option<QualifiedOriginRef> {
        Some(QualifiedOriginRef {
            object: self.object(slot)?,
            local: self.origin_indexes.get(slot as usize)?.get(kind, local)?,
        })
    }

    pub(crate) fn object(&self, slot: u32) -> Option<ObjectDigest> {
        self.objects.get(slot as usize).copied()
    }
}

/// Constant-time first-origin lookup using one allocation sized by validated arenas.
///
/// Segments are operations, regions, then definitions. Extra Region/Definition metadata
/// may contain out-of-range IDs under the existing verifier; ignore it without growing
/// the index, rejecting the unit or changing valid-ID first-match semantics.
#[derive(Clone, Debug)]
struct OriginIndex {
    /// Number of operation cells and offset of the region segment.
    operations: usize,
    /// Number of region cells; definitions occupy the remaining cells.
    regions: usize,
    /// Original table IDs, not IDs in a newly compacted or reordered source table.
    ids: Vec<Option<OriginId>>,
}
impl OriginIndex {
    /// Builds an index only after the unit has passed the unchanged structural verifier.
    fn new(unit: &RelocatableUnitIr) -> Self {
        Self::build(
            unit.ops().len(),
            unit.regions().len(),
            unit.definitions().len(),
            &unit.origins().entries,
        )
    }
    /// Records first matches while preserving original enumeration over every metadata kind.
    fn build(
        operations: usize,
        regions: usize,
        definitions: usize,
        entries: &[OriginEntry],
    ) -> Self {
        let mut index = Self {
            operations,
            regions,
            ids: vec![None; operations + regions + definitions],
        };
        for (position, entry) in entries.iter().enumerate() {
            let Some(cell) = index.cell(entry.entity_kind, entry.local_id) else {
                continue;
            };
            index.ids[cell].get_or_insert(OriginId(position as u32));
        }
        index
    }
    /// Bounds each kind by its real arena, not by any untrusted metadata ID.
    fn cell(&self, kind: EntityKind, local: u32) -> Option<usize> {
        let local = local as usize;
        match kind {
            EntityKind::Operation if local < self.operations => Some(local),
            EntityKind::Region if local < self.regions => Some(self.operations + local),
            EntityKind::Definition if local < self.ids.len() - self.operations - self.regions => {
                Some(self.operations + self.regions + local)
            }
            _ => None,
        }
    }
    /// Returns the same first table position as the previous linear successful lookup.
    fn get(&self, kind: EntityKind, local: u32) -> Option<OriginId> {
        self.ids[self.cell(kind, local)?]
    }
}

#[cfg(test)]
mod origin_index_layout_tests {
    use super::*;
    use squish_ir::{Origin, SourceRef, Span, SyntaxKind};
    fn entry(kind: EntityKind, id: u32) -> OriginEntry {
        OriginEntry {
            entity_kind: kind,
            local_id: id,
            origin: Origin {
                source: SourceRef(0),
                span: Span { start: 0, end: 1 },
                lexical_qname: None,
                syntax_kind: SyntaxKind(0),
            },
        }
    }
    #[test]
    fn flat_index_is_bounded_and_keeps_first_original_table_positions() {
        let entries = vec![
            entry(EntityKind::Import, u32::MAX),
            entry(EntityKind::Operation, 0),
            entry(EntityKind::Operation, 0),
            entry(EntityKind::Region, u32::MAX),
            entry(EntityKind::Definition, u32::MAX),
            entry(EntityKind::Definition, 0),
            entry(EntityKind::Region, 0),
        ];
        let index = OriginIndex::build(1, 1, 1, &entries);
        assert_eq!(index.ids.len(), 3);
        assert_eq!(index.get(EntityKind::Operation, 0), Some(OriginId(1)));
        assert_eq!(index.get(EntityKind::Definition, 0), Some(OriginId(5)));
        assert_eq!(index.get(EntityKind::Region, 0), Some(OriginId(6)));
        for kind in [
            EntityKind::Operation,
            EntityKind::Region,
            EntityKind::Definition,
            EntityKind::Import,
        ] {
            assert_eq!(index.get(kind, u32::MAX), None);
        }
    }
}
