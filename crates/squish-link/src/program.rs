//! 会话内链接程序视图。 / Session-local linked-program view.

use crate::{LinkError, MiddleEnd, OptimizationStats, OptimizedUnit};
use squish_ir::{
    DefAddr, EntityKind, LinkedImage, LinkedMacroDef, ObjectDigest, OriginEntry, OriginId,
    QualifiedOriginRef, RelocatableUnitIr, SourceKey, UnitRevision, Validate, ValidatedUnit,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

/// 可执行的会话视图；可由持久化 `LinkedImage` 与语义单元重建。
/// Executable session view reconstructed from a persistent `LinkedImage` and semantic units.
///
/// 该类型故意不是新的 wire artifact：它只把链接映射和已缓存单元组合成高效访问视图。
/// It deliberately is not another wire artifact; it only combines the link map and cached units.
#[derive(Clone, Debug)]
pub struct LinkedProgram {
    /// Shared immutable executable storage; cloning a program never clones unit payloads.
    data: Arc<ProgramData>,
}

/// Session data published only after validation and binding are complete.
#[derive(Debug)]
struct ProgramData {
    image: LinkedImage,
    units: Vec<Arc<OptimizedUnit>>,
    /// Link-time indexed definition addresses replace per-call linear symbol scans.
    definition_slots: Vec<Vec<usize>>,
    objects: Vec<ObjectDigest>,
    /// One bounded dense origin index per immutable unit; source table IDs are unchanged.
    origin_indexes: Vec<Arc<OriginIndex>>,
}

impl LinkedProgram {
    /// 重建并验证会话视图。 / Reconstructs and validates a session view.
    pub fn reconstruct(
        image: LinkedImage,
        units: BTreeMap<SourceKey, RelocatableUnitIr>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        Self::reconstruct_shared(
            image,
            units
                .into_iter()
                .map(|(key, unit)| (key, Arc::new(unit)))
                .collect(),
            objects,
        )
    }

    /// Reconstructs from immutable shared payloads without copying their IR or source bytes.
    ///
    /// This remains an untrusted public boundary: image, units and their exact cross-object
    /// correspondence are checked before any executable storage is published.
    pub fn reconstruct_shared(
        image: LinkedImage,
        mut units: BTreeMap<SourceKey, Arc<RelocatableUnitIr>>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        let image = ValidatedImage::new(image, "LNK001", "invalid linked image")?;
        let mut optimized = BTreeMap::new();
        for linked in &image.get().units {
            let unit = units.remove(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            let result = MiddleEnd.optimize_shared(unit).map_err(|error| {
                LinkError::new("LNK030", format!("middle-end optimization failed: {error}"))
                    .at_source(linked.source.clone())
            })?;
            optimized.insert(linked.source.clone(), Arc::new(result));
        }
        validate_correspondence(image.get(), &optimized)?;
        Self::reconstruct_optimized(image, optimized, objects)
    }

    /// Reconstructs a persistent image using sealed, already-verified immutable unit facts.
    ///
    /// Actual semantic/object revisions come from `PreparedUnit`, never supplied digest labels.
    /// Cross-object entry/definition/call evidence is still verified for this untrusted image.
    pub fn reconstruct_prepared(
        image: LinkedImage,
        units: BTreeMap<SourceKey, PreparedUnit>,
    ) -> Result<Self, LinkError> {
        let image = ValidatedImage::new(image, "LNK001", "invalid linked image")?;
        let mut optimized = BTreeMap::new();
        for linked in &image.get().units {
            let prepared = units.get(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            if prepared.revision().kind != linked.kind
                || prepared.revision().semantic != linked.semantic_digest
            {
                return Err(LinkError::new(
                    "LNK034",
                    "linked revision differs from verified prepared payload",
                )
                .at_source(linked.source.clone()));
            }
            optimized.insert(linked.source.clone(), prepared.optimized()?);
        }
        validate_correspondence(image.get(), &optimized)?;
        Self::reconstruct_prepared_optimized(image, units)
    }

    /// Builds the linked executable from already-specialized middle-end units.
    ///
    /// Fresh compilation runs the middle end before symbol binding and retains its results
    /// here. Cache hydration uses `reconstruct` to rebuild device-local regex machinery.
    pub(crate) fn reconstruct_optimized(
        image: ValidatedImage,
        units: BTreeMap<SourceKey, Arc<OptimizedUnit>>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
    ) -> Result<Self, LinkError> {
        Self::assemble(image, units, objects, |_| None)
    }

    /// Assembles already-bound prepared payloads without rechecking a trusted linker scope.
    pub(crate) fn reconstruct_prepared_optimized(
        image: ValidatedImage,
        units: BTreeMap<SourceKey, PreparedUnit>,
    ) -> Result<Self, LinkError> {
        let mut optimized = BTreeMap::new();
        let mut objects = BTreeMap::new();
        for linked in &image.get().units {
            let prepared = units.get(&linked.source).ok_or_else(|| {
                LinkError::new("LNK002", "linked unit payload is missing")
                    .at_source(linked.source.clone())
            })?;
            optimized.insert(linked.source.clone(), prepared.optimized()?);
            objects.insert(linked.source.clone(), prepared.revision().object);
        }
        Self::assemble(image, optimized, objects, |source| {
            units
                .get(source)
                .and_then(|prepared| prepared.facts().ok().map(|facts| facts.origins.clone()))
        })
    }

    /// Publishes immutable storage while sharing prepared provenance indexes when available.
    fn assemble(
        image: ValidatedImage,
        mut units: BTreeMap<SourceKey, Arc<OptimizedUnit>>,
        objects: BTreeMap<SourceKey, ObjectDigest>,
        index_for: impl Fn(&SourceKey) -> Option<Arc<OriginIndex>>,
    ) -> Result<Self, LinkError> {
        let image = image.into_inner();
        let mut origin_indexes = Vec::with_capacity(image.units.len());
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
            origin_indexes.push(
                index_for(&linked.source).unwrap_or_else(|| Arc::new(OriginIndex::new(&unit.unit))),
            );
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
            data: Arc::new(ProgramData {
                image,
                definition_slots,
                units: ordered,
                objects: ordered_objects,
                origin_indexes,
            }),
        })
    }

    /// 返回持久化链接镜像。 / Returns the persistent linked image.
    #[must_use]
    pub fn image(&self) -> &LinkedImage {
        &self.data.image
    }
    /// Aggregates deterministic middle-end counters for opt-in end-to-end telemetry.
    #[must_use]
    pub fn optimization_stats(&self) -> OptimizationStats {
        self.data
            .units
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
        self.data
            .units
            .get(slot as usize)
            .map(|unit| unit.unit.as_ref())
    }
    pub(crate) fn optimized(&self, slot: u32) -> Option<&OptimizedUnit> {
        self.data.units.get(slot as usize).map(Arc::as_ref)
    }
    pub(crate) fn definition(&self, addr: DefAddr) -> Option<&LinkedMacroDef> {
        self.data
            .definition_slots
            .get(addr.unit_slot as usize)
            .and_then(|unit| unit.get(addr.local_def.0 as usize))
            .and_then(|index| self.data.image.definitions.get(*index))
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
            local: self
                .data
                .origin_indexes
                .get(slot as usize)?
                .get(kind, local)?,
        })
    }

    pub(crate) fn object(&self, slot: u32) -> Option<ObjectDigest> {
        self.data.objects.get(slot as usize).copied()
    }

    /// Resolves an exact object-qualified origin to its retained logical source and payload span.
    ///
    /// The local ID indexes that object's original origin table, then its source archive. No
    /// provider name, current caller source or host filesystem location is guessed. Unknown,
    /// ambiguous or out-of-range references return `None`. Spans remain half-open UTF-8 byte
    /// ranges relative to the source payload after its optional BOM, without offset shifting.
    pub fn resolve_origin(
        &self,
        origin: &QualifiedOriginRef,
    ) -> Option<(SourceKey, squish_ir::Span)> {
        let mut matches = self
            .data
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| **object == origin.object);
        let (slot, _) = matches.next()?;
        // Legacy raw reconstruction can receive asserted digest labels. Do not guess which
        // unit owns a duplicated label; prepared objects derive exact identities internally.
        if matches.next().is_some() {
            return None;
        }
        let unit = &self.data.units.get(slot)?.unit;
        let entry = unit.origins().entries.get(origin.local.0 as usize)?;
        let source = unit.sources().records.get(entry.origin.source.0 as usize)?;
        let payload_bytes = source
            .exact_bytes
            .byte_len
            .checked_sub(u64::from(source.bom_len))?;
        let span = entry.origin.span;
        if span.start > span.end || span.end > payload_bytes {
            return None;
        }
        Some((source.key.clone(), span))
    }
}

