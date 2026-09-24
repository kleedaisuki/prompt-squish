//! 确定性的项目依赖解析器。 / Deterministic project dependency resolver.
//!
//! 本 crate 只拥有求解策略，不拥有 HTTP、Git 命令或文件系统。四类来源均通过窄端口注入，
//! 因而在线、离线和测试使用完全相同的算法。 / This crate owns resolution policy, not
//! HTTP, Git commands, or the filesystem. All four source kinds enter through narrow ports,
//! so online, offline, and tests exercise the same algorithm.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use squish_project::{
    DependencyDetail, DependencyResolver, DependencySpec, GitReference, LockedPackage,
    LockedSource, Lockfile, Manifest, ResolutionInput, ResolutionMode,
};
use thiserror::Error;

/// 本实现的算法与锁协议版本。 / Algorithm and lock protocol version implemented here.
pub const RESOLVER_VERSION: &str = "squish-backtracking/1";
const DEFAULT_REGISTRY: &str = "default";
const MANIFEST_FILE: &str = "xmlsquish.toml";

/// 端口可进行的外部访问级别。 / External access permitted to a source port.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    /// 可刷新远程状态。 / Remote state may be refreshed.
    Online,
    /// 只能读取本地缓存。 / Only locally cached state may be read.
    LocalOnly,
}

/// 外部来源不可用的结构化原因。 / Structured reason an external source is unavailable.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{identity}: {detail}")]
pub struct SourceUnavailable {
    /// 稳定、可诊断的来源身份。 / Stable diagnostic source identity.
    pub identity: String,
    /// Adapter-provided operational detail. / 适配器提供的操作细节。
    pub detail: String,
}

/// Registry 中一个精确、已校验和的候选版本。 / One exact checksummed registry candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCandidate {
    /// Exact published version. / 精确发布版本。
    pub version: Version,
    /// Immutable archive digest in `algorithm:hex` form. / `algorithm:hex` 形式的不可变归档摘要。
    pub checksum: String,
    /// Manifest contained in the archive. / 归档内清单。
    pub manifest: Manifest,
}

/// Registry 索引与内容可用性端口。 / Registry index and content-availability port.
pub trait RegistryPort {
    /// 返回给定包的全部本地/远程候选；顺序不影响结果。 / Returns all candidates; input order cannot affect resolution.
    fn candidates(
        &self,
        registry: &str,
        package: &str,
        access: Access,
    ) -> Result<Vec<RegistryCandidate>, SourceUnavailable>;

    /// 判断 frozen 构建所需归档是否已在本地。 / Reports whether an archive needed by a frozen build is local.
    fn contains(&self, registry: &str, package: &str, version: &Version, checksum: &str) -> bool;
}

/// 已固定 commit 的 Git 候选。 / Git candidate pinned to an immutable commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCandidate {
    /// Full 40- or 64-hex object ID. / 完整 40 或 64 位十六进制对象 ID。
    pub revision: String,
    /// Immutable tree/content digest. / 不可变 tree/content 摘要。
    pub checksum: String,
    /// Manifest at that revision. / 该 revision 的清单。
    pub manifest: Manifest,
}

/// Git 引用解析与 checkout 可用性端口。 / Git reference and checkout-availability port.
pub trait GitPort {
    /// 将 branch/tag/rev 解析为精确 commit。 / Resolves a branch/tag/rev to an exact commit.
    fn resolve(
        &self,
        repository: &str,
        reference: &GitReference,
        access: Access,
    ) -> Result<GitCandidate, SourceUnavailable>;

    /// 判断精确 checkout 是否已在本地。 / Reports whether an exact checkout is locally available.
    fn contains(&self, repository: &str, revision: &str, checksum: &str) -> bool;
}

/// 本地 path 请求。 / Local path request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalRequest<'a> {
    /// Referring manifest's workspace-relative path. / 发起引用清单的工作区相对路径。
    pub from_manifest: &'a str,
    /// Dependency locator relative to that manifest directory. / 相对该清单目录的依赖定位器。
    pub path: &'a Path,
}

/// 文件系统返回的已规范化本地包。 / Normalized local package returned by the filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPackage {
    /// Stable workspace-relative manifest path. / 稳定的工作区相对清单路径。
    pub manifest_path: String,
    /// Validated manifest. / 已验证清单。
    pub manifest: Manifest,
}

/// 未预载 path 包的文件系统端口。 / Filesystem port for path packages not already preloaded.
pub trait FilesystemPort {
    /// 加载且规范化一个 path dependency。 / Loads and normalizes a path dependency.
    fn load(&self, request: LocalRequest<'_>) -> Result<LocalPackage, SourceUnavailable>;
}

/// 无额外本地 I/O 的默认端口；适合完整 manifest 快照。 / No-I/O default for complete manifest snapshots.
#[derive(Clone, Copy, Debug, Default)]
pub struct SnapshotOnly;

impl FilesystemPort for SnapshotOnly {
    fn load(&self, request: LocalRequest<'_>) -> Result<LocalPackage, SourceUnavailable> {
        Err(SourceUnavailable {
            identity: request.path.display().to_string(),
            detail: "local manifest is absent from the resolution snapshot".into(),
        })
    }
}

/// 不可满足约束中的一条可复现证据。 / One reproducible fact in an unsatisfiable constraint set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conflict {
    /// Referring package. / 发起引用的包。
    pub depender: String,
    /// Dependency alias. / 依赖别名。
    pub alias: String,
    /// Requested package name. / 请求的包名。
    pub package: String,
    /// Human-readable exact constraint. / 可读的精确约束。
    pub requirement: String,
}

/// 解析失败；逻辑冲突与外部不可用明确分离。 / Resolution failure, separating logical conflicts from unavailable state.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResolveError {
    /// Manifest set is internally malformed for resolution. / manifest 集对解析而言内部非法。
    #[error("invalid resolution input: {0}")]
    InvalidInput(String),
    /// Locked/frozen was requested without a lock. / 请求 locked/frozen 但没有锁。
    #[error("resolution mode requires an existing lockfile")]
    LockRequired,
    /// Lock does not describe current intent. / 锁与当前意图不一致。
    #[error("lockfile is not current: {0}")]
    LockOutdated(String),
    /// Available package metadata proves the requirements unsatisfiable. / 可用包元数据证明要求不可满足。
    #[error("dependency constraints are unsatisfiable")]
    Unsatisfiable {
        /// Deterministically ordered conflict explanation. / 确定性排序的冲突解释。
        conflicts: Vec<Conflict>,
    },
    /// Required metadata/content could not be obtained under current policy. / 当前策略下无法获得所需元数据/内容。
    #[error("dependency source unavailable: {0}")]
    Unavailable(SourceUnavailable),
}

/// 可注入来源、lock-first 且确定性的解析器。 / Injectable, lock-first deterministic resolver.
pub struct Resolver<R, G, F = SnapshotOnly> {
    registry: R,
    git: G,
    filesystem: F,
}

impl<R, G> Resolver<R, G, SnapshotOnly> {
    /// 使用完整 manifest snapshot 构造解析器。 / Constructs a resolver for a complete manifest snapshot.
    pub fn new(registry: R, git: G) -> Self {
        Self {
            registry,
            git,
            filesystem: SnapshotOnly,
        }
    }
}

impl<R, G, F> Resolver<R, G, F> {
    /// 注入 path 包加载器。 / Injects the loader for path packages.
    pub fn with_filesystem<T>(self, filesystem: T) -> Resolver<R, G, T> {
        Resolver {
            registry: self.registry,
            git: self.git,
            filesystem,
        }
    }
}

