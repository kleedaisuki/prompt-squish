//! Prunes immutable dependency compile work without weakening local or archive validation.

use super::*;
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};

/// Retains every local source and only the immutable units selected by their package imports.
///
/// All archive contents and diagnostic attachments must already have been validated. This
/// optimization changes scheduled compilation, not validation of the reusable library boundary.
/// Local sources stay global and strict; archived XML is never scanned or recompiled.
pub(super) fn prune(
    snapshot: &ProjectSnapshot,
    locations: &[squish_repository::PackageLocation],
    sources: &mut Vec<FrozenSource>,
) -> Result<(), ManagerError> {
    let packages = snapshot
        .resolved_packages(locations)
        .map_err(|e| error("MGB049", Phase::Snapshot, e))?;
    let instances: BTreeMap<_, _> = packages
        .iter()
        .map(|package| (&package.instance, package.lock_id.as_str()))
        .collect();
    let lock = snapshot.lockfile().expect("authoritative lock");
    let locked: BTreeMap<_, _> = lock
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect();
    let archives: BTreeMap<_, _> = sources
        .iter()
        .filter_map(|source| {
            source
                .archive
                .as_ref()
                .map(|archive| (&source.package, archive))
        })
        .collect();
    let providers: BTreeMap<_, _> = archives
        .iter()
        .filter_map(|(instance, archive)| instances.get(instance).map(|id| (*id, *archive)))
        .collect();
    let mut roots = BTreeSet::new();
    for source in sources.iter().filter(|source| source.precompiled.is_none()) {
        let Some(id) = instances.get(&source.package) else {
            continue;
        };
        let Some(package) = locked.get(id) else {
            continue;
        };
        for (alias, export) in package_imports(source.blob.bytes()) {
            let Some(dependency) = package.dependencies.get(&alias) else {
                continue;
            };
            let Some(archive) = providers.get(dependency.as_str()) else {
                continue;
            };
            if let Some(key) = archive.exports.get(&export) {
                roots.insert(key.clone());
            }
        }
    }
    let mut edges: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for archive in archives.values() {
        for binding in &archive.imports {
            edges
                .entry(&binding.importer)
                .or_default()
                .push(&binding.target);
        }
    }
    let reachable = reachable_units(roots, &edges);
    sources
        .retain(|source| source.precompiled.is_none() || reachable.contains(&source_key(source)));
    Ok(())
}

/// Traverses typed immutable bindings, preserving cycles and cross-provider unit identities.
fn reachable_units(
    roots: BTreeSet<SourceKey>,
    edges: &BTreeMap<&SourceKey, Vec<&SourceKey>>,
) -> BTreeSet<SourceKey> {
    let mut pending: Vec<_> = roots.into_iter().collect();
    let mut reachable = BTreeSet::new();
    while let Some(key) = pending.pop() {
        if !reachable.insert(key.clone()) {
            continue;
        }
        if let Some(targets) = edges.get(&key) {
            pending.extend(targets.iter().map(|target| (*target).clone()));
        }
    }
    reachable
}

/// Reads only static DSL import declarations; frontend remains the authority for syntax errors.
///
/// Every local source is subsequently compiled even when malformed. A prescan error is therefore
/// not a permissive fallback: it cannot hide a frontend error, and cannot make invalid XML succeed.
fn package_imports(bytes: &[u8]) -> BTreeSet<(String, String)> {
    let mut reader = NsReader::from_reader(bytes);
    let mut imports = BTreeSet::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                let (namespace, _) = reader.resolve_element(element.name());
                let is_dsl = matches!(namespace, ResolveResult::Bound(ns)
                    if ns.as_ref() == squish_xml_front::DSL_NAMESPACE.as_bytes());
                if !is_dsl || element.local_name().as_ref() != b"import" {
                    continue;
                }
                for attribute in element.attributes().flatten() {
                    if attribute.key.as_ref() != b"src" {
                        continue;
                    }
                    let Ok(value) = attribute.decode_and_unescape_value(reader.decoder()) else {
                        continue;
                    };
                    let Some(spec) = value.strip_prefix("pkg:") else {
                        continue;
                    };
                    if let Some((alias, export)) = spec.split_once('/') {
                        imports.insert((alias.to_owned(), export.to_owned()));
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => return imports,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_prescan_uses_namespace_src_and_decoded_export_names() {
        let xml = format!(
            r#"<xs:pack xmlns:xs="{}"><xs:import src="pkg:dep/a&amp;b"/><foreign:import xmlns:foreign="urn:other" src="pkg:wrong/main"/><xs:import src="relative.xml"/><xs:import path="pkg:wrong/main"/></xs:pack>"#,
            squish_xml_front::DSL_NAMESPACE
        );
        assert_eq!(
            package_imports(xml.as_bytes()),
            BTreeSet::from([("dep".into(), "a&b".into())])
        );
        assert!(package_imports(b"<broken").is_empty());
    }

    #[test]
    fn immutable_selection_keeps_only_seeded_cycles_and_transitive_providers() {
        let key = |name: &str| SourceKey::AdHoc { uri: name.into() };
        let a = key("archive-a/export");
        let b = key("archive-a/transitive");
        let c = key("archive-b/provider");
        let unused = key("archive-a/unused");
        let edges = BTreeMap::from([(&a, vec![&b]), (&b, vec![&a, &c]), (&unused, vec![&c])]);
        assert_eq!(
            reachable_units(BTreeSet::from([a.clone()]), &edges),
            BTreeSet::from([a.clone(), b.clone(), c.clone()])
        );
        assert!(reachable_units(BTreeSet::new(), &edges).is_empty());
    }
}
