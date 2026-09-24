//! 冻结单元闭包的静态链接。 / Static linking of a frozen unit closure.

use crate::{LinkError, LinkedProgram};
use squish_ir::{
    DefAddr, FeatureBits, LinkedEntry, LinkedImage, LinkedMacroDef, LinkedOpRef, LinkedRegionRef,
    LinkedUnit, ModuleObject, Op, RelocatableUnitIr, ResolutionSnapshot, Signature, SourceKey,
    StaticLinkMap, SymbolKey, UnitKind, Validate,
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
    pub fn link(&self, entry: &SourceKey, closure: UnitClosure) -> Result<LinkOutput, LinkError> {
        closure
            .snapshot
            .validate()
            .map_err(|e| LinkError::new("LNK010", format!("invalid resolution snapshot: {e}")))?;
        validate_snapshot_payloads(&closure)?;
        let reachable = reachable_sources(entry, &closure)?;
        let ordered_sources: Vec<_> = reachable.into_iter().collect();
        let slots: BTreeMap<_, _> = ordered_sources
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, s)| (s, i as u32))
            .collect();
        let entry_unit = closure.units.get(entry).ok_or_else(|| {
            LinkError::new("LNK011", "entry payload is missing").at_source(entry.clone())
        })?;
        let RelocatableUnitIr::Entry(entry_object) = entry_unit else {
            return Err(
                LinkError::new("LNK012", "link root must be an entry, not a module")
                    .at_source(entry.clone()),
            );
        };

        let mut definitions = Vec::new();
        let mut symbols = BTreeMap::<SymbolKey, (DefAddr, Signature)>::new();
        for source in &ordered_sources {
            let slot = slots[source];
            if let RelocatableUnitIr::Module(module) = &closure.units[source] {
                add_definitions(source, slot, module, &mut definitions, &mut symbols)?;
            }
        }
        let mut relocations = Vec::new();
        for source in &ordered_sources {
            let slot = slots[source];
            validate_calls(
                source,
                slot,
                &closure.units[source],
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
            schema: entry_object.header.ir_schema,
            language_abi: entry_object.header.language_abi.clone(),
            entry: LinkedEntry {
                source: entry.clone(),
                semantic_digest: entry_revision.semantic,
                required_params: entry_object.required_params.clone(),
                root_region: LinkedRegionRef {
                    unit_slot: slots[entry],
                    region: entry_object.root_region,
                },
            },
            units,
            definitions,
            link_map: map.clone(),
            feature_bits: merged_features(&ordered_sources, &closure.units),
        };
        image.validate().map_err(|e| {
            LinkError::new("LNK014", format!("constructed invalid linked image: {e}"))
        })?;
        let objects = ordered_sources
            .iter()
            .map(|source| (source.clone(), revision(source).object))
            .collect();
        // 冻结闭包中的单元与刚构造的镜像已通过验证；公开重建路径仍验证外部输入。
        // Units in the frozen closure and the constructed image are validated above;
        // the public reconstruction path still validates external inputs.
        let program = LinkedProgram::from_validated(image.clone(), closure.units, objects)?;
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
        let (kind, header) = match unit {
            RelocatableUnitIr::Entry(v) => (UnitKind::Entry, &v.header),
            RelocatableUnitIr::Module(v) => (UnitKind::Module, &v.header),
        };
        if kind != revision.kind || header.source != *source {
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
        let imports = match unit {
            RelocatableUnitIr::Entry(v) => &v.header.imports,
            RelocatableUnitIr::Module(v) => &v.header.imports,
        };
        for import in imports {
            let target = bindings
                .get(&(source.clone(), import.local_id))
                .ok_or_else(|| {
                    LinkError::new(
                        "LNK020",
                        format!("import #{} has no frozen binding", import.local_id.0),
                    )
                    .at_source(source.clone())
                })?;
            // 冻结快照是验证闭包的边界，不能让额外 payload 绕过单元验证。
            // The frozen snapshot bounds the validated closure; extra payloads cannot bypass unit validation.
            if closure
                .snapshot
                .units
                .binary_search_by(|(key, _)| key.cmp(target))
                .is_err()
            {
                return Err(LinkError::new(
                    "LNK030",
                    "import target is absent from resolution snapshot",
                )
                .at_source(target.clone()));
            }
            let target_unit = closure.units.get(target).ok_or_else(|| {
                LinkError::new("LNK021", "import target payload is missing")
                    .at_source(target.clone())
            })?;
            if !matches!(target_unit, RelocatableUnitIr::Module(_)) {
                return Err(LinkError::new("LNK022", "imports may target modules only")
                    .at_source(target.clone()));
            }
            queue.push_back(target.clone());
        }
    }
    Ok(seen)
}

fn add_definitions(
    source: &SourceKey,
    slot: u32,
    module: &ModuleObject,
    out: &mut Vec<LinkedMacroDef>,
    symbols: &mut BTreeMap<SymbolKey, (DefAddr, Signature)>,
) -> Result<(), LinkError> {
    for def in &module.definitions {
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
    let (ops, declared_externals) = match unit {
        RelocatableUnitIr::Entry(v) => (&v.ops, &v.external_symbols),
        RelocatableUnitIr::Module(v) => (&v.ops, &v.external_symbols),
    };
    let local_symbols: BTreeSet<_> = match unit {
        RelocatableUnitIr::Entry(_) => BTreeSet::new(),
        RelocatableUnitIr::Module(module) => {
            module.definitions.iter().map(|def| &def.symbol).collect()
        }
    };
    let mut actual_externals = BTreeSet::new();
    for record in ops {
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
        // IR 边界已拒绝重复 fill；这里只验证跨单元签名允许的名称。
        // IR validation rejects duplicate fills; only cross-unit signature names belong here.
        if fills
            .iter()
            .any(|fill| !signature.slots.iter().any(|slot| slot.name == fill.name))
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
    if !actual_externals.iter().eq(declared_externals) {
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

fn merged_features(
    sources: &[SourceKey],
    units: &BTreeMap<SourceKey, RelocatableUnitIr>,
) -> FeatureBits {
    FeatureBits(sources.iter().fold(0, |bits, source| {
        bits | match &units[source] {
            RelocatableUnitIr::Entry(v) => v.header.feature_bits.0,
            RelocatableUnitIr::Module(v) => v.header.feature_bits.0,
        }
    }))
}
