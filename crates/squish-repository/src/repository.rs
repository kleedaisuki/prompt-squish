use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use glob::Pattern;
use squish_project::{
    LOCK_FILE_NAME, LockedSource, Lockfile, MANIFEST_FILE_NAME, Manifest, Workspace,
};

use crate::{
    LockedManifestSnapshot, ManifestSnapshot, PackageLocation, ProjectSnapshot, RepositoryError,
    transaction::{
        FaultInjector, FormatUpdate, NoFault, atomic_write, atomic_write_many, candidate_digest,
        commit_plan, recover_transactions, with_locked_repository,
    },
};

/// 用户可见构建产品在完整工具所有根下的固定命名空间。 / Fixed namespace for
/// user-visible build products below the complete tool-owned root.
const ARTIFACTS_DIR: &str = "artifacts";

/// 项目发现方式。 / Project discovery mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Discovery {
    /// 从该路径（文件则从父目录）向上寻找最近清单。 / Search upward from this path (or a file's parent) for the nearest manifest.
    Implicit(PathBuf),
    /// 使用指定清单或包含它的目录。 / Use the specified manifest or its containing directory.
    Explicit(PathBuf),
}

/// 具体文件系统项目仓库。 / Concrete filesystem project repository.
pub struct ProjectRepository {
    root: PathBuf,
    faults: Arc<dyn FaultInjector>,
}

