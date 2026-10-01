//! Invocation-scoped immutable SOPack acquisition, separate from persistent cache trust.

use squish_backend::archive::ArchiveLimits;
use squish_manager::AcquiredSopack;
use squish_resolver::SourceUnavailable;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Captures each exact locator once and shares decoded content across source adapters.
///
/// This value must be constructed per resolution/materialization invocation. Its path memo
/// is a frozen snapshot, not evidence that a mutable file remains unchanged in later runs.
/// Manager freeze retains the checksum-only authoritative recheck before consuming it.
pub(super) struct SopackAcquisition<'a> {
    root: &'a Path,
    paths: RefCell<BTreeMap<PathBuf, Arc<AcquiredSopack>>>,
    digests: RefCell<BTreeMap<String, Arc<AcquiredSopack>>>,
    #[cfg(test)]
    reads: std::cell::Cell<(usize, usize, usize)>,
}

impl<'a> SopackAcquisition<'a> {
    /// Creates an empty acquisition boundary rooted at the canonical workspace.
    pub(super) fn new(root: &'a Path) -> Self {
        Self {
            root,
            paths: RefCell::new(BTreeMap::new()),
            digests: RefCell::new(BTreeMap::new()),
            #[cfg(test)]
            reads: std::cell::Cell::new((0, 0, 0)),
        }
    }

    /// Acquires bounded bytes, pins their SHA-256, and decodes each distinct digest once.
    pub(super) fn load(
        &self,
        path: &Path,
    ) -> Result<(PathBuf, Arc<AcquiredSopack>), SourceUnavailable> {
        let fail = |detail: String| SourceUnavailable {
            identity: path.display().to_string(),
            detail,
        };
        // Exact previously captured locators need no second filesystem observation.
        // Manager's authoritative freeze recheck, not this memo, detects subsequent drift.
        if let Ok(relative) = path.strip_prefix(self.root)
            && let Some(acquired) = self.paths.borrow().get(relative)
        {
            return Ok((relative.to_path_buf(), acquired.clone()));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| fail(error.to_string()))?;
        let relative = canonical
            .strip_prefix(self.root)
            .map_err(|_| fail("SOPack dependency escapes workspace root".into()))?
            .to_path_buf();
        if let Some(acquired) = self.paths.borrow().get(&relative) {
            return Ok((relative, acquired.clone()));
        }
        let limits = ArchiveLimits::default();
        let file = File::open(&canonical).map_err(|error| fail(error.to_string()))?;
        let metadata = file.metadata().map_err(|error| fail(error.to_string()))?;
        if !metadata.is_file() || metadata.len() > limits.max_archive_bytes {
            return Err(fail(
                "SOPack archive exceeds its byte limit or is not a regular file".into(),
            ));
        }
        let bytes = read_bounded_archive(file, limits.max_archive_bytes).map_err(&fail)?;
        #[cfg(test)]
        {
            let (reads, count, decodes) = self.reads.get();
            self.reads.set((reads + 1, count + bytes.len(), decodes));
        }
        #[cfg(test)]
        let prior_decodes = self.digests.borrow().len();
        let acquired = AcquiredSopack::acquire(&bytes, &mut self.digests.borrow_mut())
            .map_err(|error| fail(error.to_string()))?;
        super::sopack_manifest(acquired.payload())?;
        #[cfg(test)]
        if self.digests.borrow().len() != prior_decodes {
            let (reads, count, decodes) = self.reads.get();
            self.reads.set((reads, count, decodes + 1));
        }
        self.paths
            .borrow_mut()
            .insert(relative.clone(), acquired.clone());
        Ok((relative, acquired))
    }

    /// Returns the same handles used by materialization, indexed by exact lock node.
    pub(super) fn locked(
        &self,
        lock: &squish_project::Lockfile,
    ) -> Result<BTreeMap<String, Arc<AcquiredSopack>>, SourceUnavailable> {
        let mut output = BTreeMap::new();
        for package in &lock.packages {
            let squish_project::LockedSource::Sopack { path, checksum } = &package.source else {
                continue;
            };
            let (_, archive) = self.load(&self.root.join(path))?;
            if archive.checksum() != checksum {
                return Err(SourceUnavailable { identity: package.id.clone(), detail: "SOPack archive differs from its locked digest; explicitly update the dependency".into() });
            }
            output.insert(package.id.clone(), archive);
        }
        Ok(output)
    }
}

