//! Frozen archive resources and provider-neutral reusable-object acquisition.

use super::*;
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use sha2::Digest as _;
use squish_backend::archive::{ArchiveLimits, SopackPayload, logical_source_path, read_sopack};
use std::io::Read;

/// Loads each immutable dependency once and freezes only syntactically declared resources.
pub(super) fn freeze_inputs(
    snapshot: &ProjectSnapshot,
    sources: &mut [FrozenSource],
) -> Result<(), ManagerError> {
    let mut archives = BTreeMap::new();
    for package in &snapshot.lockfile().expect("authoritative lock").packages {
        let squish_project::LockedSource::Sopack { path, checksum } = &package.source else {
            continue;
        };
        let bytes = read_bounded(
            &snapshot.root().join(path),
            ArchiveLimits::default().max_archive_bytes,
        )?;
        let digest = format!("sha256:{}", hex(&sha2::Sha256::digest(&bytes)));
        if digest != *checksum {
            return Err(ManagerError::new(
                "MGB140",
                Phase::Snapshot,
                format!(
                    "immutable SOPack `{}` differs from its locked checksum",
                    path.display()
                ),
            ));
        }
        let payload = read_sopack(&bytes, ArchiveLimits::default())
            .map_err(|e| error("MGB141", Phase::Snapshot, e))?;
        archives.insert(package.id.clone(), Arc::new(payload));
    }
    // The lock-derived identity is available directly from each frozen source; match exact checksum.
    for source in sources {
        let archive = snapshot
            .lockfile()
            .expect("authoritative lock")
            .packages
            .iter()
            .find_map(|p| match &p.source {
                squish_project::LockedSource::Sopack { checksum, .. }
                    if p.name == source.package.package_name
                        && source.package.canonical_source == *checksum =>
                {
                    archives.get(&p.id)
                }
                _ => None,
            });
        if let Some(archive) = archive {
            install_archive_source(source, archive.clone())?;
        } else {
            source.assets = freeze_local_assets(source)?;
        }
    }
    Ok(())
}

/// A reusable object is a declared immutable input, not a request to recompile archived XML.
pub(super) fn compile_inputs(source: &FrozenSource) -> Result<Vec<InputRef>, ManagerError> {
    let mut inputs = vec![InputRef::Blob(source.blob.digest().to_protocol())];
    if let Some(unit) = &source.precompiled {
        let bytes = encode_unit_container(unit).map_err(|e| error("MGB142", Phase::Analyze, e))?;
        inputs.push(InputRef::Blob(protocol_blake3(&bytes)));
    }
    Ok(inputs)
}

/// Rebinds diagnostic sources and resource ownership to the archive's relocated unit identity.
fn install_archive_source(
    source: &mut FrozenSource,
    archive: Arc<SopackPayload>,
) -> Result<(), ManagerError> {
    let logical = source.blob.id().path().as_str();
    let (key, unit) = archive
        .units
        .iter()
        .find(|(key, _)| logical_source_path(key).is_ok_and(|path| path == logical))
        .ok_or_else(|| {
            ManagerError::new(
                "MGB143",
                Phase::Snapshot,
                format!("archive has no compiled unit for {logical}"),
            )
        })?;
    if archive.sources.get(key).map(Vec::as_slice) != Some(source.blob.bytes()) {
        return Err(ManagerError::new(
            "MGB143",
            Phase::Snapshot,
            format!("immutable SOPack source attachment `{logical}` differs from archive bytes"),
        ));
    }
    source.precompiled = Some(unit.clone());
    source.assets = archive
        .assets
        .iter()
        .filter(|((owner, _), _)| owner == key)
        .map(|((_, path), bytes)| (path.clone(), bytes.clone()))
        .collect();
    source.archive = Some(archive);
    Ok(())
}

