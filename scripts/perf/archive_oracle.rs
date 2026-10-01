//! Untimed archive compatibility oracles using only pre-repair public owned APIs.
use squish_backend::archive::{ArchiveEntry, SopackPayload, content_digest};
use squish_ir::*;
use std::collections::BTreeMap;

/// Exact transport and member bytes; called outside every sampled operation.
pub fn zip(workload: &str, dimensions: &[(&str, u64)], encoded: &[u8], members: &[ArchiveEntry]) {
    let mut canonical: Vec<_> = members
        .iter()
        .map(|entry| (&entry.path, content_digest(&entry.bytes)))
        .collect();
    canonical.sort();
    emit(
        workload,
        dimensions,
        serde_json::json!({"zip_archive_blake3":content_digest(encoded),"zip_members_blake3":content_digest(&serde_json::to_vec(&canonical).unwrap())}),
    );
}
/// Full normalized typed IR, sources, assets, exports and frozen topology.
/// Provider namespace hashes are excluded; fixtures have unique source content identities.
pub fn sopack(workload: &str, dimensions: &[(&str, u64)], payload: &SopackPayload) {
    let identities: BTreeMap<_, _> = payload
        .sources
        .iter()
        .map(|(key, bytes)| {
            (
                key.clone(),
                SourceKey::AdHoc {
                    // A canonical virtual file URI satisfies the original IR trust boundary.
                    // This content-addressed diagnostic identity never accesses the filesystem.
                    uri: format!("file:///xmlsquish-oracle/{}.xml", content_digest(bytes)),
                },
            )
        })
        .collect();
    let source_id = |key: &SourceKey| match &identities[key] {
        SourceKey::AdHoc { uri } => uri.clone(),
        _ => unreachable!(),
    };
    let bindings: BTreeMap<_, _> = payload
        .imports
        .iter()
        .map(|binding| ((&binding.importer, binding.import), &binding.target))
        .collect();
    let mut units = Vec::new();
    for (key, unit) in &payload.units {
        let mut unit = unit.clone();
        for import in &mut unit.header_mut().imports {
            import.spec = ImportSpec::RelativeUri(source_id(bindings[&(key, import.local_id)]));
        }
        unit.header_mut().source = identities[key].clone();
        unit.attachment_mut().source = identities[key].clone();
        for record in &mut unit.sources_mut().records {
            record.key = identities[&record.key].clone();
        }
        let producer = match &mut unit {
            RelocatableUnitIr::Module(unit) => &mut unit.producer,
            RelocatableUnitIr::Sopack(unit) => &mut unit.module.producer,
            _ => panic!("library unit"),
        };
        producer.tool_version = "archive-oracle-v1".into();
        producer.build_fingerprint = "stable".into();
        units.push((
            source_id(key),
            content_digest(&encode_unit_container(&unit).unwrap()),
        ));
    }
    units.sort();
    let mut sources: Vec<_> = payload
        .sources
        .iter()
        .map(|(key, bytes)| (source_id(key), content_digest(bytes)))
        .collect();
    sources.sort();
    let mut assets: Vec<_> = payload
        .assets
        .iter()
        .map(|((key, name), bytes)| (source_id(key), name, content_digest(bytes)))
        .collect();
    assets.sort();
    let mut imports: Vec<_> = payload
        .imports
        .iter()
        .map(|edge| {
            (
                source_id(&edge.importer),
                edge.import.0,
                source_id(&edge.target),
            )
        })
        .collect();
    imports.sort();
    let exports: Vec<_> = payload
        .exports
        .iter()
        .map(|(name, key)| (name, source_id(key)))
        .collect();
    let canonical = serde_json::json!({"package":payload.package_name,"version":payload.package_version,"root":payload.root_source.as_ref().map(source_id),"units":units,"sources":sources,"assets":assets,"imports":imports,"exports":exports});
    emit(
        workload,
        dimensions,
        serde_json::json!({"sopack_semantic_blake3":content_digest(&serde_json::to_vec(&canonical).unwrap())}),
    );
}
fn emit(workload: &str, dimensions: &[(&str, u64)], checksums: serde_json::Value) {
    let dimensions: BTreeMap<_, _> = dimensions.iter().copied().collect();
    println!(
        "{}",
        serde_json::json!({"schema":"xmlsquish.archive.oracle.v1","mode":"oracle","suite":"archive","workload":workload,"dimensions":dimensions,"checksums":checksums})
    );
}
