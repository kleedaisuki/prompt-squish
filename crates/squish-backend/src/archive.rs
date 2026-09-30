//! Canonical, bounded ZIP transport for products and relocatable SOPack libraries.

use serde::{Deserialize, Serialize};
use squish_ir::{
    ImportBinding, ImportId, RelocatableUnitIr, SourceKey, decode_unit_container,
    encode_unit_container,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

/// One regular file; paths use portable, relative UTF-8 slash separators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    /// Portable archive member name.
    pub path: String,
    /// Exact file content, including arbitrary binary data.
    pub bytes: Vec<u8>,
}
/// Hard resource bounds enforced before allocating decoded file contents.
#[derive(Clone, Copy, Debug)]
pub struct ArchiveLimits {
    /// Maximum complete ZIP byte length.
    pub max_archive_bytes: u64,
    /// Maximum accumulated file content length.
    pub max_content_bytes: u64,
    /// Maximum file count, additionally limited to classic ZIP's 65,535 members.
    pub max_entries: usize,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 256 * 1024 * 1024,
            max_content_bytes: 256 * 1024 * 1024,
            max_entries: 16_384,
        }
    }
}
/// Archive transport or semantic validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveError(pub String);
impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for ArchiveError {}
fn fail(message: impl Into<String>) -> ArchiveError {
    ArchiveError(message.into())
}