/// Resolves asset bytes against the defining source, rejecting package escapes including symlinks.
fn freeze_local_assets(source: &FrozenSource) -> Result<BTreeMap<String, Vec<u8>>, ManagerError> {
    let paths = declared_assets(source.blob.bytes())?;
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut root = source.blob.locator().as_path().to_path_buf();
    for _ in source.blob.id().path().as_str().split('/') {
        root.pop();
    }
    let root = root
        .canonicalize()
        .map_err(|e| error("MGB144", Phase::Snapshot, e))?;
    let directory = source
        .blob
        .locator()
        .as_path()
        .parent()
        .expect("source file parent");
    paths
        .into_iter()
        .map(|path| {
            let owner = source.blob.id().path().as_str();
            let parent = owner
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let joined = if parent.is_empty() {
                path.clone()
            } else {
                format!("{parent}/{path}")
            };
            LogicalPath::new(joined).map_err(|e| error("MGB145", Phase::Snapshot, e))?;
            if Path::new(&path).is_absolute() {
                return Err(ManagerError::new(
                    "MGB145",
                    Phase::Snapshot,
                    "asset path must be relative",
                ));
            }
            let resolved = directory
                .join(&path)
                .canonicalize()
                .map_err(|e| error("MGB146", Phase::Snapshot, e))?;
            if !resolved.starts_with(&root) {
                return Err(ManagerError::new(
                    "MGB147",
                    Phase::Snapshot,
                    format!("asset `{path}` escapes its defining package"),
                ));
            }
            let bytes = read_bounded(&resolved, ArchiveLimits::default().max_content_bytes)?;
            Ok((path, bytes))
        })
        .collect()
}

/// Lightweight namespace-aware pre-scan avoids compiling every source during planning.
fn declared_assets(bytes: &[u8]) -> Result<BTreeSet<String>, ManagerError> {
    let mut reader = NsReader::from_reader(bytes);
    let mut paths = BTreeSet::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                let (namespace, _) = reader.resolve_element(element.name());
                let is_dsl = matches!(namespace, ResolveResult::Bound(ns)
                    if ns.as_ref() == squish_xml_front::DSL_NAMESPACE.as_bytes());
                if !is_dsl || element.local_name().as_ref() != b"asset" {
                    continue;
                }
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|e| error("MGB149", Phase::Analyze, e))?;
                    if attribute.key.as_ref() == b"path" {
                        let path = attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|e| error("MGB149", Phase::Analyze, e))?;
                        paths.insert(path.into_owned());
                    }
                }
            }
            Ok(Event::Eof) => return Ok(paths),
            Err(e) => return Err(error("MGB149", Phase::Analyze, e)),
            _ => {}
        }
    }
}

/// Freezes backend semantic identity without opening adapters or observing ambient state.
pub(super) fn identity(target: &TargetBuild) -> BackendCacheIdentity {
    let mut options = b"archive-options-v1.2.0\0".to_vec();
    options.extend_from_slice(&budgets(&target.resolved).max_output_bytes.to_le_bytes());
    let backend_id = if target.resolved.backend == "pack" {
        "xmlsquish.pack"
    } else {
        "xmlsquish.sopack"
    };
    BackendCacheIdentity {
        backend_id,
        backend_version: "reproducible-stored-zip/1",
        canonical_options: options,
    }
}

/// Emits one archive from frozen resources and provider-neutral linked objects.
pub(super) fn emit(
    executor: &BuildExecutor,
    target_name: &str,
    target: &TargetBuild,
    linked: &LinkOutput,
    instantiated: &InstantiateOutput,
) -> Result<(Vec<ProducedOutput>, Vec<ActionEvent>), ManagerError> {
    if !archive_document_is_trivia(&instantiated.document) {
        return Err(ManagerError::new(
            "MGB158",
            Phase::Emit,
            "archive roots may contain only asset/include directives and formatting trivia",
        ));
    }
    let state = executor
        .state
        .lock()
        .expect("build state mutex is not poisoned");
    let resolution = state
        .linked
        .get(target_name)
        .ok_or_else(|| {
            ManagerError::new("MGB151", Phase::Emit, "archive import evidence is absent")
        })?
        .1
        .clone();
    let compiled = state.compiled.clone();
    drop(state);
    let limits = ArchiveLimits {
        max_archive_bytes: budgets(&target.resolved).max_output_bytes,
        max_content_bytes: budgets(&target.resolved).max_output_bytes,
        ..ArchiveLimits::default()
    };
    let bytes = if target.resolved.backend == "pack" {
        pack_bytes(
            executor,
            target,
            &resolution,
            &compiled,
            instantiated,
            limits,
        )?
    } else {
        sopack_bytes(executor, target, linked, &resolution, &compiled, limits)?
    };
    let debug = serde_json::to_vec(&serde_json::json!({ "schema": 1,
        "digest": protocol_blake3(&bytes).hex(), "size": bytes.len() as u64 }))
    .map_err(|e| error("MGB152", Phase::Emit, e))?;
    executor.store_blob(&bytes, "MGB153", Phase::Cache)?;
    executor.store_blob(&debug, "MGB154", Phase::Cache)?;
    let produced = vec![
        produced("prompt", product_kind(&target.resolved.backend), &bytes),
        produced("backend-result", ArtifactKind::DebugInfo, &debug),
    ];
    let output = archive_output(executor, target_name, bytes, instantiated.trace.clone());
    executor
        .state
        .lock()
        .expect("build state mutex is not poisoned")
        .backend
        .insert(target_name.into(), BackendStage { output, debug });
    Ok((produced, Vec::new()))
}

