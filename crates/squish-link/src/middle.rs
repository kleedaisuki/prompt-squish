//! Immutable, source-preserving specialization between the front end and linker.
//!
//! Portable unit IR remains the authority for IDs, diagnostics and persistence. This second
//! representation carries proven constants and reusable execution machinery without rewriting
//! the original arenas or losing the operations needed to reconstruct expansion provenance.

use regex::Regex;
use squish_ir::{MatchInput, Op, OpId, Region, RegionId, RelocatableUnitIr, Validate};
use std::{collections::BTreeMap, error::Error, fmt, ops::Range, sync::Arc};

/// Maximum materialized bytes for one scalar fact; larger regions remain executable IR.
const MAX_SCALAR_BYTES: usize = 1024 * 1024;
/// Maximum materialized scalar bytes per unit, independent of the runtime output budget.
const MAX_UNIT_SCALAR_BYTES: usize = 8 * MAX_SCALAR_BYTES;

#[cfg(test)]
thread_local! {
    /// Per-test-thread attempts count actual pattern compilation, not reused analysis records.
    static REGEX_COMPILATION_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Reads this test thread's compilation attempts; callers compare deltas without resetting peers.
#[cfg(test)]
pub(crate) fn regex_compilation_attempts() -> usize {
    REGEX_COMPILATION_ATTEMPTS.with(std::cell::Cell::get)
}

/// Work performed while specializing a unit, suitable for opt-in pipeline telemetry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizationStats {
    /// Number of operation records inspected, including operations in unselected branches.
    pub operations: usize,
    /// Number of regex pool entries compiled once for the execution session.
    pub compiled_regexes: usize,
    /// Number of literal-input regex operations evaluated before instantiation.
    pub static_matches: usize,
    /// Number of statically false branches whose bodies need not execute.
    pub eliminated_branches: usize,
    /// Number of literal-only regions materialized as scalar facts.
    pub static_scalars: usize,
    /// Logical scalar bytes charged per region, not physical bytes after execution-text sharing.
    /// This remains bounded independently of input or immutable text-storage sharing.
    pub scalar_bytes: usize,
}

/// Compile-time regex result, with byte ranges into the unchanged literal input string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticMatch {
    /// Whether the pattern matched; false is distinct from a match without captures.
    pub matched: bool,
    /// Participating declared named captures; absent optional groups are deliberately omitted.
    pub captures: BTreeMap<String, Range<usize>>,
}

/// One literal operation contributing to a constant scalar's output and provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticScalarSegment {
    /// Original operation ID, used to reconstruct trace and decoded-source mappings.
    pub op: OpId,
    /// UTF-8 byte range in the concatenated scalar, including empty literal contributions.
    pub bytes: Range<usize>,
}

/// Constant text whose operation-level source boundaries remain available to later stages.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticScalar {
    /// Exact literal concatenation; no escaping or backend whitespace policy has been applied.
    pub text: String,
    /// Ordered contributions; consumers must retain these if they bypass region execution.
    pub segments: Vec<StaticScalarSegment>,
}

/// Session-only optimized representation of a portable compilation unit.
///
/// The unchanged `unit` remains the persistable representation for SOPack and project caches.
/// Regex engine state is deliberately not serialized across devices or library versions.
#[derive(Clone, Debug)]
pub struct OptimizedUnit {
    /// Shared immutable validated IR, preserving source identities without copying payloads.
    pub unit: Arc<RelocatableUnitIr>,
    /// Proven literal regex outcomes keyed by their original operation IDs.
    pub static_matches: BTreeMap<OpId, StaticMatch>,
    /// Conservative literal-only region facts keyed by their original region IDs.
    pub static_scalars: BTreeMap<RegionId, StaticScalar>,
    /// Compiled patterns in the same dense order as the portable regex pool.
    pub regexes: Vec<Regex>,
    /// Deterministic work counters, not wall-clock timings.
    pub stats: OptimizationStats,
}

/// Internal execution facts with shared scalar bytes and independent region provenance.
#[derive(Clone, Debug)]
pub(crate) struct ExecutableUnit {
    /// Exact immutable unit payload retained by the linked program.
    pub(crate) unit: Arc<RelocatableUnitIr>,
    /// Literal decisions retain the original operation IDs.
    pub(crate) static_matches: BTreeMap<OpId, StaticMatch>,
    /// Scalar text shares storage only; each region retains its own provenance segments.
    pub(crate) static_scalars: BTreeMap<RegionId, SharedStaticScalar>,
    /// Eagerly compiled session patterns in portable pool order.
    pub(crate) regexes: Vec<Regex>,
    /// Counters charge logical scalar bytes independently of physical storage sharing.
    pub(crate) stats: OptimizationStats,
}