/// Validates names usable unchanged on Unix and Windows; no extraction is performed.
pub fn validate_archive_path(path: &str) -> Result<(), ArchiveError> {
    if path.is_empty()
        || path.len() > u16::MAX as usize
        || path.contains(['\\', ':'])
        || path.chars().any(|c| c.is_control())
    {
        return Err(fail("unsafe archive path"));
    }
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || matches!(part, "." | "..")
            || part.ends_with(['.', ' '])
            || part.contains(['<', '>', '"', '|', '?', '*'])
            || reserved
        {
            return Err(fail("unsafe archive path component"));
        }
    }
    Ok(())
}
fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn word(bytes: &[u8], offset: usize) -> Result<u16, ArchiveError> {
    let data = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| fail("truncated ZIP"))?;
    Ok(u16::from_le_bytes([data[0], data[1]]))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, ArchiveError> {
    let data = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| fail("truncated ZIP"))?;
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}
/// Writes classic stored ZIP with sorted names, CRC32, UTF-8, 1980-01-01 and mode 0644.
///
/// Input order is irrelevant. ZIP64, compression, directory entries, extra fields and
/// comments are deliberately absent, avoiding platform and compressor variability.
pub fn write_reproducible_zip(
    entries: &[ArchiveEntry],
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ArchiveError> {
    if entries.len() > limits.max_entries || entries.len() > u16::MAX as usize {
        return Err(fail("ZIP entry limit exceeded"));
    }
    let mut sorted: Vec<_> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut names = BTreeSet::new();
    let mut content = 0u64;
    let mut size = 22u64;
    for entry in &sorted {
        validate_archive_path(&entry.path)?;
        if !names.insert(entry.path.to_lowercase()) {
            return Err(fail("duplicate ZIP member"));
        }
        content = content
            .checked_add(entry.bytes.len() as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        size = size
            .checked_add(76 + 2 * entry.path.len() as u64 + entry.bytes.len() as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        if entry.bytes.len() > u32::MAX as usize {
            return Err(fail("ZIP64 is unsupported"));
        }
    }
    for name in &names {
        for (offset, _) in name.match_indices('/') {
            if names.contains(&name[..offset]) {
                return Err(fail("ZIP file/directory collision"));
            }
        }
    }
    if content > limits.max_content_bytes
        || size > limits.max_archive_bytes
        || size > u32::MAX as u64
    {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut out = Vec::with_capacity(size as usize);
    let mut central = Vec::new();
    for entry in sorted {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(&entry.bytes);
        put32(&mut out, 0x04034b50);
        for n in [20, 0x800, 0, 0, 33] {
            put16(&mut out, n);
        }
        put32(&mut out, crc);
        put32(&mut out, entry.bytes.len() as u32);
        put32(&mut out, entry.bytes.len() as u32);
        put16(&mut out, entry.path.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(entry.path.as_bytes());
        out.extend_from_slice(&entry.bytes);
        put32(&mut central, 0x02014b50);
        for n in [0x314, 20, 0x800, 0, 0, 33] {
            put16(&mut central, n);
        }
        put32(&mut central, crc);
        put32(&mut central, entry.bytes.len() as u32);
        put32(&mut central, entry.bytes.len() as u32);
        for n in [entry.path.len() as u16, 0, 0, 0, 0] {
            put16(&mut central, n);
        }
        put32(&mut central, 0o100644 << 16);
        put32(&mut central, offset);
        central.extend_from_slice(entry.path.as_bytes());
    }
    let offset = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, 0x06054b50);
    for n in [0, 0, entries.len() as u16, entries.len() as u16] {
        put16(&mut out, n);
    }
    put32(&mut out, central_len);
    put32(&mut out, offset);
    put16(&mut out, 0);
    Ok(out)
}
/// Reads only canonical stored ZIP; bounds and CRC are checked before accepting it.
///
/// Re-encoding is an intentional strict validation of both ZIP indexes, timestamps,
/// flags, modes, ordering and absence of hidden/trailing data. General third-party
/// ZIPs must first be normalized by a trusted producer.
pub fn read_reproducible_zip(
    bytes: &[u8],
    limits: ArchiveLimits,
) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    if bytes.len() as u64 > limits.max_archive_bytes || bytes.len() > u32::MAX as usize {
        return Err(fail("ZIP byte limit exceeded"));
    }
    let mut entries = Vec::new();
    let mut offset = 0usize;
    let mut total = 0u64;
    while bytes.get(offset..offset + 4) == Some(&0x04034b50u32.to_le_bytes()) {
        if entries.len() >= limits.max_entries || entries.len() >= u16::MAX as usize {
            return Err(fail("ZIP entry limit exceeded"));
        }
        if word(bytes, offset + 8)? != 0 || word(bytes, offset + 6)? != 0x800 {
            return Err(fail("unsupported ZIP encoding"));
        }
        let size = dword(bytes, offset + 22)? as usize;
        if dword(bytes, offset + 18)? as usize != size || word(bytes, offset + 28)? != 0 {
            return Err(fail("unsupported ZIP size or extra fields"));
        }
        total = total
            .checked_add(size as u64)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        if total > limits.max_content_bytes {
            return Err(fail("ZIP content limit exceeded"));
        }
        let start = offset + 30;
        let end = start
            .checked_add(word(bytes, offset + 26)? as usize)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        let next = end
            .checked_add(size)
            .ok_or_else(|| fail("ZIP size overflow"))?;
        let path = std::str::from_utf8(
            bytes
                .get(start..end)
                .ok_or_else(|| fail("truncated ZIP name"))?,
        )
        .map_err(|_| fail("invalid ZIP UTF-8"))?;
        validate_archive_path(path)?;
        let data = bytes
            .get(end..next)
            .ok_or_else(|| fail("truncated ZIP file"))?;
        if crc32fast::hash(data) != dword(bytes, offset + 14)? {
            return Err(fail("ZIP checksum mismatch"));
        }
        entries.push(ArchiveEntry {
            path: path.into(),
            bytes: data.to_vec(),
        });
        offset = next;
    }
    if write_reproducible_zip(&entries, limits)? != bytes {
        return Err(fail("noncanonical or damaged ZIP directory"));
    }
    Ok(entries)
}

/// Portable library content, never containing finished prompt or pack products.
#[derive(Clone, Debug, Default)]
pub struct SopackPayload {
    /// Logical package name, independent of checkout location.
    pub package_name: String,
    /// Semver package version supplied by the manager.
    pub package_version: String,
    /// Stable metadata, including optional canonical package manifest TOML.
    pub metadata: BTreeMap<String, String>,
    /// Actual packaging root; prevents provider display names from deciding ownership.
    pub root_source: Option<SourceKey>,
    /// Compiled modules keyed by their source identity.
    pub units: BTreeMap<SourceKey, RelocatableUnitIr>,
    /// Fully resolved immutable import edges.
    pub imports: Vec<ImportBinding>,
    /// Exact assets keyed by defining source and source-relative asset spelling.
    pub assets: BTreeMap<(SourceKey, String), Vec<u8>>,
    /// Exact source bytes required for diagnostics.
    pub sources: BTreeMap<SourceKey, Vec<u8>>,
    /// Public export name to module source identity.
    pub exports: BTreeMap<String, SourceKey>,
}
/// Stable content identity used in all imported source URIs.
pub fn content_digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
/// Returns the portable project-relative spelling of a source identity.
pub fn logical_source_path(source: &SourceKey) -> Result<String, ArchiveError> {
    let path = match source {
        SourceKey::Project { path, .. } => path.join("/"),
        SourceKey::AdHoc { uri } if uri.starts_with("sopack://") => {
            uri.splitn(4, '/').nth(3).unwrap_or("").to_owned()
        }
        SourceKey::AdHoc { uri } => uri.rsplit(['/', '\\']).next().unwrap_or("").to_owned(),
    };
    validate_archive_path(&path)?;
    Ok(path)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    package_name: String,
    package_version: String,
    metadata: BTreeMap<String, String>,
    root_path: Option<String>,
    sources: BTreeMap<String, String>,
    units: Vec<String>,
    imports: Vec<(String, u32, String)>,
    assets: Vec<(String, String, String)>,
    exports: BTreeMap<String, String>,
}
fn source_key(prefix: &str, path: &str, package_name: &str) -> SourceKey {
    SourceKey::Project {
        package: squish_ir::PackageInstanceId {
            source_kind: 5,
            canonical_source: format!("sopack:{prefix}"),
            package_name: package_name.into(),
            exact_revision: prefix.into(),
        },
        path: path.split('/').map(str::to_owned).collect(),
    }
}
fn relocate(
    unit: &mut RelocatableUnitIr,
    map: &BTreeMap<SourceKey, SourceKey>,
) -> Result<(), ArchiveError> {
    let replace = |key: &mut SourceKey| -> Result<(), ArchiveError> {
        *key = map
            .get(key)
            .ok_or_else(|| fail("SOPack contains an unbound source identity"))?
            .clone();
        Ok(())
    };
    replace(&mut unit.header_mut().source)?;
    replace(&mut unit.attachment_mut().source)?;
    for source in &mut unit.sources_mut().records {
        replace(&mut source.key)?;
    }
    Ok(())
}
/// Encodes a closed, immutable module library with normalized internal source identities.
pub fn write_sopack(
    payload: &SopackPayload,
    limits: ArchiveLimits,
) -> Result<Vec<u8>, ArchiveError> {
    validate_payload(payload)?;
    let expanded = payload
        .sources
        .values()
        .chain(payload.assets.values())
        .try_fold(0u64, |sum, bytes| sum.checked_add(bytes.len() as u64))
        .ok_or_else(|| fail("SOPack size overflow"))?;
    if expanded > limits.max_content_bytes
        || payload.assets.len() > limits.max_entries
        || payload.imports.len() > limits.max_entries
    {
        return Err(fail("SOPack expanded content limit exceeded"));
    }
    let paths = portable_paths(payload)?;
    let map: BTreeMap<_, _> = paths
        .iter()
        .map(|(key, path)| {
            (
                key.clone(),
                source_key("internal", path, &payload.package_name),
            )
        })
        .collect();
    let path_of = |key: &SourceKey| {
        paths
            .get(key)
            .cloned()
            .ok_or_else(|| fail("missing SOPack source bytes"))
    };
    let mut manifest = Manifest {
        schema: 1,
        package_name: payload.package_name.clone(),
        package_version: payload.package_version.clone(),
        metadata: payload.metadata.clone(),
        root_path: payload.root_source.as_ref().map(path_of).transpose()?,
        sources: BTreeMap::new(),
        units: Vec::new(),
        imports: Vec::new(),
        assets: Vec::new(),
        exports: BTreeMap::new(),
    };
    let mut entries = Vec::new();
    for (key, bytes) in &payload.sources {
        let path = path_of(key)?;
        manifest.sources.insert(path.clone(), content_digest(bytes));
        entries.push(ArchiveEntry {
            path: format!("sources/{path}"),
            bytes: bytes.clone(),
        });
    }
    let bindings: BTreeMap<_, _> = payload
        .imports
        .iter()
        .map(|binding| ((&binding.importer, binding.import), &binding.target))
        .collect();
    for (key, unit) in &payload.units {
        if !matches!(
            unit.kind(),
            squish_ir::UnitKind::Module | squish_ir::UnitKind::Sopack
        ) || &unit.header().source != key
        {
            return Err(fail("SOPack only contains keyed modules"));
        }
        let path = path_of(key)?;
        let mut unit = unit.clone();
        for import in &mut unit.header_mut().imports {
            let target = bindings
                .get(&(key, import.local_id))
                .ok_or_else(|| fail("SOPack import binding missing"))?;
            import.spec =
                squish_ir::ImportSpec::RelativeUri(format!("sopack:{}", path_of(target)?));
        }
        normalize_producer(&mut unit)?;
        relocate(&mut unit, &map)?;
        let bytes = encode_unit_container(&unit).map_err(|e| fail(e.to_string()))?;
        manifest.units.push(path.clone());
        entries.push(ArchiveEntry {
            path: format!("units/{path}.xsir"),
            bytes,
        });
    }
    manifest.units.sort();
    for binding in &payload.imports {
        manifest.imports.push((
            path_of(&binding.importer)?,
            binding.import.0,
            path_of(&binding.target)?,
        ));
    }
    manifest.imports.sort();
    let mut asset_blobs = BTreeSet::new();
    for ((key, name), bytes) in &payload.assets {
        let digest = content_digest(bytes);
        manifest
            .assets
            .push((path_of(key)?, name.clone(), digest.clone()));
        let path = format!("assets/{digest}");
        if asset_blobs.insert(path.clone()) {
            entries.push(ArchiveEntry {
                path,
                bytes: bytes.clone(),
            });
        }
    }
    manifest.assets.sort();
    for (name, key) in &payload.exports {
        manifest.exports.insert(name.clone(), path_of(key)?);
    }
    validate_manifest(&manifest)?;
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| fail(e.to_string()))?;
    if manifest_bytes.len() > 8 * 1024 * 1024 {
        return Err(fail("SOPack manifest byte limit exceeded"));
    }
    entries.push(ArchiveEntry {
        path: "sopack.json".into(),
        bytes: manifest_bytes,
    });
    write_reproducible_zip(&entries, limits)
}
fn validate_manifest(manifest: &Manifest) -> Result<(), ArchiveError> {
    if manifest.schema != 1
        || manifest.package_name.is_empty()
        || manifest.package_version.is_empty()
    {
        return Err(fail("invalid SOPack metadata"));
    }
    if manifest.imports.windows(2).any(|w| w[0] >= w[1])
        || manifest.assets.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(fail("noncanonical SOPack bindings"));
    }
    let units: BTreeSet<_> = manifest.units.iter().collect();
    if units.len() != manifest.units.len() || manifest.units.windows(2).any(|w| w[0] >= w[1]) {
        return Err(fail("noncanonical SOPack modules"));
    }
    for path in &manifest.units {
        if !manifest.sources.contains_key(path) {
            return Err(fail("SOPack module source missing"));
        }
    }
    let mut bindings = BTreeSet::new();
    for (from, slot, to) in &manifest.imports {
        if !units.contains(from) || !units.contains(to) || !bindings.insert((from, slot)) {
            return Err(fail("invalid SOPack import binding"));
        }
    }
    if let Some(path) = &manifest.root_path {
        if !units.contains(path) {
            return Err(fail("SOPack root does not name a unit"));
        }
    }
    for path in manifest.exports.values() {
        if !units.contains(path) {
            return Err(fail("SOPack export does not name a module"));
        }
    }
    let mut assets = BTreeSet::new();
    for (source, name, _) in &manifest.assets {
        if !manifest.sources.contains_key(source) || !assets.insert((source, name)) {
            return Err(fail("invalid SOPack asset binding"));
        }
    }
    Ok(())
}
/// Decodes a library and relocates every source identity into its content-addressed namespace.
pub fn read_sopack(bytes: &[u8], limits: ArchiveLimits) -> Result<SopackPayload, ArchiveError> {
    let entries = read_reproducible_zip(bytes, limits)?;
    let mut files: BTreeMap<_, _> = entries
        .into_iter()
        .map(|entry| (entry.path, entry.bytes))
        .collect();
    let manifest_bytes = files
        .remove("sopack.json")
        .ok_or_else(|| fail("SOPack manifest missing"))?;
    if manifest_bytes.len() > 8 * 1024 * 1024 {
        return Err(fail("SOPack manifest byte limit exceeded"));
    }
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| fail(e.to_string()))?;
    validate_manifest(&manifest)?;
    if serde_json::to_vec(&manifest).map_err(|e| fail(e.to_string()))? != manifest_bytes {
        return Err(fail("noncanonical SOPack manifest"));
    }
    let digest = content_digest(bytes);
    let mut payload = SopackPayload {
        package_name: manifest.package_name.clone(),
        package_version: manifest.package_version.clone(),
        metadata: manifest.metadata.clone(),
        root_source: manifest
            .root_path
            .as_ref()
            .map(|path| source_key(&digest, path, &manifest.package_name)),
        ..Default::default()
    };
    let mut relocation = BTreeMap::new();
    for (path, expected) in &manifest.sources {
        validate_archive_path(path)?;
        let source = files
            .remove(&format!("sources/{path}"))
            .ok_or_else(|| fail("SOPack source missing"))?;
        if content_digest(&source) != *expected {
            return Err(fail("SOPack source digest mismatch"));
        }
        let key = source_key(&digest, path, &payload.package_name);
        relocation.insert(
            source_key("internal", path, &payload.package_name),
            key.clone(),
        );
        payload.sources.insert(key, source);
    }
    for path in &manifest.units {
        let bytes = files
            .remove(&format!("units/{path}.xsir"))
            .ok_or_else(|| fail("SOPack compiled module missing"))?;
        let mut unit = decode_unit_container(&bytes).map_err(|e| fail(e.to_string()))?;
        if !matches!(
            unit.kind(),
            squish_ir::UnitKind::Module | squish_ir::UnitKind::Sopack
        ) || unit.header().source != source_key("internal", path, &payload.package_name)
        {
            return Err(fail("invalid SOPack module identity"));
        }
        relocate(&mut unit, &relocation)?;
        payload.units.insert(unit.header().source.clone(), unit);
    }
    for (from, slot, to) in &manifest.imports {
        payload.imports.push(ImportBinding {
            importer: source_key(&digest, from, &payload.package_name),
            import: ImportId(*slot),
            target: source_key(&digest, to, &payload.package_name),
        });
    }
    let mut expanded_bytes = payload
        .sources
        .values()
        .map(|b| b.len() as u64)
        .sum::<u64>();
    if manifest.assets.len() > limits.max_entries || manifest.imports.len() > limits.max_entries {
        return Err(fail("SOPack binding limit exceeded"));
    }
    for (source, name, asset_digest) in &manifest.assets {
        let bytes = files
            .get(&format!("assets/{asset_digest}"))
            .ok_or_else(|| fail("SOPack asset missing"))?;
        if content_digest(bytes) != *asset_digest {
            return Err(fail("SOPack asset digest mismatch"));
        }
        expanded_bytes = expanded_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| fail("SOPack size overflow"))?;
        if expanded_bytes > limits.max_content_bytes {
            return Err(fail("SOPack expanded content limit exceeded"));
        }
        payload.assets.insert(
            (
                source_key(&digest, source, &payload.package_name),
                name.clone(),
            ),
            bytes.clone(),
        );
    }
    // Remove shared asset blobs only after every owner has been restored.
    for (_, _, digest) in &manifest.assets {
        files.remove(&format!("assets/{digest}"));
    }
    for (name, path) in manifest.exports {
        payload
            .exports
            .insert(name, source_key(&digest, &path, &payload.package_name));
    }
    if !files.is_empty() {
        return Err(fail(
            "SOPack contains undeclared content or finished products",
        ));
    }
    validate_payload(&payload)?;
    Ok(payload)
}