/// Restores archive output with exact byte evidence; library IR is never recompiled on cache hits.
pub(super) fn hydrate(
    executor: &BuildExecutor,
    target_name: &str,
    bytes: Vec<u8>,
    debug: Vec<u8>,
) -> Result<(), ManagerError> {
    let evidence: serde_json::Value =
        serde_json::from_slice(&debug).map_err(|e| error("MGB155", Phase::Cache, e))?;
    if evidence["schema"] != 1
        || evidence["digest"] != protocol_blake3(&bytes).hex()
        || evidence["size"] != bytes.len() as u64
    {
        return Err(ManagerError::new(
            "MGB156",
            Phase::Cache,
            "cached archive differs from emission evidence",
        ));
    }
    let trace = executor
        .state
        .lock()
        .expect("build state mutex is not poisoned")
        .instantiated
        .get(target_name)
        .ok_or_else(|| {
            ManagerError::new(
                "MGB157",
                Phase::Cache,
                "cached archive lacks its instantiated predecessor",
            )
        })?
        .1
        .trace
        .clone();
    let output = archive_output(executor, target_name, bytes, trace);
    executor
        .state
        .lock()
        .expect("build state mutex is not poisoned")
        .backend
        .insert(target_name.into(), BackendStage { output, debug });
    Ok(())
}

/// Archives have transport-level evidence, not a misleading XML artifact-byte map.
fn archive_output(
    executor: &BuildExecutor,
    target_name: &str,
    bytes: Vec<u8>,
    trace: squish_ir::ExpansionTrace,
) -> BackendOutput {
    BackendOutput {
        bytes,
        byte_map: Default::default(),
        trace,
        cache_identity: executor.prepared.backend_identities[target_name].clone(),
        metrics: Default::default(),
    }
}