/// Internal scalar value separating immutable content from its occurrence-level provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedStaticScalar {
    /// Exact concatenated literal bytes, shared only for identical canonical literal sequences.
    pub(crate) text: Arc<String>,
    /// Original operations and their ranges, owned independently by each region fact.
    pub(crate) segments: Vec<StaticScalarSegment>,
}

impl From<OptimizedUnit> for ExecutableUnit {
    /// Adapts compatibility-owned facts by moving each String once, without copying its bytes.
    fn from(unit: OptimizedUnit) -> Self {
        Self {
            unit: unit.unit,
            static_matches: unit.static_matches,
            static_scalars: unit
                .static_scalars
                .into_iter()
                .map(|(id, fact)| {
                    (
                        id,
                        SharedStaticScalar {
                            text: Arc::new(fact.text),
                            segments: fact.segments,
                        },
                    )
                })
                .collect(),
            regexes: unit.regexes,
            stats: unit.stats,
        }
    }
}

/// A middle-end validation or regex-compilation failure with a stable IR field path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MiddleError {
    /// Field path identifying the malformed input or regex pool entry.
    pub path: String,
    /// Human-readable failure description.
    pub message: String,
}

impl fmt::Display for MiddleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl Error for MiddleError {}

/// Source-preserving middle end; no dependency resolution or macro invocation occurs here.
#[derive(Clone, Copy, Debug, Default)]
pub struct MiddleEnd;

impl MiddleEnd {
    /// Validates portable IR, compiles each pattern once, and computes context-free facts.
    ///
    /// All IDs, pools, interfaces, imports and source attachments remain unchanged. Dynamic
    /// bindings and calls are not evaluated: they require linked definitions and invocation
    /// arguments. In particular, callers must not discard macro frames or runtime budget checks.
    ///
    /// ```ignore
    /// let optimized = MiddleEnd.optimize(frontend_unit)?;
    /// // Persist optimized.unit; retain optimized facts for this execution session.
    /// ```
    pub fn optimize(&self, unit: RelocatableUnitIr) -> Result<OptimizedUnit, MiddleError> {
        unit.validate().map_err(|error| MiddleError {
            path: error.path,
            message: error.message.into(),
        })?;
        // Finish borrowed fact analysis before allocating ownership metadata at publication.
        // This ordering alone does not promise allocator-independent performance.
        let facts = MiddleFacts::analyze_validated(&unit, owned_scalar_facts)?;
        Ok(facts.with_unit(Arc::new(unit)))
    }

    /// Validates and specializes shared immutable IR without cloning its arenas or source data.
    ///
    /// Validation borrows the exact shared payload subsequently retained by the result.
    /// The reference count is session state and is never part of portable IR serialization.
    pub fn optimize_shared(
        &self,
        unit: Arc<RelocatableUnitIr>,
    ) -> Result<OptimizedUnit, MiddleError> {
        unit.validate().map_err(|error| MiddleError {
            path: error.path,
            message: error.message.into(),
        })?;
        self.optimize_shared_validated(unit)
    }

    /// Specializes the exact previously validated shared payload, without copying it.
    ///
    /// The caller must have validated this unchanged unit. Shared ownership is immutable;
    /// no arena mutation may occur between validation and specialization.
    pub(crate) fn optimize_shared_validated(
        &self,
        unit: Arc<RelocatableUnitIr>,
    ) -> Result<OptimizedUnit, MiddleError> {
        let facts = MiddleFacts::analyze_validated(&unit, owned_scalar_facts)?;
        Ok(facts.with_unit(unit))
    }

    /// Validates shared IR before creating execution-only shared scalar facts.
    pub(crate) fn optimize_executable_shared(
        &self,
        unit: Arc<RelocatableUnitIr>,
    ) -> Result<ExecutableUnit, MiddleError> {
        unit.validate().map_err(|error| MiddleError {
            path: error.path,
            message: error.message.into(),
        })?;
        self.optimize_executable_shared_validated(unit)
    }

    /// Creates execution facts for this exact previously validated immutable payload.
    pub(crate) fn optimize_executable_shared_validated(
        &self,
        unit: Arc<RelocatableUnitIr>,
    ) -> Result<ExecutableUnit, MiddleError> {
        let facts = MiddleFacts::analyze_validated(&unit, shared_scalar_facts)?;
        Ok(ExecutableUnit {
            unit,
            static_matches: facts.static_matches,
            static_scalars: facts.static_scalars,
            regexes: facts.regexes,
            stats: facts.stats,
        })
    }
}

