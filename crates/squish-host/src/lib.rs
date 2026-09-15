//! 显式的生产组合根。 / Explicit production composition root.
//!
//! 本 crate 只组合各领域 crate 的公开端口；它不复制缓存、发布或 build-record 的私有
//! 格式。 / This crate only composes public domain ports; it does not copy private cache,
//! publication, or build-record formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use squish_fetch::{
    CredentialPort, FetchError, GitHost, GitRunner, HostContext, HttpRequest, HttpResponse,
    HttpTransport, Limits, Materializer, Observer, RegistryConfig, SparseRegistry, SystemGitRunner,
};
use squish_manager::{
    ArtifactLocator, ProvenanceNonApplicability, ProvenanceRelation, ResolveRequest,
    ResolvedDependencies, ServiceError, Services, StorageLayout,
};
use squish_project::{DependencyResolver, LockedSource, Lockfile, ResolutionInput, ResolutionMode};
use squish_repository::PackageLocation;
use squish_resolver::{
    Access as ResolverAccess, FilesystemPort, GitCandidate, GitPort, LocalPackage, LocalRequest,
    RegistryCandidate, RegistryPort, Resolver, SourceUnavailable,
};
use squish_store::{ActionKey as StoreActionKey, BlobDigest, Cas, VerifiedActionIndex};

/// 一个 resolver registry 名称及其稳定 fetch 配置。 / One resolver registry name and its stable fetch configuration.
#[derive(Clone, Debug)]
pub struct RegistryEndpoint {
    /// 清单与锁中使用的 registry 名称。 / Registry name used by manifests and locks.
    pub name: String,
    /// 稳定身份与可变 sparse endpoint。 / Stable identity and mutable sparse endpoint.
    pub config: RegistryConfig,
}

/// Git 执行依赖；生产可固定 executable，测试可注入 runner。 / Git execution dependency; production can pin an executable and tests can inject a runner.
pub enum GitExecution {
    /// 直接执行该路径，不搜索 shell 或 `PATH`。 / Executes this path directly without shell or `PATH` lookup.
    Executable(PathBuf),
    /// 使用完整的显式 runner 端口。 / Uses a complete explicit runner port.
    Runner(Arc<dyn GitRunner>),
}

/// 生产宿主的全部外部依赖。 / Complete external dependencies of the production host.
pub struct HostConfig {
    /// 唯一允许的项目根。 / Sole permitted project root.
    pub project_root: PathBuf,
    /// Registry/Git 获取缓存；不是构建产物 CAS。 / Registry/Git acquisition cache; not the build artifact CAS.
    pub source_cache_root: PathBuf,
    /// 构建 CAS、动作索引、发布及 catalog 的唯一权威布局。 / Sole authoritative layout for build CAS, action index, publication, and catalog.
    pub storage: StorageLayout,
    /// 名称唯一的 registry 集。 / Registries with unique names.
    pub registries: Vec<RegistryEndpoint>,
    /// Registry credential 来源。 / Registry credential source.
    pub credentials: Arc<dyn CredentialPort>,
    /// 禁止自动跳转且实行有界读取的 HTTP transport。 / HTTP transport with no automatic redirects and bounded reads.
    pub http: Arc<dyn HttpTransport>,
    /// 明确的 Git executable 或 runner。 / Explicit Git executable or runner.
    pub git: GitExecution,
    /// 获取资源限制。 / Acquisition resource limits.
    pub limits: Limits,
    /// 不接触 credential/正文的获取事件 observer。 / Acquisition event observer which never sees credentials or bodies.
    pub observer: Arc<dyn Observer>,
    /// 显式 path-dependency 文件系统端口。 / Explicit filesystem port for path dependencies.
    pub filesystem: Arc<dyn FilesystemPort + Send + Sync>,
}

impl fmt::Debug for HostConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostConfig")
            .field("project_root", &self.project_root)
            .field("source_cache_root", &self.source_cache_root)
            .field("registries", &self.registries)
            .finish_non_exhaustive()
    }
}