impl<R: RegistryPort, G: GitPort, F: FilesystemPort> DependencyResolver for Resolver<R, G, F> {
    type Error = ResolveError;

    fn resolve(&self, input: ResolutionInput<'_>) -> Result<Lockfile, Self::Error> {
        if matches!(input.mode, ResolutionMode::Locked | ResolutionMode::Frozen) {
            let lock = input.prior_lock.ok_or(ResolveError::LockRequired)?;
            self.validate_locked(input.manifests, input.manifest_digest, lock)?;
            if input.mode == ResolutionMode::Frozen {
                self.validate_frozen_content(lock)?;
            }
            return Ok(lock.clone());
        }

        // An already-current lock is a complete solution. Avoiding every source query here is
        // both the minimal-change rule and what makes ordinary builds resilient to registry
        // outages. / 已是当前状态的锁就是完整解；跳过来源查询既实现最小变更，也让普通构建耐受 registry 故障。
        if let Some(lock) = input.prior_lock
            && self
                .validate_locked(input.manifests, input.manifest_digest, lock)
                .is_ok()
        {
            return Ok(lock.clone());
        }

        let access = if input.mode == ResolutionMode::Offline {
            Access::LocalOnly
        } else {
            Access::Online
        };
        let mut state = State::new(input, access);
        state.seed_roots()?;
        self.solve(&mut state)?;
        state.finish()
    }
}

#[derive(Clone)]
struct PendingNode {
    package: LockedPackage,
    manifest_path: String,
    manifest: Option<Manifest>,
    resolved: bool,
}

/// 工作区依赖及其声明位置；相对 path 必须锚定这里，而不是使用它的成员。 /
/// A workspace dependency and its declaration site; relative paths are anchored here, not at the member using it.
#[derive(Clone)]
struct WorkspaceDependency {
    root_manifest: String,
    spec: DependencySpec,
}

#[derive(Clone)]
struct State<'a> {
    input: ResolutionInput<'a>,
    access: Access,
    nodes: BTreeMap<String, PendingNode>,
    source_nodes: BTreeMap<String, String>,
    workspace_members: BTreeSet<String>,
    workspace_dependencies: BTreeMap<String, WorkspaceDependency>,
    /// Registry 统一域已经观察到的全部约束。 / All constraints observed for each unified registry domain.
    registry_constraints: BTreeMap<(String, String), Vec<RegistryConstraint>>,
}

#[derive(Clone)]
struct RegistryConstraint {
    requirement: VersionReq,
    conflict: Conflict,
}

impl<'a> State<'a> {
    fn new(input: ResolutionInput<'a>, access: Access) -> Self {
        let mut workspace_members = BTreeSet::new();
        let mut workspace_dependencies = BTreeMap::new();
        for (path, manifest) in input.manifests {
            if let Some(workspace) = &manifest.workspace {
                let base = manifest_dir(path);
                for member in &workspace.members {
                    if let Some(value) = join_manifest(&base, member) {
                        workspace_members.insert(value);
                    }
                }
                for (alias, spec) in &workspace.dependencies {
                    workspace_dependencies.insert(
                        alias.clone(),
                        WorkspaceDependency {
                            root_manifest: normalize_manifest_key(path)
                                .unwrap_or_else(|| path.clone()),
                            spec: spec.clone(),
                        },
                    );
                }
            }
        }
        Self {
            input,
            access,
            nodes: BTreeMap::new(),
            source_nodes: BTreeMap::new(),
            workspace_members,
            workspace_dependencies,
            registry_constraints: BTreeMap::new(),
        }
    }

    fn seed_roots(&mut self) -> Result<(), ResolveError> {
        for (path, manifest) in self.input.manifests {
            let Some(package) = &manifest.package else {
                continue;
            };
            let normalized = normalize_manifest_key(path).ok_or_else(|| {
                ResolveError::InvalidInput(format!("manifest path `{path}` escapes the workspace"))
            })?;
            let member = self.workspace_members.contains(&normalized);
            let locator = manifest_dir(&normalized);
            let source = if member {
                LockedSource::Workspace {
                    member: locator.clone().into(),
                    mutable: true,
                }
            } else {
                LockedSource::Path {
                    path: locator.clone().into(),
                    mutable: true,
                }
            };
            let source_key = source_key(&source, &package.name);
            let id = package_id(&package.name, &package.version, &source_key);
            if self.source_nodes.insert(source_key, id.clone()).is_some() {
                return Err(ResolveError::InvalidInput(format!(
                    "duplicate package source `{locator}`"
                )));
            }
            self.nodes.insert(
                id.clone(),
                PendingNode {
                    package: LockedPackage {
                        id,
                        name: package.name.clone(),
                        version: package.version.clone(),
                        source,
                        manifest_digest: manifest_hash(manifest)?,
                        dependencies: BTreeMap::new(),
                    },
                    manifest_path: normalized,
                    manifest: Some(manifest.clone()),
                    resolved: false,
                },
            );
        }
        Ok(())
    }

    fn finish(self) -> Result<Lockfile, ResolveError> {
        let lock = Lockfile {
            lock_version: squish_project::LOCK_VERSION,
            resolver_version: RESOLVER_VERSION.into(),
            manifest_digest: self.input.manifest_digest.into(),
            packages: self.nodes.into_values().map(|node| node.package).collect(),
        };
        lock.validate()
            .map_err(|error| ResolveError::InvalidInput(error.to_string()))?;
        Ok(lock)
    }
}

impl<R: RegistryPort, G: GitPort, F: FilesystemPort> Resolver<R, G, F> {
    /// 对完整未决图执行确定性深度优先搜索。 / Searches the complete pending graph deterministically.
    fn solve(&self, state: &mut State<'_>) -> Result<(), ResolveError> {
        let Some(id) = state
            .nodes
            .iter()
            .find_map(|(id, node)| (!node.resolved).then(|| id.clone()))
        else {
            return Ok(());
        };
        let node = state.nodes.get(&id).expect("selected node exists").clone();
        state
            .nodes
            .get_mut(&id)
            .expect("selected node exists")
            .resolved = true;
        let dependencies: Vec<_> = node
            .manifest
            .expect("only unresolved nodes require a manifest")
            .dependencies
            .into_iter()
            .collect();
        self.solve_dependencies(
            state,
            &id,
            &node.manifest_path,
            &node.package.name,
            &dependencies,
            0,
        )
    }

