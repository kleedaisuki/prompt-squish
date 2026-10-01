//! Frozen archive resources and provider-neutral reusable-object acquisition.

use super::*;
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use sha2::Digest as _;
use squish_backend::archive::{ArchiveLimits, SharedSopackPayload, logical_source_path};
use std::io::Read;

/// Loads each immutable dependency once and freezes only syntactically declared resources.
pub(super) fn freeze_inputs(
    snapshot: &ProjectSnapshot,
    sources: &mut [FrozenSource],
    sopacks: &BTreeMap<String, Arc<crate::services::AcquiredSopack>>,
) -> Result<(), ManagerError> {
    let mut archives = BTreeMap::new();
    for package in &snapshot.lockfile().expect("authoritative lock").packages {
        let squish_project::LockedSource::Sopack { path, checksum } = &package.source else {
            continue;
        };
        let instance = squish_ir::PackageInstanceId {
            source_kind: 5,
            canonical_source: checksum.clone(),
            package_name: package.name.clone(),
            exact_revision: checksum.clone(),
        };
        let digest = frozen_archive_checksum(
            &snapshot.root().join(path),
            ArchiveLimits::default().max_archive_bytes,
        )?;
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
        let acquired = sopacks.get(&package.id).ok_or_else(|| {
            ManagerError::new(
                "MGB141",
                Phase::Snapshot,
                "immutable SOPack acquisition handle is absent",
            )
        })?;
        if acquired.checksum() != checksum {
            return Err(ManagerError::new(
                "MGB140",
                Phase::Snapshot,
                "immutable SOPack acquisition differs from its locked checksum",
            ));
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = archives.entry(instance) {
            slot.insert(FrozenArchiveIndex::new(acquired.payload().clone())?);
        }
    }
    for source in sources {
        if let Some(archive) = archives.get(&source.package) {
            install_archive_source(source, archive)?;
        } else if source.package.source_kind == 5 {
            return Err(ManagerError::new(
                "MGB143",
                Phase::Snapshot,
                "compiled archive source has no exact locked provider",
            ));
        } else {
            source.assets = freeze_local_assets(source)?;
        }
    }
    Ok(())
}

/// A reusable object is a declared immutable input, not a request to recompile archived XML.
pub(super) fn compile_inputs(source: &FrozenSource) -> Result<Vec<InputRef>, ManagerError> {
    let mut inputs = vec![InputRef::Blob(source.blob.digest().to_protocol())];
    if let Some(blob) = &source.precompiled_blob {
        inputs.push(InputRef::Blob(blob.digest().clone()));
    }
    Ok(inputs)
}

/// One immutable dependency's acquisition indexes, built once rather than per source.
struct FrozenArchiveIndex {
    /// Shared archive authority retains relocated identities and diagnostic attachments.
    archive: Arc<SharedSopackPayload>,
    /// Portable attachment paths select exact relocated units without scanning the archive.
    paths: BTreeMap<String, SourceKey>,
    /// Definition-owned bindings share bytes, including deduplicated archive blobs.
    assets: BTreeMap<SourceKey, BTreeMap<String, Arc<[u8]>>>,
}

impl FrozenArchiveIndex {
    /// Indexes validated contents once and rejects ambiguous diagnostic attachment paths.
    fn new(archive: Arc<SharedSopackPayload>) -> Result<Self, ManagerError> {
        let mut paths = BTreeMap::new();
        for key in archive.units.keys() {
            let path = logical_source_path(key).map_err(|e| error("MGB143", Phase::Snapshot, e))?;
            if paths.insert(path, key.clone()).is_some() {
                return Err(ManagerError::new(
                    "MGB143",
                    Phase::Snapshot,
                    "archive has ambiguous compiled diagnostic attachment paths",
                ));
            }
        }
        let mut assets: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
        for ((owner, path), bytes) in &archive.assets {
            assets
                .entry(owner.clone())
                .or_default()
                .insert(path.clone(), bytes.clone());
        }
        Ok(Self {
            archive,
            paths,
            assets,
        })
    }
}

/// Rebinds diagnostic sources and resource ownership to the archive's relocated unit identity.
fn install_archive_source(
    source: &mut FrozenSource,
    index: &FrozenArchiveIndex,
) -> Result<(), ManagerError> {
    let logical = source.blob.id().path().as_str();
    let key = index.paths.get(logical).ok_or_else(|| {
        ManagerError::new(
            "MGB143",
            Phase::Snapshot,
            format!("archive has no compiled unit for {logical}"),
        )
    })?;
    if index.archive.sources.get(key).map(|bytes| bytes.as_ref()) != Some(source.blob.bytes()) {
        return Err(ManagerError::new(
            "MGB143",
            Phase::Snapshot,
            format!("immutable SOPack source attachment `{logical}` differs from archive bytes"),
        ));
    }
    source.precompiled = Some(index.archive.units[key].clone());
    source.assets = index.assets.get(key).cloned().unwrap_or_default();
    source.archive = Some(index.archive.clone());
    Ok(())
}

/// Resolves asset bytes against the defining source, rejecting package escapes including symlinks.
fn freeze_local_assets(source: &FrozenSource) -> Result<BTreeMap<String, Arc<[u8]>>, ManagerError> {
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
            Ok((path, Arc::from(bytes)))
        })
        .collect()
}