/// Verifies relationships that individual image/unit validators cannot establish alone.
///
/// A persistent image must describe these exact unit definitions, root and call sites;
/// validating its slots alone does not authorize arbitrary region IDs or rebound symbols.
fn validate_correspondence(
    image: &LinkedImage,
    units: &BTreeMap<SourceKey, Arc<OptimizedUnit>>,
) -> Result<(), LinkError> {
    let mismatch = |message| LinkError::new("LNK032", message);
    let mut expected_definitions = 0usize;
    let mut feature_bits = 0u64;
    let regex_abi = image
        .units
        .first()
        .map(|linked| &units[&linked.source].unit.header().regex_abi);
    for linked in &image.units {
        let unit = &units[&linked.source].unit;
        if unit.kind() != linked.kind || unit.header().source != linked.source {
            return Err(
                LinkError::new("LNK004", "linked unit kind/source differs from payload")
                    .at_source(linked.source.clone()),
            );
        }
        if unit.header().ir_schema != image.schema
            || unit.header().language_abi != image.language_abi
        {
            return Err(
                mismatch("linked unit schema/language ABI differs from image")
                    .at_source(linked.source.clone()),
            );
        }
        if regex_abi != Some(&unit.header().regex_abi) {
            return Err(
                LinkError::new("LNK028", "linked units have incompatible regex ABI")
                    .at_source(linked.source.clone()),
            );
        }
        expected_definitions += unit.definitions().len();
        feature_bits |= unit.header().feature_bits.0;
    }
    if image.definitions.len() != expected_definitions || image.feature_bits.0 != feature_bits {
        return Err(mismatch(
            "linked definition count or feature bits differ from payloads",
        ));
    }
    let root = image.entry.root_region;
    let root_linked = &image.units[root.unit_slot as usize];
    let root_unit = &units[&root_linked.source].unit;
    if image.entry.source != root_linked.source
        || image.entry.semantic_digest != root_linked.semantic_digest
        || root_unit.root_region() != Some(root.region)
        || root_unit.required_params() != image.entry.required_params
    {
        return Err(mismatch(
            "linked entry differs from its exact payload root or signature",
        ));
    }
    let mut symbols = BTreeMap::new();
    let mut counts = vec![0usize; image.units.len()];
    for linked in &image.definitions {
        let slot = linked.addr.unit_slot as usize;
        let source = &image.units[slot].source;
        let unit = &units[source].unit;
        let Some(definition) = unit.definitions().get(linked.addr.local_def.0 as usize) else {
            return Err(mismatch("linked definition is absent from its unit arena"));
        };
        if linked.addr.local_def.0 as usize != counts[slot] {
            return Err(LinkError::new(
                "LNK031",
                "noncontiguous definition addresses",
            ));
        }
        counts[slot] += 1;
        if linked.symbol != definition.symbol
            || linked.signature != definition.signature
            || linked.body.unit_slot != linked.addr.unit_slot
            || linked.body.region != definition.body
        {
            return Err(mismatch(
                "linked definition symbol, signature or region differs from payload",
            )
            .at_source(source.clone()));
        }
        if symbols
            .insert(
                linked.symbol.clone(),
                (linked.addr, linked.signature.clone()),
            )
            .is_some()
        {
            return Err(mismatch("linked definition symbols are duplicated"));
        }
    }
    for (slot, linked) in image.units.iter().enumerate() {
        if counts[slot] != units[&linked.source].unit.definitions().len() {
            return Err(mismatch(
                "linked image does not cover every payload definition",
            ));
        }
    }
    if image.link_map.symbols.len() != symbols.len()
        || image
            .link_map
            .symbols
            .iter()
            .any(|(symbol, addr)| symbols.get(symbol).map(|x| &x.0) != Some(addr))
    {
        return Err(mismatch(
            "linked symbol map differs from exact payload definitions",
        ));
    }
    let mut relocations = Vec::new();
    for (slot, linked) in image.units.iter().enumerate() {
        crate::linker::validate_calls(
            &linked.source,
            slot as u32,
            &units[&linked.source].unit,
            &symbols,
            &mut relocations,
        )?;
    }
    if image.link_map.relocations != relocations {
        return Err(mismatch(
            "linked relocations differ from exact payload call sites",
        ));
    }
    Ok(())
}