/// 组合根构造失败。 / Composition-root construction failure.
#[derive(Debug)]
pub enum HostError {
    /// 显式配置自相矛盾。 / Explicit configuration is inconsistent.
    Config(String),
    /// 文件系统操作失败。 / Filesystem operation failed.
    Io(std::io::Error),
    /// 来源宿主初始化失败。 / Source-host initialization failed.
    Fetch(FetchError),
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(formatter, "invalid host configuration: {message}"),
            Self::Io(error) => write!(formatter, "host filesystem unavailable: {error}"),
            Self::Fetch(error) => write!(formatter, "source host unavailable: {error}"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Fetch(error) => Some(error),
            Self::Config(_) => None,
        }
    }
}

impl From<std::io::Error> for HostError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FetchError> for HostError {
    fn from(error: FetchError) -> Self {
        Self::Fetch(error)
    }
}

#[derive(Clone)]
struct SharedCredentials(Arc<dyn CredentialPort>);
impl CredentialPort for SharedCredentials {
    fn authorization(&self, registry_id: &str, origin: &str) -> Option<String> {
        self.0.authorization(registry_id, origin)
    }
}

#[derive(Clone)]
struct SharedHttp(Arc<dyn HttpTransport>);
impl HttpTransport for SharedHttp {
    fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
        self.0.execute(request)
    }
}

struct RegistryEntry {
    name: String,
    registry: SparseRegistry<SharedCredentials>,
}

/// 不读取进程全局状态的 production services 组合。 / Production services composition which reads no process-global state.
pub struct ProductionHost {
    project_root: PathBuf,
    source_cache_root: PathBuf,
    storage: StorageLayout,
    context: HostContext,
    registries: Vec<RegistryEntry>,
    git: GitHost,
    filesystem: Arc<dyn FilesystemPort + Send + Sync>,
}

impl ProductionHost {
    /// 校验所有身份并构造唯一服务图。 / Validates every identity and constructs the sole service graph.
    ///
    /// # 示例 / Example
    ///
    /// ```no_run
    /// use std::{path::PathBuf, sync::Arc};
    /// use squish_fetch::{FilesystemHost, Limits, NoCredentials, NoopObserver, ReqwestTransport};
    /// use squish_host::{GitExecution, HostConfig, ProductionHost};
    /// use squish_manager::StorageLayout;
    ///
    /// # fn compose(project: PathBuf, source_cache: PathBuf, storage: StorageLayout)
    /// # -> Result<ProductionHost, Box<dyn std::error::Error>> {
    /// let filesystem = Arc::new(FilesystemHost::new(&project)?);
    /// let host = ProductionHost::open(HostConfig {
    ///     project_root: project,
    ///     source_cache_root: source_cache,
    ///     storage,
    ///     registries: Vec::new(),
    ///     credentials: Arc::new(NoCredentials),
    ///     http: Arc::new(ReqwestTransport::new()?),
    ///     git: GitExecution::Executable("/usr/bin/git".into()),
    ///     limits: Limits::default(),
    ///     observer: Arc::new(NoopObserver),
    ///     filesystem,
    /// })?;
    /// # Ok(host) }
    /// ```
    pub fn open(config: HostConfig) -> Result<Self, HostError> {
        let project_root = std::fs::canonicalize(&config.project_root)?;
        std::fs::create_dir_all(&config.source_cache_root)?;
        let source_cache_root = std::fs::canonicalize(&config.source_cache_root)?;
        let context = HostContext {
            cache: source_cache_root.clone(),
            limits: config.limits,
            observer: config.observer,
        };
        let mut names = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut registries = Vec::with_capacity(config.registries.len());
        for endpoint in config.registries {
            if endpoint.name.trim().is_empty()
                || !names.insert(endpoint.name.clone())
                || !identities.insert(endpoint.config.id.clone())
            {
                return Err(HostError::Config(
                    "registry names and stable identities must be non-empty and unique".into(),
                ));
            }
            let registry = SparseRegistry::with_dependencies(
                context.clone(),
                endpoint.config,
                SharedCredentials(config.credentials.clone()),
                Box::new(SharedHttp(config.http.clone())),
            )?;
            registries.push(RegistryEntry {
                name: endpoint.name,
                registry,
            });
        }
        for (name, path) in [
            ("build CAS", config.storage.cas_root()),
            ("action index", config.storage.action_index()),
            ("publication root", config.storage.publication_root()),
            ("catalog root", config.storage.catalog_root()),
        ] {
            if paths_overlap(&source_cache_root, path) {
                return Err(HostError::Config(format!(
                    "source cache overlaps the {name} responsibility"
                )));
            }
        }
        let git_runner: Arc<dyn GitRunner> = match config.git {
            GitExecution::Executable(path) => {
                if !path.is_absolute() {
                    return Err(HostError::Config(
                        "Git executable must be an absolute path".into(),
                    ));
                }
                Arc::new(SystemGitRunner::new(path))
            }
            GitExecution::Runner(runner) => runner,
        };
        let git = GitHost::with_runner(context.clone(), git_runner);
        Ok(Self {
            project_root,
            source_cache_root,
            storage: config.storage,
            context,
            registries,
            git,
            filesystem: config.filesystem,
        })
    }