impl std::fmt::Debug for ProjectRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectRepository")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl ProjectRepository {
    /// 发现仓库；显式路径绝不静默回退为父级搜索。 / Discovers a repository; an explicit path never silently falls back to ancestor search.
    pub fn discover(mode: Discovery) -> Result<Self, RepositoryError> {
        Self::discover_with_faults(mode, Arc::new(NoFault))
    }

    /// 使用确定性 fault injector 发现仓库，供恢复测试与宿主诊断使用。 / Discovers with a deterministic fault injector for recovery tests and host diagnostics.
    pub fn discover_with_faults(
        mode: Discovery,
        faults: Arc<dyn FaultInjector>,
    ) -> Result<Self, RepositoryError> {
        let manifest = match mode {
            Discovery::Explicit(path) => explicit_manifest(path)?,
            Discovery::Implicit(path) => upward_manifest(path)?,
        };
        let root = manifest
            .parent()
            .expect("manifest always has a parent")
            .canonicalize()
            .map_err(|e| RepositoryError::io(&manifest, e))?;
        let repo = Self { root, faults };
        repo.recover()?;
        Ok(repo)
    }

    /// 返回规范绝对工作区根。 / Returns the canonical absolute workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 恢复所有已作出持久提交决定但未完成的事务。 / Recovers every transaction with a durable commit decision but incomplete replacements.
    pub fn recover(&self) -> Result<(), RepositoryError> {
        crate::creation::recover_creation_ancestors(&self.root, self.faults.as_ref())?;
        recover_transactions(&self.root, self.faults.as_ref())
    }

    /// 冻结根清单、成员、锁和完整权威读集。 / Freezes the root manifest, members, lock, and complete authoritative read-set.
    pub fn snapshot(&self) -> Result<ProjectSnapshot, RepositoryError> {
        self.snapshot_workspace()
    }

    /// 冻结工作区权威状态而不获取或物化远端依赖。 / Freezes authoritative workspace state without fetching or materializing remote dependencies.
    pub fn snapshot_workspace(&self) -> Result<ProjectSnapshot, RepositoryError> {
        self.authoritative_snapshot()
    }

    /// 冻结 root/member 清单、锁原始字节、读集和 glob 成员集合。 / Freezes root/member manifests, exact lock bytes, the read-set, and glob membership.
    pub fn authoritative_snapshot(&self) -> Result<ProjectSnapshot, RepositoryError> {
        self.snapshot_coordinated(&[], false)
    }

    /// 连同非本地锁包清单冻结完整项目状态。 / Freezes complete project state including non-local locked-package manifests.
    pub fn snapshot_with_locations(
        &self,
        locations: &[crate::PackageLocation],
    ) -> Result<ProjectSnapshot, RepositoryError> {
        self.snapshot_coordinated(locations, true)
    }

    fn snapshot_coordinated(
        &self,
        locations: &[crate::PackageLocation],
        require_external: bool,
    ) -> Result<ProjectSnapshot, RepositoryError> {
        with_locked_repository(&self.root, self.faults.as_ref(), || {
            self.faults
                .check(crate::FaultPoint::SnapshotRead)
                .map_err(|e| RepositoryError::io(&self.root, e))?;
            self.snapshot_locked(locations, require_external)
        })
    }

    fn snapshot_locked(
        &self,
        locations: &[crate::PackageLocation],
        require_external: bool,
    ) -> Result<ProjectSnapshot, RepositoryError> {
        let root_file = self.root.join(MANIFEST_FILE_NAME);
        let root_snapshot = read_manifest(&self.root, &root_file)?;
        let mut manifests = vec![root_snapshot];
        if let Some(workspace) = &manifests[0].manifest.workspace {
            for dir in workspace_member_dirs(&self.root, workspace)? {
                let item = read_manifest(&self.root, &dir.join(MANIFEST_FILE_NAME))?;
                if item.manifest.workspace.is_some() {
                    return Err(RepositoryError::Layout(format!(
                        "member `{}` declares a nested workspace and therefore has ambiguous ownership",
                        item.path.display()
                    )));
                }
                if item.manifest.package.is_none() {
                    return Err(RepositoryError::Layout(format!(
                        "workspace member `{}` is not a package",
                        item.path.display()
                    )));
                }
                manifests.push(item);
            }
        }
        manifests[1..].sort_by(|a, b| a.path.cmp(&b.path));
        let mut names = BTreeSet::new();
        for item in &manifests {
            if let Some(package) = &item.manifest.package
                && !names.insert(&package.name)
            {
                return Err(RepositoryError::Layout(format!(
                    "duplicate workspace package name `{}`",
                    package.name
                )));
            }
        }
        let lock_path = self.root.join(LOCK_FILE_NAME);
        let (lock_bytes, lock_digest, lockfile) = match fs::read(&lock_path) {
            Ok(bytes) => {
                let digest = digest(b"lock", &bytes);
                let source = std::str::from_utf8(&bytes)
                    .map_err(|_| RepositoryError::Layout("xmlsquish.lock is not UTF-8".into()))?;
                let lock = Lockfile::parse(source)?;
                (Some(Arc::from(bytes)), digest, Some(lock))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, missing_digest(), None),
            Err(e) => return Err(RepositoryError::io(&lock_path, e)),
        };
        let mut read_set: BTreeMap<PathBuf, String> = manifests
            .iter()
            .map(|m| (m.path.clone(), m.digest.clone()))
            .collect();
        read_set.insert(PathBuf::from(LOCK_FILE_NAME), lock_digest.clone());
        read_set.insert(
            workspace_members_observation(),
            workspace_members_digest(&self.root)?,
        );
        let manifest_digest = manifest_set_digest(&manifests);
        let target_dir = self
            .root
            .join(manifests[0].manifest.workspace.as_ref().map_or_else(
                || PathBuf::from("target/xmlsquish"),
                |w| w.target_dir.clone(),
            ));
        // `target_dir` owns all derived state. Resolve the public-output seam once so callers
        // cannot accidentally publish beside cache, metadata, or staging state.
        // `target_dir` 拥有全部派生状态。在此唯一一次解析公开产物边界，避免调用者误将
        // 产物发布到 cache、metadata 或临时工作状态旁边。
        let artifact_dir = target_dir.join(ARTIFACTS_DIR);
        validate_output_collisions(&manifests, &artifact_dir)?;
        let package_manifests = freeze_locked_manifests(
            &self.root,
            lockfile.as_ref(),
            locations,
            &manifests,
            require_external,
        )?;
        Ok(ProjectSnapshot {
            root: self.root.clone(),
            manifests,
            lock_bytes,
            lock_digest,
            lockfile,
            read_set,
            manifest_digest,
            target_dir,
            artifact_dir,
            package_manifests,
        })
    }

    /// 提交 `squish-project` 生成的 add/remove 计划。 / Commits an add/remove plan generated by `squish-project`.
    pub fn commit(&self, plan: &squish_project::MutationPlan) -> Result<(), RepositoryError> {
        commit_plan(&self.root, plan, self.faults.as_ref())
    }

    /// 以同一 journal 协议原子替换一个格式化文件。 / Atomically replaces one formatted file using the same journal protocol.
    pub fn write_formatted(
        &self,
        relative: &Path,
        expected_digest: &str,
        bytes: &[u8],
    ) -> Result<(), RepositoryError> {
        atomic_write(
            &self.root,
            relative,
            expected_digest,
            bytes,
            self.faults.as_ref(),
        )
    }

    /// 完整预检后把所有格式化结果作为一个可恢复事务提交。 / Preflights every formatted result before committing them as one recoverable transaction.
    pub fn write_formatted_files(&self, updates: &[FormatUpdate]) -> Result<(), RepositoryError> {
        atomic_write_many(&self.root, updates, self.faults.as_ref())
    }

    /// 计算事务候选必须携带的域分离摘要。 / Computes the domain-separated digest required on a transaction candidate.
    pub fn candidate_digest(relative: &Path, bytes: &[u8]) -> Result<String, RepositoryError> {
        candidate_digest(relative, bytes)
    }
}