    ///  continuation 留在候选循环内部，使后续兄弟冲突能回溯较早选择。 /
    /// Keeps the continuation inside candidate loops so later sibling conflicts can backtrack earlier choices.
    fn solve_dependencies(
        &self,
        state: &mut State<'_>,
        id: &str,
        from_manifest: &str,
        depender: &str,
        dependencies: &[(String, DependencySpec)],
        index: usize,
    ) -> Result<(), ResolveError> {
        let Some((alias, raw_spec)) = dependencies.get(index) else {
            return self.solve(state);
        };
        let spec = inherited_spec(
            raw_spec,
            alias,
            &state.workspace_dependencies,
            from_manifest,
        )?;
        if is_optional(&spec) {
            return self.solve_dependencies(
                state,
                id,
                from_manifest,
                depender,
                dependencies,
                index + 1,
            );
        }
        let detail = detail(&spec);
        let package_name = detail.package.clone().unwrap_or_else(|| alias.clone());
        // 显式 path 与 workspace 成员最终都是相对清单的本地包；只保留一个入图路径。
        // Explicit paths and workspace members are both manifest-relative local packages;
        // keep one graph-insertion path for both.
        let local_path = if let Some(path) = detail.path.clone() {
            Some(path)
        } else if detail.workspace {
            let member = workspace_package_path(state, &package_name).ok_or_else(|| {
                unsat(
                    depender,
                    alias,
                    &package_name,
                    "workspace member with matching package name",
                )
            })?;
            let path = relative_dependency_path(from_manifest, &member).ok_or_else(|| {
                ResolveError::InvalidInput(format!(
                    "cannot express workspace member `{member}` from `{from_manifest}`"
                ))
            })?;
            Some(path)
        } else {
            None
        };
        if let Some(path) = local_path {
            let target =
                self.resolve_local(state, from_manifest, depender, alias, &package_name, &path)?;
            return self.continue_after_edge(
                state,
                id,
                alias,
                target,
                from_manifest,
                depender,
                dependencies,
                index,
            );
        }
        if let Some(repository) = &detail.git {
            let target = self.resolve_git(
                state,
                id,
                alias,
                depender,
                &package_name,
                repository,
                &detail.git_reference,
            )?;
            return self.continue_after_edge(
                state,
                id,
                alias,
                target,
                from_manifest,
                depender,
                dependencies,
                index,
            );
        }
        let requirement = detail.version.clone().unwrap_or(VersionReq::STAR);
        let registry = detail.registry.as_deref().unwrap_or(DEFAULT_REGISTRY);
        self.resolve_registry_then(
            state,
            id,
            from_manifest,
            depender,
            alias,
            &package_name,
            registry,
            requirement,
            dependencies,
            index,
        )
    }

    // 参数保持显式以避免把短生命周期 continuation 偷藏进可变状态。 /
    // Parameters stay explicit to avoid hiding a short-lived continuation in mutable state.
    #[allow(clippy::too_many_arguments)]
    fn continue_after_edge(
        &self,
        state: &mut State<'_>,
        id: &str,
        alias: &str,
        target: String,
        from_manifest: &str,
        depender: &str,
        dependencies: &[(String, DependencySpec)],
        index: usize,
    ) -> Result<(), ResolveError> {
        state
            .nodes
            .get_mut(id)
            .expect("depender exists")
            .package
            .dependencies
            .insert(alias.into(), target);
        self.solve_dependencies(state, id, from_manifest, depender, dependencies, index + 1)
    }

    fn resolve_local(
        &self,
        state: &mut State<'_>,
        from_manifest: &str,
        depender: &str,
        alias: &str,
        package_name: &str,
        path: &Path,
    ) -> Result<String, ResolveError> {
        let wanted = join_manifest(&manifest_dir(from_manifest), path).ok_or_else(|| {
            ResolveError::InvalidInput(format!(
                "path dependency `{}` escapes the workspace",
                path.display()
            ))
        })?;
        let (manifest_path, manifest) = if let Some(manifest) = state.input.manifests.get(&wanted) {
            (wanted, manifest.clone())
        } else {
            let loaded = self
                .filesystem
                .load(LocalRequest {
                    from_manifest,
                    path,
                })
                .map_err(ResolveError::Unavailable)?;
            (
                normalize_manifest_key(&loaded.manifest_path).ok_or_else(|| {
                    ResolveError::InvalidInput(format!(
                        "filesystem returned escaping manifest `{}`",
                        loaded.manifest_path
                    ))
                })?,
                loaded.manifest,
            )
        };
        let package = manifest.package.as_ref().ok_or_else(|| {
            unsat(
                depender,
                alias,
                package_name,
                "local dependency must contain a package",
            )
        })?;
        if package.name != package_name {
            return Err(unsat(
                depender,
                alias,
                package_name,
                &format!("local package is named `{}`", package.name),
            ));
        }
        let source = if state.workspace_members.contains(&manifest_path) {
            LockedSource::Workspace {
                member: manifest_dir(&manifest_path).into(),
                mutable: true,
            }
        } else {
            LockedSource::Path {
                path: manifest_dir(&manifest_path).into(),
                mutable: true,
            }
        };
        self.install_candidate(state, manifest_path, manifest, source)
    }

    // Git 选择同时需要当前边与来源身份；显式参数让锁复用条件可审计。 /
    // Git selection needs both current-edge and source identity; explicit parameters keep lock reuse auditable.
    #[allow(clippy::too_many_arguments)]
    fn resolve_git(
        &self,
        state: &mut State<'_>,
        depender_id: &str,
        alias: &str,
        depender: &str,
        package_name: &str,
        repository: &str,
        reference: &GitReference,
    ) -> Result<String, ResolveError> {
        if let Some(locked) = prior_edge(state, depender_id, alias).cloned()
            && locked.name == package_name
            && matches!(&locked.source, LockedSource::Git { repository: actual, revision, .. }
                if actual == repository && reference.rev.as_ref().is_none_or(|wanted| wanted == revision))
        {
            Self::import_prior_closure(state, &locked.id)?;
            return Ok(locked.id);
        }
        let candidate = self
            .git
            .resolve(repository, reference, state.access)
            .map_err(ResolveError::Unavailable)?;
        validate_git_candidate(&candidate)?;
        let package = candidate.manifest.package.as_ref().ok_or_else(|| {
            unsat(
                depender,
                alias,
                package_name,
                "Git manifest must contain a package",
            )
        })?;
        if package.name != package_name {
            return Err(unsat(
                depender,
                alias,
                package_name,
                &format!("Git package is named `{}`", package.name),
            ));
        }
        let source = LockedSource::Git {
            repository: repository.into(),
            revision: candidate.revision.clone(),
            checksum: candidate.checksum,
        };
        self.install_candidate(
            state,
            format!("git:{repository}#{}", candidate.revision),
            candidate.manifest,
            source,
        )
    }