    /// 返回规范化项目根。 / Returns the canonical project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 返回仅供来源获取使用的规范化缓存根。 / Returns the canonical source-acquisition cache root.
    pub fn source_cache_root(&self) -> &Path {
        &self.source_cache_root
    }

    fn registry(&self, name: &str) -> Result<&RegistryEntry, SourceUnavailable> {
        self.registries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| SourceUnavailable {
                identity: name.into(),
                detail: "registry is not configured".into(),
            })
    }

    fn materialize_lock(
        &self,
        lock: &Lockfile,
        mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, SourceUnavailable> {
        lock.validate().map_err(|error| SourceUnavailable {
            identity: self.project_root.display().to_string(),
            detail: format!("invalid exact lock: {error}"),
        })?;
        let access = fetch_access(mode);
        let materializer = Materializer::new(self.context.clone());
        let mut locations = Vec::new();
        for package in &lock.packages {
            let materialized = match &package.source {
                LockedSource::Registry { registry, checksum } => {
                    let entry = self.registry(registry)?;
                    let candidates = entry
                        .registry
                        .candidates(&package.name, access)
                        .map_err(|error| unavailable(registry, error))?;
                    let candidate = candidates.into_iter().find(|candidate| {
                        candidate.version == package.version
                            && format!("sha256:{}", candidate.archive.digest.0.0) == *checksum
                    });
                    let candidate = candidate.ok_or_else(|| SourceUnavailable {
                        identity: package.id.clone(),
                        detail: "locked registry candidate is absent from exact metadata".into(),
                    })?;
                    Some(
                        entry
                            .registry
                            .materialize(&package.name, &candidate, access, &materializer)
                            .map_err(|error| unavailable(&package.id, error))?,
                    )
                }
                LockedSource::Git {
                    repository,
                    revision,
                    checksum,
                } => {
                    let candidate = self
                        .git
                        .resolve_exact(
                            repository,
                            &squish_fetch::GitSelector::Rev(revision.clone()),
                            ".",
                            access,
                        )
                        .map_err(|error| unavailable(&package.id, error))?;
                    if format!("sha256:{}", candidate.content_digest.0.0) != *checksum {
                        return Err(SourceUnavailable {
                            identity: package.id.clone(),
                            detail: "locked Git content checksum mismatch".into(),
                        });
                    }
                    Some(
                        self.git
                            .materialize_locked(
                                repository,
                                &candidate.commit,
                                &candidate.package_tree,
                                ".",
                                &candidate.content_digest,
                                access,
                            )
                            .map_err(|error| unavailable(&package.id, error))?,
                    )
                }
                LockedSource::Path { .. } | LockedSource::Workspace { .. } => None,
            };
            if let Some(materialized) = materialized {
                locations.push(PackageLocation {
                    lock_id: package.id.clone(),
                    root: materialized.root,
                });
            }
        }
        locations.sort_by(|left, right| left.lock_id.cmp(&right.lock_id));
        Ok(locations)
    }

    fn resolve_dependencies(
        &self,
        manifests: &std::collections::BTreeMap<String, squish_project::Manifest>,
        manifest_digest: &str,
        prior_lock: Option<&Lockfile>,
        mode: ResolutionMode,
    ) -> Result<(Lockfile, Vec<PackageLocation>), SourceUnavailable> {
        let resolver = Resolver::new(RegistryView(self), GitView(&self.git))
            .with_filesystem(FilesystemView(self.filesystem.as_ref()));
        let lockfile = resolver
            .resolve(ResolutionInput {
                manifests,
                manifest_digest,
                prior_lock,
                mode,
            })
            .map_err(|error| SourceUnavailable {
                identity: self.project_root.display().to_string(),
                detail: error.to_string(),
            })?;
        let packages = self.materialize_lock(&lockfile, mode)?;
        Ok((lockfile, packages))
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        let left = left.to_string_lossy().to_lowercase();
        let right = right.to_string_lossy().to_lowercase();
        left == right
            || left
                .strip_prefix(&right)
                .is_some_and(|tail| tail.starts_with(['\\', '/']))
            || right
                .strip_prefix(&left)
                .is_some_and(|tail| tail.starts_with(['\\', '/']))
    } else {
        left == right || left.starts_with(right) || right.starts_with(left)
    }
}