/// Borrowed-analysis result independent of when the unchanged input acquires shared ownership.
struct MiddleFacts<S> {
    /// Literal regex outcomes retaining original operation IDs and capture offsets.
    static_matches: BTreeMap<OpId, StaticMatch>,
    /// Materialized scalar facts retaining operation-level provenance and original region IDs.
    static_scalars: BTreeMap<RegionId, S>,
    /// Compiled session patterns in portable regex-pool order.
    regexes: Vec<Regex>,
    /// Deterministic counters for the exact borrowed analysis.
    stats: OptimizationStats,
}

impl<S> MiddleFacts<S> {
    /// Analyzes fully validated borrowed IR without copying or changing its ownership.
    fn analyze_validated(
        unit: &RelocatableUnitIr,
        build_scalars: impl FnOnce(&RelocatableUnitIr, &[bool]) -> (BTreeMap<RegionId, S>, usize),
    ) -> Result<Self, MiddleError> {
        let regexes = compile_regexes(unit)?;
        let static_matches = literal_matches(unit, &regexes);
        // Only scalar argument bodies consume these facts. Avoid duplicating every
        // literal prompt/module body during ordinary small-batch startup.
        let scalar_regions = selected_scalar_regions(unit.ops(), unit.regions().len());
        let (static_scalars, scalar_bytes) = build_scalars(unit, &scalar_regions);
        let stats = OptimizationStats {
            operations: unit.ops().len(),
            compiled_regexes: regexes.len(),
            static_matches: static_matches.len(),
            eliminated_branches: static_matches.values().filter(|fact| !fact.matched).count(),
            static_scalars: static_scalars.len(),
            scalar_bytes,
        };
        Ok(Self {
            static_matches,
            static_scalars,
            regexes,
            stats,
        })
    }
}

impl MiddleFacts<StaticScalar> {
    /// Publishes compatibility facts with their owning payload without changing public types.
    fn with_unit(self, unit: Arc<RelocatableUnitIr>) -> OptimizedUnit {
        OptimizedUnit {
            unit,
            static_matches: self.static_matches,
            static_scalars: self.static_scalars,
            regexes: self.regexes,
            stats: self.stats,
        }
    }
}

/// Keeps compatibility-owned text storage and the original scalar materialization policy.
fn owned_scalar_facts(
    unit: &RelocatableUnitIr,
    selected: &[bool],
) -> (BTreeMap<RegionId, StaticScalar>, usize) {
    if selected.is_empty() {
        return (BTreeMap::new(), 0);
    }
    let regions = unit
        .regions()
        .iter()
        .filter(|region| selected[region.id.0 as usize]);
    let facts = literal_scalars(regions, unit.ops(), &unit.header().semantic_strings);
    let bytes = facts.values().map(|fact| fact.text.len()).sum();
    (facts, bytes)
}

/// Interns canonical pool-ID sequences, never hashing or comparing the large literal payloads.
///
/// Logical budget charges remain per region even when physical text allocation is shared.
/// The cache exists only during this unit's bounded materialization pass; facts retain its Arcs.
fn shared_scalar_facts(
    unit: &RelocatableUnitIr,
    selected: &[bool],
) -> (BTreeMap<RegionId, SharedStaticScalar>, usize) {
    if selected.is_empty() {
        return (BTreeMap::new(), 0);
    }
    let strings = &unit.header().semantic_strings;
    let mut interned: BTreeMap<Vec<squish_ir::StringId>, Arc<String>> = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut remaining = MAX_UNIT_SCALAR_BYTES;
    for region in unit
        .regions()
        .iter()
        .filter(|region| selected[region.id.0 as usize])
    {
        let Some(plan) = scalar_plan(region, unit.ops(), strings, remaining.min(MAX_SCALAR_BYTES))
        else {
            continue;
        };
        let text = interned
            .entry(plan.literals)
            .or_insert_with_key(|literals| {
                let mut text = String::with_capacity(plan.bytes);
                for literal in literals {
                    text.push_str(&strings[literal.0 as usize]);
                }
                Arc::new(text)
            });
        remaining -= plan.bytes;
        facts.insert(
            region.id,
            SharedStaticScalar {
                text: Arc::clone(text),
                segments: plan.segments,
            },
        );
    }
    (facts, MAX_UNIT_SCALAR_BYTES - remaining)
}

/// Literal sequence identity and independent region provenance, without materialized text.
struct ScalarPlan {
    /// Pool IDs identify exact literal values without hashing payload bytes.
    literals: Vec<squish_ir::StringId>,
    /// Ordered contributions from this region's own original operations.
    segments: Vec<StaticScalarSegment>,
    /// Logical concatenated length, charged even when text is reused.
    bytes: usize,
}