/// Lightweight namespace-aware pre-scan avoids compiling every source during planning.
fn declared_assets(bytes: &[u8]) -> Result<BTreeSet<String>, ManagerError> {
    // XML element names cannot use character references; a missing literal is a safe fast path.
    if std::str::from_utf8(bytes).is_ok_and(|text| !text.contains("asset")) {
        return Ok(BTreeSet::new());
    }
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
    let include_index = if target.resolved.backend == "pack" {
        Some(IncludeIndex::new(&resolution)?)
    } else {
        None
    };
    let reachable = if let Some(index) = &include_index {
        index.included_units(&instantiated.directives)?
    } else {
        linked
            .image
            .units
            .iter()
            .map(|unit| unit.source.clone())
            .collect()
    };
    // The frozen resolution is project-wide; copy handles only for emitted child closures.
    let compiled: BTreeMap<_, _> = reachable
        .iter()
        .map(|key| {
            state
                .compiled
                .get(key)
                .cloned()
                .map(|unit| (key.clone(), unit))
                .ok_or_else(|| {
                    ManagerError::new("MGB151", Phase::Emit, "archive compiled closure is absent")
                })
        })
        .collect::<Result<_, _>>()?;
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
            include_index.as_ref().expect("pack include index"),
            &compiled,
            instantiated,
            limits,
        )?
    } else {
        sopack_bytes(executor, target, linked, &resolution, &compiled, limits)?
    };
    let product = squish_build::VerifiedBlob::from_owned(bytes);
    let debug = serde_json::to_vec(&serde_json::json!({ "schema": 1,
        "digest": product.digest().hex(), "size": product.bytes().len() as u64 }))
    .map_err(|e| error("MGB152", Phase::Emit, e))?;
    let debug = squish_build::VerifiedBlob::from_owned(debug);
    executor.store_verified(&product, "MGB153", Phase::Cache)?;
    executor.store_verified(&debug, "MGB154", Phase::Cache)?;
    let kind = product_kind(&target.resolved.backend);
    let produced = vec![
        produced_verified("prompt", kind.clone(), &product),
        produced_verified("backend-result", ArtifactKind::DebugInfo, &debug),
    ];
    executor
        .state
        .lock()
        .expect("build state mutex is not poisoned")
        .insert_backend(
            target_name.into(),
            BackendStage {
                product,
                debug,
                kind,
            },
        );
    Ok((produced, Vec::new()))
}

