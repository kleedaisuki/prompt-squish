use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use squish_ir::PackageInstanceId;
use squish_project::{Limits, LockedPackage, LockedSource, Lockfile, Manifest, Profile, Target};
use squish_source::{LogicalPath, PackageId, SourceId, SourceLocator};

use crate::RepositoryError;

/// 一份清单的精确冻结表示。 / Exact frozen representation of one manifest.
#[derive(Clone, Debug)]
pub struct ManifestSnapshot {
    /// 工作区相对清单路径。 / Workspace-relative manifest path.
    pub path: PathBuf,
    /// 包目录的规范绝对路径。 / Canonical absolute package directory.
    pub package_dir: PathBuf,
    /// 从磁盘读取的原始字节。 / Exact bytes read from disk.
    pub bytes: Arc<[u8]>,
    /// 原始字节的域分离摘要。 / Domain-separated digest of the exact bytes.
    pub digest: String,
    /// 由 `squish-project` 解析的纯领域模型。 / Pure domain model parsed by `squish-project`.
    pub manifest: Manifest,
}

/// 锁包的物理位置。 / Physical location of a locked package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageLocation {
    /// 锁文件内不透明包 ID。 / Opaque package ID from the lockfile.
    pub lock_id: String,
    /// 已校验的包根目录。 / Validated package root directory.
    pub root: PathBuf,
}

/// 锁节点到编译管线身份和物理位置的精确映射。 / Exact lock-node mapping to pipeline identity and physical location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPackage {
    /// 锁文件内不透明节点 ID。 / Opaque node ID in the lockfile.
    pub lock_id: String,
    /// 跨编译阶段稳定的包实例身份。 / Stable package instance identity across compilation stages.
    pub instance: PackageInstanceId,
    /// 仅供读取适配器使用的物理根位置。 / Physical root locator used only by read adapters.
    pub locator: SourceLocator,
}

/// 完整预加载所需的一个项目源码。 / One project source required for complete preloading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSource {
    /// 与检出位置无关的源码身份。 / Source identity independent of checkout location.
    pub id: SourceId,
    /// 只用于读取的物理位置。 / Physical locator used only for reading.
    pub locator: SourceLocator,
    /// 解析器锁定的精确包实例。 / Exact package instance pinned by the resolver.
    pub package: PackageInstanceId,
}

/// target 与继承 profile 合并后的不可变构建参数。 / Immutable build parameters after target/profile inheritance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTarget {
    pub package_dir: PathBuf,
    pub target_name: String,
    pub entry: PathBuf,
    pub backend: String,
    pub output: PathBuf,
    pub args: BTreeMap<String, String>,
    pub limits: Limits,
    pub features: std::collections::BTreeSet<String>,
    pub debug_info: Option<bool>,
    pub optimization: Option<String>,
}

/// 一次加载期间完全冻结的权威项目状态。 / Fully frozen authoritative project state for one load.
#[derive(Clone, Debug)]
pub struct ProjectSnapshot {
    pub(crate) root: PathBuf,
    pub(crate) manifests: Vec<ManifestSnapshot>,
    pub(crate) lock_bytes: Option<Arc<[u8]>>,
    pub(crate) lock_digest: String,
    pub(crate) lockfile: Option<Lockfile>,
    pub(crate) read_set: BTreeMap<PathBuf, String>,
    pub(crate) manifest_digest: String,
    pub(crate) target_dir: PathBuf,
    pub(crate) package_manifests: Vec<LockedManifestSnapshot>,
}

#[derive(Clone, Debug)]
/// 锁包清单的冻结表示。 / Frozen representation of a locked-package manifest.
pub struct LockedManifestSnapshot {
    /// 锁节点 ID。 / Lock-node ID.
    pub lock_id: String,
    /// 冻结时使用的规范包根。 / Canonical package root used when frozen.
    pub root: PathBuf,
    /// 精确清单字节。 / Exact manifest bytes.
    pub bytes: Arc<[u8]>,
    /// 从精确字节解析的清单。 / Manifest parsed from the exact bytes.
    pub manifest: Manifest,
}