    /// 复制既有精确 Git 子图，避免不相关 manifest 变化刷新可移动选择器。 /
    /// Copies an existing exact Git subgraph so unrelated manifest changes do not refresh a moving selector.
    fn import_prior_closure(state: &mut State<'_>, id: &str) -> Result<(), ResolveError> {
        if state.nodes.contains_key(id) {
            return Ok(());
        }
        let package = state
            .input
            .prior_lock
            .and_then(|lock| lock.packages.iter().find(|package| package.id == id))
            .cloned()
            .ok_or_else(|| {
                ResolveError::LockOutdated(format!("prior dependency `{id}` is absent"))
            })?;
        let dependencies: Vec<_> = package.dependencies.values().cloned().collect();
        let key = source_key(&package.source, &package.name);
        if let Some(existing) = state.source_nodes.get(&key)
            && existing != id
        {
            return Err(unsat(
                &package.name,
                "prior-lock",
                &package.name,
                "one canonical source identity",
            ));
        }
        state.source_nodes.insert(key, id.into());
        state.nodes.insert(
            id.into(),
            PendingNode {
                package,
                manifest_path: format!("locked:{id}"),
                manifest: None,
                resolved: true,
            },
        );
        for dependency in dependencies {
            Self::import_prior_closure(state, &dependency)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_registry_then(
        &self,
        state: &mut State<'_>,
        id: &str,
        from_manifest: &str,
        depender: &str,
        alias: &str,
        package_name: &str,
        registry: &str,
        requirement: VersionReq,
        dependencies: &[(String, DependencySpec)],
        index: usize,
    ) -> Result<(), ResolveError> {
        let domain = (registry.to_string(), package_name.to_string());
        let fact = RegistryConstraint {
            requirement: requirement.clone(),
            conflict: Conflict {
                depender: depender.into(),
                alias: alias.into(),
                package: package_name.into(),
                requirement: requirement.to_string(),
            },
        };
        state
            .registry_constraints
            .entry(domain.clone())
            .or_default()
            .push(fact);

        let key = registry_source_key(registry, package_name);
        if let Some(selected_id) = state.source_nodes.get(&key).cloned() {
            let selected = state
                .nodes
                .get(&selected_id)
                .expect("selected registry node exists");
            if !requirement.matches(&selected.package.version) {
                return Err(registry_unsat(state, &domain));
            }
            return self.continue_after_edge(
                state,
                id,
                alias,
                selected_id,
                from_manifest,
                depender,
                dependencies,
                index,
            );
        }

        let mut candidates = self
            .registry
            .candidates(registry, package_name, state.access)
            .map_err(ResolveError::Unavailable)?;
        let constraints = state
            .registry_constraints
            .get(&domain)
            .expect("constraint inserted");
        candidates.retain(|candidate| {
            constraints
                .iter()
                .all(|constraint| constraint.requirement.matches(&candidate.version))
        });
        candidates.sort_by(|a, b| {
            b.version
                .cmp(&a.version)
                .then_with(|| a.checksum.cmp(&b.checksum))
        });
        if let Some(lock) = state.input.prior_lock {
            candidates.sort_by_key(|candidate| {
                !lock.packages.iter().any(|package| {
                    package.name == package_name
                        && package.version == candidate.version
                        && matches!(&package.source, LockedSource::Registry { registry: actual, checksum }
                            if actual == registry && checksum == &candidate.checksum)
                })
            });
        }
        if candidates.is_empty() {
            return Err(registry_unsat(state, &domain));
        }

        let checkpoint = state.clone();
        let mut conflicts = Vec::new();
        for candidate in candidates {
            *state = checkpoint.clone();
            validate_registry_candidate(package_name, &candidate)?;
            let source = LockedSource::Registry {
                registry: registry.into(),
                checksum: candidate.checksum,
            };
            let target = self.install_candidate(
                state,
                format!("registry:{registry}/{package_name}/{}", candidate.version),
                candidate.manifest,
                source,
            )?;
            match self.continue_after_edge(
                state,
                id,
                alias,
                target,
                from_manifest,
                depender,
                dependencies,
                index,
            ) {
                Ok(()) => return Ok(()),
                Err(ResolveError::Unsatisfiable { conflicts: nested }) => conflicts.extend(nested),
                Err(error) => return Err(error),
            }
        }
        *state = checkpoint;
        conflicts.extend(registry_conflicts(state, &domain));
        conflicts.sort_by(conflict_order);
        conflicts.dedup();
        Err(ResolveError::Unsatisfiable { conflicts })
    }

    fn install_candidate(
        &self,
        state: &mut State<'_>,
        manifest_path: String,
        manifest: Manifest,
        source: LockedSource,
    ) -> Result<String, ResolveError> {
        manifest
            .validate()
            .map_err(|error| ResolveError::InvalidInput(error.to_string()))?;
        let package = manifest
            .package
            .as_ref()
            .ok_or_else(|| ResolveError::InvalidInput("dependency manifest is virtual".into()))?;
        let key = source_key(&source, &package.name);
        if let Some(id) = state.source_nodes.get(&key) {
            return Ok(id.clone());
        }
        let id = package_id(&package.name, &package.version, &key);
        state.source_nodes.insert(key, id.clone());
        state.nodes.insert(
            id.clone(),
            PendingNode {
                package: LockedPackage {
                    id: id.clone(),
                    name: package.name.clone(),
                    version: package.version.clone(),
                    source,
                    manifest_digest: manifest_hash(&manifest)?,
                    dependencies: BTreeMap::new(),
                },
                manifest_path,
                manifest: Some(manifest),
                resolved: false,
            },
        );
        Ok(id)
    }

    fn validate_locked(
        &self,
        manifests: &BTreeMap<String, Manifest>,
        digest: &str,
        lock: &Lockfile,
    ) -> Result<(), ResolveError> {
        lock.validate()
            .map_err(|e| ResolveError::LockOutdated(e.to_string()))?;
        if lock.resolver_version != RESOLVER_VERSION {
            return Err(ResolveError::LockOutdated(format!(
                "resolver is `{}`, expected `{RESOLVER_VERSION}`",
                lock.resolver_version
            )));
        }
        if lock.manifest_digest != digest {
            return Err(ResolveError::LockOutdated("manifest digest changed".into()));
        }
        let workspace_members = workspace_member_paths(manifests);
        let workspace_dependencies = workspace_dependencies(manifests);
        for (path, manifest) in manifests {
            let Some(package) = &manifest.package else {
                continue;
            };
            let normalized = normalize_manifest_key(path).ok_or_else(|| {
                ResolveError::LockOutdated(format!("manifest `{path}` has no canonical locator"))
            })?;
            let locator = PathBuf::from(manifest_dir(&normalized));
            let is_member = workspace_members.contains(&normalized);
            let locked = lock
                .packages
                .iter()
                .find(|candidate| {
                    candidate.name == package.name
                        && candidate.version == package.version
                        && match &candidate.source {
                            LockedSource::Workspace { member, .. } => {
                                is_member && member == &locator
                            }
                            LockedSource::Path { path, .. } => !is_member && path == &locator,
                            _ => false,
                        }
                })
                .ok_or_else(|| {
                    ResolveError::LockOutdated(format!(
                        "package `{}` at `{}` is absent",
                        package.name,
                        locator.display()
                    ))
                })?;
            if locked.manifest_digest != manifest_hash(manifest)? {
                return Err(ResolveError::LockOutdated(format!(
                    "manifest `{path}` changed"
                )));
            }
            for (alias, raw) in &manifest.dependencies {
                let spec = inherited_spec(raw, alias, &workspace_dependencies, &normalized)?;
                if is_optional(&spec) {
                    continue;
                }
                let target_id = locked.dependencies.get(alias).ok_or_else(|| {
                    ResolveError::LockOutdated(format!(
                        "dependency `{}`.`{alias}` is absent",
                        package.name
                    ))
                })?;
                let target = lock
                    .packages
                    .iter()
                    .find(|p| &p.id == target_id)
                    .ok_or_else(|| {
                        ResolveError::LockOutdated(format!(
                            "dependency edge `{target_id}` is dangling"
                        ))
                    })?;
                validate_locked_spec(
                    &package.name,
                    alias,
                    &spec,
                    target,
                    &normalized,
                    &workspace_members,
                )?;
            }
        }
        Ok(())
    }

    fn validate_frozen_content(&self, lock: &Lockfile) -> Result<(), ResolveError> {
        for package in &lock.packages {
            let present = match &package.source {
                LockedSource::Registry { registry, checksum } => {
                    self.registry
                        .contains(registry, &package.name, &package.version, checksum)
                }
                LockedSource::Git {
                    repository,
                    revision,
                    checksum,
                } => self.git.contains(repository, revision, checksum),
                LockedSource::Path { .. } | LockedSource::Workspace { .. } => true,
            };
            if !present {
                return Err(ResolveError::Unavailable(SourceUnavailable {
                    identity: package.id.clone(),
                    detail: "required frozen content is not available locally".into(),
                }));
            }
        }
        Ok(())
    }
}

fn detail(spec: &DependencySpec) -> DependencyDetail {
    match spec {
        DependencySpec::Version(version) => DependencyDetail {
            version: Some(version.clone()),
            ..DependencyDetail::default()
        },
        DependencySpec::Detail(detail) => (**detail).clone(),
    }
}

fn inherited_spec(
    spec: &DependencySpec,
    alias: &str,
    workspace: &BTreeMap<String, WorkspaceDependency>,
    from_manifest: &str,
) -> Result<DependencySpec, ResolveError> {
    let DependencySpec::Detail(member) = spec else {
        return Ok(spec.clone());
    };
    if !member.workspace {
        return Ok(spec.clone());
    }
    let inherited = workspace.get(alias).ok_or_else(|| {
        ResolveError::InvalidInput(format!(
            "workspace dependency `{alias}` has no workspace declaration"
        ))
    })?;
    let mut merged = detail(&inherited.spec);
    merged.workspace = false;
    merged.optional |= member.optional;
    merged.features.extend(member.features.iter().cloned());
    merged.default_features &= member.default_features;
    if let Some(path) = &merged.path {
        let target =
            join_manifest(&manifest_dir(&inherited.root_manifest), path).ok_or_else(|| {
                ResolveError::InvalidInput(format!(
                    "workspace dependency `{alias}` path `{}` escapes the workspace",
                    path.display()
                ))
            })?;
        merged.path = Some(
            relative_dependency_path(from_manifest, &target).ok_or_else(|| {
                ResolveError::InvalidInput(format!(
                    "cannot rebase workspace dependency `{alias}` from `{from_manifest}`"
                ))
            })?,
        );
    }
    Ok(DependencySpec::Detail(Box::new(merged)))
}

fn is_optional(spec: &DependencySpec) -> bool {
    matches!(spec, DependencySpec::Detail(detail) if detail.optional)
}

fn workspace_dependencies(
    manifests: &BTreeMap<String, Manifest>,
) -> BTreeMap<String, WorkspaceDependency> {
    let mut result = BTreeMap::new();
    for (path, manifest) in manifests {
        if let Some(workspace) = &manifest.workspace {
            for (alias, spec) in &workspace.dependencies {
                result.insert(
                    alias.clone(),
                    WorkspaceDependency {
                        root_manifest: normalize_manifest_key(path).unwrap_or_else(|| path.clone()),
                        spec: spec.clone(),
                    },
                );
            }
        }
    }
    result
}

fn workspace_member_paths(manifests: &BTreeMap<String, Manifest>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for (path, manifest) in manifests {
        if let Some(workspace) = &manifest.workspace {
            let base = manifest_dir(path);
            for member in &workspace.members {
                if let Some(path) = join_manifest(&base, member) {
                    result.insert(path);
                }
            }
        }
    }
    result
}

fn workspace_package_path(state: &State<'_>, name: &str) -> Option<String> {
    state.input.manifests.iter().find_map(|(path, manifest)| {
        (manifest
            .package
            .as_ref()
            .is_some_and(|package| package.name == name)
            && state
                .workspace_members
                .contains(&normalize_manifest_key(path)?))
        .then(|| normalize_manifest_key(path))
        .flatten()
    })
}

fn validate_locked_spec(
    depender: &str,
    alias: &str,
    spec: &DependencySpec,
    package: &LockedPackage,
    from_manifest: &str,
    workspace_members: &BTreeSet<String>,
) -> Result<(), ResolveError> {
    let detail = detail(spec);
    let expected = detail.package.as_deref().unwrap_or(alias);
    if package.name != expected {
        return Err(ResolveError::LockOutdated(format!(
            "`{depender}` dependency `{alias}` resolves to `{}`",
            package.name
        )));
    }
    if let Some(requirement) = detail.version.as_ref().or(match spec {
        DependencySpec::Version(v) => Some(v),
        _ => None,
    }) && !requirement.matches(&package.version)
    {
        return Err(ResolveError::LockOutdated(format!(
            "`{expected}` {} does not match `{requirement}`",
            package.version
        )));
    }
    let source_ok = if let Some(repository) = detail.git.as_ref() {
        matches!(&package.source, LockedSource::Git { repository: actual, .. } if actual == repository)
    } else if let Some(path) = &detail.path {
        let expected_manifest = join_manifest(&manifest_dir(from_manifest), path);
        let expected = expected_manifest
            .as_deref()
            .map(manifest_dir)
            .map(PathBuf::from);
        match (&package.source, expected.as_ref()) {
            (LockedSource::Path { path, .. }, Some(expected)) => {
                path == expected
                    && expected_manifest
                        .as_ref()
                        .is_none_or(|manifest| !workspace_members.contains(manifest))
            }
            (LockedSource::Workspace { member, .. }, Some(expected)) => {
                member == expected
                    && expected_manifest
                        .as_ref()
                        .is_some_and(|manifest| workspace_members.contains(manifest))
            }
            _ => false,
        }
    } else if detail.workspace {
        matches!(package.source, LockedSource::Workspace { .. })
    } else {
        let registry = detail.registry.as_deref().unwrap_or(DEFAULT_REGISTRY);
        matches!(&package.source, LockedSource::Registry { registry: actual, .. } if actual == registry)
    };
    if !source_ok {
        return Err(ResolveError::LockOutdated(format!(
            "`{depender}` dependency `{alias}` changed source"
        )));
    }
    Ok(())
}

fn registry_source_key(registry: &str, package: &str) -> String {
    format!("registry:{registry}/{package}")
}

fn registry_conflicts(state: &State<'_>, domain: &(String, String)) -> Vec<Conflict> {
    state
        .registry_constraints
        .get(domain)
        .into_iter()
        .flatten()
        .map(|constraint| constraint.conflict.clone())
        .collect()
}

fn registry_unsat(state: &State<'_>, domain: &(String, String)) -> ResolveError {
    let mut conflicts = registry_conflicts(state, domain);
    conflicts.sort_by(conflict_order);
    conflicts.dedup();
    ResolveError::Unsatisfiable { conflicts }
}

fn conflict_order(a: &Conflict, b: &Conflict) -> std::cmp::Ordering {
    (&a.depender, &a.alias, &a.package, &a.requirement).cmp(&(
        &b.depender,
        &b.alias,
        &b.package,
        &b.requirement,
    ))
}

fn validate_registry_candidate(
    package_name: &str,
    candidate: &RegistryCandidate,
) -> Result<(), ResolveError> {
    if candidate
        .manifest
        .package
        .as_ref()
        .map(|package| (&package.name, &package.version))
        != Some((&package_name.to_string(), &candidate.version))
    {
        return Err(ResolveError::InvalidInput(format!(
            "registry metadata for `{package_name} {}` disagrees with its manifest",
            candidate.version
        )));
    }
    validate_digest(&candidate.checksum, "registry checksum")
}

fn prior_edge<'a>(
    state: &'a State<'_>,
    depender_id: &str,
    alias: &str,
) -> Option<&'a LockedPackage> {
    let lock = state.input.prior_lock?;
    let current = state.nodes.get(depender_id)?;
    let locator = source_key(&current.package.source, &current.package.name);
    let prior_depender = lock
        .packages
        .iter()
        .find(|package| source_key(&package.source, &package.name) == locator)?;
    let target = prior_depender.dependencies.get(alias)?;
    lock.packages.iter().find(|package| &package.id == target)
}

