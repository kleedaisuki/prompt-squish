//! Immutable, source-preserving specialization between the front end and linker.
//!
//! Portable unit IR remains the authority for IDs, diagnostics and persistence. This second
//! representation carries proven constants and reusable execution machinery without rewriting
//! the original arenas or losing the operations needed to reconstruct expansion provenance.

use regex::Regex;
use squish_ir::{MatchInput, Op, OpId, Region, RegionId, RelocatableUnitIr, Validate};
use std::{collections::BTreeMap, error::Error, fmt, ops::Range};

/// Maximum materialized bytes for one scalar fact; larger regions remain executable IR.
const MAX_SCALAR_BYTES: usize = 1024 * 1024;
/// Maximum materialized scalar bytes per unit, independent of the runtime output budget.
const MAX_UNIT_SCALAR_BYTES: usize = 8 * MAX_SCALAR_BYTES;

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
    /// Total bytes materialized in scalar facts, bounded independently of input region sharing.
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
    /// Original validated relocatable IR, preserving every source identity and arena ID.
    pub unit: RelocatableUnitIr,
    /// Proven literal regex outcomes keyed by their original operation IDs.
    pub static_matches: BTreeMap<OpId, StaticMatch>,
    /// Conservative literal-only region facts keyed by their original region IDs.
    pub static_scalars: BTreeMap<RegionId, StaticScalar>,
    /// Compiled patterns in the same dense order as the portable regex pool.
    pub regexes: Vec<Regex>,
    /// Deterministic work counters, not wall-clock timings.
    pub stats: OptimizationStats,
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
        let regexes = compile_regexes(&unit)?;
        let static_matches = literal_matches(&unit, &regexes);
        // Only scalar argument bodies consume these facts. Avoid duplicating every
        // literal prompt/module body during ordinary small-batch startup.
        let scalar_regions: std::collections::BTreeSet<_> = unit
            .ops()
            .iter()
            .flat_map(|record| {
                let Op::Call { args, .. } = &record.op else {
                    return Vec::new();
                };
                args.iter()
                    .filter_map(|arg| match arg.value {
                        squish_ir::ScalarExpr::RenderText(region) => Some(region),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let selected_regions: Vec<_> = unit
            .regions()
            .iter()
            .filter(|region| scalar_regions.contains(&region.id))
            .cloned()
            .collect();
        let static_scalars = literal_scalars(
            &selected_regions,
            unit.ops(),
            &unit.header().semantic_strings,
        );
        let stats = OptimizationStats {
            operations: unit.ops().len(),
            compiled_regexes: regexes.len(),
            static_matches: static_matches.len(),
            eliminated_branches: static_matches.values().filter(|fact| !fact.matched).count(),
            static_scalars: static_scalars.len(),
            scalar_bytes: static_scalars.values().map(|fact| fact.text.len()).sum(),
        };
        Ok(OptimizedUnit {
            unit,
            static_matches,
            static_scalars,
            regexes,
            stats,
        })
    }
}

/// Compiles the validated pool in ID order, without introducing process-specific persistent data.
fn compile_regexes(unit: &RelocatableUnitIr) -> Result<Vec<Regex>, MiddleError> {
    let header = unit.header();
    header
        .regexes
        .iter()
        .enumerate()
        .map(|(index, pattern)| {
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
fn literal_scalars(
    regions: &[Region],
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
}
