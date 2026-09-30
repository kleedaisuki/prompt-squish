//! 会话内链接程序视图。 / Session-local linked-program view.

use crate::{LinkError, MiddleEnd, OptimizationStats, OptimizedUnit};
use squish_ir::{
    DefAddr, LinkedImage, LinkedMacroDef, ObjectDigest, RelocatableUnitIr, SourceKey, Validate,
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
    pub(crate) fn object(&self, slot: u32) -> Option<ObjectDigest> {
        self.objects.get(slot as usize).copied()
    }
}