impl ProjectSnapshot {
    /// 返回规范绝对工作区根。 / Returns the canonical absolute workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// 返回根清单及稳定排序的成员清单。 / Returns the root manifest followed by stably sorted members.
    pub fn manifests(&self) -> &[ManifestSnapshot] {
        &self.manifests
    }
    /// 返回锁文件的精确字节（若存在）。 / Returns exact lockfile bytes, when present.
    pub fn lock_bytes(&self) -> Option<&[u8]> {
        self.lock_bytes.as_deref()
    }
    /// 返回已验证锁模型（若存在）。 / Returns the validated lock model, when present.
    pub fn lockfile(&self) -> Option<&Lockfile> {
        self.lockfile.as_ref()
    }
    /// 返回完整权威读集；不存在的锁文件使用稳定缺失摘要。 / Returns the complete authoritative read-set; a missing lock uses a stable absence digest.
    pub fn read_set(&self) -> &BTreeMap<PathBuf, String> {
        &self.read_set
    }
    /// 返回确定性完整清单集摘要。 / Returns the deterministic complete-manifest-set digest.
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
    /// 返回锁文件观察摘要。 / Returns the observed lockfile digest.
    pub fn lock_digest(&self) -> &str {
        &self.lock_digest
    }
    /// 返回每个锁包已冻结的清单。 / Returns the frozen manifest for every locked package.
    pub fn locked_manifests(&self) -> &[LockedManifestSnapshot] {
        &self.package_manifests
    }

    /// 枚举仅由当前 workspace 拥有的 XML 源码，不访问远端依赖检出。 / Enumerates XML sources owned only by this workspace without accessing remote dependency checkouts.
    pub fn owned_workspace_sources(&self) -> Result<Vec<ProjectSource>, RepositoryError> {
        let mut output = BTreeMap::new();
        for frozen in &self.manifests {
            let Some(package) = &frozen.manifest.package else {
                continue;
            };
            let relative_root = frozen.package_dir.strip_prefix(&self.root).map_err(|_| {
                RepositoryError::Layout(format!(
                    "package `{}` is outside workspace root",
                    package.name
                ))
            })?;
            let instance = PackageInstanceId {
                source_kind: 4,
                canonical_source: normalize_text(relative_root),
                package_name: package.name.clone(),
                exact_revision: frozen.digest.clone(),
            };
            append_manifest_sources(
                &frozen.manifest,
                &frozen.package_dir,
                &self.target_dir,
                &instance,
                &mut output,
            )?;
        }
        Ok(output.into_values().collect())
    }

    /// 稳定枚举所有已解析包拥有的 XML 源码，以便在 frontend 前封闭源码快照。 / Stably enumerates every resolved package-owned XML source so the source snapshot can be sealed before the frontend runs.
    ///
    /// 目录遍历不会跟随目录符号链接，结果按逻辑源码身份排序。除递归扫描 `source-root`
    /// 外，显式 target entry 和 export 也会被包含，因此非标准布局不会变成特殊情况。
    /// Directory traversal does not follow directory symlinks and results are sorted by logical
    /// source identity. Explicit target entries and exports are included in addition to recursive
    /// `source-root` scanning, so non-standard layouts are not a special case.
    pub fn owned_sources(
        &self,
        locations: &[PackageLocation],
    ) -> Result<Vec<ProjectSource>, RepositoryError> {
        let packages = self.resolved_packages(locations)?;
        let locked = self
            .lockfile
            .as_ref()
            .ok_or_else(|| RepositoryError::Unknown("xmlsquish.lock".into()))?;
        let by_id: BTreeMap<_, _> = packages.iter().map(|p| (p.lock_id.as_str(), p)).collect();
        let mut output = BTreeMap::new();
        for locked_package in &locked.packages {
            let package = by_id
                .get(locked_package.id.as_str())
                .expect("resolved_packages maps every lock node");
            let frozen = self
                .package_manifests
                .iter()
                .find(|manifest| manifest.lock_id == locked_package.id)
                .ok_or_else(|| {
                    RepositoryError::MissingPackageLocation(locked_package.id.clone())
                })?;
            if package.locator.as_path() != frozen.root {
                return Err(RepositoryError::Layout(format!(
                    "locked package `{}` location differs from its frozen manifest location",
                    locked_package.id
                )));
            }
            let root = frozen.root.as_path();
            let manifest = &frozen.manifest;
            let declared = manifest.package.as_ref().ok_or_else(|| {
                RepositoryError::Layout(format!(
                    "locked package `{}` points to a virtual workspace",
                    locked_package.id
                ))
            })?;
            if declared.name != locked_package.name {
                return Err(RepositoryError::Layout(format!(
                    "locked package `{}` names `{}` but checkout declares `{}`",
                    locked_package.id, locked_package.name, declared.name
                )));
            }
            let mut paths = BTreeSet::new();
            let excluded = [
                root.join("target"),
                root.join(".cache"),
                root.join(".xmlsquish"),
                self.target_dir.clone(),
            ];
            collect_xml(
                &root.join(&declared.source_root),
                root,
                &excluded,
                &mut paths,
            )?;
            paths.extend(manifest.targets.values().map(|target| target.entry.clone()));
            paths.extend(manifest.exports.values().cloned());
            let identity = PackageId::new(declared.name.clone()).map_err(|e| {
                RepositoryError::Layout(format!("invalid package source identity: {e}"))
            })?;
            for path in paths {
                let logical_text = normalize_text(&path);
                let logical = LogicalPath::new(&logical_text).map_err(|e| {
                    RepositoryError::Layout(format!(
                        "invalid source path `{}` for package `{}`: {e}",
                        path.display(),
                        declared.name
                    ))
                })?;
                let id = SourceId::new(identity.clone(), logical);
                let locator = SourceLocator::file(root.join(&path));
                let source = ProjectSource {
                    id: id.clone(),
                    locator,
                    package: package.instance.clone(),
                };
                if output.insert(id, source).is_some() {
                    return Err(RepositoryError::Layout(format!(
                        "duplicate source identity in package `{}`",
                        declared.name
                    )));
                }
            }
        }
        Ok(output.into_values().collect())
    }