/// Restores archive output with exact byte evidence; library IR is never recompiled on cache hits.
pub(super) fn hydrate(
    executor: &BuildExecutor,
    target_name: &str,
    product: squish_build::VerifiedBlob,
    debug: squish_build::VerifiedBlob,
) -> Result<(), ManagerError> {
    let evidence: serde_json::Value =
        serde_json::from_slice(debug.bytes()).map_err(|e| error("MGB155", Phase::Cache, e))?;
    if evidence["schema"] != 1
        || evidence["digest"] != product.digest().hex()
        || evidence["size"] != product.bytes().len() as u64
    {
        return Err(ManagerError::new(
            "MGB156",
            Phase::Cache,
            "cached archive differs from emission evidence",
        ));
    }
    let kind = product_kind(&executor.target(target_name)?.resolved.backend);
    executor
        .state
        .lock()
        .expect("build state mutex is not poisoned")
        .insert_backend(
            target_name.into(),
            BackendStage {
                product,
                debug,
                kind,
            },
        );
    Ok(())
}

/// Independently links entry members while deduplicating repeated includes within one pack.
fn pack_bytes(
    executor: &BuildExecutor,
    target: &TargetBuild,
    include_index: &IncludeIndex<'_>,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
    instantiated: &InstantiateOutput,
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ManagerError> {
    let resources: BTreeMap<_, _> = executor
        .prepared
        .sources
        .iter()
        .map(|s| (source_key(s), &s.assets))
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
                (name.as_str(), Arc::clone(bytes))
            }
            squish_link::ArchiveDirective::Include {
                source,
                import,
                name,
                ..
            } => {
                let entry = include_index
                    .bindings
                    .get(&(source, import.0))
                    .ok_or_else(|| {
                        ManagerError::new("MGB160", Phase::Emit, "include binding is absent")
                    })?;
                let bytes =
                    match includes.entry((*entry).clone()) {
                        std::collections::btree_map::Entry::Occupied(value) => value.into_mut(),
                        std::collections::btree_map::Entry::Vacant(value) => value.insert(Arc::<
                            [u8],
                        >::from(
                            include_bytes(executor, target, entry, include_index, compiled)?,
                        )),
                    };
                (name.as_str(), Arc::clone(bytes))
            }
        };
        entries.push(entry);
    }
    let borrowed: Vec<_> = entries
        .iter()
        .map(|(path, bytes)| squish_backend::archive::ArchiveEntryRef {
            path,
            bytes: bytes.as_ref(),
        })
        .collect();
    squish_backend::archive::write_reproducible_zip_ref(&borrowed, limits)
        .map_err(|e| error("MGB161", Phase::Emit, e))
}

/// Include uses a fresh parameter environment rather than capturing caller macro state.
fn include_bytes(
    executor: &BuildExecutor,
    target: &TargetBuild,
    entry: &SourceKey,
    index: &IncludeIndex<'_>,
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
) -> Result<Vec<u8>, ManagerError> {
    let linked = executor
        .runtime
        .link_prepared(entry, index.closure(entry, compiled)?)
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
    compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
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
    let package_version = package.version.to_string();
    let metadata = BTreeMap::from([(
        "manifest".into(),
        manifest
            .to_toml()
            .map_err(|e| error("MGB167", Phase::Emit, e))?,
    )]);
    let mut payload = squish_backend::archive::SopackPayloadRef {
        package_name: &package.name,
        package_version: &package_version,
        root_source: Some(&target.entry),
        metadata: &metadata,
        ..Default::default()
    };
    for source in executor
        .prepared
        .sources
        .iter()
        .filter(|s| reachable.contains(&source_key(s)))
    {
        let key = source_key(source);
        let unit = compiled[&key].unit.as_ref();
        if !matches!(
            unit,
            RelocatableUnitIr::Module(_) | RelocatableUnitIr::Sopack(_)
        ) {
            return Err(ManagerError::new(
                "MGB168",
                Phase::Emit,
                "SOPack cannot contain finished entry/pack units",
            ));
        }
        payload.sources.insert(key.clone(), source.blob.bytes());
        payload.units.insert(key.clone(), unit);
        for (path, bytes) in &source.assets {
            payload
                .assets
                .insert((key.clone(), path.clone()), bytes.as_ref());
        }
    }
    payload.imports = resolution
        .imports
        .iter()
        .filter(|b| reachable.contains(&b.importer) && reachable.contains(&b.target))
        .cloned()
        .collect();
    let SourceKey::Project { package: owner, .. } = &target.entry else {
        return Err(ManagerError::new(
            "MGB170",
            Phase::Emit,
            "SOPack packaging root has no exact package identity",
        ));
    };
    let exported_paths: BTreeMap<_, _> = payload
        .units
        .keys()
        .filter_map(|key| match key {
            SourceKey::Project { package, path } if package == owner => Some((path.join("/"), key)),
            _ => None,
        })
        .collect();
    for (name, path) in &manifest.exports {
        let logical = LogicalPath::new(path.to_string_lossy())
            .map_err(|e| error("MGB169", Phase::Emit, e))?;
        let key = exported_paths.get(logical.as_str()).ok_or_else(|| {
            ManagerError::new(
                "MGB170",
                Phase::Emit,
                format!("export `{name}` is not in the SOPack module closure"),
            )
        })?;
        payload.exports.insert(name.clone(), (*key).clone());
    }
    payload
        .exports
        .entry("main".into())
        .or_insert_with(|| target.entry.clone());
    squish_backend::archive::write_sopack_ref(&payload, limits)
        .map_err(|e| error("MGB171", Phase::Emit, e))
}

