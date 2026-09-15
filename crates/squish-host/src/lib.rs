//! 显式的生产组合根。 / Explicit production composition root.
//!
//! 本 crate 只组合各领域 crate 的公开端口；它不复制缓存、发布或 build-record 的私有
//! 格式。 / This crate only composes public domain ports; it does not copy private cache,
//! publication, or build-record formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use squish_config::AuthScope;
use url::Url;

use squish_fetch::{
    AuthorizationValue, CredentialError, CredentialPort, FetchError, GitHost, GitRunner,
    HostContext, HttpRequest, HttpResponse, HttpTransport, Limits, Materializer, Observer,
    RegistryConfig, SourceEvent, SparseRegistry, SystemGitRunner,
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

/// 环境 credential namespace 到 registry origin 的稳定路由。 / Stable route from an
/// environment credential namespace to a registry origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialRoute {
    /// Registry 的稳定身份；别名不属于该身份。 / Stable registry identity; aliases are
    /// not part of this identity.
    pub registry_id: String,
    /// 配置的非秘密 credential scope。 / Configured non-secret credential scope.
    pub auth_scope: String,
    /// 规范化 sparse index origin。 / Canonical sparse-index origin.
    pub primary_origin: String,
}

/// 启动时环境 credential snapshot 的配置错误。 / Configuration error in the startup
/// environment credential snapshot.
#[derive(Debug, thiserror::Error)]
pub enum CredentialConfigError {
    /// Registry route 自相矛盾或 origin 无效。 / A registry route conflicts or has an
    /// invalid origin.
    #[error("invalid credential route: {0}")]
    Route(String),
    /// 环境变量名称不可移植或存在大小写折叠重复。 / An environment variable name is
    /// non-portable or duplicated after ASCII case folding.
    #[error("invalid credential environment: {0}")]
    Environment(String),
    /// 环境中的 Authorization 字段值无效。 / An Authorization field value in the
    /// environment is invalid.
    #[error("invalid credential value in `{variable}`")]
    Value {
        /// 非秘密的变量名称。 / Non-secret variable name.
        variable: String,
    },
}

#[derive(Clone)]
struct CredentialRouteState {
    auth_scope: String,
    stem: String,
    primary_origin: String,
}

/// 从一次启动环境快照读取 registry credentials 的生产适配器。 / Production adapter
/// which reads registry credentials from one startup environment snapshot.
pub struct EnvironmentCredentials {
    routes: BTreeMap<String, CredentialRouteState>,
    values: BTreeMap<String, AuthorizationValue>,
}

impl fmt::Debug for EnvironmentCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentCredentials")
            .field("route_count", &self.routes.len())
            .field("credential_count", &self.values.len())
            .finish()
    }
}