/// Reads at most limit plus one byte, including files that grow after metadata checking.
pub(super) fn read_bounded_archive(
    reader: impl std::io::Read,
    limit: u64,
) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("SOPack archive exceeds its byte limit".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_backend::archive::{SopackPayload, write_sopack};

    /// Keeps archive fixtures inside the project-owned temporary namespace.
    fn fixture() -> (tempfile::TempDir, Vec<u8>) {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let directory = tempfile::tempdir_in(scratch).unwrap();
        let payload = SopackPayload {
            package_name: "library".into(),
            package_version: "1.0.0".into(),
            metadata: BTreeMap::from([(
                "manifest".into(),
                "manifest-version = 1\n[package]\nname = \"library\"\nversion = \"1.0.0\"\n".into(),
            )]),
            ..SopackPayload::default()
        };
        let bytes = write_sopack(&payload, ArchiveLimits::default()).unwrap();
        std::fs::write(directory.path().join("first.sopack"), &bytes).unwrap();
        (directory, bytes)
    }

    #[test]
    fn invocation_reuses_verified_archive_and_decodes_identical_alias_bytes_once() {
        let (directory, bytes) = fixture();
        let root = directory.path().canonicalize().unwrap();
        let invocation = SopackAcquisition::new(&root);
        let first = root.join("first.sopack");
        let (_, acquired) = invocation.load(&first).unwrap();
        let (_, repeated) = invocation.load(&first).unwrap();
        assert!(Arc::ptr_eq(&acquired, &repeated));
        assert_eq!(invocation.reads.get(), (1, bytes.len(), 1));
        let alias = root.join("alias.sopack");
        std::fs::write(&alias, &bytes).unwrap();
        let (_, alias_acquired) = invocation.load(&alias).unwrap();
        assert!(Arc::ptr_eq(&acquired, &alias_acquired));
        assert_eq!(invocation.reads.get(), (2, 2 * bytes.len(), 1));
        assert_eq!(acquired.payload().package_name, "library");
        assert_eq!(acquired.payload().package_version, "1.0.0");
    }

    #[test]
    fn locked_handles_preserve_exact_checksum_without_reacquisition() {
        let (directory, bytes) = fixture();
        let root = directory.path().canonicalize().unwrap();
        let invocation = SopackAcquisition::new(&root);
        let (_, acquired) = invocation.load(&root.join("first.sopack")).unwrap();
        let mut lock = squish_project::Lockfile {
            lock_version: squish_project::LOCK_VERSION,
            resolver_version: "test".into(),
            manifest_digest: format!("sha256:{}", "aa".repeat(32)),
            skills: Vec::new(),
            packages: vec![squish_project::LockedPackage {
                id: "library@1".into(),
                name: "library".into(),
                version: "1.0.0".parse().unwrap(),
                source: squish_project::LockedSource::Sopack {
                    path: "first.sopack".into(),
                    checksum: acquired.checksum().to_owned(),
                },
                manifest_digest: format!("sha256:{}", "bb".repeat(32)),
                dependencies: BTreeMap::new(),
            }],
        };
        let handles = invocation.locked(&lock).unwrap();
        assert!(Arc::ptr_eq(&handles["library@1"], &acquired));
        assert_eq!(invocation.reads.get(), (1, bytes.len(), 1));
        if let squish_project::LockedSource::Sopack { checksum, .. } = &mut lock.packages[0].source
        {
            *checksum = format!("sha256:{}", "ff".repeat(32));
        }
        assert!(invocation.locked(&lock).is_err());
        assert_eq!(invocation.reads.get(), (1, bytes.len(), 1));
    }

    #[test]
    fn later_invocation_reacquires_and_rejects_mutated_malformed_archive() {
        let (directory, bytes) = fixture();
        let root = directory.path().canonicalize().unwrap();
        let first = root.join("first.sopack");
        let invocation = SopackAcquisition::new(&root);
        let (_, old) = invocation.load(&first).unwrap();
        let next = SopackAcquisition::new(&root);
        let (_, fresh) = next.load(&first).unwrap();
        assert!(!Arc::ptr_eq(&old, &fresh));
        assert_eq!(old.checksum(), fresh.checksum());
        assert_eq!(next.reads.get(), (1, bytes.len(), 1));
        std::fs::write(&first, b"not a reproducible SOPack").unwrap();
        assert!(SopackAcquisition::new(&root).load(&first).is_err());
    }
}