/// Independently links entry members while deduplicating repeated includes within one pack.
fn pack_bytes(
    executor: &BuildExecutor,
    target: &TargetBuild,
    resolution: &ResolutionSnapshot,
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
    instantiated: &InstantiateOutput,
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ManagerError> {
    let resources: BTreeMap<_, _> = executor
        .prepared
        .sources
        .iter()
        .map(|s| (source_key(s), &s.assets))
        .collect();
    let bindings: BTreeMap<_, _> = resolution
        .imports
        .iter()
        .map(|b| ((&b.importer, b.import.0), &b.target))
        .collect();
    let mut entries = Vec::with_capacity(instantiated.directives.len());
    let mut includes = BTreeMap::new();
    for directive in &instantiated.directives {
        let entry = match directive {
            squish_link::ArchiveDirective::Asset {
                source, path, name, ..
            } => {
                let bytes = resources
                    .get(source)
                    .and_then(|assets| assets.get(path))
                    .ok_or_else(|| {
                        ManagerError::new(
                            "MGB159",
                            Phase::Emit,
                            format!("asset `{path}` is absent from the frozen defining source"),
                        )
                    })?;
                squish_backend::archive::ArchiveEntry {
                    path: name.clone(),
                    bytes: bytes.clone(),
                }
            }
            squish_link::ArchiveDirective::Include {
                source,
                import,
                name,
                ..
            } => {
                let entry = bindings.get(&(source, import.0)).ok_or_else(|| {
                    ManagerError::new("MGB160", Phase::Emit, "include binding is absent")
                })?;
                if !includes.contains_key(*entry) {
                    let bytes = include_bytes(executor, target, entry, resolution, compiled)?;
                    includes.insert((*entry).clone(), bytes);
                }
                squish_backend::archive::ArchiveEntry {
                    path: name.clone(),
                    bytes: includes[*entry].clone(),
                }
            }
        };
        entries.push(entry);
    }
    squish_backend::archive::write_reproducible_zip(&entries, limits)
        .map_err(|e| error("MGB161", Phase::Emit, e))
}

/// Include uses a fresh parameter environment rather than capturing caller macro state.
fn include_bytes(
    executor: &BuildExecutor,
    target: &TargetBuild,
    entry: &SourceKey,
    resolution: &ResolutionSnapshot,
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
) -> Result<Vec<u8>, ManagerError> {
    let linked = executor
        .runtime
        .link(entry, include_closure(entry, resolution, compiled)?)
        .map_err(|e| error("MGB162", Phase::Link, e))?;
    let instantiated = executor
        .runtime
        .instantiate(&linked.program, BTreeMap::new(), budgets(&target.resolved))
        .map_err(|e| error("MGB163", Phase::Instantiate, e))?;
    if !instantiated.directives.is_empty() {
        return Err(ManagerError::new(
            "MGB164",
            Phase::Emit,
            "included entries cannot emit archive directives",
        ));
    }
    executor
        .runtime
        .render(BackendRequest {
            document: instantiated.document,
            trace: instantiated.trace,
            options: SquishOptions {
                max_output_bytes: budgets(&target.resolved).max_output_bytes,
            },
        })
        .map(|output| output.bytes)
        .map_err(|e| error("MGB165", Phase::Emit, e))
}

/// Packages the linked library closure, preserving reusable IR and definition-relative resources.
fn sopack_bytes(
    executor: &BuildExecutor,
    target: &TargetBuild,
    linked: &LinkOutput,
    resolution: &ResolutionSnapshot,
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ManagerError> {
    let manifest = &executor
        .prepared
        .snapshot
        .manifests()
        .iter()
        .find(|m| {
            m.manifest
                .package
                .as_ref()
                .is_some_and(|p| p.name == target.package)
        })
        .ok_or_else(|| {
            ManagerError::new("MGB166", Phase::Emit, "SOPack package manifest is absent")
        })?
        .manifest;
    let package = manifest.package.as_ref().expect("selected package target");
    let reachable: BTreeSet<_> = linked
        .image
        .units
        .iter()
        .map(|u| u.source.clone())
        .collect();
    let mut payload = SopackPayload {
        package_name: package.name.clone(),
        package_version: package.version.to_string(),
        root_source: Some(target.entry.clone()),
        metadata: BTreeMap::from([(
            "manifest".into(),
            manifest
                .to_toml()
                .map_err(|e| error("MGB167", Phase::Emit, e))?,
        )]),
        ..SopackPayload::default()
    };
    for source in executor
        .prepared
        .sources
        .iter()
        .filter(|s| reachable.contains(&source_key(s)))
    {
        let key = source_key(source);
        let unit = match &compiled[&key].unit {
            RelocatableUnitIr::Module(unit) => RelocatableUnitIr::Module(unit.clone()),
            RelocatableUnitIr::Sopack(unit) => RelocatableUnitIr::Sopack(unit.clone()),
            _ => {
                return Err(ManagerError::new(
                    "MGB168",
                    Phase::Emit,
                    "SOPack cannot contain finished entry/pack units",
                ));
            }
        };
        payload
            .sources
            .insert(key.clone(), source.blob.bytes().to_vec());
        payload.units.insert(key.clone(), unit);
        for (path, bytes) in &source.assets {
            payload
                .assets
                .insert((key.clone(), path.clone()), bytes.clone());
        }
    }
    payload.imports = resolution
        .imports
        .iter()
        .filter(|b| reachable.contains(&b.importer) && reachable.contains(&b.target))
        .cloned()
        .collect();
    for (name, path) in &manifest.exports {
        let logical = LogicalPath::new(path.to_string_lossy())
            .map_err(|e| error("MGB169", Phase::Emit, e))?;
        let key = payload
            .units
            .keys()
            .find(|key| {
                matches!(key, SourceKey::Project { package: p, path }
            if p.package_name == package.name && path.join("/") == logical.as_str())
            })
            .ok_or_else(|| {
                ManagerError::new(
                    "MGB170",
                    Phase::Emit,
                    format!("export `{name}` is not in the SOPack module closure"),
                )
            })?;
        payload.exports.insert(name.clone(), key.clone());
    }
    payload
        .exports
        .entry("main".into())
        .or_insert_with(|| target.entry.clone());
    squish_backend::archive::write_sopack(&payload, limits)
        .map_err(|e| error("MGB171", Phase::Emit, e))
}

/// Bounds both metadata-declared and concurrently changing files before allocating full contents.
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ManagerError> {
    let file = std::fs::File::open(path).map_err(|e| error("MGB148", Phase::Snapshot, e))?;
    let size = file
        .metadata()
        .map_err(|e| error("MGB148", Phase::Snapshot, e))?
        .len();
    if size > limit {
        return Err(ManagerError::new(
            "MGB148",
            Phase::Snapshot,
            "archive/resource file exceeds snapshot budget",
        ));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| error("MGB148", Phase::Snapshot, e))?;
    if bytes.len() as u64 > limit {
        return Err(ManagerError::new(
            "MGB148",
            Phase::Snapshot,
            "archive/resource grew beyond snapshot budget",
        ));
    }
    Ok(bytes)
}

/// Reduces independent include work to its own transitive import closure, not every project unit.
fn include_closure(
    entry: &SourceKey,
    resolution: &ResolutionSnapshot,
    compiled: &BTreeMap<SourceKey, CompiledUnit>,
) -> Result<UnitClosure, ManagerError> {
    let mut edges = BTreeMap::<&SourceKey, Vec<&SourceKey>>::new();
    for binding in &resolution.imports {
        edges
            .entry(&binding.importer)
            .or_default()
            .push(&binding.target);
    }
    let mut reachable = BTreeSet::new();
    let mut queue = vec![entry];
    while let Some(key) = queue.pop() {
        if !reachable.insert(key.clone()) {
            continue;
        }
        if let Some(imports) = edges.get(key) {
            queue.extend(imports.iter().copied());
        }
    }
    let mut units = BTreeMap::new();
    for key in &reachable {
        let unit = compiled.get(key).ok_or_else(|| {
            ManagerError::new(
                "MGB172",
                Phase::Link,
                "include transitive unit is absent from the frozen closure",
            )
        })?;
        units.insert(key.clone(), unit.unit.clone());
    }
    Ok(UnitClosure {
        units,
        snapshot: ResolutionSnapshot {
            units: resolution
                .units
                .iter()
                .filter(|(key, _)| reachable.contains(key))
                .cloned()
                .collect(),
            imports: resolution
                .imports
                .iter()
                .filter(|b| reachable.contains(&b.importer))
                .cloned()
                .collect(),
        },
    })
}

/// Checks persistent side-channel provenance against its inseparable companion trace.
pub(super) fn validate_directives(
    directives: &[squish_link::ArchiveDirective],
    trace: &squish_ir::ExpansionTrace,
) -> Result<(), ManagerError> {
    let frames: BTreeSet<_> = trace.frames.iter().map(|frame| frame.id).collect();
    for directive in directives {
        let (name, provenance) = match directive {
            squish_link::ArchiveDirective::Asset { name, trace, .. }
            | squish_link::ArchiveDirective::Include { name, trace, .. } => (name, trace),
        };
        squish_backend::archive::validate_archive_path(name)
            .map_err(|e| error("MGB173", Phase::Cache, e))?;
        if !frames.contains(&provenance.frame) {
            return Err(ManagerError::new(
                "MGB173",
                Phase::Cache,
                "cached archive directive refers to an absent expansion frame",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_resources_follow_namespace_not_spelling() {
        let xml = format!(
            r#"<xs:pack xmlns:xs="{}"><xs:asset path="nested/a&amp;b.bin"/><foreign:asset xmlns:foreign="urn:other" path="ignored"/><xs:asset path="nested/a&amp;b.bin"/></xs:pack>"#,
            squish_xml_front::DSL_NAMESPACE
        );
        assert_eq!(
            declared_assets(xml.as_bytes()).unwrap(),
            BTreeSet::from(["nested/a&b.bin".into()])
        );
    }

    #[test]
    fn macro_body_assets_are_frozen_without_calling_the_macro() {
        let xml = format!(
            r#"<xs:sopack xmlns:xs="{}"><xs:macro name="Unused"><xs:asset path="../shared/bytes.bin"/></xs:macro></xs:sopack>"#,
            squish_xml_front::DSL_NAMESPACE
        );
        assert_eq!(
            declared_assets(xml.as_bytes()).unwrap(),
            BTreeSet::from(["../shared/bytes.bin".into()])
        );
    }

    #[test]
    fn bounded_reads_reject_declared_oversize_before_reading() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let file = dir.path().join("asset.bin");
        std::fs::write(&file, [0, 255, 7]).unwrap();
        assert!(read_bounded(&file, 2).is_err());
        assert_eq!(read_bounded(&file, 3).unwrap(), [0, 255, 7]);
    }
}

/// Formatting around member-producing macros is semantically inert, but content is not.
fn archive_document_is_trivia(document: &squish_ir::LinkedDocumentIr) -> bool {
    document.items.iter().all(|item| match item {
        squish_ir::DocumentItem::Text { value } => {
            document.strings.get(value.0 as usize).is_some_and(|text| {
                text.bytes()
                    .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            })
        }
        squish_ir::DocumentItem::Comment { .. }
        | squish_ir::DocumentItem::ProcessingInstruction { .. } => true,
        _ => false,
    })
}