/// Rechecks archive drift with bounded streaming hashing, never allocating another archive body.
fn frozen_archive_checksum(path: &Path, limit: u64) -> Result<String, ManagerError> {
    let file = std::fs::File::open(path).map_err(|e| error("MGB148", Phase::Snapshot, e))?;
    if file
        .metadata()
        .map_err(|e| error("MGB148", Phase::Snapshot, e))?
        .len()
        > limit
    {
        return Err(ManagerError::new(
            "MGB148",
            Phase::Snapshot,
            "archive/resource file exceeds snapshot budget",
        ));
    }
    let mut reader = file.take(limit.saturating_add(1));
    let mut hash = sha2::Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| error("MGB148", Phase::Snapshot, e))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err(ManagerError::new(
                "MGB148",
                Phase::Snapshot,
                "archive/resource file grew beyond snapshot budget",
            ));
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("sha256:{}", hex(&hash.finalize())))
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

/// Immutable metadata for one source in a shared per-pack include graph.
struct IncludeNode<'a> {
    /// Exact revision evidence remains paired with the corresponding compiled object.
    revision: &'a UnitRevision,
    /// Borrowed outgoing bindings retain their original import IDs and source identities.
    imports: Vec<&'a ImportBinding>,
}

/// Indexes a validated frozen snapshot once, avoiding a full project scan for each include.
struct IncludeIndex<'a> {
    /// Source-neutral unit and binding metadata; no payload is copied while indexing.
    nodes: BTreeMap<&'a SourceKey, IncludeNode<'a>>,
    /// Exact import IDs locate include roots without rescanning each owner's edges.
    bindings: BTreeMap<(&'a SourceKey, u32), &'a SourceKey>,
}

impl<'a> IncludeIndex<'a> {
    /// Groups exact revisions and outgoing edges without weakening linker validation.
    fn new(snapshot: &'a ResolutionSnapshot) -> Result<Self, ManagerError> {
        let mut nodes: BTreeMap<_, _> = snapshot
            .units
            .iter()
            .map(|(key, revision)| {
                (
                    key,
                    IncludeNode {
                        revision,
                        imports: Vec::new(),
                    },
                )
            })
            .collect();
        for binding in &snapshot.imports {
            let node = nodes.get_mut(&binding.importer).ok_or_else(|| {
                ManagerError::new(
                    "MGB172",
                    Phase::Link,
                    "include binding owner is absent from the frozen snapshot",
                )
            })?;
            node.imports.push(binding);
        }
        let bindings = snapshot
            .imports
            .iter()
            .map(|binding| ((&binding.importer, binding.import.0), &binding.target))
            .collect();
        Ok(Self { nodes, bindings })
    }