impl EnvironmentCredentials {
    /// 校验路由和注入的环境快照；本函数不读取进程全局状态。 / Validates routes and an
    /// injected environment snapshot; this function never reads process-global state.
    ///
    /// # 示例 / Example
    ///
    /// ```
    /// use std::ffi::OsString;
    /// use squish_host::{CredentialRoute, EnvironmentCredentials};
    /// let credentials = EnvironmentCredentials::from_snapshot(
    ///     [CredentialRoute {
    ///         registry_id: "https://registry.example/v1".into(),
    ///         auth_scope: "corp-read".into(),
    ///         primary_origin: "https://index.example".into(),
    ///     }],
    ///     [(OsString::from("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION"),
    ///       OsString::from("Bearer example"))],
    /// ).expect("valid environment snapshot");
    /// # let _ = credentials;
    /// ```
    pub fn from_snapshot(
        routes: impl IntoIterator<Item = CredentialRoute>,
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, CredentialConfigError> {
        let mut route_map = BTreeMap::<String, CredentialRouteState>::new();
        let mut stems = BTreeMap::<String, String>::new();
        for route in routes {
            if route.auth_scope.is_empty() {
                return Err(CredentialConfigError::Route(format!(
                    "registry `{}` has an empty auth scope",
                    route.registry_id
                )));
            }
            let primary_origin =
                canonical_https_origin(&route.primary_origin).ok_or_else(|| {
                    CredentialConfigError::Route(format!(
                        "registry `{}` has a non-canonical HTTPS primary origin",
                        route.registry_id
                    ))
                })?;
            if primary_origin != route.primary_origin {
                return Err(CredentialConfigError::Route(format!(
                    "registry `{}` primary origin must be `{primary_origin}`",
                    route.registry_id
                )));
            }
            let stem = AuthScope::environment_stem_for(&route.auth_scope);
            if let Some(first_scope) = stems.insert(stem.clone(), route.auth_scope.clone())
                && first_scope != route.auth_scope
            {
                return Err(CredentialConfigError::Route(format!(
                    "auth scopes `{first_scope}` and `{}` share environment stem `{stem}`",
                    route.auth_scope
                )));
            }
            let state = CredentialRouteState {
                auth_scope: route.auth_scope,
                stem,
                primary_origin,
            };
            if let Some(first) = route_map.insert(route.registry_id.clone(), state.clone())
                && (first.auth_scope != state.auth_scope
                    || first.primary_origin != state.primary_origin)
            {
                return Err(CredentialConfigError::Route(format!(
                    "registry `{}` has conflicting credential routes",
                    route.registry_id
                )));
            }
        }

        let relevant_names = route_map
            .values()
            .map(|route| {
                (
                    format!("XMLSQUISH_REGISTRY_{}_AUTHORIZATION", route.stem),
                    format!("XMLSQUISH_REGISTRY_{}_ORIGIN_", route.stem),
                )
            })
            .collect::<Vec<_>>();
        let mut values = BTreeMap::new();
        for (raw_name, raw_value) in variables {
            let Some(name) = raw_name.to_str() else {
                if raw_name
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("XMLSQUISH_REGISTRY_")
                {
                    return Err(CredentialConfigError::Environment(
                        "reserved variable name is not Unicode".into(),
                    ));
                }
                continue;
            };
            let folded = name.to_ascii_uppercase();
            if !folded.starts_with("XMLSQUISH_REGISTRY_") {
                continue;
            }
            if !relevant_names.iter().any(|(primary, origin_prefix)| {
                &folded == primary || origin_variable_matches(&folded, origin_prefix)
            }) {
                continue;
            }
            if values.contains_key(&folded) {
                return Err(CredentialConfigError::Environment(format!(
                    "variables collide after ASCII case folding at `{folded}`"
                )));
            }
            let value = raw_value
                .into_string()
                .map_err(|_| CredentialConfigError::Value {
                    variable: folded.clone(),
                })?;
            let value =
                AuthorizationValue::new(value).map_err(|_| CredentialConfigError::Value {
                    variable: folded.clone(),
                })?;
            values.insert(folded, value);
        }
        Ok(Self {
            routes: route_map,
            values,
        })
    }
}

impl CredentialPort for EnvironmentCredentials {
    fn authorization(
        &self,
        registry_id: &str,
        auth_scope: &str,
        origin: &str,
    ) -> Result<Option<AuthorizationValue>, CredentialError> {
        let route = self.routes.get(registry_id).ok_or_else(|| {
            CredentialError::Invalid(format!("registry `{registry_id}` has no credential route"))
        })?;
        if route.auth_scope != auth_scope {
            return Err(CredentialError::Invalid(format!(
                "registry `{registry_id}` requested an inconsistent auth scope"
            )));
        }
        let origin = canonical_https_origin(origin).ok_or_else(|| {
            CredentialError::Invalid(format!(
                "registry `{registry_id}` requested a non-canonical HTTPS origin"
            ))
        })?;
        let name = if origin == route.primary_origin {
            format!("XMLSQUISH_REGISTRY_{}_AUTHORIZATION", route.stem)
        } else {
            let digest = hex::encode_upper(Sha256::digest(origin.as_bytes()));
            format!(
                "XMLSQUISH_REGISTRY_{}_ORIGIN_{}_AUTHORIZATION",
                route.stem, digest
            )
        };
        Ok(self.values.get(&name).cloned())
    }
}

fn origin_variable_matches(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|tail| tail.strip_suffix("_AUTHORIZATION"))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
        })
}