/// Immutable raw-unit verification capability with reusable whole-unit specialization.
///
/// Private validated storage prevents stale facts or supplied digest labels from being paired
/// with another unit. Only a reachable link scope triggers specialization; then every regex
/// in that unit, including uncalled macro patterns, is compiled before binding or execution.
#[derive(Clone, Debug)]
pub struct PreparedUnit {
    /// One shared capability; Clone never duplicates raw arenas or initialized facts.
    data: Arc<PreparedData>,
}

/// Immutable identity and one fallible session-local specialization result.
#[derive(Debug)]
struct PreparedData {
    /// Sealed persistence proof, including actual source-qualified object identity.
    validated: ValidatedUnit,
    /// Whole-unit lazy facts; errors are cached with the exact original source context.
    facts: OnceLock<Result<PreparedFacts, LinkError>>,
}

/// Reusable executable analyses independent of any entry's unit-slot assignment.
#[derive(Debug)]
struct PreparedFacts {
    /// Regex engines and source-preserving constant facts over the retained raw IR.
    optimized: Arc<OptimizedUnit>,
    /// Reusable object-local provenance lookup, never renumbered by linking.
    origins: Arc<OriginIndex>,
}

impl PreparedUnit {
    /// Validates raw IR and derives its actual canonical identities once, without specializing.
    ///
    /// Unreferenced units do not acquire new middle-end engine work. A referenced unit's
    /// complete regex pool is still checked before execution, even for uncalled macros.
    /// ```ignore
    /// let prepared = PreparedUnit::new(Arc::new(frontend_unit))?;
    /// // Independent entries bind symbols separately and share first-reachable facts.
    /// ```
    pub fn new(unit: Arc<RelocatableUnitIr>) -> Result<Self, LinkError> {
        let source = unit.header().source.clone();
        let validated = ValidatedUnit::new(unit).map_err(|error| {
            LinkError::new("LNK016", format!("invalid relocatable unit: {error}")).at_source(source)
        })?;
        Ok(Self::from_validated(validated))
    }