impl Services for ProductionHost {
    fn storage_layout(&self, project_root: &Path) -> Result<StorageLayout, ServiceError> {
        self.require_project(project_root)?;
        Ok(self.storage.clone())
    }

    fn materialize_locked(
        &self,
        project: &Path,
        lock: &Lockfile,
        mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        self.require_project(project)?;
        self.materialize_lock(lock, mode)
            .map_err(|error| service_error("source_materialization_failed", error))
    }

    fn resolve(&self, request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        let (lockfile, packages) = self
            .resolve_dependencies(
                request.manifests,
                request.manifest_digest,
                request.prior_lock,
                request.mode,
            )
            .map_err(|error| service_error("dependency_resolution_failed", error))?;
        Ok(ResolvedDependencies { lockfile, packages })
    }

    fn cache_records(
        &self,
        project: &Path,
    ) -> Result<Vec<squish_protocol::CachedAction>, ServiceError> {
        self.require_project(project)?;
        cache_records(self.storage.cas_root(), self.storage.action_index())
            .map_err(|error| ServiceError::new("cache_catalog_failed", error))
    }

    fn read_blob(
        &self,
        project: &Path,
        digest: &squish_protocol::Digest,
    ) -> Result<Option<Vec<u8>>, ServiceError> {
        self.require_project(project)?;
        read_blob(self.storage.cas_root(), digest)
            .map_err(|error| ServiceError::new("blob_read_failed", error))
    }

    fn artifact(
        &self,
        project: &Path,
        id: &squish_protocol::ArtifactId,
    ) -> Result<Option<squish_protocol::Artifact>, ServiceError> {
        self.require_project(project)?;
        Ok(self
            .current_catalog()?
            .and_then(|catalog| catalog.artifact(id).cloned()))
    }

    fn artifact_at(
        &self,
        project: &Path,
        locator: &ArtifactLocator,
    ) -> Result<Option<squish_protocol::Artifact>, ServiceError> {
        self.require_project(project)?;
        let relative = if locator.as_path().is_absolute() {
            locator
                .as_path()
                .strip_prefix(&self.project_root)
                .map_err(|_| {
                    ServiceError::new(
                        "invalid_artifact_path",
                        "artifact path is outside the configured project",
                    )
                })?
        } else {
            locator.as_path()
        };
        let relative = ArtifactLocator::new(relative.to_path_buf())
            .map_err(|error| ServiceError::new("invalid_artifact_path", error.to_string()))?;
        Ok(self
            .current_catalog()?
            .and_then(|catalog| catalog.artifact_at(&relative).cloned()))
    }