/// Verifies closure completeness and exact diagnostic source attachments.
fn validate_payload(payload: &SopackPayload) -> Result<(), ArchiveError> {
    let mut bindings = BTreeMap::new();
    for binding in &payload.imports {
        if !payload.units.contains_key(&binding.importer)
            || !payload.units.contains_key(&binding.target)
            || bindings
                .insert((&binding.importer, binding.import), &binding.target)
                .is_some()
        {
            return Err(fail("invalid SOPack closure binding"));
        }
    }
    let mut imports = 0;
    for (key, unit) in &payload.units {
        if !matches!(
            unit.kind(),
            squish_ir::UnitKind::Module | squish_ir::UnitKind::Sopack
        ) || unit.header().source != *key
        {
            return Err(fail("SOPack contains a non-module or mismatched source"));
        }
        for import in &unit.header().imports {
            imports += 1;
            if import.expected_kind != squish_ir::UnitKind::Module
                || !bindings.contains_key(&(key, import.local_id))
            {
                return Err(fail("SOPack import closure is incomplete"));
            }
        }
        for op in unit.ops() {
            if let squish_ir::Op::Asset { path, .. } = &op.op {
                let name = unit
                    .header()
                    .semantic_strings
                    .get(path.0 as usize)
                    .ok_or_else(|| fail("invalid SOPack asset path"))?;
                if !payload.assets.contains_key(&(key.clone(), name.clone())) {
                    return Err(fail("SOPack asset binding missing"));
                }
            }
        }
        for record in &unit.sources().records {
            let bytes = payload
                .sources
                .get(&record.key)
                .ok_or_else(|| fail("SOPack diagnostic source missing"))?;
            if squish_ir::SourceDigest::of(bytes) != record.digest
                || record.exact_bytes.digest != record.digest.0
                || record.exact_bytes.byte_len != bytes.len() as u64
            {
                return Err(fail("SOPack diagnostic source identity mismatch"));
            }
        }
    }
    if imports != bindings.len() {
        return Err(fail("SOPack has undeclared import bindings"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entries() -> Vec<ArchiveEntry> {
        vec![
            ArchiveEntry {
                path: "z.bin".into(),
                bytes: vec![0, 255, 1],
            },
            ArchiveEntry {
                path: "skills/你好.txt".into(),
                bytes: b"hello\r\n".to_vec(),
            },
        ]
    }
    #[test]
    fn reproducible_order_and_binary_roundtrip() {
        let mut entries = entries();
        let bytes = write_reproducible_zip(&entries, ArchiveLimits::default()).unwrap();
        entries.reverse();
        assert_eq!(
            bytes,
            write_reproducible_zip(&entries, ArchiveLimits::default()).unwrap()
        );
        let decoded = read_reproducible_zip(&bytes, ArchiveLimits::default()).unwrap();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(decoded, entries);
        assert_eq!(&bytes[10..14], &[0, 0, 33, 0]);
    }
    #[test]
    fn canonical_empty_zip() {
        let bytes = write_reproducible_zip(&[], ArchiveLimits::default()).unwrap();
        assert_eq!(bytes.len(), 22);
        assert!(
            read_reproducible_zip(&bytes, ArchiveLimits::default())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn rejects_traversal_duplicates_and_platform_aliases() {
        for path in [
            "../x", "/x", "a//b", "a\\b", "C:x", "a/./b", "a/NUL", "x.", "x ",
        ] {
            assert!(
                write_reproducible_zip(
                    &[ArchiveEntry {
                        path: path.into(),
                        bytes: vec![]
                    }],
                    ArchiveLimits::default()
                )
                .is_err(),
                "{path}"
            );
        }
        let entries = vec![entries()[0].clone(), entries()[0].clone()];
        assert!(write_reproducible_zip(&entries, ArchiveLimits::default()).is_err());
        let entries = vec![
            ArchiveEntry {
                path: "a".into(),
                bytes: vec![],
            },
            ArchiveEntry {
                path: "a/b".into(),
                bytes: vec![],
            },
        ];
        assert!(write_reproducible_zip(&entries, ArchiveLimits::default()).is_err());
    }
    #[test]
    fn rejects_checksums_central_damage_and_trailing_bytes() {
        let bytes = write_reproducible_zip(&entries(), ArchiveLimits::default()).unwrap();
        for position in [14, 10, bytes.len() - 5] {
            let mut damaged = bytes.clone();
            damaged[position] ^= 1;
            assert!(read_reproducible_zip(&damaged, ArchiveLimits::default()).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(read_reproducible_zip(&trailing, ArchiveLimits::default()).is_err());
    }
    #[test]
    fn enforces_bounds_and_every_truncation() {
        let bytes = write_reproducible_zip(&entries(), ArchiveLimits::default()).unwrap();
        for end in 0..bytes.len() {
            assert!(read_reproducible_zip(&bytes[..end], ArchiveLimits::default()).is_err());
        }
        assert!(
            read_reproducible_zip(
                &bytes,
                ArchiveLimits {
                    max_content_bytes: 1,
                    ..ArchiveLimits::default()
                }
            )
            .is_err()
        );
        assert!(
            write_reproducible_zip(
                &entries(),
                ArchiveLimits {
                    max_entries: 1,
                    ..ArchiveLimits::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn sopack_empty_metadata_roundtrip_and_rejects_unlisted_products() {
        let payload = SopackPayload {
            package_name: "demo".into(),
            package_version: "1.0.0".into(),
            ..Default::default()
        };
        let bytes = write_sopack(&payload, ArchiveLimits::default()).unwrap();
        let restored = read_sopack(&bytes, ArchiveLimits::default()).unwrap();
        assert_eq!(restored.package_name, "demo");
        assert_eq!(
            write_sopack(&restored, ArchiveLimits::default()).unwrap(),
            bytes
        );
        let mut entries = read_reproducible_zip(&bytes, ArchiveLimits::default()).unwrap();
        entries.push(ArchiveEntry {
            path: "finished.prompt".into(),
            bytes: b"not allowed".to_vec(),
        });
        let bytes = write_reproducible_zip(&entries, ArchiveLimits::default()).unwrap();
        assert!(read_sopack(&bytes, ArchiveLimits::default()).is_err());
    }
}

#[cfg(test)]
mod sopack_tests {
    use super::*;
    use squish_ir::*;
    fn fixture(location: &str) -> SopackPayload {
        let source = SourceKey::Project {
            package: PackageInstanceId {
                source_kind: 4,
                canonical_source: location.into(),
                package_name: "demo".into(),
                exact_revision: "1.0.0".into(),
            },
            path: vec!["library.xml".into()],
        };
        let bytes = b"<library/>".to_vec();
        let digest = SourceDigest::of(&bytes);
        let module = ModuleObject {
            header: UnitHeader {
                ir_schema: Version { major: 1, minor: 0 },
                language_abi: AbiId("dsl-0007".into()),
                frontend_abi: AbiId("xml-v1".into()),
                regex_abi: AbiId("regex-1.12".into()),
                source: source.clone(),
                imports: vec![],
                semantic_strings: vec![],
                qnames: vec![],
                regexes: vec![],
                feature_bits: FeatureBits(0),
            },
            definitions: vec![],
            external_symbols: vec![],
            interface: InterfaceSummary::default(),
            regions: vec![],
            ops: vec![],
            origins: OriginTable::default(),
            sources: SourceArchive {
                records: vec![SourceRecord {
                    key: source.clone(),
                    digest,
                    bom_len: 0,
                    exact_bytes: BlobRef {
                        digest: digest.0,
                        byte_len: bytes.len() as u64,
                    },
                    line_start_offsets: vec![0],
                }],
            },
            attachment: UnitSourceAttachment {
                source: source.clone(),
                source_digest: digest,
                source_record: SourceRef(0),
            },
            producer: Producer {
                tool_version: "test".into(),
                build_fingerprint: location.into(),
            },
        };
        SopackPayload {
            package_name: "demo".into(),
            package_version: "1.0.0".into(),
            units: BTreeMap::from([(source.clone(), RelocatableUnitIr::Module(module))]),
            sources: BTreeMap::from([(source.clone(), bytes)]),
            exports: BTreeMap::from([("main".into(), source)]),
            ..Default::default()
        }
    }
    #[test]
    fn device_independent_identities_and_exact_source_roundtrip() {
        let a = write_sopack(&fixture("C:/work/a"), ArchiveLimits::default()).unwrap();
        let b = write_sopack(&fixture("/home/user/b"), ArchiveLimits::default()).unwrap();
        assert_eq!(a, b);
        let restored = read_sopack(&a, ArchiveLimits::default()).unwrap();
        let key = restored.exports.get("main").unwrap();
        let unit = restored.units.get(key).unwrap();
        assert_eq!(&unit.header().source, key);
        let RelocatableUnitIr::Module(module) = unit else {
            panic!("expected module")
        };
        assert_eq!(&module.attachment.source, key);
        assert_eq!(&unit.sources().records[0].key, key);
        assert_eq!(logical_source_path(key).unwrap(), "library.xml");
        if let SourceKey::Project { package, .. } = key {
            assert_eq!(
                package.canonical_source,
                format!("sopack:{}", content_digest(&a))
            );
        } else {
            panic!("expected project key");
        }
        assert_eq!(
            write_sopack(&restored, ArchiveLimits::default()).unwrap(),
            a
        );
    }
    #[test]
    fn source_checksum_mismatch_rejected() {
        let mut payload = fixture("workspace");
        payload.sources.values_mut().next().unwrap().push(0);
        assert!(write_sopack(&payload, ArchiveLimits::default()).is_err());
    }
    #[test]
    fn sopack_root_is_importable_module() {
        let mut payload = fixture("workspace");
        let key = payload.units.keys().next().unwrap().clone();
        let RelocatableUnitIr::Module(mut module) = payload.units.remove(&key).unwrap() else {
            unreachable!()
        };
        module.regions.push(Region {
            id: RegionId(0),
            ops: vec![],
        });
        module.regions.push(Region {
            id: RegionId(1),
            ops: vec![],
        });
        let symbol = ExpandedName {
            namespace_uri: "".into(),
            local_name: "root-macro".into(),
        };
        module.definitions.push(MacroDef {
            id: LocalDefId(0),
            symbol: symbol.clone(),
            signature: Signature::default(),
            body: RegionId(0),
        });
        module.interface.definitions.push(InterfaceDef {
            id: LocalDefId(0),
            symbol,
            signature: Signature::default(),
        });
        payload.units.insert(
            key,
            RelocatableUnitIr::Sopack(SopackObject {
                module,
                root_region: RegionId(1),
            }),
        );
        let bytes = write_sopack(&payload, ArchiveLimits::default()).unwrap();
        let restored = read_sopack(&bytes, ArchiveLimits::default()).unwrap();
        assert_eq!(
            restored.units.values().next().unwrap().kind(),
            UnitKind::Sopack
        );
        assert!(restored.exports.contains_key("main"));
        let RelocatableUnitIr::Sopack(root) = restored.units.values().next().unwrap() else {
            panic!("expected SOPack root")
        };
        assert_eq!(root.root_region, RegionId(1));
        assert_eq!(
            root.module.interface.definitions[0].symbol.local_name,
            "root-macro"
        );
    }
    fn same_name_providers(location: &str) -> SopackPayload {
        let mut payload = fixture(&format!("{location}/root"));
        payload.root_source = payload.units.keys().next().cloned();
        for (provider, bytes) in [
            ("first", b"<one/>".as_slice()),
            ("second", b"<two/>".as_slice()),
        ] {
            let mut foreign = fixture(&format!("{location}/{provider}"));
            let key = foreign.units.keys().next().unwrap().clone();
            let RelocatableUnitIr::Module(mut module) = foreign.units.remove(&key).unwrap() else {
                unreachable!()
            };
            let digest = SourceDigest::of(bytes);
            module.sources.records[0].digest = digest;
            module.sources.records[0].exact_bytes = BlobRef {
                digest: digest.0,
                byte_len: bytes.len() as u64,
            };
            module.attachment.source_digest = digest;
            module.regions.push(Region {
                id: RegionId(0),
                ops: vec![],
            });
            payload.units.insert(
                key.clone(),
                RelocatableUnitIr::Sopack(SopackObject {
                    module,
                    root_region: RegionId(0),
                }),
            );
            payload.sources.insert(key, bytes.to_vec());
        }
        payload
    }
    #[test]
    fn same_name_version_providers_have_content_namespaces_and_explicit_root() {
        let bytes = write_sopack(
            &same_name_providers("C:/device/a"),
            ArchiveLimits::default(),
        )
        .unwrap();
        assert_eq!(
            bytes,
            write_sopack(
                &same_name_providers("/home/device/b"),
                ArchiveLimits::default()
            )
            .unwrap()
        );
        let restored = read_sopack(&bytes, ArchiveLimits::default()).unwrap();
        assert_eq!(restored.units.len(), 3);
        assert_eq!(
            logical_source_path(restored.root_source.as_ref().unwrap()).unwrap(),
            "library.xml"
        );
        assert_eq!(
            restored
                .units
                .keys()
                .filter(|key| logical_source_path(key).unwrap().starts_with("_providers/"))
                .count(),
            2
        );
        assert_eq!(
            write_sopack(&restored, ArchiveLimits::default()).unwrap(),
            bytes
        );
    }
    #[test]
    fn foreign_provider_paths_do_not_collide() {
        let mut payload = fixture("workspace");
        let mut foreign = fixture("foreign-workspace");
        let old = foreign.units.keys().next().unwrap().clone();
        let mut key = old.clone();
        if let SourceKey::Project { package, .. } = &mut key {
            package.package_name = "other".into();
        }
        let mut unit = foreign.units.remove(&old).unwrap();
        relocate(&mut unit, &BTreeMap::from([(old.clone(), key.clone())])).unwrap();
        payload
            .sources
            .insert(key.clone(), foreign.sources.remove(&old).unwrap());
        payload.units.insert(key, unit);
        let bytes = write_sopack(&payload, ArchiveLimits::default()).unwrap();
        let restored = read_sopack(&bytes, ArchiveLimits::default()).unwrap();
        assert_eq!(restored.units.len(), 2);
        assert!(
            restored
                .units
                .keys()
                .any(|key| logical_source_path(key).unwrap().starts_with("_providers/"))
        );
    }
}

/// Provider membership uses exact resolver identities only inside this process.
/// Its host-specific fields never enter content fingerprints or archive metadata.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SourceGroup {
    Project(squish_ir::PackageInstanceId),
    AdHoc,
}
fn source_group(key: &SourceKey) -> SourceGroup {
    match key {
        SourceKey::Project { package, .. } => SourceGroup::Project(package.clone()),
        SourceKey::AdHoc { .. } => SourceGroup::AdHoc,
    }
}
fn hash_field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}
/// Fingerprints frozen provider content and graph without physical checkout paths.
/// G graph-color rounds cover every simple provider path, including cyclic closures.
fn portable_paths(payload: &SopackPayload) -> Result<BTreeMap<SourceKey, String>, ArchiveError> {
    let mut groups: BTreeMap<SourceGroup, Vec<&SourceKey>> = BTreeMap::new();
    let mut logical = BTreeMap::new();
    for key in payload.sources.keys() {
        logical.insert(key.clone(), logical_source_path(key)?);
        groups.entry(source_group(key)).or_default().push(key);
    }
    let explicit = payload.root_source.as_ref();
    if explicit.is_some_and(|key| !payload.units.contains_key(key)) {
        return Err(fail("SOPack packaging root is missing"));
    }
    let roots: BTreeSet<_> = payload
        .units
        .iter()
        .filter(|(_, unit)| unit.kind() == squish_ir::UnitKind::Sopack)
        .map(|(key, _)| source_group(key))
        .collect();
    let root = if let Some(key) = explicit {
        source_group(key)
    } else if roots.len() == 1 {
        roots
            .into_iter()
            .next()
            .ok_or_else(|| fail("SOPack root missing"))?
    } else if roots.len() > 1 {
        return Err(fail(
            "multiple SOPack provider roots require explicit root_source",
        ));
    } else if let Some(key) = payload.exports.values().next() {
        source_group(key)
    } else if groups.len() == 1 {
        groups
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| fail("SOPack source group missing"))?
    } else if groups.is_empty() {
        return Ok(BTreeMap::new());
    } else {
        return Err(fail(
            "SOPack root_source is required for multiple providers",
        ));
    };
    let normalized: BTreeMap<_, _> = logical
        .iter()
        .map(|(key, path)| (key.clone(), source_key("provider-seed", path, "seed")))
        .collect();
    let bindings: BTreeMap<_, _> = payload
        .imports
        .iter()
        .map(|b| ((&b.importer, b.import), &b.target))
        .collect();
    let mut owned_assets: BTreeMap<&SourceKey, Vec<(&String, &Vec<u8>)>> = BTreeMap::new();
    for ((owner, name), bytes) in &payload.assets {
        owned_assets.entry(owner).or_default().push((name, bytes));
    }
    let mut base = BTreeMap::new();
    for (group, keys) in &mut groups {
        keys.sort_by(|a, b| logical[*a].cmp(&logical[*b]));
        let mut hash = blake3::Hasher::new();
        hash_field(&mut hash, b"sopack-provider-v2");
        if let SourceGroup::Project(package) = group {
            hash_field(&mut hash, &package.source_kind.to_le_bytes());
            hash_field(&mut hash, package.package_name.as_bytes());
            hash_field(&mut hash, package.exact_revision.as_bytes());
        }
        let mut seen = BTreeSet::new();
        for key in keys {
            let path = &logical[*key];
            if !seen.insert(path) {
                return Err(fail("duplicate path inside SOPack provider"));
            }
            hash_field(&mut hash, path.as_bytes());
            hash_field(&mut hash, &payload.sources[*key]);
            if let Some(unit) = payload.units.get(*key) {
                let mut unit = unit.clone();
                for import in &mut unit.header_mut().imports {
                    let target = bindings
                        .get(&(*key, import.local_id))
                        .ok_or_else(|| fail("SOPack import binding missing"))?;
                    import.spec =
                        squish_ir::ImportSpec::RelativeUri(format!("sopack:{}", logical[*target]));
                }
                normalize_producer(&mut unit)?;
                relocate(&mut unit, &normalized)?;
                hash_field(
                    &mut hash,
                    &encode_unit_container(&unit).map_err(|e| fail(e.to_string()))?,
                );
            }
            for (name, bytes) in owned_assets.get(*key).into_iter().flatten() {
                hash_field(&mut hash, name.as_bytes());
                hash_field(&mut hash, bytes);
            }
        }
        base.insert(group.clone(), hash.finalize().to_hex().to_string());
    }
    let mut edges: BTreeMap<SourceGroup, Vec<(String, u32, SourceGroup, String)>> = BTreeMap::new();
    for binding in &payload.imports {
        edges
            .entry(source_group(&binding.importer))
            .or_default()
            .push((
                logical[&binding.importer].clone(),
                binding.import.0,
                source_group(&binding.target),
                logical[&binding.target].clone(),
            ));
    }
    let mut colors = base.clone();
    for _ in 0..groups.len() {
        let mut next = BTreeMap::new();
        for (group, seed) in &base {
            let mut hash = blake3::Hasher::new();
            hash_field(&mut hash, seed.as_bytes());
            let mut ordered: Vec<_> = edges
                .get(group)
                .into_iter()
                .flatten()
                .map(|(from, slot, target, path)| (from, *slot, &colors[target], path))
                .collect();
            ordered.sort();
            for (from, slot, target, path) in ordered {
                hash_field(&mut hash, from.as_bytes());
                hash_field(&mut hash, &slot.to_le_bytes());
                hash_field(&mut hash, target.as_bytes());
                hash_field(&mut hash, path.as_bytes());
            }
            next.insert(group.clone(), hash.finalize().to_hex().to_string());
        }
        colors = next;
    }
    let mut paths = BTreeMap::new();
    let mut unique = BTreeSet::new();
    for (key, path) in logical {
        let group = source_group(&key);
        let path = if group == root {
            path
        } else {
            format!("_providers/{}/{path}", colors[&group])
        };
        if !unique.insert(path.clone()) {
            return Err(fail(
                "content-equivalent SOPack providers have duplicate logical paths",
            ));
        }
        paths.insert(key, path);
    }
    Ok(paths)
}
/// Removes non-semantic producer machine information before hashing or encoding.
fn normalize_producer(unit: &mut RelocatableUnitIr) -> Result<(), ArchiveError> {
    let module = match unit {
        RelocatableUnitIr::Module(module) => module,
        RelocatableUnitIr::Sopack(root) => &mut root.module,
        _ => return Err(fail("invalid SOPack unit kind")),
    };
    module.producer.tool_version = "xmlsquish/1.2.0".into();
    module.producer.build_fingerprint = "sopack-v1".into();
    Ok(())
}