    /// Consumes a sealed verified decoder/encoder receipt without revalidating or rehashing IR.
    ///
    /// The receipt's private fields ensure all revisions belong to this exact unchanged
    /// allocation. No caller-supplied digest or unchecked constructor is accepted.
    pub fn from_validated(validated: ValidatedUnit) -> Self {
        Self {
            data: Arc::new(PreparedData {
                validated,
                facts: OnceLock::new(),
            }),
        }
    }

    /// Borrows the exact raw allocation covered by this validation capability.
    pub fn unit(&self) -> &Arc<RelocatableUnitIr> {
        self.data.validated.unit()
    }
    /// Returns actual kind, semantic and source-qualified object identities.
    pub fn revision(&self) -> &UnitRevision {
        self.data.validated.revision()
    }
    /// Returns initialized successful facts, or `None` before specialization or after failure.
    ///
    /// Reading these counters never triggers regex compilation; they are retained fact volume,
    /// not newly performed work for each link scope.
    pub fn stats(&self) -> Option<OptimizationStats> {
        self.data
            .facts
            .get()
            .and_then(|result| result.as_ref().ok())
            .map(|facts| facts.optimized.stats)
    }
    /// Specializes the whole verified unit once and returns its retained analysis counters.
    ///
    /// Failure is cached too. This does not bind symbols or merge independent entry scopes.
    pub fn specialize(&self) -> Result<OptimizationStats, LinkError> {
        self.facts().map(|facts| facts.optimized.stats)
    }
    /// Shares checked facts inside a new independent symbol-binding session.
    pub(crate) fn optimized(&self) -> Result<Arc<OptimizedUnit>, LinkError> {
        self.facts().map(|facts| facts.optimized.clone())
    }
    /// Initializes only the referenced unit, retaining all pool-validation errors.
    fn facts(&self) -> Result<&PreparedFacts, LinkError> {
        self.data
            .facts
            .get_or_init(|| {
                let optimized = MiddleEnd
                    .optimize_shared_validated(self.unit().clone())
                    .map_err(|error| {
                        LinkError::new("LNK030", format!("middle-end optimization failed: {error}"))
                            .at_source(self.unit().header().source.clone())
                    })?;
                let origins = Arc::new(OriginIndex::new(&optimized.unit));
                Ok(PreparedFacts {
                    optimized: Arc::new(optimized),
                    origins,
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Structural image validation witness used only inside the trusted compiler pipeline.
///
/// Its fields are private and its only constructor performs the public verifier. It removes
/// repeated full-image checks without admitting arbitrary unverified persisted images.
#[derive(Clone, Debug)]
pub(crate) struct ValidatedImage(LinkedImage);

impl ValidatedImage {
    /// Checks an image once while retaining the boundary-specific diagnostic code.
    pub(crate) fn new(
        image: LinkedImage,
        code: &'static str,
        label: &str,
    ) -> Result<Self, LinkError> {
        image
            .validate()
            .map_err(|error| LinkError::new(code, format!("{label}: {error}")))?;
        Ok(Self(image))
    }
    /// Borrows the verified immutable image for assembly and publication.
    pub(crate) fn get(&self) -> &LinkedImage {
        &self.0
    }
    /// Transfers the verified image without another structural scan.
    pub(crate) fn into_inner(self) -> LinkedImage {
        self.0
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