    fn link_map(
        &self,
        project: &Path,
        target: &squish_protocol::TargetName,
    ) -> Result<Option<squish_protocol::Artifact>, ServiceError> {
        self.require_project(project)?;
        self.current_catalog()?
            .map(|catalog| {
                catalog
                    .link_map(target)
                    .map(|artifact| artifact.cloned())
                    .map_err(|error| {
                        ServiceError::new("artifact_catalog_ambiguous", error.to_string())
                    })
            })
            .transpose()
            .map(Option::flatten)
    }

    fn provenance_evidence(
        &self,
        project: &Path,
        artifact: &squish_protocol::ArtifactId,
    ) -> Result<ProvenanceRelation, ServiceError> {
        self.require_project(project)?;
        Ok(match self.current_catalog()? {
            Some(catalog) => catalog.provenance_relation(artifact),
            None => ProvenanceRelation::NotApplicable(ProvenanceNonApplicability::UnsupportedKind),
        })
    }

    fn planned_actions(
        &self,
        project: &Path,
    ) -> Result<Option<squish_protocol::PlanInspection>, ServiceError> {
        self.require_project(project)?;
        Ok(self
            .current_catalog()?
            .map(|catalog| catalog.record.planned_actions().clone()))
    }
}

impl ProductionHost {
    fn require_project(&self, project: &Path) -> Result<(), ServiceError> {
        let canonical = std::fs::canonicalize(project)
            .map_err(|error| ServiceError::new("project_root_unavailable", error.to_string()))?;
        if canonical != self.project_root {
            return Err(ServiceError::new(
                "project_root_mismatch",
                format!(
                    "requested project `{}` differs from configured project `{}`",
                    canonical.display(),
                    self.project_root.display()
                ),
            ));
        }
        Ok(())
    }

    fn current_catalog(
        &self,
    ) -> Result<Option<squish_manager::build::BuildCatalogSnapshot>, ServiceError> {
        squish_manager::build::read_current_build_catalog(&self.storage)
            .map_err(|error| ServiceError::new("build_catalog_failed", error.to_string()))
    }
}

fn service_error(code: &str, error: SourceUnavailable) -> ServiceError {
    ServiceError::new(code, format!("{}: {}", error.identity, error.detail))
}

fn protocol_digest(digest: BlobDigest) -> squish_protocol::Digest {
    squish_protocol::Digest::new(
        squish_protocol::DigestAlgorithm::Blake3,
        digest.as_bytes().to_vec(),
    )
    .expect("BLAKE3 digest has the protocol's canonical length")
}

fn blob_digest(digest: &squish_protocol::Digest) -> Option<BlobDigest> {
    if digest.algorithm() != &squish_protocol::DigestAlgorithm::Blake3 {
        return None;
    }
    let bytes: [u8; 32] = digest.bytes().try_into().ok()?;
    Some(BlobDigest::from_bytes(bytes))
}

