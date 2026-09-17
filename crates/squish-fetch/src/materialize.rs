use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::read::GzDecoder;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::{
    ArchiveDigest, ContentDigest, FetchError, FileRecord, HostContext, ManifestDigest,
    MaterializedPackage, Sha256Digest, SourceEvent,
};

static NONCE: AtomicU64 = AtomicU64::new(0);
const MARKER: &str = ".xmlsquish-source.json";

/// 一个已验证的逻辑普通文件。 / A validated logical regular file.
#[derive(Clone, Debug)]
pub struct LogicalFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// 来源无关、按 UTF-8 路径排序的文件树。 / Source-neutral file tree sorted by UTF-8 path bytes.
#[derive(Clone, Debug)]
pub struct LogicalTree {
    pub files: Vec<LogicalFile>,
    pub content_digest: ContentDigest,
}

impl LogicalTree {
    /// 从路径/字节构造树，并应用跨平台冲突规则。 / Builds a tree and applies portable collision rules.
    pub fn build(mut files: Vec<LogicalFile>, limits: &crate::Limits) -> Result<Self, FetchError> {
        if files.len() > limits.max_files {
            return Err(FetchError::Integrity("file count exceeds limit".into()));
        }
        let mut exact = BTreeSet::new();
        let mut portable_files = BTreeSet::new();
        let mut portable_dirs = BTreeMap::<String, String>::new();
        let mut exact_dirs = BTreeSet::new();
        let mut total = 0u64;
        for file in &files {
            validate_path(&file.path, limits)?;
            if !exact.insert(file.path.clone()) {
                return Err(FetchError::Path(format!("duplicate path {}", file.path)));
            }
            let key = portable_key(&file.path);
            if exact_dirs.contains(&file.path) || portable_dirs.contains_key(&key) {
                return Err(FetchError::Path(format!(
                    "file/directory conflict at {}",
                    file.path
                )));
            }
            if !portable_files.insert(key.clone()) {
                return Err(FetchError::Path(format!(
                    "portable path collision at {}",
                    file.path
                )));
            }
            let segments = file.path.split('/').collect::<Vec<_>>();
            for depth in 1..segments.len() {
                let exact_parent = segments[..depth].join("/");
                let portable_parent = portable_key(&exact_parent);
                if exact.contains(&exact_parent) || portable_files.contains(&portable_parent) {
                    return Err(FetchError::Path(format!(
                        "file/directory conflict at {}",
                        file.path
                    )));
                }
                if let Some(previous) = portable_dirs.get(&portable_parent)
                    && previous != &exact_parent
                {
                    return Err(FetchError::Path(format!(
                        "portable directory collision at {}",
                        file.path
                    )));
                }
                portable_dirs.insert(portable_parent, exact_parent.clone());
                exact_dirs.insert(exact_parent);
            }
            let length = u64::try_from(file.bytes.len())
                .map_err(|_| FetchError::Integrity("file too large".into()))?;
            if length > limits.max_file_bytes {
                return Err(FetchError::Integrity(format!(
                    "file exceeds limit: {}",
                    file.path
                )));
            }
            total = total
                .checked_add(length)
                .ok_or_else(|| FetchError::Integrity("expanded length overflow".into()))?;
        }
        if total > limits.max_expanded_bytes {
            return Err(FetchError::Integrity("expanded bytes exceed limit".into()));
        }
        files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
        let mut hash = Sha256::new();
        hash.update(b"xmlsquish\0package-tree/v1\0");
        hash.update((files.len() as u64).to_be_bytes());
        for file in &files {
            hash.update((file.path.len() as u32).to_be_bytes());
            hash.update(file.path.as_bytes());
            hash.update((file.bytes.len() as u64).to_be_bytes());
            hash.update(&file.bytes);
        }
        Ok(Self {
            files,
            content_digest: ContentDigest(Sha256Digest(hex::encode(hash.finalize()))),
        })
    }
}

fn validate_path(path: &str, limits: &crate::Limits) -> Result<(), FetchError> {
    if path.is_empty()
        || path.len() > limits.max_path_bytes
        || path.starts_with('/')
        || path.contains('\\')
    {
        return Err(FetchError::Path(path.into()));
    }
    for segment in path.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.len() > limits.max_segment_bytes
            || segment.contains(':')
            || segment.ends_with([' ', '.'])
            || segment.chars().any(char::is_control)
        {
            return Err(FetchError::Path(path.into()));
        }
        if segment.eq_ignore_ascii_case(".git") || segment.eq_ignore_ascii_case(MARKER) {
            return Err(FetchError::Path(path.into()));
        }
        let base = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .to_ascii_uppercase();
        if matches!(
            base.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        ) {
            return Err(FetchError::Path(path.into()));
        }
    }
    Ok(())
}