fn validate_git_candidate(candidate: &GitCandidate) -> Result<(), ResolveError> {
    if !matches!(candidate.revision.len(), 40 | 64)
        || !candidate.revision.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ResolveError::InvalidInput(
            "Git port returned a non-immutable revision".into(),
        ));
    }
    validate_digest(&candidate.checksum, "Git checksum")
}

fn validate_digest(value: &str, label: &str) -> Result<(), ResolveError> {
    let valid = value.split_once(':').is_some_and(|(algorithm, hex)| {
        !algorithm.is_empty() && hex.len() >= 32 && hex.bytes().all(|b| b.is_ascii_hexdigit())
    });
    if valid {
        Ok(())
    } else {
        Err(ResolveError::InvalidInput(format!(
            "{label} must be `algorithm:hex` with at least 128 bits"
        )))
    }
}

fn unsat(depender: &str, alias: &str, package: &str, requirement: &str) -> ResolveError {
    ResolveError::Unsatisfiable {
        conflicts: vec![Conflict {
            depender: depender.into(),
            alias: alias.into(),
            package: package.into(),
            requirement: requirement.into(),
        }],
    }
}

fn manifest_hash(manifest: &Manifest) -> Result<String, ResolveError> {
    let normalized = manifest
        .to_toml()
        .map_err(|e| ResolveError::InvalidInput(e.to_string()))?;
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(normalized.as_bytes())
    ))
}