fn cache_records(
    cas_root: &Path,
    action_index: &Path,
) -> Result<Vec<squish_protocol::CachedAction>, String> {
    let cas = Arc::new(Cas::open(cas_root).map_err(|error| error.to_string())?);
    let index = VerifiedActionIndex::open(action_index, cas).map_err(|error| error.to_string())?;
    let mut after: Option<StoreActionKey> = None;
    let mut records = Vec::new();
    loop {
        let page = index
            .manifest_page(after, squish_store::MAX_MANIFEST_PAGE_SIZE)
            .map_err(|error| error.to_string())?;
        for manifest in page.manifests {
            let action_key =
                squish_protocol::ActionKeyId::new(manifest.record.key.as_str().to_owned())
                    .map_err(|error| error.to_string())?;
            let outputs = manifest
                .record
                .outputs
                .into_iter()
                .map(|output| {
                    Ok(squish_protocol::Artifact {
                        id: squish_protocol::ArtifactId::new(output.name.as_str().to_owned())
                            .map_err(|error| error.to_string())?,
                        kind: output.kind,
                        uri: format!("cas:blake3:{}", output.digest.hex()),
                        size: output.size,
                        digest: output.digest,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            records.push(squish_protocol::CachedAction {
                action_key,
                result_digest: protocol_digest(manifest.result_digest),
                outputs,
            });
        }
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    Ok(records)
}

fn read_blob(cas_root: &Path, digest: &squish_protocol::Digest) -> Result<Option<Vec<u8>>, String> {
    let Some(digest) = blob_digest(digest) else {
        return Ok(None);
    };
    Cas::open(cas_root)
        .and_then(|cas| cas.get(digest))
        .map_err(|error| error.to_string())
}

struct RegistryView<'a>(&'a ProductionHost);
impl RegistryPort for RegistryView<'_> {
    fn candidates(
        &self,
        registry: &str,
        package: &str,
        access: ResolverAccess,
    ) -> Result<Vec<RegistryCandidate>, SourceUnavailable> {
        let entry = self.0.registry(registry)?;
        entry
            .registry
            .candidates(package, source_access(access))
            .map(|rows| {
                rows.into_iter()
                    .filter(|row| !row.yanked)
                    .map(|row| RegistryCandidate {
                        version: row.version,
                        checksum: format!("sha256:{}", row.archive.digest.0.0),
                        manifest: row.manifest,
                    })
                    .collect()
            })
            .map_err(|error| unavailable(registry, error))
    }

    fn contains(
        &self,
        registry: &str,
        package: &str,
        version: &semver::Version,
        checksum: &str,
    ) -> bool {
        self.0.registry(registry).is_ok_and(|entry| {
            RegistryPort::contains(&entry.registry, registry, package, version, checksum)
        })
    }
}

struct GitView<'a>(&'a GitHost);
impl GitPort for GitView<'_> {
    fn resolve(
        &self,
        repository: &str,
        reference: &squish_project::GitReference,
        access: ResolverAccess,
    ) -> Result<GitCandidate, SourceUnavailable> {
        GitPort::resolve(self.0, repository, reference, access)
    }

    fn contains(&self, repository: &str, revision: &str, checksum: &str) -> bool {
        GitPort::contains(self.0, repository, revision, checksum)
    }
}

struct FilesystemView<'a>(&'a (dyn FilesystemPort + Send + Sync));
impl FilesystemPort for FilesystemView<'_> {
    fn load(&self, request: LocalRequest<'_>) -> Result<LocalPackage, SourceUnavailable> {
        self.0.load(request)
    }
}

fn source_access(access: ResolverAccess) -> squish_fetch::Access {
    match access {
        ResolverAccess::Online => squish_fetch::Access::Online,
        ResolverAccess::LocalOnly => squish_fetch::Access::LocalOnly,
    }
}

fn fetch_access(mode: ResolutionMode) -> squish_fetch::Access {
    match mode {
        ResolutionMode::Online | ResolutionMode::Locked => squish_fetch::Access::Online,
        ResolutionMode::Offline | ResolutionMode::Frozen => squish_fetch::Access::LocalOnly,
    }
}