    /// Selects the union of independently included closures, excluding unrelated project units.
    fn included_units(
        &self,
        directives: &[squish_link::ArchiveDirective],
    ) -> Result<BTreeSet<SourceKey>, ManagerError> {
        let mut roots = BTreeSet::new();
        for directive in directives {
            if let squish_link::ArchiveDirective::Include { source, import, .. } = directive {
                let root = self.bindings.get(&(source, import.0)).ok_or_else(|| {
                    ManagerError::new("MGB160", Phase::Emit, "include binding is absent")
                })?;
                roots.insert(*root);
            }
        }
        let mut units = BTreeSet::new();
        for root in roots {
            units.extend(self.project(root)?.units.into_iter().map(|(key, _)| key));
        }
        Ok(units)
    }

    /// Copies only reachable evidence, preserving canonical ordering and legal import cycles.
    fn project(&self, entry: &SourceKey) -> Result<ResolutionSnapshot, ManagerError> {
        let mut visited = BTreeSet::new();
        let mut pending = vec![entry];
        let mut units = Vec::new();
        let mut imports = Vec::new();
        while let Some(key) = pending.pop() {
            if !visited.insert(key) {
                continue;
            }
            let node = self.nodes.get(key).ok_or_else(|| {
                ManagerError::new(
                    "MGB172",
                    Phase::Link,
                    "include transitive unit is absent from the frozen snapshot",
                )
            })?;
            units.push((key.clone(), node.revision.clone()));
            for binding in &node.imports {
                pending.push(&binding.target);
                imports.push((*binding).clone());
            }
        }
        units.sort_by(|a, b| a.0.cmp(&b.0));
        imports.sort_by(|a, b| (&a.importer, a.import.0).cmp(&(&b.importer, b.import.0)));
        Ok(ResolutionSnapshot { units, imports })
    }