fn package_id(name: &str, version: &Version, source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    format!("{name}@{version}#sha256:{digest:x}")
}

fn source_key(source: &LockedSource, name: &str) -> String {
    match source {
        LockedSource::Registry { registry, .. } => registry_source_key(registry, name),
        LockedSource::Git {
            repository,
            revision,
            checksum,
        } => format!("git:{repository}#{revision}/{checksum}"),
        LockedSource::Path { path, .. } => format!("path:{}", slash(path)),
        LockedSource::Workspace { member, .. } => format!("workspace:{}", slash(member)),
    }
}

fn manifest_dir(path: &str) -> String {
    let path = Path::new(path);
    let dir = if path.file_name().and_then(|v| v.to_str()) == Some(MANIFEST_FILE) {
        path.parent().unwrap_or(Path::new(""))
    } else {
        path
    };
    let value = slash(dir);
    if value.is_empty() { ".".into() } else { value }
}

fn join_manifest(base: &str, dependency: &Path) -> Option<String> {
    let mut path = PathBuf::from(base);
    path.push(dependency);
    path.push(MANIFEST_FILE);
    normalize_path(&path).map(|p| slash(&p))
}

fn normalize_manifest_key(path: &str) -> Option<String> {
    let mut value = normalize_path(Path::new(path))?;
    if value.file_name().and_then(|v| v.to_str()) != Some(MANIFEST_FILE) {
        value.push(MANIFEST_FILE);
    }
    Some(slash(&value))
}

fn normalize_path(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => result.push(value),
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                _ => result.push(".."),
            },
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    Some(result)
}