    /// 按包名解析 target 和 profile 继承。 / Resolves target and profile inheritance by package name.
    pub fn resolve_target(
        &self,
        package: &str,
        target_name: &str,
        profile_name: Option<&str>,
    ) -> Result<ResolvedTarget, RepositoryError> {
        let item = self
            .manifests
            .iter()
            .find(|m| {
                m.manifest
                    .package
                    .as_ref()
                    .is_some_and(|p| p.name == package)
            })
            .ok_or_else(|| RepositoryError::Unknown(format!("package `{package}`")))?;
        let target =
            item.manifest.targets.get(target_name).ok_or_else(|| {
                RepositoryError::Unknown(format!("target `{package}:{target_name}`"))
            })?;
        let profile = profile_name
            .map(|name| merged_profile(&item.manifest, name))
            .transpose()?;
        Ok(resolve(
            item,
            target_name,
            target,
            profile.as_ref(),
            &self.target_dir,
        ))
    }

    /// 将锁图逐节点映射到精确编译身份；远端节点必须提供 checkout 根。 / Maps every lock node to an exact compiler identity; remote nodes require checkout roots.
    pub fn resolved_packages(
        &self,
        locations: &[PackageLocation],
    ) -> Result<Vec<ResolvedPackage>, RepositoryError> {
        let lock = self
            .lockfile
            .as_ref()
            .ok_or_else(|| RepositoryError::Unknown("xmlsquish.lock".into()))?;
        let locations: BTreeMap<&str, &Path> = locations
            .iter()
            .map(|p| (p.lock_id.as_str(), p.root.as_path()))
            .collect();
        lock.packages
            .iter()
            .map(|package| self.map_package(package, &locations))
            .collect()
    }

    fn map_package(
        &self,
        package: &LockedPackage,
        supplied: &BTreeMap<&str, &Path>,
    ) -> Result<ResolvedPackage, RepositoryError> {
        let (kind, canonical, revision, local): (u16, String, String, Option<PathBuf>) =
            match &package.source {
                LockedSource::Registry { registry, checksum } => (
                    1,
                    registry.clone(),
                    format!("{}@{checksum}", package.version),
                    None,
                ),
                LockedSource::Git {
                    repository,
                    revision,
                    checksum,
                } => (
                    2,
                    repository.clone(),
                    format!("{revision}@{checksum}"),
                    None,
                ),
                LockedSource::Path { path, .. } => (
                    3,
                    normalize_text(path),
                    package.manifest_digest.clone(),
                    Some(self.root.join(path)),
                ),
                LockedSource::Workspace { member, .. } => (
                    4,
                    normalize_text(member),
                    package.manifest_digest.clone(),
                    Some(self.root.join(member)),
                ),
            };
        let root = match local {
            Some(path) => path,
            None => supplied
                .get(package.id.as_str())
                .map(|p| p.to_path_buf())
                .ok_or_else(|| RepositoryError::MissingPackageLocation(package.id.clone()))?,
        };
        let root = root
            .canonicalize()
            .map_err(|e| RepositoryError::io(&root, e))?;
        Ok(ResolvedPackage {
            lock_id: package.id.clone(),
            instance: PackageInstanceId {
                source_kind: kind,
                canonical_source: canonical,
                package_name: package.name.clone(),
                exact_revision: revision,
            },
            locator: SourceLocator::file(root),
        })
    }
}