    /// Shares child payloads only; independent entries never share an executable symbol scope.
    fn closure(
        &self,
        entry: &SourceKey,
        compiled: &BTreeMap<SourceKey, Arc<CompiledUnit>>,
    ) -> Result<squish_link::PreparedUnitClosure, ManagerError> {
        let snapshot = self.project(entry)?;
        let mut units = BTreeMap::new();
        for (key, _) in &snapshot.units {
            let unit = compiled.get(key).ok_or_else(|| {
                ManagerError::new(
                    "MGB172",
                    Phase::Link,
                    "include transitive compiled unit is absent from the frozen closure",
                )
            })?;
            units.insert(key.clone(), unit.prepared.clone());
        }
        Ok(squish_link::PreparedUnitClosure {
            units,
            snapshot: Arc::new(snapshot),
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_index_shares_exact_objects_and_assets_and_rejects_attachment_drift() {
        let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let directory = tempfile::tempdir_in(scratch).unwrap();
        let file = directory.path().join("module.xml");
        let xml = format!(
            r#"<xs:module xmlns:xs="{}"/>"#,
            squish_xml_front::DSL_NAMESPACE
        );
        std::fs::write(&file, &xml).unwrap();
        let mut builder = squish_source::SnapshotBuilder::new(squish_source::FileSourceProvider);
        let blob = builder
            .load(
                squish_source::SourceId::new(
                    squish_source::PackageId::new("demo").unwrap(),
                    LogicalPath::new("module.xml").unwrap(),
                ),
                squish_source::SourceLocator::file(file),
            )
            .unwrap();
        let package = squish_ir::PackageInstanceId {
            source_kind: 5,
            canonical_source: "sha256:locked".into(),
            package_name: "demo".into(),
            exact_revision: "sha256:locked".into(),
        };
        let context = squish_xml_front::FrontendSourceContext::new(package.clone());
        let unit = Arc::new(squish_xml_front::compile(&blob, &context).unwrap().unit);
        let key = unit.header().source.clone();
        let validated = squish_ir::ValidatedUnit::new(unit.clone()).unwrap();
        let bytes = validated.encode().unwrap();
        let compiled = BTreeMap::from([(
            key.clone(),
            Arc::new(CompiledUnit {
                unit: unit.clone(),
                prepared: squish_link::PreparedUnit::from_validated(validated.clone()),
                object: validated.revision().object,
                semantic: validated.revision().semantic,
                debug: validated.debug_digest(),
                bytes: squish_build::VerifiedBlob::from_owned(bytes),
            }),
        )]);
        let resolution = ResolutionSnapshot {
            units: vec![(
                key.clone(),
                UnitRevision {
                    kind: UnitKind::Module,
                    object: compiled[&key].object,
                    semantic: compiled[&key].semantic,
                },
            )],
            imports: Vec::new(),
        };
        let includes = IncludeIndex::new(&resolution).unwrap();
        let first = includes.closure(&key, &compiled).unwrap();
        let second = includes.closure(&key, &compiled).unwrap();
        assert!(Arc::ptr_eq(first.units[&key].unit(), &unit));
        assert!(Arc::ptr_eq(second.units[&key].unit(), &unit));
        assert_eq!(*first.snapshot, resolution);
        let asset: Arc<[u8]> = Arc::from([0u8, 255, 7]);
        let archive = Arc::new(SharedSopackPayload {
            units: BTreeMap::from([(key.clone(), unit.clone())]),
            sources: BTreeMap::from([(key.clone(), Arc::from(blob.bytes()))]),
            assets: BTreeMap::from([
                ((key.clone(), "a.bin".into()), asset.clone()),
                ((key.clone(), "b.bin".into()), asset.clone()),
            ]),
            ..Default::default()
        });
        let index = FrozenArchiveIndex::new(archive.clone()).unwrap();
        let mut source = FrozenSource {
            blob,
            package,
            precompiled: None,
            precompiled_blob: None,
            precompiled_validated: None,
            assets: BTreeMap::new(),
            archive: None,
        };
        install_archive_source(&mut source, &index).unwrap();
        assert!(source.precompiled_blob.is_none());
        // Planning seals encoded bytes only after immutable reachability pruning.
        let proof =
            squish_ir::ValidatedUnit::new(source.precompiled.as_ref().unwrap().clone()).unwrap();
        source.precompiled_blob = Some(squish_build::VerifiedBlob::from_owned(
            proof.encode().unwrap(),
        ));
        source.precompiled_validated = Some(proof);
        let frozen_blob = source.precompiled_blob.as_ref().unwrap();
        assert_eq!(frozen_blob.bytes(), compiled[&key].bytes.bytes());
        assert_eq!(
            compile_inputs(&source).unwrap(),
            vec![
                InputRef::Blob(source.blob.digest().to_protocol()),
                InputRef::Blob(frozen_blob.digest().clone()),
            ]
        );
        assert!(Arc::ptr_eq(source.precompiled.as_ref().unwrap(), &unit));
        assert!(Arc::ptr_eq(&source.assets["a.bin"], &asset));
        assert!(Arc::ptr_eq(&source.assets["b.bin"], &asset));
        assert!(Arc::ptr_eq(source.archive.as_ref().unwrap(), &archive));
        let mut corrupt = (*archive).clone();
        corrupt
            .sources
            .insert(key.clone(), Arc::from(b"changed".as_slice()));
        let corrupt = FrozenArchiveIndex::new(Arc::new(corrupt)).unwrap();
        assert_eq!(
            install_archive_source(&mut source, &corrupt)
                .unwrap_err()
                .code(),
            "MGB143"
        );
        let mut ambiguous = (*archive).clone();
        let mut other = key;
        if let SourceKey::Project { package, .. } = &mut other {
            package.exact_revision = "different-provider".into();
        }
        ambiguous.units.insert(other, unit);
        assert!(FrozenArchiveIndex::new(Arc::new(ambiguous)).is_err());
    }

    #[test]
    fn shared_include_index_projects_independent_cycles_and_retains_exact_evidence() {
        let key = |name: &str| SourceKey::AdHoc {
            uri: format!("test:{name}"),
        };
        let left = key("entry-left");
        let right = key("entry-right");
        let a = key("module-a");
        let b = key("module-b");
        let c = key("module-c");
        let rows = [&left, &right, &a, &b, &c]
            .into_iter()
            .map(|source| {
                let bytes = format!("{source:?}");
                (
                    source.clone(),
                    UnitRevision {
                        kind: if source == &left || source == &right {
                            UnitKind::Entry
                        } else {
                            UnitKind::Module
                        },
                        semantic: squish_ir::SemanticUnitDigest::of(bytes.as_bytes()),
                        object: ObjectDigest::of(bytes.as_bytes()),
                    },
                )
            })
            .collect();
        let binding = |from: &SourceKey, to: &SourceKey| ImportBinding {
            importer: from.clone(),
            import: squish_ir::ImportId(0),
            target: to.clone(),
        };
        let snapshot = ResolutionSnapshot {
            units: rows,
            imports: vec![
                binding(&left, &a),
                binding(&right, &c),
                binding(&a, &b),
                binding(&b, &a),
            ],
        };
        let index = IncludeIndex::new(&snapshot).unwrap();
        // Selection only consumes the frozen binding; provenance is validated by instantiation.
        let include = squish_link::ArchiveDirective::Include {
            source: left.clone(),
            import: squish_ir::ImportId(0),
            path: "entry.xml".into(),
            name: "entry.prompt".into(),
            trace: squish_ir::TraceRef {
                producer_op: squish_ir::LinkedOpRef {
                    unit_slot: 0,
                    op: squish_ir::OpId(0),
                },
                frame: squish_ir::FrameId(0),
                definition_origin: squish_ir::QualifiedOriginRef {
                    object: ObjectDigest::of(b"entry"),
                    local: squish_ir::OriginId(0),
                },
                call_origin: None,
                substitution_chain: Vec::new(),
            },
        };
        assert!(index.included_units(&[]).unwrap().is_empty());
        assert_eq!(
            index.included_units(&[include.clone(), include]).unwrap(),
            BTreeSet::from([a.clone(), b.clone()])
        );
        let left_projection = index.project(&left).unwrap();
        let right_projection = index.project(&right).unwrap();
        assert_eq!(
            left_projection
                .units
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            vec![&left, &a, &b]
        );
        assert_eq!(
            right_projection
                .units
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            vec![&right, &c]
        );
        assert_eq!(
            left_projection.imports,
            vec![binding(&left, &a), binding(&a, &b), binding(&b, &a)]
        );
        assert_eq!(right_projection.imports, vec![binding(&right, &c)]);
        for (source, revision) in &left_projection.units {
            assert_eq!(
                snapshot
                    .units
                    .iter()
                    .find(|(key, _)| key == source)
                    .unwrap()
                    .1,
                *revision
            );
        }
        assert_eq!(index.project(&left).unwrap(), left_projection);
        assert_eq!(index.nodes.len(), 5);
        assert!(index.closure(&left, &BTreeMap::new()).is_err());
    }

    #[test]
    fn shared_include_index_rejects_unknown_roots_and_dangling_targets() {
        let source = SourceKey::AdHoc {
            uri: "test:entry".into(),
        };
        let missing = SourceKey::AdHoc {
            uri: "test:missing".into(),
        };
        let revision = UnitRevision {
            kind: UnitKind::Entry,
            semantic: squish_ir::SemanticUnitDigest::of(b"entry"),
            object: ObjectDigest::of(b"entry"),
        };
        let mut snapshot = ResolutionSnapshot {
            units: vec![(source.clone(), revision)],
            imports: vec![ImportBinding {
                importer: source.clone(),
                import: squish_ir::ImportId(0),
                target: missing.clone(),
            }],
        };
        let index = IncludeIndex::new(&snapshot).unwrap();
        assert!(index.project(&source).is_err());
        assert!(index.project(&missing).is_err());
        snapshot.imports[0].importer = missing;
        assert!(IncludeIndex::new(&snapshot).is_err());
    }

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
        assert!(frozen_archive_checksum(&file, 2).is_err());
        let expected = format!("sha256:{}", hex(&sha2::Sha256::digest([0, 255, 7])));
        assert_eq!(frozen_archive_checksum(&file, 3).unwrap(), expected);
        std::fs::write(&file, [0, 254, 7]).unwrap();
        assert_ne!(frozen_archive_checksum(&file, 3).unwrap(), expected);
        std::fs::remove_file(&file).unwrap();
        assert!(frozen_archive_checksum(&file, 3).is_err());
    }
}
