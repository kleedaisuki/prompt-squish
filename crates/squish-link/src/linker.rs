//! 冻结单元闭包的静态链接。 / Static linking of a frozen unit closure.

use crate::{LinkError, LinkedProgram, MiddleEnd};
use squish_ir::{
    DefAddr, FeatureBits, LinkedEntry, LinkedImage, LinkedMacroDef, LinkedOpRef, LinkedRegionRef,
    LinkedUnit, Op, RelocatableUnitIr, ResolutionSnapshot, Signature, SourceKey, StaticLinkMap,
    SymbolKey, Validate,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// 冻结 resolver 快照与对应语义单元。 / Frozen resolver snapshot and its semantic units.
#[derive(Clone, Debug)]
pub struct UnitClosure {
    /// Resolver 冻结的精确单元与 import 绑定。 / Exact frozen units and import bindings.
    pub snapshot: ResolutionSnapshot,
    /// 按逻辑源身份索引的语义 payload。 / Semantic payloads indexed by logical source identity.
    pub units: BTreeMap<SourceKey, RelocatableUnitIr>,
}

/// 静态链接的全部结果。 / Complete result of static linking.
#[derive(Clone, Debug)]
pub struct LinkOutput {
    /// 可持久化的静态重定位映射。 / Persistable static relocation map.
    pub map: StaticLinkMap,
    /// 可持久化的链接镜像。 / Persistable linked image.
    pub image: LinkedImage,
    /// 本会话可执行视图。 / Executable view for this session.
    pub program: LinkedProgram,
}

/// 无状态静态链接器。 / Stateless static linker.
#[derive(Clone, Copy, Debug, Default)]
pub struct StaticLinker;

impl StaticLinker {
    /// 从 entry 运算 import 闭包，允许 import 环，并对所有操作做符号与签名验证。
    /// Computes the import closure from an entry, permits import cycles, and validates every
    /// operation's symbol and signature before constructing a linked image.
    pub fn link(
        &self,
        entry: &SourceKey,
        mut closure: UnitClosure,
    ) -> Result<LinkOutput, LinkError> {
        closure
            .snapshot
            .validate()
            .map_err(|e| LinkError::new("LNK010", format!("invalid resolution snapshot: {e}")))?;
        validate_snapshot_payloads(&closure)?;
        let reachable = reachable_sources(entry, &closure)?;
        let ordered_sources: Vec<_> = reachable.into_iter().collect();
        // Explicit pre-link middle-end boundary: facts are computed on unbound units,
        // then retained through symbol binding and link-time executable indexing.
        let mut optimized_units = BTreeMap::new();
        for source in &ordered_sources {
            let optimized = MiddleEnd
                .optimize(
                    closure
                        .units
                        .remove(source)
                        .expect("validated reachable payload"),
                )
                .map_err(|error| {
                    LinkError::new("LNK030", format!("middle-end optimization failed: {error}"))
                        .at_source(source.clone())
                })?;
            optimized_units.insert(source.clone(), optimized);
        }
        let slots: BTreeMap<_, _> = ordered_sources
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, s)| (s, i as u32))
            .collect();
        let entry_unit = &optimized_units
            .get(entry)
            .ok_or_else(|| {
                LinkError::new("LNK011", "entry payload is missing").at_source(entry.clone())
            })?
            .unit;
        let Some(root_region) = entry_unit.root_region() else {
            return Err(LinkError::new(
                "LNK012",
                "link root must be an entry, pack, or sopack, not a module",
            )
            .at_source(entry.clone()));
        };

        let mut definitions = Vec::new();
        let mut symbols = BTreeMap::<SymbolKey, (DefAddr, Signature)>::new();
        for source in &ordered_sources {
            let slot = slots[source];
            add_definitions(
                source,
                slot,
                optimized_units[source].unit.definitions(),
                &mut definitions,
                &mut symbols,
            )?;
        }
        let mut relocations = Vec::new();
        for source in &ordered_sources {
            let slot = slots[source];
            validate_calls(
                source,
                slot,
                &optimized_units[source].unit,
                &symbols,
                &mut relocations,
            )?;
        }
        let map = StaticLinkMap {
            symbols: symbols.iter().map(|(k, (a, _))| (k.clone(), *a)).collect(),
            relocations,
        };
        map.validate()
            .map_err(|e| LinkError::new("LNK013", format!("constructed invalid link map: {e}")))?;
        let revision = |source: &SourceKey| {
            closure
                .snapshot
                .units
                .binary_search_by(|x| x.0.cmp(source))
                .ok()
                .map(|i| &closure.snapshot.units[i].1)
                .unwrap()
        };
        let units = ordered_sources
            .iter()
            .map(|source| {
                let r = revision(source);
                LinkedUnit {
                    kind: r.kind,
                    source: source.clone(),
                    semantic_digest: r.semantic,
                }
            })
            .collect();
        let entry_revision = revision(entry);
        let image = LinkedImage {
            schema: entry_unit.header().ir_schema,
            language_abi: entry_unit.header().language_abi.clone(),
            entry: LinkedEntry {
                source: entry.clone(),
                semantic_digest: entry_revision.semantic,
                required_params: entry_unit.required_params().to_vec(),
                root_region: LinkedRegionRef {
                    unit_slot: slots[entry],
                    region: root_region,
                },
            },
            units,
            definitions,
            link_map: map.clone(),
            feature_bits: FeatureBits(
                optimized_units
                    .values()
                    .fold(0, |bits, unit| bits | unit.unit.header().feature_bits.0),
            ),
        };
        image.validate().map_err(|e| {
            LinkError::new("LNK014", format!("constructed invalid linked image: {e}"))
        })?;
        let objects = ordered_sources
            .iter()
            .map(|source| (source.clone(), revision(source).object))
            .collect();
        let program =
            LinkedProgram::reconstruct_optimized(image.clone(), optimized_units, objects)?;
        Ok(LinkOutput {
            map,
            image,
            program,
        })
    }
}

