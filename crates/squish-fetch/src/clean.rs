use std::fs::{self, File, Metadata, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::{ContentDigest, FetchError, HostContext, Materializer, Sha256Digest};

const MATERIALIZED: &str = "v1/materialized/sha256";
const QUARANTINE: &str = "v1/quarantine";
const REGISTRY_MAPPINGS: &str = "v1/registry-materialized";

/// 共享依赖缓存清理统计。 / Statistics from cleaning the shared dependency cache.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DependencyCacheCleanStats {
    /// 已移除的顶层缓存条目数。 / Number of top-level cache entries removed.
    pub removed_entries: u64,
    /// 已移除普通文件的总字节数。 / Total bytes of regular files removed.
    pub removed_bytes: u64,
    /// 因生产者持锁而跳过的条目数。 / Entries skipped because a producer held their lock.
    pub skipped_busy: u64,
}

impl DependencyCacheCleanStats {
    fn removed(&mut self, path: &Path) -> Result<(), FetchError> {
        let bytes = tree_bytes(path)?;
        if !remove_tree(path)? {
            return Ok(());
        }
        self.removed_entries = self
            .removed_entries
            .checked_add(1)
            .ok_or_else(|| FetchError::Integrity("clean entry count overflow".into()))?;
        self.removed_bytes = self
            .removed_bytes
            .checked_add(bytes)
            .ok_or_else(|| FetchError::Integrity("clean byte count overflow".into()))?;
        Ok(())
    }
}

/// 删除可证明损坏或已隔离的共享依赖数据。 / Removes only provably corrupt or quarantined shared dependency data.
///
/// 健康树、全局 CAS、sparse metadata 与 Git object database 永远不在扫描范围内；繁忙的
/// digest 或 registry mapping 会被跳过。 / Healthy trees, the global CAS, sparse metadata,
/// and Git object databases are outside the scan; busy digests and mappings are skipped.
///
/// # 示例 / Example
///
/// ```no_run
/// # fn main() -> Result<(), squish_fetch::FetchError> {
/// let context = squish_fetch::HostContext::new(".xmlsquish/cache".into())?;
/// let stats = squish_fetch::clean_dependency_cache(&context)?;
/// println!("removed {} entries", stats.removed_entries);
/// # Ok(())
/// # }
/// ```
pub fn clean_dependency_cache(
    context: &HostContext,
) -> Result<DependencyCacheCleanStats, FetchError> {
    let mut stats = DependencyCacheCleanStats::default();
    fs::create_dir_all(context.cache.join("v1"))?;
    let Some(clean_lock) = try_lock(&context.cache.join("v1/clean.lock"), &mut stats)? else {
        return Ok(stats);
    };
    clean_quarantine(context, &mut stats)?;
    clean_materialized(context, &mut stats)?;
    clean_registry_mappings(context, &mut stats)?;
    FileExt::unlock(&clean_lock)?;
    Ok(stats)
}

fn clean_quarantine(
    context: &HostContext,
    stats: &mut DependencyCacheCleanStats,
) -> Result<(), FetchError> {
    for path in children(&context.cache.join(QUARANTINE))? {
        stats.removed(&path)?;
    }
    Ok(())
}

fn clean_materialized(
    context: &HostContext,
    stats: &mut DependencyCacheCleanStats,
) -> Result<(), FetchError> {
    let root = context.cache.join(MATERIALIZED);
    for shard in children(&root)? {
        let Some(prefix) = file_name(&shard) else {
            continue;
        };
        if !is_hex(prefix, 2) || !is_plain_directory(&shard)? {
            continue;
        }
        for tree in children(&shard)? {
            let Some(digest) = file_name(&tree) else {
                continue;
            };
            if !is_hex(digest, 64) || !digest.starts_with(prefix) {
                continue;
            }
            let Some(lock) = try_digest_lock(context, digest, stats)? else {
                continue;
            };
            if tree_has_reparse_point(&tree)? || !is_complete(context, digest) {
                stats.removed(&tree)?;
            }
            FileExt::unlock(&lock)?;
        }
    }
    Ok(())
}