fn unavailable(identity: &str, error: FetchError) -> SourceUnavailable {
    SourceUnavailable {
        identity: identity.into(),
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_build::{ActionIndex, ActionRecord, ProducedOutput};
    use squish_fetch::{GitInvocation, GitRunOutput, NoCredentials, NoopObserver};
    use squish_protocol::{ActionKeyId, ArtifactKind, Digest, DigestAlgorithm};

    struct NoHttp;
    impl HttpTransport for NoHttp {
        fn execute(&self, _request: HttpRequest) -> Result<HttpResponse, FetchError> {
            Err(FetchError::Http(
                "network is forbidden by the fixture".into(),
            ))
        }
    }

    struct NoGit;
    impl GitRunner for NoGit {
        fn execute(&self, _invocation: GitInvocation) -> std::io::Result<GitRunOutput> {
            Err(std::io::Error::other("Git is forbidden by the fixture"))
        }
    }

    fn fixture() -> (tempfile::TempDir, ProductionHost, StorageLayout) {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(scratch).unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-")
            .tempdir_in(scratch)
            .unwrap();
        let root = temporary.path().join("project");
        let source_cache = temporary.path().join("source-cache");
        std::fs::create_dir_all(&root).unwrap();
        let storage = StorageLayout::new(
            temporary.path().join("cas"),
            temporary.path().join("actions.sqlite"),
            temporary.path().join("publish"),
            temporary.path().join("catalog"),
        )
        .unwrap();
        let filesystem = Arc::new(squish_fetch::FilesystemHost::new(&root).unwrap());
        let host = ProductionHost::open(HostConfig {
            project_root: root,
            source_cache_root: source_cache,
            storage: storage.clone(),
            registries: Vec::new(),
            credentials: Arc::new(NoCredentials),
            http: Arc::new(NoHttp),
            git: GitExecution::Runner(Arc::new(NoGit)),
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
            filesystem,
        })
        .unwrap();
        (temporary, host, storage)
    }

    #[test]
    fn path_only_resolution_never_calls_external_transports() {
        let (_temporary, host, _) = fixture();
        let manifest = squish_project::Manifest::parse(
            "manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let manifests = std::collections::BTreeMap::from([("xmlsquish.toml".into(), manifest)]);
        let (lock, locations) = host
            .resolve_dependencies(
                &manifests,
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                None,
                ResolutionMode::Online,
            )
            .unwrap();
        assert_eq!(lock.packages.len(), 1);
        assert!(locations.is_empty());
        for mode in [ResolutionMode::Locked, ResolutionMode::Frozen] {
            let (reused, locations) = host
                .resolve_dependencies(
                    &manifests,
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    Some(&lock),
                    mode,
                )
                .unwrap();
            assert_eq!(reused, lock);
            assert!(locations.is_empty());
        }
    }

    #[test]
    fn cache_catalog_materializes_result_record_in_the_authoritative_cas() {
        let (_temporary, host, storage) = fixture();
        let cas = Arc::new(Cas::open(storage.cas_root()).unwrap());
        let bytes = b"cached output";
        let digest = cas.put(bytes).unwrap();
        let index = VerifiedActionIndex::open(storage.action_index(), cas.clone()).unwrap();
        let key_text = format!("blake3:{}", BlobDigest::of(b"action").to_hex());
        let key = ActionKeyId::new(key_text.clone()).unwrap();
        let output_digest =
            Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec()).unwrap();
        ActionIndex::record(
            &index,
            &ActionRecord {
                key: squish_build::ActionKey::new(key_text).unwrap(),
                outputs: vec![ProducedOutput {
                    name: squish_build::OutputName::new("prompt").unwrap(),
                    kind: ArtifactKind::Prompt,
                    digest: output_digest.clone(),
                    size: bytes.len() as u64,
                }],
            },
        )
        .unwrap();

        let records = Services::cache_records(&host, host.project_root()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].action_key, key);
        assert!(
            Services::read_blob(&host, host.project_root(), &records[0].result_digest)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            Services::read_blob(&host, host.project_root(), &output_digest).unwrap(),
            Some(bytes.to_vec())
        );
    }

    #[test]
    fn corrupt_cache_output_is_omitted_and_its_rebuildable_row_is_repaired() {
        let (_temporary, host, storage) = fixture();
        let cas = Arc::new(Cas::open(storage.cas_root()).unwrap());
        let digest = cas.put(b"expected").unwrap();
        let index = VerifiedActionIndex::open(storage.action_index(), cas.clone()).unwrap();
        let key_text = format!("blake3:{}", BlobDigest::of(b"corrupt-action").to_hex());
        let key = squish_build::ActionKey::new(key_text).unwrap();
        ActionIndex::record(
            &index,
            &ActionRecord {
                key: key.clone(),
                outputs: vec![ProducedOutput {
                    name: squish_build::OutputName::new("prompt").unwrap(),
                    kind: ArtifactKind::Prompt,
                    digest: Digest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
                        .unwrap(),
                    size: 8,
                }],
            },
        )
        .unwrap();
        std::fs::write(cas.path_for(digest), b"corrupt").unwrap();

        assert!(
            Services::cache_records(&host, host.project_root())
                .unwrap()
                .is_empty()
        );
        assert!(ActionIndex::lookup(&index, &key).unwrap().is_none());
    }
}