fn relative_dependency_path(from_manifest: &str, target_manifest: &str) -> Option<PathBuf> {
    let from: Vec<_> = Path::new(&manifest_dir(from_manifest))
        .components()
        .filter_map(|c| match c {
            Component::Normal(v) => Some(v.to_owned()),
            _ => None,
        })
        .collect();
    let target_dir = manifest_dir(target_manifest);
    let to: Vec<_> = Path::new(&target_dir)
        .components()
        .filter_map(|c| match c {
            Component::Normal(v) => Some(v.to_owned()),
            _ => None,
        })
        .collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for item in &to[common..] {
        result.push(item);
    }
    (!result.as_os_str().is_empty())
        .then_some(result)
        .or_else(|| Some(PathBuf::from(".")))
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use squish_project::{MANIFEST_VERSION, Package, Workspace};

    use super::*;

    #[derive(Default)]
    struct Registry {
        packages: BTreeMap<String, Vec<RegistryCandidate>>,
        calls: RefCell<Vec<(String, Access)>>,
        local: bool,
        failure: Option<SourceUnavailable>,
    }

    impl RegistryPort for Registry {
        fn candidates(
            &self,
            registry: &str,
            package: &str,
            access: Access,
        ) -> Result<Vec<RegistryCandidate>, SourceUnavailable> {
            self.calls
                .borrow_mut()
                .push((format!("{registry}/{package}"), access));
            if let Some(error) = &self.failure {
                return Err(error.clone());
            }
            Ok(self.packages.get(package).cloned().unwrap_or_default())
        }

        fn contains(&self, _: &str, _: &str, _: &Version, _: &str) -> bool {
            self.local
        }
    }

    #[derive(Default)]
    struct Git {
        candidate: Option<GitCandidate>,
        calls: RefCell<Vec<Access>>,
        local: bool,
    }

    impl GitPort for Git {
        fn resolve(
            &self,
            repository: &str,
            _: &GitReference,
            access: Access,
        ) -> Result<GitCandidate, SourceUnavailable> {
            self.calls.borrow_mut().push(access);
            self.candidate.clone().ok_or_else(|| SourceUnavailable {
                identity: repository.into(),
                detail: "not cached".into(),
            })
        }

        fn contains(&self, _: &str, _: &str, _: &str) -> bool {
            self.local
        }
    }

    fn manifest(
        name: &str,
        version: &str,
        dependencies: BTreeMap<String, DependencySpec>,
    ) -> Manifest {
        Manifest {
            manifest_version: MANIFEST_VERSION,
            workspace: None,
            package: Some(Package {
                name: name.into(),
                version: version.parse().unwrap(),
                dialect: "xmlsquish/1".into(),
                source_root: "src".into(),
            }),
            targets: BTreeMap::new(),
            dependencies,
            exports: BTreeMap::new(),
            profiles: BTreeMap::new(),
        }
    }

    fn registry_candidate(
        name: &str,
        version: &str,
        dependencies: BTreeMap<String, DependencySpec>,
    ) -> RegistryCandidate {
        RegistryCandidate {
            version: version.parse().unwrap(),
            checksum: format!(
                "sha256:{}",
                if version.starts_with('2') {
                    "22".repeat(32)
                } else {
                    "11".repeat(32)
                }
            ),
            manifest: manifest(name, version, dependencies),
        }
    }

    fn root_with_dependency(alias: &str, dependency: DependencySpec) -> BTreeMap<String, Manifest> {
        BTreeMap::from([(
            MANIFEST_FILE.into(),
            manifest("app", "1.0.0", BTreeMap::from([(alias.into(), dependency)])),
        )])
    }

    fn resolve<R: RegistryPort, G: GitPort, F: FilesystemPort>(
        resolver: &Resolver<R, G, F>,
        manifests: &BTreeMap<String, Manifest>,
        prior: Option<&Lockfile>,
        mode: ResolutionMode,
    ) -> Result<Lockfile, ResolveError> {
        resolver.resolve(ResolutionInput {
            manifests,
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            prior_lock: prior,
            mode,
        })
    }

    #[test]
    fn selects_highest_version_and_is_independent_of_adapter_order() {
        let root = manifest(
            "app",
            "1.0.0",
            BTreeMap::from([("dep".into(), DependencySpec::Version("^1".parse().unwrap()))]),
        );
        let mut manifests = BTreeMap::new();
        manifests.insert(MANIFEST_FILE.into(), root);
        let registry = Registry {
            packages: BTreeMap::from([(
                "dep".into(),
                vec![
                    registry_candidate("dep", "1.2.0", BTreeMap::new()),
                    registry_candidate("dep", "1.9.0", BTreeMap::new()),
                ],
            )]),
            ..Registry::default()
        };
        let lock = resolve(
            &Resolver::new(registry, Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        assert!(
            lock.packages
                .iter()
                .any(|package| package.name == "dep" && package.version == Version::new(1, 9, 0))
        );
        assert_eq!(lock.packages, {
            let mut packages = lock.packages.clone();
            packages.sort_by(|a, b| a.id.cmp(&b.id));
            packages
        });
    }

    #[test]
    fn backtracks_when_newest_candidate_has_an_unsatisfiable_transitive_dependency() {
        let manifests = root_with_dependency("dep", DependencySpec::Version("*".parse().unwrap()));
        let broken = BTreeMap::from([(
            "missing".into(),
            DependencySpec::Version("^9".parse().unwrap()),
        )]);
        let registry = Registry {
            packages: BTreeMap::from([(
                "dep".into(),
                vec![
                    registry_candidate("dep", "2.0.0", broken),
                    registry_candidate("dep", "1.0.0", BTreeMap::new()),
                ],
            )]),
            ..Registry::default()
        };
        let lock = resolve(
            &Resolver::new(registry, Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        assert!(
            lock.packages
                .iter()
                .any(|package| package.name == "dep" && package.version == Version::new(1, 0, 0))
        );
        assert!(
            !lock
                .packages
                .iter()
                .any(|package| package.version == Version::new(2, 0, 0))
        );
    }

    #[test]
    fn unifies_registry_domain_and_backtracks_across_sibling_packages() {
        let shared_v1 = BTreeMap::from([(
            "shared".into(),
            DependencySpec::Version("^1".parse().unwrap()),
        )]);
        let shared_v2 = BTreeMap::from([(
            "shared".into(),
            DependencySpec::Version("^2".parse().unwrap()),
        )]);
        let manifests = BTreeMap::from([(
            MANIFEST_FILE.into(),
            manifest(
                "app",
                "1.0.0",
                BTreeMap::from([
                    ("a".into(), DependencySpec::Version("*".parse().unwrap())),
                    ("b".into(), DependencySpec::Version("*".parse().unwrap())),
                ]),
            ),
        )]);
        let registry = Registry {
            packages: BTreeMap::from([
                (
                    "a".into(),
                    vec![
                        registry_candidate("a", "2.0.0", shared_v2),
                        registry_candidate("a", "1.0.0", shared_v1.clone()),
                    ],
                ),
                (
                    "b".into(),
                    vec![registry_candidate("b", "1.0.0", shared_v1)],
                ),
                (
                    "shared".into(),
                    vec![
                        registry_candidate("shared", "2.0.0", BTreeMap::new()),
                        registry_candidate("shared", "1.0.0", BTreeMap::new()),
                    ],
                ),
            ]),
            ..Registry::default()
        };
        let lock = resolve(
            &Resolver::new(registry, Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        assert!(
            lock.packages
                .iter()
                .any(|package| package.name == "a" && package.version == Version::new(1, 0, 0))
        );
        assert_eq!(
            lock.packages
                .iter()
                .filter(|package| package.name == "shared")
                .count(),
            1
        );
    }

    #[test]
    fn distinguishes_unsatisfiable_metadata_from_unavailable_source() {
        let manifests = root_with_dependency("dep", DependencySpec::Version("^3".parse().unwrap()));
        let unsat = resolve(
            &Resolver::new(Registry::default(), Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap_err();
        assert!(matches!(unsat, ResolveError::Unsatisfiable { .. }));

        let registry = Registry {
            failure: Some(SourceUnavailable {
                identity: "index".into(),
                detail: "offline cache miss".into(),
            }),
            ..Registry::default()
        };
        let unavailable = resolve(
            &Resolver::new(registry, Git::default()),
            &manifests,
            None,
            ResolutionMode::Offline,
        )
        .unwrap_err();
        assert!(matches!(unavailable, ResolveError::Unavailable(_)));
    }

    #[test]
    fn path_to_member_uses_workspace_identity_and_inherited_dependencies() {
        let shared = DependencySpec::Version("^1".parse().unwrap());
        let mut root = manifest(
            "app",
            "1.0.0",
            BTreeMap::from([("member".into(), DependencySpec::path("member"))]),
        );
        root.workspace = Some(Workspace {
            members: vec!["member".into()],
            exclude: vec![],
            target_dir: "target".into(),
            dependencies: BTreeMap::from([("shared".into(), shared)]),
        });
        let member = manifest(
            "member",
            "1.0.0",
            BTreeMap::from([("shared".into(), DependencySpec::workspace())]),
        );
        let manifests = BTreeMap::from([
            (MANIFEST_FILE.into(), root),
            (format!("member/{MANIFEST_FILE}"), member),
        ]);
        let registry = Registry {
            packages: BTreeMap::from([(
                "shared".into(),
                vec![registry_candidate("shared", "1.4.0", BTreeMap::new())],
            )]),
            ..Registry::default()
        };
        let lock = resolve(
            &Resolver::new(registry, Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        let member = lock
            .packages
            .iter()
            .find(|package| package.name == "member")
            .unwrap();
        assert!(
            matches!(&member.source, LockedSource::Workspace { member, mutable: true } if member == Path::new("member"))
        );
        assert!(member.dependencies.contains_key("shared"));

        let reused = resolve(
            &Resolver::new(Registry::default(), Git::default()),
            &manifests,
            Some(&lock),
            ResolutionMode::Locked,
        )
        .unwrap();
        assert_eq!(reused, lock);
    }

    #[test]
    fn inherited_workspace_path_is_relative_to_workspace_root() {
        let mut root = manifest("app", "1.0.0", BTreeMap::new());
        root.workspace = Some(Workspace {
            members: vec!["crates/member".into()],
            exclude: vec![],
            target_dir: "target".into(),
            dependencies: BTreeMap::from([(
                "shared".into(),
                DependencySpec::path("vendor/shared"),
            )]),
        });
        let member = manifest(
            "member",
            "1.0.0",
            BTreeMap::from([("shared".into(), DependencySpec::workspace())]),
        );
        let manifests = BTreeMap::from([
            (MANIFEST_FILE.into(), root),
            (format!("crates/member/{MANIFEST_FILE}"), member),
            (
                format!("vendor/shared/{MANIFEST_FILE}"),
                manifest("shared", "1.0.0", BTreeMap::new()),
            ),
        ]);
        let lock = resolve(
            &Resolver::new(Registry::default(), Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        let member = lock
            .packages
            .iter()
            .find(|package| package.name == "member")
            .unwrap();
        let shared = lock
            .packages
            .iter()
            .find(|package| package.name == "shared")
            .unwrap();
        assert_eq!(member.dependencies.get("shared"), Some(&shared.id));
        assert!(
            matches!(&shared.source, LockedSource::Path { path, .. } if path == Path::new("vendor/shared"))
        );
    }

    #[test]
    fn inherited_workspace_modifiers_are_merged() {
        let mut base = detail(&DependencySpec::path("vendor/shared"));
        base.features.insert("base".into());
        base.default_features = false;
        let workspace = BTreeMap::from([(
            "shared".into(),
            WorkspaceDependency {
                root_manifest: MANIFEST_FILE.into(),
                spec: DependencySpec::Detail(Box::new(base)),
            },
        )]);
        let mut member = detail(&DependencySpec::workspace());
        member.optional = true;
        member.features.insert("member".into());
        let merged = detail(
            &inherited_spec(
                &DependencySpec::Detail(Box::new(member)),
                "shared",
                &workspace,
                &format!("crates/member/{MANIFEST_FILE}"),
            )
            .unwrap(),
        );
        assert!(merged.optional);
        assert!(!merged.default_features);
        assert_eq!(
            merged.features,
            BTreeSet::from(["base".into(), "member".into()])
        );
        assert_eq!(merged.path, Some(PathBuf::from("../../vendor/shared")));
    }

    #[test]
    fn canonical_local_locator_disambiguates_equal_name_and_version() {
        let mut chosen = detail(&DependencySpec::path("two"));
        chosen.package = Some("dup".into());
        let manifests = BTreeMap::from([
            (
                MANIFEST_FILE.into(),
                manifest(
                    "app",
                    "1.0.0",
                    BTreeMap::from([("chosen".into(), DependencySpec::Detail(Box::new(chosen)))]),
                ),
            ),
            (
                format!("one/{MANIFEST_FILE}"),
                manifest("dup", "1.0.0", BTreeMap::new()),
            ),
            (
                format!("two/{MANIFEST_FILE}"),
                manifest(
                    "dup",
                    "1.0.0",
                    BTreeMap::from([(
                        "leaf".into(),
                        DependencySpec::Version("*".parse().unwrap()),
                    )]),
                ),
            ),
        ]);
        let registry = Registry {
            packages: BTreeMap::from([(
                "leaf".into(),
                vec![registry_candidate("leaf", "1.0.0", BTreeMap::new())],
            )]),
            ..Registry::default()
        };
        let resolver = Resolver::new(registry, Git::default());
        let lock = resolve(&resolver, &manifests, None, ResolutionMode::Online).unwrap();
        resolve(&resolver, &manifests, Some(&lock), ResolutionMode::Locked).unwrap();
        let app = lock
            .packages
            .iter()
            .find(|package| package.name == "app")
            .unwrap();
        let chosen = lock
            .packages
            .iter()
            .find(|package| package.id == app.dependencies["chosen"])
            .unwrap();
        assert!(
            matches!(&chosen.source, LockedSource::Path { path, .. } if path == Path::new("two"))
        );
    }

    #[test]
    fn current_lock_is_reused_without_source_queries_and_locked_rejects_drift() {
        let manifests = root_with_dependency("dep", DependencySpec::Version("^1".parse().unwrap()));
        let registry = Registry {
            packages: BTreeMap::from([(
                "dep".into(),
                vec![registry_candidate("dep", "1.0.0", BTreeMap::new())],
            )]),
            ..Registry::default()
        };
        let resolver = Resolver::new(registry, Git::default());
        let lock = resolve(&resolver, &manifests, None, ResolutionMode::Online).unwrap();
        resolver.registry.calls.borrow_mut().clear();
        let reused = resolve(&resolver, &manifests, Some(&lock), ResolutionMode::Offline).unwrap();
        assert_eq!(reused, lock);
        assert!(resolver.registry.calls.borrow().is_empty());

        let mut stale = lock.clone();
        stale.manifest_digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert!(matches!(
            resolve(&resolver, &manifests, Some(&stale), ResolutionMode::Locked),
            Err(ResolveError::LockOutdated(_))
        ));
    }

    #[test]
    fn frozen_requires_remote_content_but_allows_mutable_local_sources() {
        let manifests = root_with_dependency("dep", DependencySpec::Version("^1".parse().unwrap()));
        let registry = Registry {
            packages: BTreeMap::from([(
                "dep".into(),
                vec![registry_candidate("dep", "1.0.0", BTreeMap::new())],
            )]),
            ..Registry::default()
        };
        let resolver = Resolver::new(registry, Git::default());
        let lock = resolve(&resolver, &manifests, None, ResolutionMode::Online).unwrap();
        let result = resolve(&resolver, &manifests, Some(&lock), ResolutionMode::Frozen);
        assert!(matches!(result, Err(ResolveError::Unavailable(_))));
    }

    #[test]
    fn offline_policy_reaches_git_port_without_network_permission() {
        let manifests = root_with_dependency(
            "dep",
            DependencySpec::git(
                "https://example.invalid/dep",
                GitReference {
                    branch: Some("main".into()),
                    ..GitReference::default()
                },
            ),
        );
        let git = Git {
            candidate: Some(GitCandidate {
                revision: "ab".repeat(20),
                checksum: format!("sha256:{}", "cd".repeat(32)),
                manifest: manifest("dep", "1.0.0", BTreeMap::new()),
            }),
            ..Git::default()
        };
        let resolver = Resolver::new(Registry::default(), git);
        resolve(&resolver, &manifests, None, ResolutionMode::Offline).unwrap();
        assert_eq!(&*resolver.git.calls.borrow(), &[Access::LocalOnly]);
    }

    #[test]
    fn unchanged_git_selector_reuses_prior_exact_revision_after_unrelated_change() {
        let git_spec = DependencySpec::git(
            "https://example.invalid/dep",
            GitReference {
                branch: Some("main".into()),
                ..GitReference::default()
            },
        );
        let manifests = root_with_dependency("dep", git_spec.clone());
        let git = Git {
            candidate: Some(GitCandidate {
                revision: "aa".repeat(20),
                checksum: format!("sha256:{}", "11".repeat(32)),
                manifest: manifest("dep", "1.0.0", BTreeMap::new()),
            }),
            ..Git::default()
        };
        let resolver = Resolver::new(Registry::default(), git);
        let lock = resolve(&resolver, &manifests, None, ResolutionMode::Online).unwrap();
        resolver.git.calls.borrow_mut().clear();
        resolver.git.candidate.as_ref().unwrap();

        let mut changed = root_with_dependency("dep", git_spec);
        changed
            .get_mut(MANIFEST_FILE)
            .unwrap()
            .exports
            .insert("unrelated".into(), "src/unrelated.prompt".into());
        let reused = resolve(&resolver, &changed, Some(&lock), ResolutionMode::Online).unwrap();
        let dependency = reused
            .packages
            .iter()
            .find(|package| package.name == "dep")
            .unwrap();
        assert!(
            matches!(&dependency.source, LockedSource::Git { revision, .. } if revision == &"aa".repeat(20))
        );
        assert!(resolver.git.calls.borrow().is_empty());
    }

    #[test]
    fn local_cycles_terminate_and_resolution_is_deterministic() {
        let mut root = manifest(
            "app",
            "1.0.0",
            BTreeMap::from([("a".into(), DependencySpec::path("a"))]),
        );
        root.workspace = Some(Workspace {
            members: vec!["a".into(), "b".into()],
            exclude: vec![],
            target_dir: "target".into(),
            dependencies: BTreeMap::new(),
        });
        let manifests = BTreeMap::from([
            (MANIFEST_FILE.into(), root),
            (
                format!("a/{MANIFEST_FILE}"),
                manifest(
                    "a",
                    "1.0.0",
                    BTreeMap::from([("b".into(), DependencySpec::path("../b"))]),
                ),
            ),
            (
                format!("b/{MANIFEST_FILE}"),
                manifest(
                    "b",
                    "1.0.0",
                    BTreeMap::from([("a".into(), DependencySpec::path("../a"))]),
                ),
            ),
        ]);
        let first = resolve(
            &Resolver::new(Registry::default(), Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        let second = resolve(
            &Resolver::new(Registry::default(), Git::default()),
            &manifests,
            None,
            ResolutionMode::Online,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.packages.len(), 3);
    }
}