fn clean_registry_mappings(
    context: &HostContext,
    stats: &mut DependencyCacheCleanStats,
) -> Result<(), FetchError> {
    let root = context.cache.join(REGISTRY_MAPPINGS);
    for mapping in children(&root)? {
        let Some(name) = file_name(&mapping) else {
            continue;
        };
        if !is_hex(name, 64) {
            continue;
        }
        let Some(mapping_lock) = try_lock(&mapping.with_extension("update.lock"), stats)? else {
            continue;
        };
        let content = read_mapping(&mapping);
        let valid = match content {
            Some(ref digest) => {
                let Some(digest_lock) = try_digest_lock(context, digest, stats)? else {
                    FileExt::unlock(&mapping_lock)?;
                    continue;
                };
                let valid = is_complete(context, digest);
                FileExt::unlock(&digest_lock)?;
                valid
            }
            None => false,
        };
        if !valid {
            stats.removed(&mapping)?;
        }
        FileExt::unlock(&mapping_lock)?;
    }
    Ok(())
}

fn read_mapping(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata_is_reparse(&metadata) || !metadata.is_file() || metadata.len() != 64 {
        return None;
    }
    let value = fs::read_to_string(path).ok()?;
    Sha256Digest::parse(value.clone()).ok()?;
    Some(value)
}

fn is_complete(context: &HostContext, digest: &str) -> bool {
    let Ok(digest) = Sha256Digest::parse(digest.to_owned()) else {
        return false;
    };
    Materializer::new(context.clone()).contains_complete(&ContentDigest(digest))
}

fn try_digest_lock(
    context: &HostContext,
    digest: &str,
    stats: &mut DependencyCacheCleanStats,
) -> Result<Option<File>, FetchError> {
    fs::create_dir_all(context.cache.join("v1/stage"))?;
    try_lock(
        &context
            .cache
            .join("v1/stage")
            .join(format!("{digest}.lock")),
        stats,
    )
}

fn try_lock(
    path: &Path,
    stats: &mut DependencyCacheCleanStats,
) -> Result<Option<File>, FetchError> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    match FileExt::try_lock_exclusive(&lock) {
        Ok(()) => Ok(Some(lock)),
        Err(error) if lock_is_busy(&error) => {
            stats.skipped_busy = stats
                .skipped_busy
                .checked_add(1)
                .ok_or_else(|| FetchError::Integrity("clean busy count overflow".into()))?;
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn lock_is_busy(error: &std::io::Error) -> bool {
    if error.kind() == ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        // LockFileEx reports ERROR_LOCK_VIOLATION without mapping it to WouldBlock.
        // LockFileEx 会返回未映射为 WouldBlock 的 ERROR_LOCK_VIOLATION。
        error.raw_os_error() == Some(33)
    }
    #[cfg(not(windows))]
    false
}

fn children(path: &Path) -> Result<Vec<PathBuf>, FetchError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if metadata_is_reparse(&metadata) || !metadata.is_dir() {
        return Ok(Vec::new());
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    entries
        .map(|entry| entry.map(|entry| entry.path()).map_err(FetchError::from))
        .collect()
}

fn file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

fn is_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_plain_directory(path: &Path) -> Result<bool, FetchError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(metadata.is_dir() && !metadata_is_reparse(&metadata))
}

fn tree_has_reparse_point(path: &Path) -> Result<bool, FetchError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata_is_reparse(&metadata) {
        return Ok(true);
    }
    if !metadata.is_dir() {
        return Ok(false);
    }
    for child in children(path)? {
        if tree_has_reparse_point(&child)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn tree_bytes(path: &Path) -> Result<u64, FetchError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    if metadata_is_reparse(&metadata) {
        return Ok(0);
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    children(path)?.into_iter().try_fold(0u64, |total, child| {
        total
            .checked_add(tree_bytes(&child)?)
            .ok_or_else(|| FetchError::Integrity("clean byte count overflow".into()))
    })
}

fn remove_tree(path: &Path) -> Result<bool, FetchError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata_is_reparse(&metadata) {
        return remove_if_present(path, || remove_reparse(path, &metadata));
    }
    if metadata.is_dir() {
        for child in children(path)? {
            remove_tree(&child)?;
        }
        remove_if_present(path, || fs::remove_dir(path))
    } else {
        remove_if_present(path, || fs::remove_file(path))
    }
}