/// Preflights the unchanged logical limits before allocating sequence IDs or provenance.
fn scalar_plan(
    region: &Region,
    ops: &[squish_ir::OpRecord],
    strings: &[String],
    limit: usize,
) -> Option<ScalarPlan> {
    let mut bytes = 0usize;
    for id in &region.ops {
        let Op::EmitText { value } = &ops[id.0 as usize].op else {
            return None;
        };
        bytes = bytes.checked_add(strings[value.0 as usize].len())?;
        if bytes > limit {
            return None;
        }
    }
    let mut literals = Vec::with_capacity(region.ops.len());
    let mut segments = Vec::with_capacity(region.ops.len());
    let mut offset = 0;
    for id in &region.ops {
        let Op::EmitText { value } = &ops[id.0 as usize].op else {
            unreachable!();
        };
        let end = offset + strings[value.0 as usize].len();
        literals.push(*value);
        segments.push(StaticScalarSegment {
            op: *id,
            bytes: offset..end,
        });
        offset = end;
    }
    Some(ScalarPlan {
        literals,
        segments,
        bytes,
    })
}

/// Selects validated scalar arguments without per-call allocation or region arena copies.
///
/// The dense flag vector is allocated only if a RenderText argument exists. Duplicate and
/// unordered references merely set the same flag; materialization follows canonical arena order.
fn selected_scalar_regions(ops: &[squish_ir::OpRecord], region_count: usize) -> Vec<bool> {
    let mut selected = Vec::new();
    let args = ops
        .iter()
        .filter_map(|record| match &record.op {
            Op::Call { args, .. } => Some(args),
            _ => None,
        })
        .flatten();
    for arg in args {
        let squish_ir::ScalarExpr::RenderText(region) = &arg.value else {
            continue;
        };
        if selected.is_empty() {
            selected.resize(region_count, false);
        }
        selected[region.0 as usize] = true;
    }
    selected
}

/// Compiles the validated pool in ID order, without introducing process-specific persistent data.
fn compile_regexes(unit: &RelocatableUnitIr) -> Result<Vec<Regex>, MiddleError> {
    let header = unit.header();
    header
        .regexes
        .iter()
        .enumerate()
        .map(|(index, pattern)| {
            #[cfg(test)]
            REGEX_COMPILATION_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
            Regex::new(&header.semantic_strings[pattern.pattern.0 as usize]).map_err(|error| {
                MiddleError {
                    path: format!("header.regexes[{index}]"),
                    message: error.to_string(),
                }
            })
        })
        .collect()
}

/// Computes only literal-input operations; dynamic bindings are never speculated upon.
fn literal_matches(unit: &RelocatableUnitIr, regexes: &[Regex]) -> BTreeMap<OpId, StaticMatch> {
    let mut facts = BTreeMap::new();
    for record in unit.ops() {
        if let Op::MatchRegex {
            input: MatchInput::Literal(input),
            pattern,
            captures,
            ..
        } = &record.op
        {
            let text = &unit.header().semantic_strings[input.0 as usize];
            facts.insert(
                record.id,
                match_literal(&regexes[pattern.0 as usize], text, captures),
            );
        }
    }
    facts
}

/// Keeps named-capture byte offsets so later evaluation can preserve exact capture traces.
fn match_literal(regex: &Regex, text: &str, names: &[String]) -> StaticMatch {
    if names.is_empty() {
        return StaticMatch {
            matched: regex.is_match(text),
            captures: BTreeMap::new(),
        };
    }
    let Some(found) = regex.captures(text) else {
        return StaticMatch {
            matched: false,
            captures: BTreeMap::new(),
        };
    };
    let captures = names
        .iter()
        .filter_map(|name| {
            found
                .name(name)
                .map(|capture| (name.clone(), capture.range()))
        })
        .collect();
    StaticMatch {
        matched: true,
        captures,
    }
}

/// Materializes direct text regions in one flat scan, without recursion or fixpoint iteration.
///
/// Control flow is intentionally excluded even when static: retaining its provenance and capture
/// scope requires the evaluator. Bounds avoid multiplying giant shared literals into every region.
fn literal_scalars<'a>(
    regions: impl IntoIterator<Item = &'a Region>,
    ops: &[squish_ir::OpRecord],
    strings: &[String],
) -> BTreeMap<RegionId, StaticScalar> {
    let mut facts = BTreeMap::new();
    let mut remaining = MAX_UNIT_SCALAR_BYTES;
    for region in regions {
        if let Some(fact) = literal_scalar(region, ops, strings, remaining.min(MAX_SCALAR_BYTES)) {
            remaining -= fact.text.len();
            facts.insert(region.id, fact);
        }
    }
    facts
}