fn merged_profile(manifest: &Manifest, name: &str) -> Result<Profile, RepositoryError> {
    let mut chain = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    let mut cursor = Some(name);
    while let Some(current) = cursor {
        if !visited.insert(current) {
            return Err(RepositoryError::Layout(format!(
                "profile inheritance cycle at `{current}`"
            )));
        }
        let profile = manifest
            .profiles
            .get(current)
            .ok_or_else(|| RepositoryError::Unknown(format!("profile `{current}`")))?;
        chain.push(profile);
        cursor = profile.inherits.as_deref();
    }
    let mut result = Profile {
        inherits: None,
        debug_info: None,
        optimization: None,
        args: BTreeMap::new(),
        limits: Limits::default(),
        features: Default::default(),
    };
    for profile in chain.into_iter().rev() {
        if profile.debug_info.is_some() {
            result.debug_info = profile.debug_info;
        }
        if profile.optimization.is_some() {
            result.optimization.clone_from(&profile.optimization);
        }
        result.args.extend(profile.args.clone());
        merge_limits(&mut result.limits, &profile.limits);
        result.features.extend(profile.features.iter().cloned());
    }
    Ok(result)
}

fn resolve(
    item: &ManifestSnapshot,
    name: &str,
    target: &Target,
    profile: Option<&Profile>,
    target_dir: &Path,
) -> ResolvedTarget {
    let mut args = target.args.clone();
    let mut limits = target.limits.clone();
    let mut features = target.features.clone();
    let (debug_info, optimization) = if let Some(profile) = profile {
        args.extend(profile.args.clone());
        merge_limits(&mut limits, &profile.limits);
        features.extend(profile.features.iter().cloned());
        (profile.debug_info, profile.optimization.clone())
    } else {
        (None, None)
    };
    ResolvedTarget {
        package_dir: item.package_dir.clone(),
        target_name: name.into(),
        entry: item.package_dir.join(&target.entry),
        backend: target.backend.clone(),
        output: target_dir.join(target.output_path(name)),
        args,
        limits,
        features,
        debug_info,
        optimization,
    }
}

fn merge_limits(dst: &mut Limits, src: &Limits) {
    if src.max_depth.is_some() {
        dst.max_depth = src.max_depth;
    }
    if src.max_expansions.is_some() {
        dst.max_expansions = src.max_expansions;
    }
    if src.max_output_bytes.is_some() {
        dst.max_output_bytes = src.max_output_bytes;
    }
}
fn normalize_text(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn collect_xml(
    dir: &Path,
    package_root: &Path,
    excluded: &[PathBuf],
    output: &mut BTreeSet<PathBuf>,
) -> Result<(), RepositoryError> {
    if excluded.iter().any(|root| dir.starts_with(root)) {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(RepositoryError::io(dir, e)),
    };
    let mut entries = entries
        .map(|entry| entry.map_err(|e| RepositoryError::io(dir, e)))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let ty = entry
            .file_type()
            .map_err(|e| RepositoryError::io(entry.path(), e))?;
        if ty.is_dir() {
            collect_xml(&entry.path(), package_root, excluded, output)?;
        } else if ty.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            && !entry.path().to_string_lossy().ends_with(".i.xml")
            && !entry.path().to_string_lossy().ends_with(".o.xml")
        {
            let relative = entry
                .path()
                .strip_prefix(package_root)
                .map_err(|_| {
                    RepositoryError::Layout(format!(
                        "source `{}` is outside package root",
                        entry.path().display()
                    ))
                })?
                .to_path_buf();
            output.insert(relative);
        }
    }
    Ok(())
}

fn append_manifest_sources(
    manifest: &Manifest,
    root: &Path,
    target_dir: &Path,
    instance: &PackageInstanceId,
    output: &mut BTreeMap<SourceId, ProjectSource>,
) -> Result<(), RepositoryError> {
    let declared = manifest.package.as_ref().ok_or_else(|| {
        RepositoryError::Layout(format!("package root `{}` has no package", root.display()))
    })?;
    let mut paths = BTreeSet::new();
    let excluded = [
        root.join("target"),
        root.join(".cache"),
        root.join(".xmlsquish"),
        target_dir.to_path_buf(),
    ];
    collect_xml(
        &root.join(&declared.source_root),
        root,
        &excluded,
        &mut paths,
    )?;
    paths.extend(manifest.targets.values().map(|target| target.entry.clone()));
    paths.extend(manifest.exports.values().cloned());
    let identity = PackageId::new(declared.name.clone())
        .map_err(|e| RepositoryError::Layout(format!("invalid package source identity: {e}")))?;
    for path in paths {
        let logical = LogicalPath::new(normalize_text(&path)).map_err(|e| {
            RepositoryError::Layout(format!(
                "invalid source path `{}` for package `{}`: {e}",
                path.display(),
                declared.name
            ))
        })?;
        let id = SourceId::new(identity.clone(), logical);
        let source = ProjectSource {
            id: id.clone(),
            locator: SourceLocator::file(root.join(path)),
            package: instance.clone(),
        };
        if output.insert(id, source).is_some() {
            return Err(RepositoryError::Layout(format!(
                "duplicate source identity in package `{}`",
                declared.name
            )));
        }
    }
    Ok(())
}