fn validate_snapshot_payloads(closure: &UnitClosure) -> Result<(), LinkError> {
    let mut compatibility = None;
    for (source, revision) in &closure.snapshot.units {
        let unit = closure.units.get(source).ok_or_else(|| {
            LinkError::new("LNK015", "resolution unit has no payload").at_source(source.clone())
        })?;
        unit.validate().map_err(|e| {
            LinkError::new("LNK016", format!("invalid relocatable unit: {e}"))
                .at_source(source.clone())
        })?;
        let header = unit.header();
        if unit.kind() != revision.kind || header.source != *source {
            return Err(
                LinkError::new("LNK017", "snapshot kind/source differs from payload")
                    .at_source(source.clone()),
            );
        }
        let current = (
            header.ir_schema,
            header.language_abi.clone(),
            header.regex_abi.clone(),
        );
        if compatibility
            .as_ref()
            .is_some_and(|expected| expected != &current)
        {
            return Err(LinkError::new(
                "LNK028",
                "linked units have incompatible schema or language/regex ABI",
            )
            .at_source(source.clone()));
        }
        compatibility.get_or_insert(current);
    }
    Ok(())
}

fn reachable_sources(
    entry: &SourceKey,
    closure: &UnitClosure,
) -> Result<BTreeSet<SourceKey>, LinkError> {
    if !closure.snapshot.units.iter().any(|x| &x.0 == entry) {
        return Err(
            LinkError::new("LNK018", "entry is absent from resolution snapshot")
                .at_source(entry.clone()),
        );
    }
    let bindings: BTreeMap<_, _> = closure
        .snapshot
        .imports
        .iter()
        .map(|b| ((b.importer.clone(), b.import), b.target.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([entry.clone()]);
    while let Some(source) = queue.pop_front() {
        if !seen.insert(source.clone()) {
            continue;
        }
        let unit = closure.units.get(&source).ok_or_else(|| {
            LinkError::new("LNK019", "reachable payload is missing").at_source(source.clone())
        })?;
        for import in &unit.header().imports {
            let target = bindings
                .get(&(source.clone(), import.local_id))
                .ok_or_else(|| {
                    LinkError::new(
                        "LNK020",
                        format!("import #{} has no frozen binding", import.local_id.0),
                    )
                    .at_source(source.clone())
                })?;
            let target_unit = closure.units.get(target).ok_or_else(|| {
                LinkError::new("LNK021", "import target payload is missing")
                    .at_source(target.clone())
            })?;
            let expected = import.expected_kind;
            let matches_kind = match expected {
                squish_ir::UnitKind::Module => matches!(
                    target_unit.kind(),
                    squish_ir::UnitKind::Module | squish_ir::UnitKind::Sopack
                ),
                other => target_unit.kind() == other,
            };
            if !matches_kind {
                return Err(LinkError::new(
                    "LNK022",
                    "import/include target has the wrong unit kind",
                )
                .at_source(target.clone()));
            }
            // Include is a product boundary, not a symbol-aggregation edge.
            // Its entry is linked independently by the manager.
            if expected != squish_ir::UnitKind::Entry {
                queue.push_back(target.clone());
            }
        }
    }
    Ok(seen)
}

fn add_definitions(
    source: &SourceKey,
    slot: u32,
    definitions: &[squish_ir::MacroDef],
    out: &mut Vec<LinkedMacroDef>,
    symbols: &mut BTreeMap<SymbolKey, (DefAddr, Signature)>,
) -> Result<(), LinkError> {
    for def in definitions {
        let addr = DefAddr {
            unit_slot: slot,
            local_def: def.id,
        };
        if symbols
            .insert(def.symbol.clone(), (addr, def.signature.clone()))
            .is_some()
        {
            return Err(
                LinkError::new("LNK023", "duplicate macro symbol in linked closure")
                    .at_source(source.clone())
                    .at_symbol(def.symbol.clone()),
            );
        }
        out.push(LinkedMacroDef {
            addr,
            symbol: def.symbol.clone(),
            signature: def.signature.clone(),
            body: LinkedRegionRef {
                unit_slot: slot,
                region: def.body,
            },
        });
    }
    out.sort_by_key(|a| a.addr);
    Ok(())
}

fn validate_calls(
    source: &SourceKey,
    slot: u32,
    unit: &RelocatableUnitIr,
    symbols: &BTreeMap<SymbolKey, (DefAddr, Signature)>,
    relocations: &mut Vec<(LinkedOpRef, DefAddr)>,
) -> Result<(), LinkError> {
    let local_symbols: BTreeSet<_> = unit.definitions().iter().map(|def| &def.symbol).collect();
    let mut actual_externals = BTreeSet::new();
    for record in unit.ops() {
        let Op::Call {
            target,
            args,
            fills,
        } = &record.op
        else {
            continue;
        };
        let Some((addr, signature)) = symbols.get(target) else {
            return Err(LinkError::new("LNK024", "unresolved macro symbol")
                .at_source(source.clone())
                .at_symbol(target.clone()));
        };
        if !local_symbols.contains(target) {
            actual_externals.insert(target.clone());
        }
        exact_names(
            source,
            target,
            "argument",
            args.iter().map(|x| x.name.as_str()),
            signature.params.iter().map(String::as_str),
        )?;
        let actual_fills: Vec<_> = fills.iter().map(|x| x.name.as_str()).collect();
        let unique_fills: BTreeSet<_> = actual_fills.iter().copied().collect();
        if unique_fills.len() != actual_fills.len()
            || unique_fills
                .iter()
                .any(|name| !signature.slots.iter().any(|slot| slot.name == **name))
        {
            return Err(
                LinkError::new("LNK027", "fill names do not match signature")
                    .at_source(source.clone())
                    .at_symbol(target.clone()),
            );
        }
        for required in signature.slots.iter().filter(|x| x.required) {
            if !fills.iter().any(|x| x.name == required.name) {
                return Err(LinkError::new(
                    "LNK025",
                    format!("missing required fill '{}'", required.name),
                )
                .at_source(source.clone())
                .at_symbol(target.clone()));
            }
        }
        relocations.push((
            LinkedOpRef {
                unit_slot: slot,
                op: record.id,
            },
            *addr,
        ));
    }
    if !actual_externals.iter().eq(unit.external_symbols()) {
        return Err(LinkError::new(
            "LNK029",
            "external symbol summary does not match call operations",
        )
        .at_source(source.clone()));
    }
    relocations.sort_by_key(|x| x.0);
    Ok(())
}

fn exact_names<'a>(
    source: &SourceKey,
    symbol: &SymbolKey,
    kind: &str,
    actual: impl Iterator<Item = &'a str>,
    declared: impl Iterator<Item = &'a str>,
) -> Result<(), LinkError> {
    let actual: Vec<_> = actual.collect();
    let expected: BTreeSet<_> = declared.collect();
    let unique: BTreeSet<_> = actual.iter().copied().collect();
    if unique.len() != actual.len() {
        return Err(LinkError::new("LNK026", format!("duplicate {kind}"))
            .at_source(source.clone())
            .at_symbol(symbol.clone()));
    }
    if unique != expected {
        return Err(
            LinkError::new("LNK027", format!("{kind} names do not match signature"))
                .at_source(source.clone())
                .at_symbol(symbol.clone()),
        );
    }
    Ok(())
}