/// Preflights size and operation kind before allocating the scalar or its provenance segments.
fn literal_scalar(
    region: &Region,
    ops: &[squish_ir::OpRecord],
    strings: &[String],
    limit: usize,
) -> Option<StaticScalar> {
    let mut bytes = 0usize;
    for id in &region.ops {
        let Op::EmitText { value } = &ops[id.0 as usize].op else {
            return None;
        };
        bytes = bytes.checked_add(strings[value.0 as usize].len())?;
        if bytes > limit {
            return None;
        }
    }
    let mut text = String::with_capacity(bytes);
    let mut segments = Vec::with_capacity(region.ops.len());
    for id in &region.ops {
        let Op::EmitText { value } = &ops[id.0 as usize].op else {
            unreachable!();
        };
        let start = text.len();
        text.push_str(&strings[value.0 as usize]);
        segments.push(StaticScalarSegment {
            op: *id,
            bytes: start..text.len(),
        });
    }
    Some(StaticScalar { text, segments })
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_ir::{BindingRef, OpRecord, StringId};

    #[test]
    fn literal_match_retains_unicode_capture_offsets_and_missing_optional_groups() {
        let regex = Regex::new(r"(?P<word>猫)(?P<optional>x)?").unwrap();
        let fact = match_literal(&regex, "a猫b", &["word".into(), "optional".into()]);
        assert!(fact.matched);
        assert_eq!(fact.captures["word"], 1..4);
        assert!(!fact.captures.contains_key("optional"));
    }

    #[test]
    fn false_match_and_captureless_match_are_distinct() {
        let regex = Regex::new("^x$").unwrap();
        assert!(!match_literal(&regex, "y", &[]).matched);
        assert!(match_literal(&regex, "x", &[]).matched);
    }

    #[test]
    fn scalar_preserves_each_operation_including_empty_literals() {
        let ops = vec![
            OpRecord {
                id: OpId(0),
                op: Op::EmitText { value: StringId(0) },
            },
            OpRecord {
                id: OpId(1),
                op: Op::EmitText { value: StringId(1) },
            },
        ];
        let region = Region {
            id: RegionId(0),
            ops: vec![OpId(0), OpId(1), OpId(0)],
        };
        let fact = literal_scalar(&region, &ops, &["".into(), "猫".into()], 3).unwrap();
        assert_eq!(fact.text, "猫");
        assert_eq!(
            fact.segments
                .iter()
                .map(|s| (s.op, s.bytes.clone()))
                .collect::<Vec<_>>(),
            vec![(OpId(0), 0..0), (OpId(1), 0..3), (OpId(0), 3..3)]
        );
        assert!(literal_scalar(&region, &ops, &["".into(), "猫".into()], 2).is_none());
    }

    #[test]
    fn scalar_does_not_speculate_on_bindings_or_control_flow() {
        let ops = vec![OpRecord {
            id: OpId(0),
            op: Op::InsertScalar {
                value: BindingRef::Arg("x".into()),
            },
        }];
        let region = Region {
            id: RegionId(0),
            ops: vec![OpId(0)],
        };
        assert!(literal_scalar(&region, &ops, &[], MAX_SCALAR_BYTES).is_none());
        let empty = Region {
            id: RegionId(1),
            ops: vec![],
        };
        assert_eq!(literal_scalar(&empty, &ops, &[], 0).unwrap().text, "");
    }

    #[test]
    fn literal_regions_keep_original_ids_and_respect_the_aggregate_limit() {
        let text = "x".repeat(MAX_SCALAR_BYTES);
        let ops = vec![OpRecord {
            id: OpId(0),
            op: Op::EmitText { value: StringId(0) },
        }];
        let regions: Vec<_> = (0..9)
            .map(|id| Region {
                id: RegionId(id),
                ops: vec![OpId(0)],
            })
            .collect();
        let facts = literal_scalars(&regions, &ops, &[text]);
        assert_eq!(facts.len(), 8);
        assert!(!facts.contains_key(&RegionId(8)));
        assert_eq!(facts[&RegionId(3)].segments[0].op, OpId(0));
    }

    #[test]
    fn scalar_selection_deduplicates_unordered_arguments_without_changing_caps_or_facts() {
        let strings = vec!["x".repeat(MAX_SCALAR_BYTES)];
        let mut ops = vec![OpRecord {
            id: OpId(0),
            op: Op::EmitText { value: StringId(0) },
        }];
        let regions: Vec<_> = (0..11)
            .map(|id| Region {
                id: RegionId(id),
                ops: if id == 9 { vec![] } else { vec![OpId(0)] },
            })
            .collect();
        ops.push(OpRecord {
            id: OpId(1),
            op: Op::Call {
                target: squish_ir::ExpandedName {
                    namespace_uri: "urn:test".into(),
                    local_name: "scalar".into(),
                },
                args: [8, 3, 0, 8, 9, 7, 6, 5, 4, 2, 1]
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| squish_ir::Argument {
                        name: format!("arg{index}"),
                        value: squish_ir::ScalarExpr::RenderText(RegionId(id)),
                    })
                    .collect(),
                fills: vec![],
            },
        });
        let selected = selected_scalar_regions(&ops, regions.len());
        assert_eq!(selected.len(), regions.len());
        assert!(!selected[10]);
        let actual = literal_scalars(
            regions
                .iter()
                .filter(|region| selected[region.id.0 as usize]),
            &ops,
            &strings,
        );
        let expected = literal_scalars(regions.iter().take(10), &ops, &strings);
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 9);
        assert!(!actual.contains_key(&RegionId(8)));
        assert_eq!(
            actual[&RegionId(9)],
            StaticScalar {
                text: String::new(),
                segments: vec![]
            }
        );
        assert_eq!(actual[&RegionId(3)].segments[0].op, OpId(0));
    }

    #[test]
    fn scalar_selection_needs_no_flag_allocation_without_render_text_arguments() {
        let ops = vec![OpRecord {
            id: OpId(0),
            op: Op::EmitText { value: StringId(0) },
        }];
        assert!(selected_scalar_regions(&ops, 100).is_empty());
    }

    #[test]
    fn public_optimization_rejects_invalid_ir_before_trusted_indexing() {
        let mut unit = empty_unit();
        unit.header_mut().ir_schema.major = 99;
        // This dangling ID must not reach direct pool indexing.
        unit.header_mut().regexes.push(squish_ir::RegexPattern {
            pattern: StringId(99),
            named_captures: vec![],
        });
        let shared = Arc::new(unit);
        let error = MiddleEnd.optimize_shared(Arc::clone(&shared)).unwrap_err();
        assert_eq!(error.path, "header");
        assert_eq!(error.message, "unsupported schema major");
        let executable_error = MiddleEnd
            .optimize_executable_shared(Arc::clone(&shared))
            .unwrap_err();
        assert_eq!(executable_error, error);
        let owned_error = MiddleEnd
            .optimize(Arc::unwrap_or_clone(shared))
            .unwrap_err();
        assert_eq!(owned_error, error);
    }

    #[test]
    fn shared_optimization_retains_the_exact_payload_and_owned_facts() {
        let shared = Arc::new(empty_unit());
        let optimized = MiddleEnd.optimize_shared(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(&shared, &optimized.unit));
        let trusted = MiddleEnd
            .optimize_shared_validated(Arc::clone(&shared))
            .unwrap();
        assert!(Arc::ptr_eq(&shared, &trusted.unit));
        let owned = MiddleEnd.optimize(empty_unit()).unwrap();
        assert_eq!(optimized.stats, trusted.stats);
        assert_eq!(optimized.static_scalars, owned.static_scalars);
        assert_eq!(optimized.static_matches, owned.static_matches);
        assert_eq!(optimized.unit.as_ref(), owned.unit.as_ref());
    }

    #[test]
    fn owned_publication_after_analysis_preserves_large_scalar_and_regex_facts() {
        let original = literal_unit();
        let shared = Arc::new(original.clone());
        let owned = MiddleEnd.optimize(original.clone()).unwrap();
        let borrowed = MiddleEnd.optimize_shared(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(&shared, &borrowed.unit));
        assert_eq!(owned.unit.as_ref(), &original);
        assert_eq!(borrowed.unit.as_ref(), &original);
        assert_eq!(owned.static_scalars, borrowed.static_scalars);
        assert_eq!(owned.static_matches, borrowed.static_matches);
        assert_eq!(owned.stats, borrowed.stats);
        assert_eq!(owned.stats.scalar_bytes, 65536);
        assert_eq!(owned.static_scalars[&RegionId(1)].segments[0].op, OpId(2));
        assert!(owned.static_matches[&OpId(1)].matched);
        assert_eq!(owned.regexes[0].as_str(), borrowed.regexes[0].as_str());
    }

    #[test]
    fn execution_facts_share_large_text_but_keep_distinct_original_operations() {
        let mut raw = literal_unit();
        let RelocatableUnitIr::Module(unit) = &mut raw else {
            unreachable!()
        };
        unit.regions.push(Region {
            id: RegionId(3),
            ops: vec![OpId(4)],
        });
        unit.ops.push(OpRecord {
            id: OpId(4),
            op: Op::EmitText { value: StringId(1) },
        });
        let Op::Call { args, .. } = &mut unit.ops[0].op else {
            unreachable!()
        };
        args.push(squish_ir::Argument {
            name: "other".into(),
            value: squish_ir::ScalarExpr::RenderText(RegionId(3)),
        });
        let mut origin = unit.origins.entries[0].clone();
        origin.local_id = 4;
        unit.origins.entries.push(origin);
        let shared = Arc::new(raw);
        let executable = MiddleEnd
            .optimize_executable_shared(Arc::clone(&shared))
            .unwrap();
        let public = MiddleEnd.optimize_shared(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(&executable.unit, &shared));
        assert!(Arc::ptr_eq(
            &executable.static_scalars[&RegionId(1)].text,
            &executable.static_scalars[&RegionId(3)].text
        ));
        assert_eq!(
            executable.static_scalars[&RegionId(1)].segments[0].op,
            OpId(2)
        );
        assert_eq!(
            executable.static_scalars[&RegionId(3)].segments[0].op,
            OpId(4)
        );
        assert_eq!(executable.stats.scalar_bytes, 2 * 65536);
        assert_eq!(executable.stats, public.stats);
        assert_eq!(executable.static_matches, public.static_matches);
        for (id, fact) in &executable.static_scalars {
            assert_eq!(fact.text.as_ref(), &public.static_scalars[id].text);
            assert_eq!(fact.segments, public.static_scalars[id].segments);
        }
        let text_pointer = public.static_scalars[&RegionId(1)].text.as_ptr();
        let compatibility: ExecutableUnit = public.into();
        assert_eq!(
            compatibility.static_scalars[&RegionId(1)].text.as_ptr(),
            text_pointer
        );
    }

    #[test]
    fn execution_sharing_does_not_expand_the_logical_materialization_budget() {
        let RelocatableUnitIr::Module(mut raw) = empty_unit() else {
            unreachable!()
        };
        raw.header.semantic_strings = vec!["x".repeat(MAX_SCALAR_BYTES)];
        raw.ops = vec![OpRecord {
            id: OpId(0),
            op: Op::EmitText { value: StringId(0) },
        }];
        raw.regions = (0..10)
            .map(|id| Region {
                id: RegionId(id),
                ops: if id == 9 { vec![] } else { vec![OpId(0)] },
            })
            .collect();
        let raw = RelocatableUnitIr::Module(raw);
        let selected = vec![true; 10];
        let (shared, logical_bytes) = shared_scalar_facts(&raw, &selected);
        let (owned, owned_bytes) = owned_scalar_facts(&raw, &selected);
        assert_eq!(logical_bytes, MAX_UNIT_SCALAR_BYTES);
        assert_eq!(logical_bytes, owned_bytes);
        assert_eq!(shared.len(), 9);
        assert!(!shared.contains_key(&RegionId(8)));
        assert_eq!(shared[&RegionId(9)].text.as_str(), "");
        assert!(Arc::ptr_eq(
            &shared[&RegionId(0)].text,
            &shared[&RegionId(7)].text
        ));
        for (id, fact) in shared {
            assert_eq!(fact.text.as_ref(), &owned[&id].text);
            assert_eq!(fact.segments, owned[&id].segments);
        }
    }

    #[test]
    fn execution_sharing_retains_unicode_ranges_and_empty_region_identity() {
        let RelocatableUnitIr::Module(mut raw) = empty_unit() else {
            unreachable!()
        };
        raw.header.semantic_strings = vec!["".into(), "猫".into()];
        raw.ops = vec![
            OpRecord {
                id: OpId(0),
                op: Op::EmitText { value: StringId(0) },
            },
            OpRecord {
                id: OpId(1),
                op: Op::EmitText { value: StringId(1) },
            },
            OpRecord {
                id: OpId(2),
                op: Op::EmitText { value: StringId(1) },
            },
        ];
        raw.regions = vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0), OpId(1)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(0), OpId(2)],
            },
            Region {
                id: RegionId(2),
                ops: vec![],
            },
            Region {
                id: RegionId(3),
                ops: vec![],
            },
        ];
        let raw = RelocatableUnitIr::Module(raw);
        let (facts, bytes) = shared_scalar_facts(&raw, &[true; 4]);
        assert_eq!(bytes, 6);
        assert_eq!(facts[&RegionId(0)].text.as_str(), "猫");
        assert!(Arc::ptr_eq(
            &facts[&RegionId(0)].text,
            &facts[&RegionId(1)].text
        ));
        assert!(Arc::ptr_eq(
            &facts[&RegionId(2)].text,
            &facts[&RegionId(3)].text
        ));
        assert_eq!(facts[&RegionId(0)].segments[0].bytes, 0..0);
        assert_eq!(facts[&RegionId(0)].segments[1].bytes, 0..3);
        assert_eq!(facts[&RegionId(1)].segments[1].op, OpId(2));
    }

    /// Adds one large scalar and literal regex decision to a structurally valid module.
    fn literal_unit() -> RelocatableUnitIr {
        let RelocatableUnitIr::Module(mut unit) = empty_unit() else {
            unreachable!()
        };
        let symbol = squish_ir::ExpandedName {
            namespace_uri: "urn:test".into(),
            local_name: "macro".into(),
        };
        let external = squish_ir::ExpandedName {
            namespace_uri: "urn:external".into(),
            local_name: "echo".into(),
        };
        unit.header.semantic_strings = vec!["^x+$".into(), "x".repeat(65536)];
        let source_digest = squish_ir::SourceDigest::of(b"x");
        unit.sources.records[0].digest = source_digest;
        unit.sources.records[0].exact_bytes = squish_ir::BlobRef {
            digest: squish_ir::Digest::sha256("blob", b"x"),
            byte_len: 1,
        };
        unit.attachment.source_digest = source_digest;
        unit.header.regexes = vec![squish_ir::RegexPattern {
            pattern: StringId(0),
            named_captures: vec![],
        }];
        unit.definitions = vec![squish_ir::MacroDef {
            id: squish_ir::LocalDefId(0),
            symbol: symbol.clone(),
            signature: squish_ir::Signature::default(),
            body: RegionId(0),
        }];
        unit.interface.definitions = vec![squish_ir::InterfaceDef {
            id: squish_ir::LocalDefId(0),
            symbol,
            signature: squish_ir::Signature::default(),
        }];
        unit.external_symbols = vec![external.clone()];
        unit.regions = vec![
            Region {
                id: RegionId(0),
                ops: vec![OpId(0), OpId(1)],
            },
            Region {
                id: RegionId(1),
                ops: vec![OpId(2)],
            },
            Region {
                id: RegionId(2),
                ops: vec![OpId(3)],
            },
        ];
        unit.ops = vec![
            OpRecord {
                id: OpId(0),
                op: Op::Call {
                    target: external,
                    args: vec![squish_ir::Argument {
                        name: "text".into(),
                        value: squish_ir::ScalarExpr::RenderText(RegionId(1)),
                    }],
                    fills: vec![],
                },
            },
            OpRecord {
                id: OpId(1),
                op: Op::MatchRegex {
                    input: MatchInput::Literal(StringId(1)),
                    pattern: squish_ir::RegexId(0),
                    captures: vec![],
                    matched: RegionId(2),
                },
            },
            OpRecord {
                id: OpId(2),
                op: Op::EmitText { value: StringId(1) },
            },
            OpRecord {
                id: OpId(3),
                op: Op::EmitText { value: StringId(1) },
            },
        ];
        unit.origins.entries = (0..4)
            .map(|local_id| squish_ir::OriginEntry {
                entity_kind: squish_ir::EntityKind::Operation,
                local_id,
                origin: squish_ir::Origin {
                    source: squish_ir::SourceRef(0),
                    span: squish_ir::Span { start: 0, end: 1 },
                    lexical_qname: None,
                    syntax_kind: squish_ir::SyntaxKind(1),
                },
            })
            .collect();
        RelocatableUnitIr::Module(unit)
    }

    /// Constructs an empty valid module for public/shared ownership boundary tests.
    fn empty_unit() -> RelocatableUnitIr {
        let source = squish_ir::SourceKey::AdHoc {
            uri: "file:///empty.xml".into(),
        };
        let digest = squish_ir::SourceDigest::of(b"");
        RelocatableUnitIr::Module(squish_ir::ModuleObject {
            header: squish_ir::UnitHeader {
                ir_schema: squish_ir::Version { major: 1, minor: 0 },
                language_abi: squish_ir::AbiId("test".into()),
                frontend_abi: squish_ir::AbiId("test".into()),
                regex_abi: squish_ir::AbiId("test".into()),
                source: source.clone(),
                imports: vec![],
                semantic_strings: vec![],
                qnames: vec![],
                regexes: vec![],
                feature_bits: squish_ir::FeatureBits(0),
            },
            definitions: vec![],
            external_symbols: vec![],
            interface: squish_ir::InterfaceSummary::default(),
            regions: vec![],
            ops: vec![],
            origins: squish_ir::OriginTable::default(),
            sources: squish_ir::SourceArchive {
                records: vec![squish_ir::SourceRecord {
                    key: source.clone(),
                    digest,
                    bom_len: 0,
                    exact_bytes: squish_ir::BlobRef {
                        digest: squish_ir::Digest::sha256("blob", b""),
                        byte_len: 0,
                    },
                    line_start_offsets: vec![0],
                }],
            },
            attachment: squish_ir::UnitSourceAttachment {
                source,
                source_digest: digest,
                source_record: squish_ir::SourceRef(0),
            },
            producer: squish_ir::Producer {
                tool_version: "test".into(),
                build_fingerprint: "test".into(),
            },
        })
    }
}