fn explicit_manifest(path: PathBuf) -> Result<PathBuf, RepositoryError> {
    let candidate = if path.file_name().is_some_and(|n| n == MANIFEST_FILE_NAME) {
        path
    } else {
        path.join(MANIFEST_FILE_NAME)
    };
    candidate.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RepositoryError::NotFound(candidate)
        } else {
            RepositoryError::io(candidate, e)
        }
    })
}
fn upward_manifest(path: PathBuf) -> Result<PathBuf, RepositoryError> {
    let original = path.clone();
    let mut dir = if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path
    };
    dir = dir
        .canonicalize()
        .map_err(|e| RepositoryError::io(&dir, e))?;
    loop {
        let candidate = dir.join(MANIFEST_FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            return Err(RepositoryError::NotFound(original));
        }
    }
}
fn read_manifest(root: &Path, path: &Path) -> Result<ManifestSnapshot, RepositoryError> {
    let bytes = fs::read(path).map_err(|e| RepositoryError::io(path, e))?;
    let source = std::str::from_utf8(&bytes).map_err(|_| {
        RepositoryError::Layout(format!("manifest `{}` is not UTF-8", path.display()))
    })?;
    let manifest = Manifest::parse(source)?;
    let package_dir = path
        .parent()
        .unwrap()
        .canonicalize()
        .map_err(|e| RepositoryError::io(path, e))?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| {
            RepositoryError::Layout(format!("manifest `{}` is outside root", path.display()))
        })?
        .to_path_buf();
    Ok(ManifestSnapshot {
        path: relative,
        package_dir,
        digest: digest(b"manifest", &bytes),
        bytes: Arc::from(bytes),
        manifest,
    })
}
fn expand_member(root: &Path, member: &Path) -> Result<Vec<PathBuf>, RepositoryError> {
    let text = member.to_str().ok_or_else(|| {
        RepositoryError::Layout(format!("non-Unicode glob member `{}`", member.display()))
    })?;
    if !text.contains(['*', '?', '[']) {
        return Ok(vec![root.join(member)]);
    }
    let pattern = root
        .join(member)
        .join(MANIFEST_FILE_NAME)
        .to_string_lossy()
        .into_owned();
    glob::glob(&pattern)
        .map_err(|e| RepositoryError::Layout(e.to_string()))?
        .map(|entry| {
            entry
                .map(|p| p.parent().unwrap().to_path_buf())
                .map_err(|e| {
                    RepositoryError::io(
                        e.path().to_path_buf(),
                        std::io::Error::new(e.error().kind(), e.error().to_string()),
                    )
                })
        })
        .collect()
}