fn remove_if_present(
    path: &Path,
    mut operation: impl FnMut() -> std::io::Result<()>,
) -> Result<bool, FetchError> {
    const ATTEMPTS: usize = 8;
    for attempt in 0..ATTEMPTS {
        match operation() {
            Ok(()) => return Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::PermissionDenied | ErrorKind::DirectoryNotEmpty
                ) && attempt + 1 < ATTEMPTS =>
            {
                match fs::symlink_metadata(path) {
                    Err(missing) if missing.kind() == ErrorKind::NotFound => return Ok(false),
                    Err(check) => return Err(check.into()),
                    Ok(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("bounded removal loop always returns")
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn remove_reparse(path: &Path, _: &Metadata) -> std::io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn remove_reparse(path: &Path, metadata: &Metadata) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    if metadata.file_attributes() & 0x10 != 0 {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogicalFile, LogicalTree};
    use std::sync::{Arc, Barrier};

    fn fixture(name: &str) -> (tempfile::TempDir, HostContext, LogicalTree) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.temp")
            .join(format!("clean-{name}"));
        fs::create_dir_all(&root).unwrap();
        let temporary = tempfile::tempdir_in(root).unwrap();
        let context = HostContext::new(temporary.path().to_path_buf()).unwrap();
        let tree = LogicalTree::build(
            vec![LogicalFile {
                path: "xmlsquish.toml".into(),
                bytes: b"manifest-version = 1\n[package]\nname = \"clean\"\nversion = \"1.0.0\"\n"
                    .to_vec(),
            }],
            &context.limits,
        )
        .unwrap();
        (temporary, context, tree)
    }

    fn mapping(context: &HostContext, archive: char, content: &str) -> PathBuf {
        let path = context
            .cache
            .join(REGISTRY_MAPPINGS)
            .join(archive.to_string().repeat(64));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn healthy_tree_and_mapping_are_retained() {
        let (_temporary, context, tree) = fixture("healthy");
        let package = Materializer::new(context.clone())
            .materialize("test", &tree)
            .unwrap();
        let map = mapping(&context, 'a', &tree.content_digest.0.0);

        assert_eq!(
            clean_dependency_cache(&context).unwrap(),
            DependencyCacheCleanStats::default()
        );
        assert!(package.root.is_dir());
        assert!(map.is_file());
    }

    #[test]
    fn corrupt_tree_quarantine_and_stale_mapping_are_removed_idempotently() {
        let (_temporary, context, tree) = fixture("corrupt");
        let package = Materializer::new(context.clone())
            .materialize("test", &tree)
            .unwrap();
        fs::write(package.root.join("xmlsquish.toml"), b"broken").unwrap();
        let map = mapping(&context, 'b', &tree.content_digest.0.0);
        let quarantined = context.cache.join(QUARANTINE).join("known-bad");
        fs::create_dir_all(&quarantined).unwrap();
        fs::write(quarantined.join("payload"), b"bad").unwrap();

        let first = clean_dependency_cache(&context).unwrap();
        assert_eq!(first.removed_entries, 3);
        assert!(first.removed_bytes > 3);
        assert!(!package.root.exists());
        assert!(!map.exists());
        assert!(!quarantined.exists());
        assert_eq!(
            clean_dependency_cache(&context).unwrap(),
            DependencyCacheCleanStats::default()
        );
    }

    #[test]
    fn busy_corrupt_tree_is_skipped() {
        let (_temporary, context, tree) = fixture("busy");
        let package = Materializer::new(context.clone())
            .materialize("test", &tree)
            .unwrap();
        fs::write(package.root.join("xmlsquish.toml"), b"broken").unwrap();
        let lock_path = context
            .cache
            .join("v1/stage")
            .join(format!("{}.lock", tree.content_digest.0.0));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        FileExt::lock_exclusive(&lock).unwrap();

        let stats = clean_dependency_cache(&context).unwrap();
        assert_eq!(stats.skipped_busy, 1);
        assert_eq!(stats.removed_entries, 0);
        assert!(package.root.exists());
    }

    #[test]
    fn concurrent_cleaners_treat_already_removed_entries_as_success() {
        let (_temporary, context, _tree) = fixture("concurrent");
        let quarantine = context.cache.join(QUARANTINE);
        for index in 0..256 {
            let entry = quarantine.join(format!("invalid-{index}"));
            fs::create_dir_all(&entry).unwrap();
            fs::write(entry.join("payload"), b"bad").unwrap();
        }

        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let context = context.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    clean_dependency_cache(&context).unwrap()
                })
            })
            .collect();
        let removed: u64 = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().removed_entries)
            .sum();

        assert_eq!(removed, 256);
        assert!(children(&quarantine).unwrap().is_empty());
    }
}