fn portable_key(path: &str) -> String {
    path.split('/')
        .map(|s| {
            s.trim_end_matches([' ', '.'])
                .nfc()
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Serialize, Deserialize)]
struct Marker {
    v: u32,
    content_digest: String,
    manifest_digest: String,
    files: Vec<FileRecord>,
    file_count: usize,
    total_bytes: u64,
}

/// `.xspkg` 校验与统一原子物化器。 / `.xspkg` verifier and unified atomic materializer.
#[derive(Clone)]
pub struct Materializer {
    context: HostContext,
}

impl Materializer {
    /// 使用共享缓存上下文创建。 / Creates a materializer over shared cache state.
    pub fn new(context: HostContext) -> Self {
        Self { context }
    }

    /// 完整复验 marker、精确文件集、每个文件字节及树摘要。 / Fully verifies the marker, exact file set, every file byte, and the tree digest.
    pub fn contains_complete(&self, digest: &ContentDigest) -> bool {
        self.load_complete(digest).is_ok()
    }

    fn load_complete(&self, digest: &ContentDigest) -> Result<(), FetchError> {
        let value = &digest.0.0;
        let base = self
            .context
            .cache
            .join("v1/materialized/sha256")
            .join(&value[..2])
            .join(value);
        let marker: Marker = serde_json::from_slice(&fs::read(base.join(MARKER))?)?;
        let recorded_total = marker
            .files
            .iter()
            .try_fold(0u64, |total, record| total.checked_add(record.length))
            .ok_or_else(|| FetchError::Integrity("materialization byte count overflow".into()))?;
        if marker.v != 1
            || marker.content_digest != *value
            || marker.file_count != marker.files.len()
            || marker.total_bytes != recorded_total
        {
            return Err(FetchError::Integrity(
                "invalid materialization marker".into(),
            ));
        }
        let mut files = Vec::new();
        collect_files(
            &base.join("root"),
            &base.join("root"),
            &mut files,
            &self.context.limits,
        )?;
        files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
        if files.len() != marker.files.len() {
            return Err(FetchError::Integrity(
                "materialized file set mismatch".into(),
            ));
        }
        for (file, record) in files.iter().zip(&marker.files) {
            if file.path != record.path
                || file.bytes.len() as u64 != record.length
                || hex::encode(Sha256::digest(&file.bytes)) != record.sha256
            {
                return Err(FetchError::Integrity(
                    "materialized file bytes mismatch".into(),
                ));
            }
        }
        let manifest = marker
            .files
            .iter()
            .find(|record| record.path == "xmlsquish.toml")
            .ok_or_else(|| {
                FetchError::Integrity("materialization manifest record missing".into())
            })?;
        if manifest.sha256 != marker.manifest_digest {
            return Err(FetchError::Integrity(
                "materialization manifest digest mismatch".into(),
            ));
        }
        let tree = LogicalTree::build(files, &self.context.limits)?;
        if tree.content_digest != *digest {
            return Err(FetchError::Integrity(
                "materialized tree digest mismatch".into(),
            ));
        }
        Ok(())
    }

    /// 校验精确 archive、归档结构和内容摘要。 / Validates exact archive bytes, structure, and content identity.
    pub fn tree_from_xspkg(
        &self,
        bytes: &[u8],
        package: &str,
        version: &semver::Version,
        expected_archive: &ArchiveDigest,
        expected_size: u64,
        expected_content: &ContentDigest,
    ) -> Result<LogicalTree, FetchError> {
        if bytes.len() as u64 != expected_size {
            return Err(FetchError::Integrity("archive size mismatch".into()));
        }
        if bytes.len() as u64 > self.context.limits.max_archive_bytes {
            return Err(FetchError::Integrity("archive exceeds limit".into()));
        }
        let actual = hex::encode(Sha256::digest(bytes));
        if actual != expected_archive.0.0 {
            return Err(FetchError::Integrity("archive digest mismatch".into()));
        }
        let prefix = format!("{package}-{version}/");
        let mut archive = tar::Archive::new(GzDecoder::new(bytes));
        let mut files = Vec::new();
        let mut members = BTreeSet::new();
        let mut expanded = 0u64;
        let mut member_count = 0usize;
        for item in archive.entries()? {
            member_count += 1;
            if member_count > self.context.limits.max_files {
                return Err(FetchError::Integrity(
                    "archive member count exceeds limit".into(),
                ));
            }
            let mut item = item?;
            let kind = item.header().entry_type();
            let raw = item.path_bytes();
            let full = std::str::from_utf8(raw.as_ref())
                .map_err(|_| FetchError::Path("non-UTF-8 archive path".into()))?;
            if !full.starts_with(&prefix) || full == prefix.trim_end_matches('/') {
                return Err(FetchError::Path(format!("member outside {prefix}")));
            }
            let relative = full[prefix.len()..].to_owned();
            let member_key = relative.trim_end_matches('/');
            if !member_key.is_empty() && !members.insert(member_key.to_owned()) {
                return Err(FetchError::Path(format!(
                    "duplicate archive member {relative}"
                )));
            }
            if kind.is_dir() {
                if !relative.is_empty() {
                    validate_path(relative.trim_end_matches('/'), &self.context.limits)?;
                }
                continue;
            }
            if !kind.is_file() {
                return Err(FetchError::Unsupported(relative));
            }
            validate_path(&relative, &self.context.limits)?;
            if files.len() >= self.context.limits.max_files {
                return Err(FetchError::Integrity("file count exceeds limit".into()));
            }
            let declared = item.size();
            if declared > self.context.limits.max_file_bytes {
                return Err(FetchError::Integrity(format!(
                    "file exceeds limit: {relative}"
                )));
            }
            expanded = expanded
                .checked_add(declared)
                .ok_or_else(|| FetchError::Integrity("expanded length overflow".into()))?;
            if expanded > self.context.limits.max_expanded_bytes {
                return Err(FetchError::Integrity("expanded bytes exceed limit".into()));
            }
            let mut body = Vec::new();
            item.by_ref()
                .take(self.context.limits.max_file_bytes + 1)
                .read_to_end(&mut body)?;
            if body.len() as u64 > self.context.limits.max_file_bytes {
                return Err(FetchError::Integrity(format!(
                    "file exceeds limit: {relative}"
                )));
            }
            files.push(LogicalFile {
                path: relative,
                bytes: body,
            });
        }
        let tree = LogicalTree::build(files, &self.context.limits)?;
        if tree.content_digest != *expected_content {
            return Err(FetchError::Integrity("content digest mismatch".into()));
        }
        if !tree.files.iter().any(|f| f.path == "xmlsquish.toml") {
            return Err(FetchError::Integrity("root manifest missing".into()));
        }
        Ok(tree)
    }

    /// 将已验证树原子发布，以完整 marker 和全部文件字节作为命中条件。 / Atomically publishes a verified tree; a hit requires its marker and every exact file byte.
    pub fn materialize(
        &self,
        identity: &str,
        tree: &LogicalTree,
    ) -> Result<MaterializedPackage, FetchError> {
        let digest = &tree.content_digest.0.0;
        let base = self
            .context
            .cache
            .join("v1/materialized/sha256")
            .join(&digest[..2])
            .join(digest);
        fs::create_dir_all(base.parent().expect("digest path has parent"))?;
        let lock_path = self
            .context
            .cache
            .join("v1/stage")
            .join(format!("{digest}.lock"));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        if let Some(package) = self.verify_existing(&base, tree)? {
            self.context
                .observer
                .emit(SourceEvent::MaterializationCommitted {
                    content_digest: digest.clone(),
                    files: tree.files.len(),
                    bytes: tree.files.iter().map(|f| f.bytes.len() as u64).sum(),
                    cache_status: "hit".into(),
                });
            FileExt::unlock(&lock)?;
            return Ok(package);
        }
        if base.exists() {
            let quarantine = self.context.cache.join("v1/quarantine");
            fs::create_dir_all(&quarantine)?;
            let target = quarantine.join(format!(
                "{digest}-{}",
                NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::rename(&base, target)?;
            self.context
                .observer
                .emit(SourceEvent::CacheFaultRecovered {
                    layer: "materialization".into(),
                    expected_digest: digest.clone(),
                    recovery: "local-tree".into(),
                });
        }
        self.context
            .observer
            .emit(SourceEvent::MaterializationStarted {
                identity: identity.into(),
                content_digest: digest.clone(),
            });
        let stage = self.context.cache.join("v1/stage").join(format!(
            "{}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed),
            &digest[..12]
        ));
        let root = stage.join("root");
        fs::create_dir(&stage)?;
        fs::create_dir(&root)?;
        let mut records = Vec::new();
        for logical in &tree.files {
            let destination = root.join(logical.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(destination.parent().expect("logical file has parent"))?;
            let mut out = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)?;
            out.write_all(&logical.bytes)?;
            out.sync_all()?;
            records.push(FileRecord {
                path: logical.path.clone(),
                length: logical.bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(&logical.bytes)),
            });
        }
        let manifest = tree
            .files
            .iter()
            .find(|f| f.path == "xmlsquish.toml")
            .ok_or_else(|| FetchError::Integrity("root manifest missing".into()))?;
        let manifest_digest =
            ManifestDigest(Sha256Digest(hex::encode(Sha256::digest(&manifest.bytes))));
        let parsed = squish_project::Manifest::parse(
            std::str::from_utf8(&manifest.bytes)
                .map_err(|_| FetchError::Integrity("manifest is not UTF-8".into()))?,
        )?;
        let total = records.iter().map(|r| r.length).sum();
        let marker = Marker {
            v: 1,
            content_digest: digest.clone(),
            manifest_digest: manifest_digest.0.0.clone(),
            file_count: records.len(),
            total_bytes: total,
            files: records.clone(),
        };
        let mut marker_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.join(MARKER))?;
        serde_json::to_writer(&mut marker_file, &marker)?;
        marker_file.sync_all()?;
        // Windows 禁止重命名包含打开文件的目录。 / Windows forbids renaming a directory containing an open file.
        drop(marker_file);
        match fs::rename(&stage, &base) {
            Ok(()) => {}
            Err(_e) if base.exists() => {
                if self.verify_existing(&base, tree)?.is_none() {
                    return Err(FetchError::Integrity(
                        "atomic publication destination is invalid".into(),
                    ));
                }
                fs::remove_dir_all(&stage)?;
            }
            Err(e) => return Err(e.into()),
        }
        self.context
            .observer
            .emit(SourceEvent::MaterializationCommitted {
                content_digest: digest.clone(),
                files: records.len(),
                bytes: total,
                cache_status: "committed".into(),
            });
        FileExt::unlock(&lock)?;
        Ok(MaterializedPackage {
            content_digest: tree.content_digest.clone(),
            manifest_digest,
            manifest: parsed,
            root: base.join("root"),
            files: records.into(),
        })
    }

    fn verify_existing(
        &self,
        base: &Path,
        expected: &LogicalTree,
    ) -> Result<Option<MaterializedPackage>, FetchError> {
        if self.load_complete(&expected.content_digest).is_err() {
            return Ok(None);
        }
        let bytes = match fs::read(base.join(MARKER)) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let marker: Marker = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        if marker.v != 1
            || marker.content_digest != expected.content_digest.0.0
            || marker.files.len() != expected.files.len()
        {
            return Ok(None);
        }
        let map: BTreeMap<_, _> = expected
            .files
            .iter()
            .map(|f| (f.path.as_str(), f))
            .collect();
        for record in &marker.files {
            let Some(logical) = map.get(record.path.as_str()) else {
                return Ok(None);
            };
            let disk = match fs::read(
                base.join("root")
                    .join(record.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            ) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            };
            if disk != logical.bytes || record.sha256 != hex::encode(Sha256::digest(&disk)) {
                return Ok(None);
            }
        }
        let manifest_bytes = fs::read(base.join("root/xmlsquish.toml"))?;
        let manifest = squish_project::Manifest::parse(
            std::str::from_utf8(&manifest_bytes)
                .map_err(|_| FetchError::Integrity("manifest is not UTF-8".into()))?,
        )?;
        Ok(Some(MaterializedPackage {
            content_digest: expected.content_digest.clone(),
            manifest_digest: ManifestDigest(Sha256Digest(marker.manifest_digest)),
            manifest,
            root: base.join("root"),
            files: marker.files.into(),
        }))
    }
}

fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<LogicalFile>,
    limits: &crate::Limits,
) -> Result<(), FetchError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(root, &entry.path(), output, limits)?;
        } else if kind.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| FetchError::Integrity("materialization escaped root".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let length = entry.metadata()?.len();
            if length > limits.max_file_bytes {
                return Err(FetchError::Integrity(
                    "materialized file exceeds limit".into(),
                ));
            }
            let mut bytes = Vec::new();
            fs::File::open(entry.path())?
                .take(limits.max_file_bytes + 1)
                .read_to_end(&mut bytes)?;
            output.push(LogicalFile {
                path: relative,
                bytes,
            });
        } else {
            return Err(FetchError::Unsupported(
                "non-file in materialization".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digest_is_order_independent_and_exact_bytes_matter() {
        let l = crate::Limits::default();
        let a = LogicalTree::build(
            vec![
                LogicalFile {
                    path: "b".into(),
                    bytes: b"\r\n".to_vec(),
                },
                LogicalFile {
                    path: "a".into(),
                    bytes: vec![0xef, 0xbb, 0xbf],
                },
            ],
            &l,
        )
        .unwrap();
        let b = LogicalTree::build(
            vec![
                LogicalFile {
                    path: "a".into(),
                    bytes: vec![0xef, 0xbb, 0xbf],
                },
                LogicalFile {
                    path: "b".into(),
                    bytes: b"\r\n".to_vec(),
                },
            ],
            &l,
        )
        .unwrap();
        assert_eq!(a.content_digest, b.content_digest);
        let changed = LogicalTree::build(
            vec![
                LogicalFile {
                    path: "a".into(),
                    bytes: vec![0xef, 0xbb, 0xbf],
                },
                LogicalFile {
                    path: "b".into(),
                    bytes: b"\n".to_vec(),
                },
            ],
            &l,
        )
        .unwrap();
        assert_ne!(a.content_digest, changed.content_digest);
    }
    #[test]
    fn rejects_portable_collisions() {
        let err = LogicalTree::build(
            vec![
                LogicalFile {
                    path: "A.xml".into(),
                    bytes: vec![],
                },
                LogicalFile {
                    path: "a.XML".into(),
                    bytes: vec![],
                },
            ],
            &crate::Limits::default(),
        )
        .unwrap_err();
        assert!(matches!(err, FetchError::Path(_)));
        assert!(
            LogicalTree::build(
                vec![
                    LogicalFile {
                        path: "A/x".into(),
                        bytes: vec![]
                    },
                    LogicalFile {
                        path: "a/y".into(),
                        bytes: vec![]
                    },
                ],
                &crate::Limits::default()
            )
            .is_err()
        );
        assert!(
            LogicalTree::build(
                vec![
                    LogicalFile {
                        path: "a".into(),
                        bytes: vec![]
                    },
                    LogicalFile {
                        path: "a/b".into(),
                        bytes: vec![]
                    },
                ],
                &crate::Limits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn completeness_rechecks_file_bytes_not_only_marker_presence() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("fetch-materialize-{}", std::process::id()));
        let materializer = Materializer::new(HostContext::new(root.clone()).unwrap());
        let tree = LogicalTree::build(
            vec![LogicalFile {
                path: "xmlsquish.toml".into(),
                bytes:
                    b"manifest-version = 1\n[package]\nname = \"complete\"\nversion = \"1.0.0\"\n"
                        .to_vec(),
            }],
            &crate::Limits::default(),
        )
        .unwrap();
        let package = materializer.materialize("test", &tree).unwrap();
        assert!(materializer.contains_complete(&tree.content_digest));
        fs::write(package.root.join("xmlsquish.toml"), b"mutated").unwrap();
        assert!(!materializer.contains_complete(&tree.content_digest));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_declared_file_size_is_rejected_before_materialization() {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_ustar();
            header.set_size(4);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "demo-1.0.0/large", &b"data"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut gzip = flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::default());
        gzip.write_all(&tar_bytes).unwrap();
        let archive = gzip.finish().unwrap();
        let context = HostContext {
            cache: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.temp/archive-limit"),
            limits: crate::Limits {
                max_file_bytes: 3,
                ..Default::default()
            },
            observer: std::sync::Arc::new(crate::NoopObserver),
        };
        let materializer = Materializer::new(context);
        let archive_digest = ArchiveDigest(Sha256Digest(hex::encode(Sha256::digest(&archive))));
        let content = ContentDigest(Sha256Digest("0".repeat(64)));
        assert!(
            matches!(materializer.tree_from_xspkg(&archive, "demo", &"1.0.0".parse().unwrap(), &archive_digest, archive.len() as u64, &content), Err(FetchError::Integrity(message)) if message.contains("file exceeds"))
        );
    }
}