pub(crate) fn workspace_members_observation() -> PathBuf {
    PathBuf::from(".xmlsquish/workspace-members.v1")
}

pub(crate) fn workspace_members_digest(root: &Path) -> Result<String, RepositoryError> {
    let root_file = root.join(MANIFEST_FILE_NAME);
    let bytes = fs::read(&root_file).map_err(|e| RepositoryError::io(&root_file, e))?;
    let source = std::str::from_utf8(&bytes)
        .map_err(|_| RepositoryError::Layout("xmlsquish.toml is not UTF-8".into()))?;
    let manifest = Manifest::parse(source)?;
    let dirs = manifest
        .workspace
        .as_ref()
        .map(|workspace| workspace_member_dirs(root, workspace))
        .transpose()?
        .unwrap_or_default();
    let mut bytes = Vec::new();
    for dir in dirs {
        let relative = dir.strip_prefix(root).map_err(|_| {
            RepositoryError::Layout(format!("member `{}` is outside workspace", dir.display()))
        })?;
        let value = slash(relative);
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    Ok(digest(b"workspace-members", &bytes))
}

pub(crate) fn workspace_member_dirs(
    root: &Path,
    workspace: &Workspace,
) -> Result<Vec<PathBuf>, RepositoryError> {
    let exclusions = workspace
        .exclude
        .iter()
        .map(|value| {
            Pattern::new(value).map_err(|e| {
                RepositoryError::Layout(format!("invalid workspace exclude `{value}`: {e}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut members = BTreeSet::new();
    for member in &workspace.members {
        for dir in expand_member(root, member)? {
            let relative = dir.strip_prefix(root).map_err(|_| {
                RepositoryError::Layout(format!(
                    "workspace member `{}` escapes the workspace root",
                    dir.display()
                ))
            })?;
            if exclusions
                .iter()
                .any(|pattern| pattern.matches(&slash(relative)))
            {
                continue;
            }
            let canonical = dir
                .canonicalize()
                .map_err(|e| RepositoryError::io(&dir, e))?;
            if !canonical.starts_with(root) {
                return Err(RepositoryError::Layout(format!(
                    "workspace member `{}` escapes through a link",
                    relative.display()
                )));
            }
            if canonical == root {
                return Err(RepositoryError::Layout(
                    "workspace root must not also be listed as a member".into(),
                ));
            }
            if !members.insert(canonical) {
                return Err(RepositoryError::Layout(format!(
                    "duplicate workspace member `{}`",
                    relative.display()
                )));
            }
        }
    }
    Ok(members.into_iter().collect())
}

fn freeze_locked_manifests(
    root: &Path,
    lock: Option<&Lockfile>,
    locations: &[PackageLocation],
    workspace_manifests: &[ManifestSnapshot],
    require_external: bool,
) -> Result<Vec<LockedManifestSnapshot>, RepositoryError> {
    let Some(lock) = lock else {
        return Ok(Vec::new());
    };
    let supplied: BTreeMap<&str, &Path> = locations
        .iter()
        .map(|item| (item.lock_id.as_str(), item.root.as_path()))
        .collect();
    let mut output = Vec::with_capacity(lock.packages.len());
    for package in &lock.packages {
        let candidate = match &package.source {
            LockedSource::Path { path, .. } => root.join(path),
            LockedSource::Workspace { member, .. } => root.join(member),
            LockedSource::Registry { .. } | LockedSource::Git { .. } => {
                let Some(path) = supplied.get(package.id.as_str()) else {
                    if require_external {
                        return Err(RepositoryError::MissingPackageLocation(package.id.clone()));
                    }
                    continue;
                };
                path.to_path_buf()
            }
        };
        let package_root = candidate
            .canonicalize()
            .map_err(|e| RepositoryError::io(&candidate, e))?;
        if let Some(existing) = workspace_manifests
            .iter()
            .find(|item| item.package_dir == package_root)
        {
            output.push(LockedManifestSnapshot {
                lock_id: package.id.clone(),
                root: package_root,
                bytes: Arc::clone(&existing.bytes),
                manifest: existing.manifest.clone(),
            });
            continue;
        }
        if !require_external {
            // Formatting and other workspace-only operations freeze precisely root/member
            // authority. A path dependency outside that set is build input, not workspace-owned.
            continue;
        }
        let path = package_root.join(MANIFEST_FILE_NAME);
        let bytes = fs::read(&path).map_err(|e| RepositoryError::io(&path, e))?;
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            RepositoryError::Layout(format!("manifest `{}` is not UTF-8", path.display()))
        })?;
        let manifest = Manifest::parse(source)?;
        output.push(LockedManifestSnapshot {
            lock_id: package.id.clone(),
            root: package_root,
            bytes: Arc::from(bytes),
            manifest,
        });
    }
    Ok(output)
}

fn validate_output_collisions(
    manifests: &[ManifestSnapshot],
    target_dir: &Path,
) -> Result<(), RepositoryError> {
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    for item in manifests {
        let Some(package) = &item.manifest.package else {
            continue;
        };
        for (target_name, target) in &item.manifest.targets {
            let path = target_dir.join(target.output_path(target_name));
            let key = slash(&path).to_ascii_lowercase();
            let label = format!("{}:{target_name}", package.name);
            if let Some(first) = outputs.insert(key, label.clone()) {
                return Err(RepositoryError::Layout(format!(
                    "target output collision between `{first}` and `{label}` at `{}`",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}
pub(crate) fn digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"xmlsquish\0repository\0");
    h.update(domain);
    h.update(&[0]);
    h.update(bytes);
    format!("blake3:{}", h.finalize().to_hex())
}
pub(crate) fn missing_digest() -> String {
    digest(b"missing", b"")
}
fn manifest_set_digest(items: &[ManifestSnapshot]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"xmlsquish\0manifest-set\0");
    for item in items {
        let p = slash(&item.path);
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
        h.update(&(item.bytes.len() as u64).to_le_bytes());
        h.update(&item.bytes);
    }
    format!("blake3:{}", h.finalize().to_hex())
}
fn slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier, mpsc},
        time::Duration,
    };

    use tempfile::TempDir;

    use super::*;

    struct SnapshotBarrier {
        entered: Barrier,
        release: Barrier,
    }

    impl FaultInjector for SnapshotBarrier {
        fn check(&self, point: crate::FaultPoint) -> std::io::Result<()> {
            if point == crate::FaultPoint::SnapshotRead {
                self.entered.wait();
                self.release.wait();
            }
            Ok(())
        }
    }

    const MEMBER: &str = r#"manifest-version = 1
[package]
name = "member"
version = "1.0.0"
source-root = "src"

[target.main]
entry = "special/entry.xml"

[exports]
api = "exports/api.xml"
"#;

    fn write(path: impl AsRef<Path>, text: &str) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn explicit_discovery_never_falls_back_to_an_ancestor() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\n",
        );
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();

        let result = ProjectRepository::discover(Discovery::Explicit(child));

        assert!(matches!(result, Err(RepositoryError::NotFound(_))));
    }

    #[test]
    fn workspace_members_are_sorted_excluded_and_use_root_target_policy() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            r#"manifest-version = 1
[workspace]
members = ["packages/*"]
exclude = ["packages/skip"]
target-dir = "build-output"
"#,
        );
        write(temp.path().join("packages/z/xmlsquish.toml"), MEMBER);
        write(
            temp.path().join("packages/skip/xmlsquish.toml"),
            &MEMBER.replace("member", "skip"),
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();

        let snapshot = repo.snapshot().unwrap();

        assert_eq!(snapshot.manifests().len(), 2);
        assert!(
            snapshot.manifests()[1]
                .path
                .ends_with("packages/z/xmlsquish.toml")
        );
        assert_eq!(
            snapshot
                .resolve_target("member", "main", None)
                .unwrap()
                .output,
            temp.path()
                .canonicalize()
                .unwrap()
                .join("build-output/artifacts/main.prompt")
        );
    }

    #[test]
    fn default_and_declared_outputs_are_confined_to_the_artifacts_namespace() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            r#"manifest-version = 1
[package]
name = "root"
version = "1.0.0"
source-root = "src"

[target.default]
entry = "src/default.xml"

[target.cache]
entry = "src/cache.xml"
output = "cache/cache.prompt"

[target.metadata]
entry = "src/metadata.xml"
output = "metadata/metadata.prompt"

[target.work]
entry = "src/work.xml"
output = "work/work.prompt"
"#,
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();
        let snapshot = repo.snapshot().unwrap();
        let root = temp.path().canonicalize().unwrap().join("target/xmlsquish");

        assert_eq!(
            snapshot
                .resolve_target("root", "default", None)
                .unwrap()
                .output,
            root.join("artifacts/default.prompt")
        );
        for (name, relative) in [
            ("cache", "cache/cache.prompt"),
            ("metadata", "metadata/metadata.prompt"),
            ("work", "work/work.prompt"),
        ] {
            let output = snapshot.resolve_target("root", name, None).unwrap().output;
            assert_eq!(output, root.join("artifacts").join(relative));
            assert!(!output.starts_with(root.join(name)));
        }
    }

    #[test]
    fn owned_sources_cover_recursive_xml_and_explicit_nonstandard_paths() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"packages/member\"]\n",
        );
        write(temp.path().join("packages/member/xmlsquish.toml"), MEMBER);
        write(temp.path().join("packages/member/src/a.xml"), "<a/>");
        write(temp.path().join("packages/member/src/nested/b.xml"), "<b/>");
        write(
            temp.path().join("packages/member/src/generated.i.xml"),
            "<i/>",
        );
        write(
            temp.path().join("packages/member/special/entry.xml"),
            "<entry/>",
        );
        write(
            temp.path().join("packages/member/exports/api.xml"),
            "<api/>",
        );
        write(
            temp.path().join(LOCK_FILE_NAME),
            r#"lock-version = 1
resolver-version = "test/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
id = "member@1"
name = "member"
version = "1.0.0"
manifest-digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
source = { kind = "workspace", member = "packages/member", mutable = true }
"#,
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();

        let sources = repo.snapshot().unwrap().owned_sources(&[]).unwrap();
        let paths = sources
            .iter()
            .map(|source| source.id.path().as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                "exports/api.xml",
                "special/entry.xml",
                "src/a.xml",
                "src/nested/b.xml"
            ]
        );
        assert!(
            sources
                .iter()
                .all(|source| source.package.package_name == "member")
        );
    }

    #[test]
    fn snapshot_holds_coordination_lock_across_authoritative_reads() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\n",
        );
        write(temp.path().join("a.xml"), "old");
        let barrier = Arc::new(SnapshotBarrier {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        });
        let repo = Arc::new(
            ProjectRepository::discover_with_faults(
                Discovery::Explicit(temp.path().into()),
                barrier.clone(),
            )
            .unwrap(),
        );
        let snapshot_repo = Arc::clone(&repo);
        let snapshot_thread = std::thread::spawn(move || snapshot_repo.snapshot().unwrap());
        barrier.entered.wait();
        let writer_repo = Arc::clone(&repo);
        let (sent, received) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let digest = ProjectRepository::candidate_digest(Path::new("a.xml"), b"old").unwrap();
            sent.send(writer_repo.write_formatted(Path::new("a.xml"), &digest, b"new"))
                .unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(50)).is_err());
        barrier.release.wait();
        snapshot_thread.join().unwrap();
        assert!(
            received
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok()
        );
        writer.join().unwrap();
    }

    #[test]
    fn a_new_glob_member_invalidates_an_old_read_set() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"packages/*\"]\n",
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();
        let snapshot = repo.snapshot().unwrap();
        write(
            temp.path().join("packages/phantom/xmlsquish.toml"),
            &MEMBER.replace("member", "phantom"),
        );
        let plan = squish_project::MutationPlan {
            id: squish_project::TransactionId("phantom".into()),
            kind: squish_project::MutationKind::AddDependency { alias: "x".into() },
            manifest_digest: snapshot.manifest_digest().into(),
            observations: snapshot.read_set().clone(),
            files: Vec::new(),
            retry_limit: 0,
        };

        let error = repo.commit(&plan).unwrap_err();

        assert!(
            matches!(error, RepositoryError::Contended(paths) if paths.contains(&workspace_members_observation()))
        );
    }

    #[test]
    fn two_package_targets_cannot_publish_the_same_output() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"a\", \"b\"]\n",
        );
        write(
            temp.path().join("a/xmlsquish.toml"),
            &MEMBER.replace("member", "a"),
        );
        write(
            temp.path().join("b/xmlsquish.toml"),
            &MEMBER.replace("member", "b"),
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();

        let error = repo.snapshot().unwrap_err().to_string();
        let coordinate = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("target/xmlsquish/artifacts/main.prompt");

        assert!(error.contains("a:main"));
        assert!(error.contains("b:main"));
        assert!(error.contains(&coordinate.display().to_string()));
    }

    #[test]
    fn external_package_source_enumeration_uses_its_frozen_manifest() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\n",
        );
        let checkout = temp.path().join("downloaded/remote");
        write(
            checkout.join(MANIFEST_FILE_NAME),
            &MEMBER.replace("member", "remote"),
        );
        write(checkout.join("src/a.xml"), "<a/>");
        write(checkout.join("special/entry.xml"), "<entry/>");
        write(checkout.join("exports/api.xml"), "<api/>");
        write(
            temp.path().join(LOCK_FILE_NAME),
            r#"lock-version = 1
resolver-version = "test/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
id = "remote@1"
name = "remote"
version = "1.0.0"
manifest-digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
source = { kind = "registry", registry = "test", checksum = "sha256:cccccccccccccccccccccccccccccccc" }
"#,
        );
        let location = PackageLocation {
            lock_id: "remote@1".into(),
            root: checkout.clone(),
        };
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();
        let snapshot = repo
            .snapshot_with_locations(std::slice::from_ref(&location))
            .unwrap();
        write(
            checkout.join(MANIFEST_FILE_NAME),
            &MEMBER.replace("member", "changed"),
        );

        let sources = snapshot.owned_sources(&[location]).unwrap();

        assert!(
            sources
                .iter()
                .all(|source| source.id.package().as_str() == "remote")
        );
        assert!(
            sources
                .iter()
                .any(|source| source.id.path().as_str() == "special/entry.xml")
        );
        assert_eq!(
            snapshot.locked_manifests()[0]
                .manifest
                .package
                .as_ref()
                .unwrap()
                .name,
            "remote"
        );
    }

    #[test]
    fn workspace_snapshot_with_remote_lock_needs_no_checkout() {
        let temp = TempDir::new().unwrap();
        write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"member\"]\n",
        );
        write(temp.path().join("member/xmlsquish.toml"), MEMBER);
        write(temp.path().join("member/src/a.xml"), "<a/>");
        write(temp.path().join("member/special/entry.xml"), "<entry/>");
        write(temp.path().join("member/exports/api.xml"), "<api/>");
        write(
            temp.path().join(LOCK_FILE_NAME),
            r#"lock-version = 1
resolver-version = "test/1"
manifest-digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
id = "remote@1"
name = "remote"
version = "1.0.0"
manifest-digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
source = { kind = "registry", registry = "test", checksum = "sha256:cccccccccccccccccccccccccccccccc" }
"#,
        );
        let repo = ProjectRepository::discover(Discovery::Explicit(temp.path().into())).unwrap();

        let snapshot = repo.snapshot_workspace().unwrap();
        let sources = snapshot.owned_workspace_sources().unwrap();

        assert!(snapshot.lock_bytes().is_some());
        assert!(snapshot.locked_manifests().is_empty());
        assert!(
            sources
                .iter()
                .all(|source| source.id.package().as_str() == "member")
        );
        assert!(matches!(
            snapshot.owned_sources(&[]),
            Err(RepositoryError::MissingPackageLocation(id)) if id == "remote@1"
        ));
    }
}