fn canonical_https_origin(origin: &str) -> Option<String> {
    let url = Url::parse(origin).ok()?;
    if url.scheme() != "https"
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
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
    fn authorization(
        &self,
        registry_id: &str,
        auth_scope: &str,
        origin: &str,
    ) -> Result<Option<AuthorizationValue>, CredentialError> {
        self.0.authorization(registry_id, auth_scope, origin)
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
    identity: String,
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
        let source_cache_root = native_host_path(std::fs::canonicalize(&config.source_cache_root)?);
        let mut context = HostContext::new(source_cache_root.clone())?;
        context.limits = config.limits;
        context.observer = config.observer;
        validate_registry_routes(&config.registries)?;
        let mut registries = Vec::with_capacity(config.registries.len());
        for endpoint in config.registries {
            let identity = endpoint.config.id.clone();
            let registry = SparseRegistry::with_dependencies(
                context.clone(),
                endpoint.config,
                SharedCredentials(config.credentials.clone()),
                Box::new(SharedHttp(config.http.clone())),
            )?;
            registries.push(RegistryEntry {
                name: endpoint.name,
                identity,
                registry,
            });
        }
        let storage_paths = [
            ("build CAS", config.storage.cas_root()),
            ("action index", config.storage.action_index()),
            ("publication root", config.storage.publication_root()),
            ("catalog root", config.storage.catalog_root()),
        ]
        .map(|(name, path)| Ok((name, normalize_from_existing_ancestor(path)?)))
        .into_iter()
        .collect::<Result<Vec<_>, std::io::Error>>()?;
        for (name, physical) in &storage_paths {
            if paths_overlap(&source_cache_root, physical) {
                return Err(HostError::Config(format!(
                    "source cache overlaps the {name} responsibility"
                )));
            }
        }
        for left in 0..storage_paths.len() {
            for right in left + 1..storage_paths.len() {
                if paths_overlap(&storage_paths[left].1, &storage_paths[right].1) {
                    return Err(HostError::Config(format!(
                        "{} physically overlaps the {} responsibility",
                        storage_paths[left].0, storage_paths[right].0
                    )));
                }
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
            .find(|entry| entry.name == name || entry.identity == name)
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
                    let mut result = self.materialize_registry(
                        entry,
                        package,
                        checksum,
                        squish_fetch::Access::LocalOnly,
                        &materializer,
                    );
                    if let Err(error) = &result {
                        self.observe_local_miss(&package.id, error);
                    }
                    if result.as_ref().is_err_and(retryable_registry_miss)
                        && access == squish_fetch::Access::Online
                    {
                        result = self.materialize_registry(
                            entry,
                            package,
                            checksum,
                            squish_fetch::Access::Online,
                            &materializer,
                        );
                    }
                    Some(result.map_err(|error| unavailable(&package.id, error))?)
                }
                LockedSource::Git {
                    repository,
                    revision,
                    checksum,
                } => {
                    let format = match revision.len() {
                        40 => squish_fetch::GitObjectFormat::Sha1,
                        64 => squish_fetch::GitObjectFormat::Sha256,
                        _ => {
                            return Err(SourceUnavailable {
                                identity: package.id.clone(),
                                detail: "locked Git revision has no supported object format".into(),
                            });
                        }
                    };
                    let commit = squish_fetch::GitOid::new(format, revision.clone())
                        .map_err(|error| unavailable(&package.id, error))?;
                    let content =
                        checksum
                            .strip_prefix("sha256:")
                            .ok_or_else(|| SourceUnavailable {
                                identity: package.id.clone(),
                                detail: "locked Git checksum is not SHA-256".into(),
                            })?;
                    let content = squish_fetch::ContentDigest(
                        squish_fetch::Sha256Digest::parse(content.to_owned())
                            .map_err(|error| unavailable(&package.id, error))?,
                    );
                    let mut candidate = self.git.materialize_locked_revision(
                        repository,
                        &commit,
                        ".",
                        &content,
                        squish_fetch::Access::LocalOnly,
                    );
                    if let Err(error) = &candidate {
                        self.observe_local_miss(&package.id, error);
                    }
                    if matches!(&candidate, Err(FetchError::OfflineMiss(_)))
                        && access == squish_fetch::Access::Online
                    {
                        candidate = self.git.materialize_locked_revision(
                            repository,
                            &commit,
                            ".",
                            &content,
                            squish_fetch::Access::Online,
                        );
                    }
                    let candidate = candidate.map_err(|error| unavailable(&package.id, error))?;
                    Some(
                        self.git
                            .materialize_locked(
                                repository,
                                &candidate.commit,
                                &candidate.package_tree,
                                ".",
                                &candidate.content_digest,
                                squish_fetch::Access::LocalOnly,
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

    fn materialize_registry(
        &self,
        entry: &RegistryEntry,
        package: &squish_project::LockedPackage,
        checksum: &str,
        access: squish_fetch::Access,
        materializer: &Materializer,
    ) -> Result<squish_fetch::MaterializedPackage, FetchError> {
        let candidates = entry.registry.candidates(&package.name, access)?;
        let candidate = candidates.into_iter().find(|candidate| {
            candidate.version == package.version
                && format!("sha256:{}", candidate.archive.digest.0.0) == checksum
        });
        let candidate = candidate.ok_or_else(|| {
            FetchError::OfflineMiss(format!(
                "locked registry candidate {} is absent from exact metadata",
                package.id
            ))
        })?;
        entry
            .registry
            .materialize(&package.name, &candidate, access, materializer)
    }

    fn observe_local_miss(&self, identity: &str, error: &FetchError) {
        self.context.observer.emit(SourceEvent::SourceUnavailable {
            identity: identity.into(),
            mode: "local-only".into(),
            reason: error.to_string(),
        });
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

fn retryable_registry_miss(error: &FetchError) -> bool {
    match error {
        FetchError::OfflineMiss(_)
        | FetchError::Metadata(_)
        | FetchError::Integrity(_)
        | FetchError::Json(_)
        | FetchError::Manifest(_) => true,
        FetchError::Io(error) => error.kind() == std::io::ErrorKind::NotFound,
        FetchError::Config(_)
        | FetchError::AuthenticationUnavailable(_)
        | FetchError::AuthenticationRejected(_)
        | FetchError::Credential(_)
        | FetchError::Path(_)
        | FetchError::Unsupported(_)
        | FetchError::Git(_)
        | FetchError::Http(_) => false,
    }
}

fn normalize_from_existing_ancestor(path: &Path) -> Result<PathBuf, std::io::Error> {
    let mut cursor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or(error)?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("storage path `{}` has no existing ancestor", path.display()),
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
}

fn validate_registry_routes(endpoints: &[RegistryEndpoint]) -> Result<(), HostError> {
    let mut aliases = BTreeMap::<&str, usize>::new();
    let mut identities = BTreeMap::<&str, usize>::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        if endpoint.name.trim().is_empty()
            || endpoint.config.id.trim().is_empty()
            || endpoint.config.auth_scope.is_empty()
        {
            return Err(HostError::Config(
                "registry names, stable identities, and auth scopes must be non-empty".into(),
            ));
        }
        if aliases.insert(&endpoint.name, index).is_some() {
            return Err(HostError::Config(format!(
                "registry alias `{}` is defined more than once",
                endpoint.name
            )));
        }
        if let Some(owner) = identities.insert(&endpoint.config.id, index) {
            let first = &endpoints[owner].config;
            if first.index != endpoint.config.index
                || first.auth_scope != endpoint.config.auth_scope
            {
                return Err(HostError::Config(format!(
                    "stable registry `{}` has conflicting endpoint or auth-scope routes",
                    endpoint.config.id
                )));
            }
        }
    }
    for (alias, owner) in aliases {
        if let Some(identity_owner) = identities.get(alias)
            && endpoints[owner].config.id != endpoints[*identity_owner].config.id
        {
            return Err(HostError::Config(format!(
                "registry route `{alias}` ambiguously names multiple endpoints"
            )));
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        let left = windows_path_key(left);
        let right = windows_path_key(right);
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

fn native_host_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

fn windows_path_key(path: &Path) -> String {
    let value = path.to_string_lossy();
    let normalized = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        value.into_owned()
    };
    normalized.replace('/', "\\").to_lowercase()
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
    use flate2::{Compression, write::GzEncoder};
    use sha2::{Digest as _, Sha256};
    use squish_build::{ActionIndex, ActionRecord, ProducedOutput};
    use squish_fetch::{GitInvocation, GitRunOutput, NoCredentials, NoopObserver};
    use squish_protocol::{ActionKeyId, ArtifactKind, Digest, DigestAlgorithm};
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

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

    struct RegistryHttp {
        calls: Arc<AtomicUsize>,
        bodies: Mutex<BTreeMap<String, Vec<u8>>>,
    }
    impl HttpTransport for RegistryHttp {
        fn execute(&self, request: HttpRequest) -> Result<HttpResponse, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body = self
                .bodies
                .lock()
                .unwrap()
                .get(request.url.as_str())
                .cloned()
                .ok_or_else(|| {
                    FetchError::Http(format!("unexpected fixture URL {}", request.url))
                })?;
            Ok(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body,
            })
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
    fn registry_routes_accept_aliases_and_stable_cross_registry_identities() {
        let (_temporary, mut host, _) = fixture();
        let make = |name: &str, identity: &str| RegistryEntry {
            name: name.into(),
            identity: identity.into(),
            registry: SparseRegistry::with_dependencies(
                host.context.clone(),
                RegistryConfig {
                    id: identity.into(),
                    auth_scope: identity.into(),
                    index: "sparse+https://index.example/".into(),
                },
                SharedCredentials(Arc::new(NoCredentials)),
                Box::new(NoHttp),
            )
            .unwrap(),
        };
        host.registries = vec![
            make("default", "https://registry.example/a"),
            make("secondary", "https://registry.example/b"),
        ];

        assert_eq!(host.registry("default").unwrap().name, "default");
        assert_eq!(
            host.registry("https://registry.example/a").unwrap().name,
            "default"
        );
        assert_eq!(
            host.registry("https://registry.example/b").unwrap().name,
            "secondary"
        );
        let collision = vec![
            RegistryEndpoint {
                name: "default".into(),
                config: RegistryConfig {
                    id: "https://registry.example/a".into(),
                    auth_scope: "a".into(),
                    index: "sparse+https://a.example/".into(),
                },
            },
            RegistryEndpoint {
                name: "https://registry.example/a".into(),
                config: RegistryConfig {
                    id: "https://registry.example/b".into(),
                    auth_scope: "b".into(),
                    index: "sparse+https://b.example/".into(),
                },
            },
        ];
        assert!(validate_registry_routes(&collision).is_err());
        let shared = vec![
            RegistryEndpoint {
                name: "corp".into(),
                config: RegistryConfig {
                    id: "https://registry.example/shared".into(),
                    auth_scope: "corp-read".into(),
                    index: "sparse+https://index.example/".into(),
                },
            },
            RegistryEndpoint {
                name: "corp-mirror-name".into(),
                config: RegistryConfig {
                    id: "https://registry.example/shared".into(),
                    auth_scope: "corp-read".into(),
                    index: "sparse+https://index.example/".into(),
                },
            },
        ];
        assert!(validate_registry_routes(&shared).is_ok());
    }

    #[test]
    fn transitive_cross_registry_stable_id_routes_to_the_intended_endpoint() {
        let (_temporary, mut host, _) = fixture();
        let a = "https://registry.example/a";
        let b = "https://registry.example/b";
        let row = |name: &str, dependency: &str| {
            format!(
                "{{\"v\":1,\"name\":\"{name}\",\"vers\":\"1.0.0\",\"package\":{{\"dialect\":\"xmlsquish/1\",\"source-root\":\"src\"}},\"deps\":{dependency},\"archive\":{{\"format\":\"xspkg-tar-gzip/1\",\"size\":1,\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"content-sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}},\"manifest-sha256\":\"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"yanked\":false}}"
            )
        };
        let http = Arc::new(RegistryHttp {
            calls: Arc::new(AtomicUsize::new(0)),
            bodies: Mutex::new(BTreeMap::from([
                ("https://a.example/config.json".into(), format!("{{\"v\":1,\"registry-id\":\"{a}\",\"dl\":\"https://download.example/{{package}}/{{version}}/{{archive-sha256}}\"}}").into_bytes()),
                ("https://a.example/ro/ot/root".into(), row("root", &format!("[{{\"alias\":\"dep\",\"package\":\"dep\",\"req\":\"^1\",\"registry-id\":\"{b}\",\"optional\":false,\"default-features\":true,\"features\":[]}}]" )).into_bytes()),
                ("https://b.example/config.json".into(), format!("{{\"v\":1,\"registry-id\":\"{b}\",\"dl\":\"https://download.example/{{package}}/{{version}}/{{archive-sha256}}\"}}").into_bytes()),
                ("https://b.example/3/d/dep".into(), row("dep", "[]").into_bytes()),
            ])),
        });
        let make = |name: &str, identity: &str, index: &str| RegistryEntry {
            name: name.into(),
            identity: identity.into(),
            registry: SparseRegistry::with_dependencies(
                host.context.clone(),
                RegistryConfig {
                    id: identity.into(),
                    auth_scope: identity.into(),
                    index: index.into(),
                },
                SharedCredentials(Arc::new(NoCredentials)),
                Box::new(SharedHttp(http.clone())),
            )
            .unwrap(),
        };
        host.registries = vec![
            make("default", a, "sparse+https://a.example/"),
            make("secondary", b, "sparse+https://b.example/"),
        ];
        let roots = RegistryPort::candidates(
            &RegistryView(&host),
            "default",
            "root",
            ResolverAccess::Online,
        )
        .unwrap();
        let detail = match &roots[0].manifest.dependencies["dep"] {
            squish_project::DependencySpec::Detail(detail) => detail,
            squish_project::DependencySpec::Version(_) => panic!("projection lost registry ID"),
        };
        assert_eq!(detail.registry.as_deref(), Some(b));
        assert_eq!(
            RegistryPort::candidates(&RegistryView(&host), b, "dep", ResolverAccess::Online,)
                .unwrap()[0]
                .manifest
                .package
                .as_ref()
                .unwrap()
                .name,
            "dep"
        );
    }

    #[test]
    fn nonexistent_storage_descendant_cannot_alias_source_cache() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-overlap-")
            .tempdir_in(std::fs::canonicalize(scratch).unwrap())
            .unwrap();
        let root = temporary.path().join("project");
        let shared = temporary.path().join("shared");
        std::fs::create_dir_all(&root).unwrap();
        let filesystem = Arc::new(squish_fetch::FilesystemHost::new(&root).unwrap());
        let storage = StorageLayout::new(
            shared.join("not-yet/cas"),
            temporary.path().join("actions.sqlite"),
            temporary.path().join("publish"),
            temporary.path().join("catalog"),
        )
        .unwrap();
        let result = ProductionHost::open(HostConfig {
            project_root: root,
            source_cache_root: shared,
            storage,
            registries: Vec::new(),
            credentials: Arc::new(NoCredentials),
            http: Arc::new(NoHttp),
            git: GitExecution::Runner(Arc::new(NoGit)),
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
            filesystem,
        });
        assert!(matches!(result, Err(HostError::Config(_))));
    }

    #[test]
    fn locked_registry_materialization_uses_verified_local_content_before_http() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-registry-")
            .tempdir_in(std::fs::canonicalize(scratch).unwrap())
            .unwrap();
        let root = temporary.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let manifest = b"manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\ndialect = \"xmlsquish/1\"\nsource-root = \"src\"\n";
        let tree = squish_fetch::LogicalTree::build(
            vec![squish_fetch::LogicalFile {
                path: "xmlsquish.toml".into(),
                bytes: manifest.to_vec(),
            }],
            &Limits::default(),
        )
        .unwrap();
        let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "demo-1.0.0/xmlsquish.toml", &manifest[..])
            .unwrap();
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let archive_digest = hex::encode(Sha256::digest(&bytes));
        let manifest_digest = hex::encode(Sha256::digest(manifest));
        let identity = "https://registry.example/v1";
        let download = format!("https://download.example/demo/1.0.0/{archive_digest}.xspkg");
        let row = format!(
            "{{\"v\":1,\"name\":\"demo\",\"vers\":\"1.0.0\",\"package\":{{\"dialect\":\"xmlsquish/1\",\"source-root\":\"src\"}},\"deps\":[],\"archive\":{{\"format\":\"xspkg-tar-gzip/1\",\"size\":{},\"sha256\":\"{}\",\"content-sha256\":\"{}\"}},\"manifest-sha256\":\"{}\",\"yanked\":false}}",
            bytes.len(),
            archive_digest,
            tree.content_digest.0.0,
            manifest_digest
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let http = Arc::new(RegistryHttp {
            calls: calls.clone(),
            bodies: Mutex::new(BTreeMap::from([
                ("https://index.example/config.json".into(), format!("{{\"v\":1,\"registry-id\":\"{identity}\",\"dl\":\"https://download.example/{{package}}/{{version}}/{{archive-sha256}}.xspkg\"}}").into_bytes()),
                ("https://index.example/de/mo/demo".into(), row.into_bytes()),
                (download, bytes),
            ])),
        });
        let storage = StorageLayout::new(
            temporary.path().join("cas"),
            temporary.path().join("actions.sqlite"),
            temporary.path().join("publish"),
            temporary.path().join("catalog"),
        )
        .unwrap();
        let host = ProductionHost::open(HostConfig {
            project_root: root.clone(),
            source_cache_root: temporary.path().join("sources"),
            storage,
            registries: vec![RegistryEndpoint {
                name: "default".into(),
                config: RegistryConfig {
                    id: identity.into(),
                    auth_scope: identity.into(),
                    index: "sparse+https://index.example/".into(),
                },
            }],
            credentials: Arc::new(NoCredentials),
            http,
            git: GitExecution::Runner(Arc::new(NoGit)),
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
            filesystem: Arc::new(squish_fetch::FilesystemHost::new(root).unwrap()),
        })
        .unwrap();
        let lock = Lockfile {
            lock_version: squish_project::LOCK_VERSION,
            resolver_version: "fixture/1".into(),
            manifest_digest:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: vec![squish_project::LockedPackage {
                id: "demo@1.0.0#registry".into(),
                name: "demo".into(),
                version: "1.0.0".parse().unwrap(),
                source: LockedSource::Registry {
                    registry: identity.into(),
                    checksum: format!("sha256:{archive_digest}"),
                },
                manifest_digest: format!("sha256:{manifest_digest}"),
                dependencies: BTreeMap::new(),
            }],
        };
        assert_eq!(
            Services::materialize_locked(&host, host.project_root(), &lock, ResolutionMode::Online)
                .unwrap()
                .len(),
            1
        );
        let after_online = calls.load(Ordering::SeqCst);
        assert!(after_online > 0);
        for mode in [ResolutionMode::Locked, ResolutionMode::Frozen] {
            assert_eq!(
                Services::materialize_locked(&host, host.project_root(), &lock, mode)
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(calls.load(Ordering::SeqCst), after_online);
        }
    }

    #[test]
    fn frozen_git_materialization_needs_no_revision_selector_observation() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(scratch).unwrap();
        #[cfg(windows)]
        let scratch = PathBuf::from(scratch.to_string_lossy().trim_start_matches(r"\\?\"));
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-git-")
            .tempdir_in(scratch)
            .unwrap();
        let repository = temporary.path().join("repository");
        std::fs::create_dir_all(repository.join("src")).unwrap();
        std::fs::write(
            repository.join("xmlsquish.toml"),
            "manifest-version = 1\n[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(repository.join("src/main.xml"), "<prompt />").unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["config", "user.name", "Fixture"],
            vec!["add", "."],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&repository)
                .status()
                .unwrap();
            assert!(status.success());
        }
        let canonical_repository = std::fs::canonicalize(&repository).unwrap();
        let raw = canonical_repository.to_string_lossy().replace('\\', "/");
        let spelling = raw.strip_prefix("//?/").unwrap_or(&raw);
        let repository_url = if spelling.starts_with('/') {
            format!("file://{spelling}")
        } else {
            format!("file:///{spelling}")
        };
        let discovery = GitHost::with_runner(
            HostContext::new(temporary.path().join("discovery-cache")).unwrap(),
            Arc::new(SystemGitRunner::new("git")),
        );
        let candidate = discovery
            .resolve_exact(
                &repository_url,
                &squish_fetch::GitSelector::Head,
                ".",
                squish_fetch::Access::Online,
            )
            .unwrap();

        let project = temporary.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let storage = StorageLayout::new(
            temporary.path().join("cas"),
            temporary.path().join("actions.sqlite"),
            temporary.path().join("publish"),
            temporary.path().join("catalog"),
        )
        .unwrap();
        let host = ProductionHost::open(HostConfig {
            project_root: project.clone(),
            source_cache_root: temporary.path().join("exact-cache"),
            storage,
            registries: Vec::new(),
            credentials: Arc::new(NoCredentials),
            http: Arc::new(NoHttp),
            git: GitExecution::Runner(Arc::new(SystemGitRunner::new("git"))),
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
            filesystem: Arc::new(squish_fetch::FilesystemHost::new(project).unwrap()),
        })
        .unwrap();
        let lock = Lockfile {
            lock_version: squish_project::LOCK_VERSION,
            resolver_version: "fixture/1".into(),
            manifest_digest:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: vec![squish_project::LockedPackage {
                id: "demo@1.0.0#git".into(),
                name: "demo".into(),
                version: "1.0.0".parse().unwrap(),
                source: LockedSource::Git {
                    repository: repository_url,
                    revision: candidate.commit.hex,
                    checksum: format!("sha256:{}", candidate.content_digest.0.0),
                },
                manifest_digest: format!("sha256:{}", candidate.manifest_digest.0.0),
                dependencies: BTreeMap::new(),
            }],
        };
        assert_eq!(
            Services::materialize_locked(&host, host.project_root(), &lock, ResolutionMode::Online)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            Services::materialize_locked(&host, host.project_root(), &lock, ResolutionMode::Frozen)
                .unwrap()
                .len(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_and_drive_spelling_are_the_same_storage_identity() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(scratch).unwrap();
        let ordinary = PathBuf::from(scratch.to_string_lossy().trim_start_matches(r"\\?\"));
        let verbatim = PathBuf::from(format!(r"\\?\{}", ordinary.display()));
        assert_eq!(
            normalize_from_existing_ancestor(&ordinary).unwrap(),
            normalize_from_existing_ancestor(&verbatim).unwrap()
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

    #[test]
    fn environment_credentials_scope_primary_and_cross_origin_values() {
        let cross_origin = "https://packages.example.com";
        let digest = hex::encode_upper(Sha256::digest(cross_origin.as_bytes()));
        let credentials = EnvironmentCredentials::from_snapshot(
            [CredentialRoute {
                registry_id: "https://registry.example/v1".into(),
                auth_scope: "corp-read".into(),
                primary_origin: "https://index.example.com".into(),
            }],
            [
                (
                    OsString::from("xmlsquish_registry_corp_read_authorization"),
                    OsString::from("Bearer primary-secret"),
                ),
                (
                    OsString::from(format!(
                        "XMLSQUISH_REGISTRY_CORP_READ_ORIGIN_{digest}_AUTHORIZATION"
                    )),
                    OsString::from("Basic cross-secret"),
                ),
                (OsString::from("IGNORED"), OsString::from("not retained")),
            ],
        )
        .unwrap();
        let primary = credentials
            .authorization(
                "https://registry.example/v1",
                "corp-read",
                "https://index.example.com",
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            primary,
            AuthorizationValue::new("Bearer primary-secret".into()).unwrap()
        );
        let cross = credentials
            .authorization("https://registry.example/v1", "corp-read", cross_origin)
            .unwrap()
            .unwrap();
        assert_eq!(
            cross,
            AuthorizationValue::new("Basic cross-secret".into()).unwrap()
        );
        assert!(
            credentials
                .authorization(
                    "https://registry.example/v1",
                    "corp-read",
                    "https://other.example.com",
                )
                .unwrap()
                .is_none()
        );
        let rendered = format!("{credentials:?} {primary:?} {cross}");
        assert!(!rendered.contains("primary-secret"));
        assert!(!rendered.contains("cross-secret"));
    }

    #[test]
    fn environment_credentials_reject_portability_and_value_errors_without_leaking() {
        let route = CredentialRoute {
            registry_id: "https://registry.example/v1".into(),
            auth_scope: "corp-read".into(),
            primary_origin: "https://index.example.com".into(),
        };
        let duplicate = EnvironmentCredentials::from_snapshot(
            [route.clone()],
            [
                (
                    OsString::from("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION"),
                    OsString::from("Bearer first-secret"),
                ),
                (
                    OsString::from("xmlsquish_registry_corp_read_authorization"),
                    OsString::from("Bearer second-secret"),
                ),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(duplicate.contains("collide"));
        assert!(!duplicate.contains("first-secret"));
        assert!(!duplicate.contains("second-secret"));

        let invalid = EnvironmentCredentials::from_snapshot(
            [route],
            [(
                OsString::from("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION"),
                OsString::from("Bearer secret\nInjected: value"),
            )],
        )
        .unwrap_err()
        .to_string();
        assert!(invalid.contains("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION"));
        assert!(!invalid.contains("secret"));
    }

    #[test]
    fn environment_credentials_reject_scope_stem_collision_but_allow_exact_reuse() {
        let route = |id: &str, scope: &str| CredentialRoute {
            registry_id: id.into(),
            auth_scope: scope.into(),
            primary_origin: "https://index.example.com".into(),
        };
        assert!(
            EnvironmentCredentials::from_snapshot(
                [
                    route("https://registry.example/a", "corp-read"),
                    route("https://registry.example/b", "CORP_READ"),
                ],
                [],
            )
            .is_err()
        );
        assert!(
            EnvironmentCredentials::from_snapshot(
                [
                    route("https://registry.example/a", "corp-read"),
                    route("https://registry.example/b", "corp-read"),
                ],
                [],
            )
            .is_ok()
        );
    }

    #[test]
    fn credential_origins_canonicalize_default_ports_and_ipv6() {
        assert_eq!(
            canonical_https_origin("https://example.com:443"),
            Some("https://example.com".into())
        );
        assert_eq!(
            canonical_https_origin("https://[2001:db8::1]:8443"),
            Some("https://[2001:db8::1]:8443".into())
        );
        assert!(canonical_https_origin("https://example.com/path").is_none());
    }
}
