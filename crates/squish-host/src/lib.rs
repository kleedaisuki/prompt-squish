//! 显式的生产组合根。 / Explicit production composition root.
//!
//! 本 crate 只组合各领域 crate 的公开端口；它不复制缓存、发布或 build-record 的私有
//! 格式。 / This crate only composes public domain ports; it does not copy private cache,
//! publication, or build-record formats.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod runtime;

pub use runtime::ProductionBuildRuntime;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squish_config::AuthScope;
use url::Url;

use squish_fetch::{
    AuthorizationValue, CredentialError, CredentialLookup, CredentialPort, FetchError, GitHost,
    GitInvocation, GitRunner, HostContext, HttpRequest, HttpResponse, HttpTransport, Limits,
    Materializer, Observer, RegistryConfig, SourceEvent, SparseRegistry, SystemGitRunner,
};
use squish_manager::{
    ArtifactLocator, BuildRuntime, ProjectBuildLayout, ProjectCleanStatus, ProjectCreationLocation,
    ProjectCreationStatus, ProvenanceNonApplicability, ProvenanceRelation, ResolveRequest,
    ResolvedDependencies, ServiceError, Services,
};
use squish_project::{
    DependencyResolver, LockedSource, Lockfile, Manifest, ResolutionInput, ResolutionMode,
};
use squish_protocol::{CleanResult, VcsChoice};
use squish_publish::{NoopObserver as NoopPublishObserver, PublishObserver};
use squish_repository::{
    CreateProjectRequest, FaultInjector, PackageLocation, ProjectVcs, RepositoryError,
    StagePreparer, WorkspaceMembership, create_project,
};
use squish_resolver::{
    Access as ResolverAccess, FilesystemPort, GitCandidate, GitPort, LocalPackage, LocalRequest,
    RegistryCandidate, RegistryPort, Resolver, SourceUnavailable,
};
#[cfg(test)]
use squish_store::{BlobDigest, Cas, VerifiedActionIndex};

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
    ) -> Result<CredentialLookup, CredentialError> {
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
        CredentialLookup::new(self.values.get(&name).cloned(), Some(name))
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

/// 尚未存在的项目所使用的最小生产宿主。 / Minimal production host for a project that does not yet exist.
///
/// 该宿主只执行只读落位和候选目录内的 Git 初始化；它不会规范化未来项目根、创建
/// cache/CAS，或假装未来项目已经可发现。 / This host performs only read-only placement and
/// Git initialization inside the staged candidate; it neither canonicalizes the future project
/// root, opens caches/CAS, nor pretends the future project is discoverable.
pub struct ProjectCreationHost {
    git: Arc<dyn GitRunner>,
}

impl ProjectCreationHost {
    /// 由显式 Git 执行端口构造。 / Constructs the host from an explicit Git execution port.
    pub fn new(git: GitExecution) -> Result<Self, HostError> {
        Ok(Self {
            git: git_runner(git)?,
        })
    }
}

impl StagePreparer for ProjectCreationHost {
    fn prepare(&self, candidate_root: &Path, vcs: ProjectVcs) -> std::io::Result<()> {
        if vcs != ProjectVcs::InitializeGit {
            return Ok(());
        }
        let output = self
            .git
            .execute(GitInvocation {
                cwd: Some(candidate_root.to_path_buf()),
                args: ["init", "--quiet"]
                    .into_iter()
                    .map(OsString::from)
                    .collect(),
                env: BTreeMap::new(),
            })
            .map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("could not run git init: {error}; retry with --vcs=none"),
                )
            })?;
        if output.success {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::other(format!(
            "git init failed{}{}; retry with --vcs=none",
            output
                .code
                .map_or_else(String::new, |code| format!(" ({code})")),
            if detail.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", detail.trim())
            }
        )))
    }
}

struct CancellablePreparation<'a, P> {
    inner: &'a P,
    cancellation: squish_kernel::CancellationToken,
    observed: &'a AtomicBool,
}

impl<P: StagePreparer> StagePreparer for CancellablePreparation<'_, P> {
    fn prepare(&self, candidate_root: &Path, vcs: ProjectVcs) -> std::io::Result<()> {
        if self.cancellation.is_cancelled() {
            self.observed.store(true, Ordering::Release);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "project creation cancelled before Git preparation",
            ));
        }
        self.inner.prepare(candidate_root, vcs)?;
        if self.cancellation.is_cancelled() {
            self.observed.store(true, Ordering::Release);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "project creation cancelled after Git preparation",
            ));
        }
        Ok(())
    }

    fn cancellation_requested(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

fn git_runner(git: GitExecution) -> Result<Arc<dyn GitRunner>, HostError> {
    match git {
        GitExecution::Executable(path) => {
            if !path.is_absolute() {
                return Err(HostError::Config(
                    "Git executable must be an absolute path".into(),
                ));
            }
            Ok(Arc::new(SystemGitRunner::new(path)))
        }
        GitExecution::Runner(runner) => Ok(runner),
    }
}

/// 生产宿主的全部外部依赖。 / Complete external dependencies of the production host.
pub struct HostConfig {
    /// 唯一允许的项目根。 / Sole permitted project root.
    pub project_root: PathBuf,
    /// 从规范项目根和 manifest target 目录派生的唯一项目构建布局。 /
    /// Sole project-build layout derived from the canonical project root and manifest target.
    pub storage: ProjectBuildLayout,
    /// 此次调用的协作式取消令牌。 / Cooperative cancellation token for this invocation.
    pub cancellation: squish_kernel::CancellationToken,
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
            .field("ownership_root", &self.storage.ownership_root())
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
    ) -> Result<CredentialLookup, CredentialError> {
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

const CLEAN_JOURNAL_VERSION: u32 = 2;
#[cfg(windows)]
const CLEAN_RENAME_ATTEMPTS: usize = 8;

/// 外置 clean redo journal；只保存同父目录的受控叶名。 /
/// External clean redo journal containing only controlled sibling leaf names.
#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CleanJournal {
    version: u32,
    root_leaf: PathBuf,
    trash_leaf: PathBuf,
}

/// 从唯一项目构建根派生的根外协调路径。 /
/// Out-of-root coordination paths derived from the sole project build root.
struct CleanPaths {
    storage: ProjectBuildLayout,
    root: PathBuf,
    lock: PathBuf,
    journal: PathBuf,
}

impl CleanPaths {
    /// 构造 ADR 规定的同级 lock/journal 路径。 /
    /// Builds the sibling lock/journal paths required by the project-layout ADR.
    fn new(storage: &ProjectBuildLayout) -> io::Result<Self> {
        let root = storage.ownership_root().to_path_buf();
        root.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "project build root has no parent",
            )
        })?;
        root.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "project build root has no leaf",
            )
        })?;
        let paths = Self {
            storage: storage.clone(),
            root,
            lock: storage.coordination_lock().to_path_buf(),
            journal: storage.clean_journal().to_path_buf(),
        };
        paths.validate_root_identity()?;
        Ok(paths)
    }

    /// 确保构造后引入的文件系统别名不能改变所有权根身份。 /
    /// Ensures a filesystem alias introduced after construction cannot change root identity.
    fn validate_root_identity(&self) -> io::Result<()> {
        self.storage
            .validate_existing_aliases()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }
}

fn next_trash_leaf(paths: &CleanPaths) -> io::Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let leaf = paths
        .root
        .file_name()
        .expect("validated build root leaf")
        .to_string_lossy();
    for attempt in 0_u32..1024 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let candidate = PathBuf::from(format!(
            ".{leaf}.xmlsquish-trash-{}-{nonce:x}-{attempt:x}",
            std::process::id()
        ));
        if !paths
            .root
            .parent()
            .expect("validated build root parent")
            .join(&candidate)
            .exists()
        {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate clean trash name",
    ))
}

/// 获取与 target/catalog publisher 完全相同的跨进程项目锁。 /
/// Acquires exactly the same cross-process project lock as target/catalog publishers.
#[cfg(test)]
fn open_project_lock(paths: &CleanPaths) -> io::Result<File> {
    let lock = open_project_lock_file(paths)?;
    lock.lock_exclusive()?;
    Ok(lock)
}

/// 等待共享项目锁时持续响应取消；返回 `None` 表示尚未删除任何状态。 /
/// Remains cancellation-responsive while waiting for the shared project lock; `None` means no
/// state was removed.
fn open_project_lock_cancellable(
    paths: &CleanPaths,
    cancellation: &squish_kernel::CancellationToken,
) -> io::Result<Option<File>> {
    let lock = open_project_lock_file(paths)?;
    loop {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        match lock.try_lock_exclusive() {
            Ok(()) => return Ok(Some(lock)),
            Err(error) if lock_is_busy(&error) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}

fn open_project_lock_file(paths: &CleanPaths) -> io::Result<File> {
    let parent = paths
        .lock
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "project lock has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock)?;
    Ok(lock)
}

fn lock_is_busy(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(33)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn expected_clean_journal(paths: &CleanPaths, trash_leaf: PathBuf) -> CleanJournal {
    CleanJournal {
        version: CLEAN_JOURNAL_VERSION,
        root_leaf: PathBuf::from(paths.root.file_name().expect("validated build root leaf")),
        trash_leaf,
    }
}

/// 原子持久化 redo journal，确保首次 detach 之前已有恢复依据。 /
/// Atomically persists the redo journal before the first detach has recovery evidence.
fn persist_clean_journal(paths: &CleanPaths, trash_leaf: PathBuf) -> io::Result<()> {
    persist_clean_journal_value(&expected_clean_journal(paths, trash_leaf), paths)
}

/// 原子写入已冻结的 journal 值。 / Atomically writes a frozen journal value.
fn persist_clean_journal_value(journal: &CleanJournal, paths: &CleanPaths) -> io::Result<()> {
    let bytes = serde_json::to_vec(journal)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let parent = paths.journal.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "clean journal has no parent")
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write as _;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&paths.journal)
        .map_err(|error| error.error)?;
    sync_parent(&paths.journal)
}

/// 校验 journal 中的两个同级叶名。 / Validates the two sibling leaf names in a journal.
fn validate_clean_journal(paths: &CleanPaths) -> io::Result<CleanJournal> {
    let journal = &paths.journal;
    let metadata = std::fs::symlink_metadata(journal)?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "clean journal `{}` is not a regular file",
                journal.display()
            ),
        ));
    }
    let actual: CleanJournal = serde_json::from_slice(&std::fs::read(journal)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let expected_root = Path::new(paths.root.file_name().expect("validated build root leaf"));
    let trash = actual.trash_leaf.to_string_lossy();
    let expected_prefix = format!(".{}.xmlsquish-trash-", expected_root.to_string_lossy());
    if actual.version != CLEAN_JOURNAL_VERSION
        || actual.root_leaf != expected_root
        || actual.trash_leaf.components().count() != 1
        || !trash.starts_with(&expected_prefix)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "clean journal `{}` does not match this storage layout",
                journal.display()
            ),
        ));
    }
    Ok(actual)
}

fn journal_exists(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// 幂等 roll-forward 一个已提交的 clean journal。 /
/// Idempotently rolls a committed clean journal forward.
fn recover_clean_journal(
    paths: &CleanPaths,
    result: &mut CleanResult,
    changed: &mut bool,
) -> io::Result<()> {
    if !journal_exists(&paths.journal)? {
        return Ok(());
    }
    let journal = validate_clean_journal(paths)?;
    *changed = true;
    let trash = paths
        .root
        .parent()
        .expect("validated build root parent")
        .join(journal.trash_leaf);
    if journal_exists(&paths.root)? && journal_exists(&trash)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "clean recovery is ambiguous: both `{}` and `{}` exist",
                paths.root.display(),
                trash.display()
            ),
        ));
    }
    clean_root(&paths.root, &trash, result, changed)?;
    std::fs::remove_file(&paths.journal)?;
    sync_parent(&paths.journal)
}

/// 先独占 detach 根，再以不跟随链接的方式删除 tombstone。 /
/// Exclusively detaches a root, then deletes its tombstone without following links.
fn clean_root(
    root: &Path,
    tombstone: &Path,
    result: &mut CleanResult,
    changed: &mut bool,
) -> io::Result<()> {
    if journal_exists(tombstone)? {
        *changed = true;
        remove_tree_no_follow(tombstone, result)?;
        sync_parent(tombstone)?;
    }
    if !journal_exists(root)? {
        return Ok(());
    }
    rename_with_retry(root, tombstone)?;
    *changed = true;
    sync_parent(root)?;
    remove_tree_no_follow(tombstone, result)?;
    sync_parent(tombstone)
}

/// 递归删除并只统计成功删除的普通文件；链接与 reparse point 只删除目录项。 /
/// Recursively removes entries and counts only successfully removed regular files; links and
/// reparse points are removed as entries without traversal.
fn remove_tree_no_follow(path: &Path, result: &mut CleanResult) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata_is_link_or_reparse(&metadata) {
        return remove_link_or_reparse(path, &metadata);
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            remove_tree_no_follow(&entry?.path(), result)?;
        }
        return std::fs::remove_dir(path);
    }
    std::fs::remove_file(path)?;
    if metadata.is_file() {
        result.build_files = result.build_files.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "clean file count overflow")
        })?;
        result.build_bytes = result
            .build_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "clean byte count overflow")
            })?;
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn remove_link_or_reparse(path: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        if metadata.file_attributes() & 0x10 != 0 {
            return std::fs::remove_dir(path);
        }
    }
    let _ = metadata;
    std::fs::remove_file(path)
}

/// 以 no-clobber 语义 rename，并对 Windows 暂时共享冲突作有界重试。 /
/// Renames with no-clobber semantics and bounded retries for transient Windows sharing failures.
#[cfg(not(windows))]
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    squish_platform_fs::rename_exclusive(from, to)
}

#[cfg(windows)]
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempts = 0;
    loop {
        match squish_platform_fs::rename_exclusive(from, to) {
            Ok(()) => return Ok(()),
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    && attempts + 1 < CLEAN_RENAME_ATTEMPTS =>
            {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "durable path has no parent")
    })?)?
    .sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// 不读取进程全局状态的 production services 组合。 / Production services composition which reads no process-global state.
pub struct ProductionHost {
    project_root: PathBuf,
    storage: ProjectBuildLayout,
    context: HostContext,
    registries: Vec<RegistryEntry>,
    git: GitHost,
    git_runner: Arc<dyn GitRunner>,
    filesystem: Arc<dyn FilesystemPort + Send + Sync>,
    cancellation: squish_kernel::CancellationToken,
    build_runtime: Arc<ProductionBuildRuntime>,
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
    /// use squish_manager::ProjectBuildLayout;
    ///
    /// # fn compose(project: PathBuf, storage: ProjectBuildLayout)
    /// # -> Result<ProductionHost, Box<dyn std::error::Error>> {
    /// let filesystem = Arc::new(FilesystemHost::new(&project)?);
    /// let host = ProductionHost::open(HostConfig {
    ///     project_root: project,
    ///     storage,
    ///     cancellation: Default::default(),
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
        let context = HostContext {
            cache: native_host_path(config.storage.source_cache_root().to_path_buf()),
            limits: config.limits,
            observer: config.observer,
        };
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
        let physical_root = normalize_from_existing_ancestor(config.storage.ownership_root())?;
        if physical_root == project_root || !physical_root.starts_with(&project_root) {
            return Err(HostError::Config(
                "project build root must remain strictly below the canonical project root".into(),
            ));
        }
        let git_runner = git_runner(config.git)?;
        let git = GitHost::with_runner(context.clone(), git_runner.clone());
        let build_runtime = Arc::new(ProductionBuildRuntime::new(
            config.storage.clone(),
            config.cancellation.clone(),
            Arc::new(NoopPublishObserver),
            Arc::new(NoopPublishObserver),
        ));
        Ok(Self {
            project_root,
            storage: config.storage,
            context,
            registries,
            git,
            git_runner,
            filesystem: config.filesystem,
            cancellation: config.cancellation,
            build_runtime,
        })
    }

    /// 注入彼此隔离的目标产物与 build-catalog 持久化观察者。 /
    /// Installs isolated durability observers for target artifacts and the build catalog.
    ///
    /// 观察者必须只观察已完成的发布边界；运行时不会借此发射 kernel 事件。 / Observers
    /// must observe only completed publication boundaries; the runtime never uses them to emit
    /// kernel events.
    #[must_use]
    pub fn with_build_observers(
        mut self,
        target: Arc<dyn PublishObserver>,
        catalog: Arc<dyn PublishObserver>,
    ) -> Self {
        self.build_runtime = Arc::new(ProductionBuildRuntime::new(
            self.storage.clone(),
            self.cancellation.clone(),
            target,
            catalog,
        ));
        self
    }

    /// 返回规范化项目根。 / Returns the canonical project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 返回项目内来源获取缓存根。 / Returns the project-local source-acquisition cache root.
    pub fn source_cache_root(&self) -> &Path {
        self.storage.source_cache_root()
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
        self.build_runtime
            .acquire_maintenance()
            .map_err(|error| SourceUnavailable {
                identity: self.project_root.display().to_string(),
                detail: error.to_string(),
            })?;
        std::fs::create_dir_all(self.context.cache.join("v1/stage")).map_err(|error| {
            SourceUnavailable {
                identity: self.project_root.display().to_string(),
                detail: error.to_string(),
            }
        })?;
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
                    let locked = candidate.map_err(|error| unavailable(&package.id, error))?;
                    if locked.candidate.commit != commit
                        || locked.candidate.content_digest != content
                        || locked.materialized.content_digest != content
                    {
                        return Err(SourceUnavailable {
                            identity: package.id.clone(),
                            detail:
                                "locked Git materialization returned a different exact identity"
                                    .into(),
                        });
                    }
                    Some(locked.materialized)
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
        self.build_runtime
            .acquire_maintenance()
            .map_err(|error| SourceUnavailable {
                identity: self.project_root.display().to_string(),
                detail: error.to_string(),
            })?;
        std::fs::create_dir_all(self.context.cache.join("v1/stage")).map_err(|error| {
            SourceUnavailable {
                identity: self.project_root.display().to_string(),
                detail: error.to_string(),
            }
        })?;
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

#[derive(Deserialize, Serialize)]
struct LayoutMarker {
    schema: u32,
    format_epoch: u32,
}

const LAYOUT_SCHEMA: u32 = 1;
const LAYOUT_FORMAT_EPOCH: u32 = 1;

/// 恢复 clean，校验/初始化布局标记，再以重校验循环安全交接给共享租约。 /
/// Recovers clean, validates/initializes the marker, then safely hands off to a shared lease with
/// a revalidation loop covering the portable unlock/relock gap.
fn acquire_build_lease(
    storage: &ProjectBuildLayout,
    cancellation: &squish_kernel::CancellationToken,
) -> io::Result<File> {
    check_cancellation(cancellation)?;
    let paths = CleanPaths::new(storage)?;
    let lock = open_project_lock_file(&paths)?;
    loop {
        lock_cancellable(&lock, LockMode::Shared, cancellation)?;
        paths.validate_root_identity()?;
        if !journal_exists(&paths.journal)? && layout_marker_is_current(storage)? {
            return Ok(lock);
        }
        FileExt::unlock(&lock)?;

        lock_cancellable(&lock, LockMode::Exclusive, cancellation)?;
        paths.validate_root_identity()?;
        let mut result = CleanResult::default();
        let mut changed = false;
        recover_clean_journal(&paths, &mut result, &mut changed)?;
        ensure_layout_marker(storage, &paths, &mut result)?;
        FileExt::unlock(&lock)?;
    }
}

#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

/// 轮询获取维护锁，使等待不会掩盖调用取消。 / Polls for a maintenance lock so waiting never
/// masks invocation cancellation.
fn lock_cancellable(
    lock: &File,
    mode: LockMode,
    cancellation: &squish_kernel::CancellationToken,
) -> io::Result<()> {
    loop {
        check_cancellation(cancellation)?;
        let acquired = match mode {
            LockMode::Shared => FileExt::try_lock_shared(lock),
            LockMode::Exclusive => FileExt::try_lock_exclusive(lock),
        };
        match acquired {
            Ok(()) => {
                if let Err(error) = check_cancellation(cancellation) {
                    FileExt::unlock(lock)?;
                    return Err(error);
                }
                return Ok(());
            }
            Err(error) if lock_is_busy(&error) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
}

fn check_cancellation(cancellation: &squish_kernel::CancellationToken) -> io::Result<()> {
    if cancellation.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "build maintenance acquisition cancelled",
        ))
    } else {
        Ok(())
    }
}

fn layout_marker_is_current(storage: &ProjectBuildLayout) -> io::Result<bool> {
    match std::fs::read(storage.layout_marker()) {
        Ok(bytes) => Ok(
            serde_json::from_slice::<LayoutMarker>(&bytes).is_ok_and(|value| {
                value.schema == LAYOUT_SCHEMA && value.format_epoch == LAYOUT_FORMAT_EPOCH
            }),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_layout_marker(
    storage: &ProjectBuildLayout,
    paths: &CleanPaths,
    result: &mut CleanResult,
) -> io::Result<()> {
    let marker = storage.layout_marker();
    let current = LayoutMarker {
        schema: LAYOUT_SCHEMA,
        format_epoch: LAYOUT_FORMAT_EPOCH,
    };
    let valid = layout_marker_is_current(storage)?;
    if valid {
        return Ok(());
    }

    let root_nonempty = match std::fs::read_dir(storage.ownership_root()) {
        Ok(mut entries) => entries.next().transpose()?.is_some(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if root_nonempty {
        let trash_leaf = next_trash_leaf(paths)?;
        persist_clean_journal(paths, trash_leaf)?;
        let mut changed = false;
        recover_clean_journal(paths, result, &mut changed)?;
    }

    std::fs::create_dir_all(storage.metadata_root())?;
    let bytes = serde_json::to_vec(&current)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut temporary = tempfile::NamedTempFile::new_in(storage.metadata_root())?;
    use std::io::Write as _;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(marker).map_err(|error| error.error)?;
    sync_parent(marker)
}

impl StagePreparer for ProductionHost {
    fn prepare(&self, candidate_root: &Path, vcs: ProjectVcs) -> std::io::Result<()> {
        ProjectCreationHost {
            git: self.git_runner.clone(),
        }
        .prepare(candidate_root, vcs)
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

impl Services for ProjectCreationHost {
    fn locate_project_creation(
        &self,
        destination: &Path,
        vcs: VcsChoice,
    ) -> Result<ProjectCreationLocation, ServiceError> {
        let destination = normalize_prospective_destination(destination)
            .map_err(|error| external_service_error("project_destination_unavailable", error))?;
        let workspace = locate_enclosing_workspace(&destination)?;
        let vcs = match vcs {
            VcsChoice::None => ProjectVcs::None,
            VcsChoice::Git => self.git_placement(&destination)?,
        };
        Ok(ProjectCreationLocation {
            destination,
            vcs,
            workspace,
        })
    }

    fn create_project(
        &self,
        request: &CreateProjectRequest,
        cancellation: squish_kernel::CancellationToken,
        faults: Arc<dyn FaultInjector>,
    ) -> Result<ProjectCreationStatus, ServiceError> {
        publish_project(request, self, cancellation, faults)
    }

    fn storage_layout(&self, _project_root: &Path) -> Result<ProjectBuildLayout, ServiceError> {
        Err(creation_only_service("storage layout"))
    }

    fn materialize_locked(
        &self,
        _project: &Path,
        _lock: &Lockfile,
        _mode: ResolutionMode,
    ) -> Result<Vec<PackageLocation>, ServiceError> {
        Err(creation_only_service("dependency materialization"))
    }

    fn resolve(&self, _request: ResolveRequest<'_>) -> Result<ResolvedDependencies, ServiceError> {
        Err(creation_only_service("dependency resolution"))
    }
}

impl ProjectCreationHost {
    fn git_placement(&self, destination: &Path) -> Result<ProjectVcs, ServiceError> {
        let cwd = nearest_existing_directory(destination)
            .map_err(|error| external_service_error("project_destination_unavailable", error))?;
        let output = self
            .git
            .execute(GitInvocation {
                cwd: Some(cwd),
                args: ["rev-parse", "--is-inside-work-tree"]
                    .into_iter()
                    .map(OsString::from)
                    .collect(),
                env: BTreeMap::from([
                    (OsString::from("LC_ALL"), OsString::from("C")),
                    (OsString::from("LANG"), OsString::from("C")),
                ]),
            })
            .map_err(|error| {
                ServiceError::new(
                    "git_discovery_failed",
                    format!("could not run Git worktree discovery: {error}; retry with --vcs=none"),
                )
            })?;
        if !output.success {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not a git repository") {
                return Ok(ProjectVcs::InitializeGit);
            }
            return Err(ServiceError::new(
                "git_discovery_failed",
                format!(
                    "could not inspect the enclosing Git worktree; retry with --vcs=none{}",
                    if stderr.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", stderr.trim())
                    }
                ),
            ));
        }
        let state = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
        let state = state.strip_suffix(b"\r").unwrap_or(state);
        if state == b"false" {
            return Ok(ProjectVcs::InitializeGit);
        }
        if state != b"true" {
            return Err(ServiceError::new(
                "git_discovery_failed",
                "Git returned an invalid worktree status; retry with --vcs=none",
            ));
        }
        Ok(ProjectVcs::InheritedGit)
    }
}

fn creation_only_service(capability: &str) -> ServiceError {
    ServiceError::new(
        "creation_host_scope",
        format!("{capability} is unavailable while creating a project"),
    )
}

fn publish_project<P: StagePreparer>(
    request: &CreateProjectRequest,
    preparer: &P,
    cancellation: squish_kernel::CancellationToken,
    faults: Arc<dyn FaultInjector>,
) -> Result<ProjectCreationStatus, ServiceError> {
    if cancellation.is_cancelled() {
        return Ok(ProjectCreationStatus::Cancelled);
    }
    let observed = AtomicBool::new(false);
    let cancellable = CancellablePreparation {
        inner: preparer,
        cancellation: cancellation.clone(),
        observed: &observed,
    };
    match create_project(request, &cancellable, faults.as_ref()) {
        Ok(created) => Ok(ProjectCreationStatus::Created(created)),
        Err(_) if observed.load(Ordering::Acquire) => Ok(ProjectCreationStatus::Cancelled),
        Err(RepositoryError::CreationCancelled(_)) => Ok(ProjectCreationStatus::Cancelled),
        Err(error) if error.committed_creation().is_some() => {
            Ok(ProjectCreationStatus::CommittedFailure(
                external_service_error("project_creation_committed_failure", error),
            ))
        }
        Err(error) => Err(external_service_error("project_creation_failed", error)),
    }
}

fn external_service_error(code: &str, error: impl fmt::Display) -> ServiceError {
    ServiceError::new(code, error.to_string())
}

fn normalize_prospective_destination(destination: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(destination)?;
    let mut cursor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(native_host_path(canonical));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or(error)?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "project destination has no existing ancestor",
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
}

/// 只读定位待创建路径的唯一外围工作区。 / Read-only locates the sole enclosing workspace of a prospective path.
///
/// 此函数只读取已有祖先及其清单，因此可在加载 workspace 配置前使用。 / This function
/// reads only existing ancestors and manifests, so it is safe before workspace configuration is loaded.
pub fn locate_enclosing_workspace(
    destination: &Path,
) -> Result<Option<WorkspaceMembership>, ServiceError> {
    let destination = normalize_prospective_destination(destination)
        .map_err(|error| external_service_error("project_destination_unavailable", error))?;
    let mut found = Vec::new();
    for ancestor in destination.parent().into_iter().flat_map(Path::ancestors) {
        let manifest_path = ancestor.join(squish_project::MANIFEST_FILE_NAME);
        let source = match std::fs::read_to_string(&manifest_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(external_service_error(
                    "workspace_manifest_unavailable",
                    error,
                ));
            }
        };
        let manifest = Manifest::parse(&source)
            .map_err(|error| external_service_error("workspace_manifest_invalid", error))?;
        if manifest.workspace.is_some() {
            found.push(ancestor.to_path_buf());
        }
    }
    if found.len() > 1 {
        return Err(ServiceError::new(
            "ambiguous_enclosing_workspace",
            "project destination is enclosed by multiple xmlsquish workspaces",
        ));
    }
    let Some(root) = found.pop() else {
        return Ok(None);
    };
    let relative = destination.strip_prefix(&root).map_err(|_| {
        ServiceError::new(
            "invalid_workspace_member",
            "project destination is outside its enclosing workspace",
        )
    })?;
    let member = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            ServiceError::new(
                "invalid_workspace_member",
                "workspace member path must be valid Unicode",
            )
        })?
        .join("/");
    Ok(Some(WorkspaceMembership { root, member }))
}

fn nearest_existing_directory(path: &Path) -> std::io::Result<PathBuf> {
    let mut cursor = path.parent().unwrap_or(path);
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(existing) if existing.is_dir() => return Ok(native_host_path(existing)),
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "the nearest existing destination ancestor is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                cursor = cursor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
}

impl Services for ProductionHost {
    fn open_build_runtime(
        &self,
        project_root: &Path,
    ) -> Result<Arc<dyn BuildRuntime>, ServiceError> {
        self.require_project(project_root)?;
        Ok(self.build_runtime.clone())
    }

    fn locate_project_creation(
        &self,
        destination: &Path,
        vcs: VcsChoice,
    ) -> Result<ProjectCreationLocation, ServiceError> {
        ProjectCreationHost {
            git: self.git_runner.clone(),
        }
        .locate_project_creation(destination, vcs)
    }

    fn create_project(
        &self,
        request: &CreateProjectRequest,
        cancellation: squish_kernel::CancellationToken,
        faults: Arc<dyn FaultInjector>,
    ) -> Result<ProjectCreationStatus, ServiceError> {
        publish_project(request, self, cancellation, faults)
    }

    fn clean_project(
        &self,
        project_root: &Path,
        cancellation: squish_kernel::CancellationToken,
    ) -> Result<ProjectCleanStatus, ServiceError> {
        self.require_project(project_root)?;
        if cancellation.is_cancelled() {
            return Ok(ProjectCleanStatus::Cancelled);
        }
        if self
            .build_runtime
            .has_maintenance_lease()
            .map_err(|error| ServiceError::new(error.code(), error.message()))?
        {
            return Err(ServiceError::new(
                "project_clean_busy",
                "this host already opened project build storage; clean requires a fresh invocation",
            ));
        }

        let paths = CleanPaths::new(&self.storage)
            .map_err(|error| ServiceError::new("project_clean_failed", error.to_string()))?;
        let Some(_lock) = open_project_lock_cancellable(&paths, &cancellation)
            .map_err(|error| ServiceError::new("project_clean_failed", error.to_string()))?
        else {
            return Ok(ProjectCleanStatus::Cancelled);
        };
        if cancellation.is_cancelled() {
            return Ok(ProjectCleanStatus::Cancelled);
        }
        paths
            .validate_root_identity()
            .map_err(|error| ServiceError::new("project_clean_failed", error.to_string()))?;

        let mut result = CleanResult::default();
        let mut changed = false;
        if let Err(error) = recover_clean_journal(&paths, &mut result, &mut changed) {
            return clean_io_failure(result, changed, error);
        }
        if !changed && cancellation.is_cancelled() {
            return Ok(ProjectCleanStatus::Cancelled);
        }

        if !journal_exists(&paths.root)
            .map_err(|error| ServiceError::new("project_clean_failed", error.to_string()))?
        {
            return Ok(ProjectCleanStatus::Cleaned(result));
        }
        let trash_leaf = match next_trash_leaf(&paths) {
            Ok(leaf) => leaf,
            Err(error) => return clean_io_failure(result, changed, error),
        };
        if let Err(error) = paths.validate_root_identity() {
            return clean_io_failure(result, changed, error);
        }
        if let Err(error) = persist_clean_journal(&paths, trash_leaf.clone()) {
            return clean_io_failure(result, changed, error);
        }
        changed = true;
        let trash = paths
            .root
            .parent()
            .expect("validated build root parent")
            .join(trash_leaf);
        let project_clean = clean_root(&paths.root, &trash, &mut result, &mut changed)
            .and_then(|()| std::fs::remove_file(&paths.journal))
            .and_then(|()| sync_parent(&paths.journal));
        if let Err(error) = project_clean {
            if !changed {
                let _ = std::fs::remove_file(&paths.journal);
            }
            return clean_io_failure(result, changed, error);
        }

        Ok(ProjectCleanStatus::Cleaned(result))
    }

    fn storage_layout(&self, project_root: &Path) -> Result<ProjectBuildLayout, ServiceError> {
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
        self.build_runtime
            .cache_records()
            .map_err(|error| ServiceError::new(error.code(), error.message()))
    }

    fn read_blob(
        &self,
        project: &Path,
        digest: &squish_protocol::Digest,
    ) -> Result<Option<Vec<u8>>, ServiceError> {
        self.require_project(project)?;
        self.build_runtime
            .read_blob(digest)
            .map_err(|error| ServiceError::new(error.code(), error.message()))
    }

    fn artifact(
        &self,
        project: &Path,
        id: &squish_protocol::ArtifactId,
    ) -> Result<Option<squish_protocol::Artifact>, ServiceError> {
        self.require_project(project)?;
        Ok(self
            .current_catalog()?
            .and_then(|catalog| catalog.artifact(id)))
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
            .and_then(|catalog| catalog.artifact_at(&relative)))
    }

    fn link_map(
        &self,
        project: &Path,
        target: &squish_protocol::TargetName,
    ) -> Result<Option<squish_protocol::Artifact>, ServiceError> {
        self.require_project(project)?;
        self.current_catalog()?
            .map(|catalog| {
                catalog.link_map(target).map_err(|error| {
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

fn clean_io_failure(
    result: CleanResult,
    changed: bool,
    error: io::Error,
) -> Result<ProjectCleanStatus, ServiceError> {
    let error = ServiceError::new("project_clean_failed", error.to_string());
    if changed {
        Ok(ProjectCleanStatus::CommittedFailure { result, error })
    } else {
        Err(error)
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
        squish_manager::build::read_current_build_catalog(self.build_runtime.as_ref())
            .map_err(|error| ServiceError::new("build_catalog_failed", error.to_string()))
    }
}

fn service_error(code: &str, error: SourceUnavailable) -> ServiceError {
    ServiceError::new(code, format!("{}: {}", error.identity, error.detail))
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
    use squish_repository::{FaultPoint, ProjectFile};
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

    struct RecordingGit {
        calls: Mutex<Vec<GitInvocation>>,
    }

    impl GitRunner for RecordingGit {
        fn execute(&self, invocation: GitInvocation) -> std::io::Result<GitRunOutput> {
            let discovery = invocation
                .args
                .first()
                .is_some_and(|value| value == "rev-parse");
            self.calls.lock().unwrap().push(invocation);
            Ok(GitRunOutput {
                success: true,
                code: Some(0),
                stdout: if discovery {
                    b"true\n".to_vec()
                } else {
                    Vec::new()
                },
                stderr: Vec::new(),
            })
        }
    }

    struct FailedGitDiscovery(&'static str);

    impl GitRunner for FailedGitDiscovery {
        fn execute(&self, _invocation: GitInvocation) -> std::io::Result<GitRunOutput> {
            Ok(GitRunOutput {
                success: false,
                code: Some(128),
                stdout: Vec::new(),
                stderr: self.0.as_bytes().to_vec(),
            })
        }
    }

    struct MissingGit;

    impl GitRunner for MissingGit {
        fn execute(&self, _invocation: GitInvocation) -> std::io::Result<GitRunOutput> {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "git executable missing",
            ))
        }
    }

    struct FailAt(FaultPoint);

    impl FaultInjector for FailAt {
        fn check(&self, point: FaultPoint) -> std::io::Result<()> {
            if point == self.0 {
                return Err(std::io::Error::other("injected creation boundary failure"));
            }
            Ok(())
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

    fn fixture() -> (tempfile::TempDir, ProductionHost, ProjectBuildLayout) {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(scratch).unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-")
            .tempdir_in(scratch)
            .unwrap();
        let root = temporary.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let storage = ProjectBuildLayout::new(&root, "target/xmlsquish").unwrap();
        let host = fixture_host(&root, &storage);
        (temporary, host, storage)
    }

    fn fixture_host(root: &Path, storage: &ProjectBuildLayout) -> ProductionHost {
        fixture_host_with_cancellation(root, storage, squish_kernel::CancellationToken::default())
    }

    fn fixture_host_with_cancellation(
        root: &Path,
        storage: &ProjectBuildLayout,
        cancellation: squish_kernel::CancellationToken,
    ) -> ProductionHost {
        let filesystem = Arc::new(squish_fetch::FilesystemHost::new(root).unwrap());
        ProductionHost::open(HostConfig {
            project_root: root.to_path_buf(),
            storage: storage.clone(),
            cancellation,
            registries: Vec::new(),
            credentials: Arc::new(NoCredentials),
            http: Arc::new(NoHttp),
            git: GitExecution::Runner(Arc::new(NoGit)),
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
            filesystem,
        })
        .unwrap()
    }

    #[cfg(unix)]
    fn create_test_directory_alias(target: &Path, alias: &Path) {
        std::os::unix::fs::symlink(target, alias).unwrap();
    }

    #[cfg(windows)]
    fn create_test_directory_alias(target: &Path, alias: &Path) {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(alias)
            .arg(target)
            .status()
            .unwrap();
        assert!(status.success());
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
        host.build_runtime.acquire_maintenance().unwrap();
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
    fn host_derives_source_cache_from_project_layout() {
        let (_temporary, host, storage) = fixture();
        assert_eq!(host.source_cache_root(), storage.source_cache_root());
        assert!(host.source_cache_root().starts_with(host.project_root()));
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
        let storage = ProjectBuildLayout::new(&root, "target/xmlsquish").unwrap();
        let host = ProductionHost::open(HostConfig {
            project_root: root.clone(),
            storage,
            cancellation: squish_kernel::CancellationToken::default(),
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
        let project = std::fs::canonicalize(project).unwrap();
        let storage = ProjectBuildLayout::new(&project, "target/xmlsquish").unwrap();
        let host = ProductionHost::open(HostConfig {
            project_root: project.clone(),
            storage,
            cancellation: squish_kernel::CancellationToken::default(),
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
        host.build_runtime.acquire_maintenance().unwrap();
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
            .into_parts()
            .0
            .unwrap();
        assert_eq!(
            primary,
            AuthorizationValue::new("Bearer primary-secret".into()).unwrap()
        );
        let cross = credentials
            .authorization("https://registry.example/v1", "corp-read", cross_origin)
            .unwrap()
            .into_parts()
            .0
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
                .into_parts()
                .0
                .is_none()
        );
        let rendered = format!("{credentials:?} {primary:?} {cross}");
        assert!(!rendered.contains("primary-secret"));
        assert!(!rendered.contains("cross-secret"));

        let missing = EnvironmentCredentials::from_snapshot(
            [CredentialRoute {
                registry_id: "https://registry.example/v1".into(),
                auth_scope: "corp-read".into(),
                primary_origin: "https://index.example.com".into(),
            }],
            [],
        )
        .unwrap();
        let (_, primary_hint) = missing
            .authorization(
                "https://registry.example/v1",
                "corp-read",
                "https://index.example.com",
            )
            .unwrap()
            .into_parts();
        assert_eq!(
            primary_hint.as_deref(),
            Some("XMLSQUISH_REGISTRY_CORP_READ_AUTHORIZATION")
        );
        let (_, cross_hint) = missing
            .authorization("https://registry.example/v1", "corp-read", cross_origin)
            .unwrap()
            .into_parts();
        assert_eq!(
            cross_hint.as_deref(),
            Some(format!("XMLSQUISH_REGISTRY_CORP_READ_ORIGIN_{digest}_AUTHORIZATION").as_str())
        );
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

    #[test]
    fn creation_location_is_read_only_and_reuses_enclosing_git_workspace() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        let git = Arc::new(RecordingGit {
            calls: Mutex::new(Vec::new()),
        });
        let host = ProjectCreationHost::new(GitExecution::Runner(git.clone())).unwrap();
        let destination = temp.path().join("nested/project");
        let expected_destination =
            squish_repository::normalize_new_destination(&destination).unwrap();

        let location = host
            .locate_project_creation(&destination, VcsChoice::Git)
            .unwrap();

        assert_eq!(
            squish_repository::normalize_new_destination(&location.destination).unwrap(),
            expected_destination
        );
        assert_eq!(location.vcs, ProjectVcs::InheritedGit);
        assert_eq!(location.workspace.unwrap().member, "nested/project");
        assert!(!destination.exists());
        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args, ["rev-parse", "--is-inside-work-tree"]);
    }

    #[test]
    fn creation_stage_runs_shell_free_git_init_only_for_standalone_git() {
        let temp = tempfile::tempdir().unwrap();
        let git = Arc::new(RecordingGit {
            calls: Mutex::new(Vec::new()),
        });
        let host = ProjectCreationHost::new(GitExecution::Runner(git.clone())).unwrap();

        host.prepare(temp.path(), ProjectVcs::InitializeGit)
            .unwrap();
        host.prepare(temp.path(), ProjectVcs::InheritedGit).unwrap();
        host.prepare(temp.path(), ProjectVcs::None).unwrap();

        let calls = git.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].cwd.as_deref(), Some(temp.path()));
        assert_eq!(calls[0].args, ["init", "--quiet"]);
        assert!(calls[0].env.is_empty());
    }

    #[test]
    fn git_discovery_distinguishes_absence_from_execution_failure() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("demo");
        let absent = ProjectCreationHost::new(GitExecution::Runner(Arc::new(FailedGitDiscovery(
            "fatal: not a git repository",
        ))))
        .unwrap();
        assert_eq!(
            absent
                .locate_project_creation(&destination, VcsChoice::Git)
                .unwrap()
                .vcs,
            ProjectVcs::InitializeGit
        );

        let broken = ProjectCreationHost::new(GitExecution::Runner(Arc::new(FailedGitDiscovery(
            "fatal: detected dubious ownership",
        ))))
        .unwrap();
        let error = broken
            .locate_project_creation(&destination, VcsChoice::Git)
            .unwrap_err();
        assert_eq!(error.code(), "git_discovery_failed");
        assert!(error.message().contains("--vcs=none"));
    }

    #[test]
    fn missing_git_is_actionable_during_discovery_and_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let host = ProjectCreationHost::new(GitExecution::Runner(Arc::new(MissingGit))).unwrap();

        let discovery = host
            .locate_project_creation(&temp.path().join("demo"), VcsChoice::Git)
            .unwrap_err();
        assert_eq!(discovery.code(), "git_discovery_failed");
        assert!(discovery.message().contains("--vcs=none"));

        let initialization = host
            .prepare(temp.path(), ProjectVcs::InitializeGit)
            .unwrap_err();
        assert_eq!(initialization.kind(), std::io::ErrorKind::NotFound);
        assert!(initialization.to_string().contains("--vcs=none"));
    }

    #[test]
    fn post_publication_repository_failure_retains_committed_status() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("committed");
        let host = ProjectCreationHost::new(GitExecution::Runner(Arc::new(MissingGit))).unwrap();
        let request = CreateProjectRequest {
            destination: destination.clone(),
            files: vec![ProjectFile {
                path: PathBuf::from("xmlsquish.toml"),
                bytes: b"manifest-version = 1\n\n[workspace]\n".to_vec(),
            }],
            package_name: "committed".into(),
            vcs: ProjectVcs::None,
            workspace: None,
        };

        let status = host
            .create_project(
                &request,
                squish_kernel::CancellationToken::default(),
                Arc::new(FailAt(FaultPoint::CreationPublished)),
            )
            .unwrap();

        assert!(matches!(status, ProjectCreationStatus::CommittedFailure(_)));
        assert!(destination.exists());
    }

    #[test]
    fn clean_removes_the_entire_project_owned_tree() {
        let (_temporary, host, storage) = fixture();
        std::fs::create_dir_all(storage.artifacts_root()).unwrap();
        std::fs::create_dir_all(storage.cas_root()).unwrap();
        std::fs::create_dir_all(storage.catalog_root()).unwrap();
        std::fs::create_dir_all(storage.source_cache_root()).unwrap();
        std::fs::write(storage.artifacts_root().join("one"), b"abc").unwrap();
        std::fs::write(storage.cas_root().join("two"), b"12").unwrap();
        std::fs::write(storage.catalog_root().join("three"), b"1234").unwrap();
        std::fs::write(storage.source_cache_root().join("four"), b"1").unwrap();

        let status = Services::clean_project(
            &host,
            host.project_root(),
            squish_kernel::CancellationToken::default(),
        )
        .unwrap();
        let ProjectCleanStatus::Cleaned(result) = status else {
            panic!("clean should complete")
        };
        assert_eq!(result.build_files, 4);
        assert_eq!(result.build_bytes, 10);
        assert!(!storage.ownership_root().exists());

        assert_eq!(
            Services::clean_project(
                &host,
                host.project_root(),
                squish_kernel::CancellationToken::default(),
            )
            .unwrap(),
            ProjectCleanStatus::Cleaned(CleanResult::default())
        );
    }

    #[test]
    fn clean_honors_pre_cancellation_without_removing_state() {
        let (_temporary, host, storage) = fixture();
        std::fs::create_dir_all(storage.ownership_root()).unwrap();
        std::fs::write(storage.ownership_root().join("keep"), b"still here").unwrap();
        let cancellation = squish_kernel::CancellationToken::default();
        cancellation.cancel();

        assert_eq!(
            Services::clean_project(&host, host.project_root(), cancellation).unwrap(),
            ProjectCleanStatus::Cancelled
        );
        assert!(storage.ownership_root().join("keep").is_file());
    }

    #[test]
    fn clean_honors_cancellation_while_waiting_for_maintenance_lock() {
        let (_temporary, host, storage) = fixture();
        std::fs::create_dir_all(storage.ownership_root()).unwrap();
        std::fs::write(storage.ownership_root().join("keep"), b"still here").unwrap();
        let paths = CleanPaths::new(&storage).unwrap();
        let lock = open_project_lock(&paths).unwrap();
        let host = Arc::new(host);
        let cancellation = squish_kernel::CancellationToken::default();
        let worker_host = host.clone();
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || {
            Services::clean_project(
                worker_host.as_ref(),
                worker_host.project_root(),
                worker_cancellation,
            )
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        cancellation.cancel();

        assert_eq!(
            worker.join().unwrap().unwrap(),
            ProjectCleanStatus::Cancelled
        );
        assert!(storage.ownership_root().join("keep").is_file());
        drop(lock);
    }

    #[cfg(unix)]
    #[test]
    fn clean_does_not_follow_or_count_symbolic_links() {
        let (temporary, host, storage) = fixture();
        let external = temporary.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep"), b"outside").unwrap();
        std::fs::create_dir_all(storage.ownership_root()).unwrap();
        std::fs::write(storage.ownership_root().join("ordinary"), b"one").unwrap();
        std::os::unix::fs::symlink(&external, storage.ownership_root().join("linked")).unwrap();

        let status = Services::clean_project(
            &host,
            host.project_root(),
            squish_kernel::CancellationToken::default(),
        )
        .unwrap();
        let ProjectCleanStatus::Cleaned(result) = status else {
            panic!("clean should complete")
        };
        assert_eq!(result.build_files, 1);
        assert_eq!(std::fs::read(external.join("keep")).unwrap(), b"outside");
    }

    #[test]
    fn first_storage_use_rolls_forward_journaled_detached_root() {
        let (temporary, host, storage) = fixture();
        let project_root = host.project_root().to_path_buf();
        std::fs::create_dir_all(storage.ownership_root()).unwrap();
        std::fs::write(storage.ownership_root().join("old"), b"old").unwrap();
        let paths = CleanPaths::new(&storage).unwrap();
        let trash_leaf = next_trash_leaf(&paths).unwrap();
        let trash = paths.root.parent().unwrap().join(&trash_leaf);
        let lock = open_project_lock(&paths).unwrap();
        persist_clean_journal(&paths, trash_leaf).unwrap();
        rename_with_retry(storage.ownership_root(), &trash).unwrap();
        drop(lock);
        drop(host);

        let recovered = fixture_host(&project_root, &storage);
        recovered.build_runtime.write_blob(b"new").unwrap();
        assert!(!storage.ownership_root().join("old").exists());
        assert!(storage.layout_marker().is_file());
        assert!(!trash.exists());
        assert!(!paths.journal.exists());
        drop(recovered);
        drop(temporary);
    }

    #[test]
    fn first_storage_use_writes_relative_layout_marker_lazily() {
        let (_temporary, host, storage) = fixture();
        assert!(!storage.ownership_root().exists());

        host.build_runtime.write_blob(b"fixture").unwrap();

        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(storage.layout_marker()).unwrap()).unwrap();
        assert_eq!(marker, serde_json::json!({"schema": 1, "format_epoch": 1}));
        assert!(
            !std::fs::read_to_string(storage.layout_marker())
                .unwrap()
                .contains(host.project_root().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn incompatible_nonempty_layout_is_reset_under_maintenance_lock() {
        let (_temporary, host, storage) = fixture();
        std::fs::create_dir_all(storage.metadata_root()).unwrap();
        std::fs::write(
            storage.layout_marker(),
            br#"{"schema":99,"format_epoch":1}"#,
        )
        .unwrap();
        std::fs::write(storage.ownership_root().join("stale"), b"old").unwrap();

        host.build_runtime.write_blob(b"fresh").unwrap();

        assert!(!storage.ownership_root().join("stale").exists());
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(storage.layout_marker()).unwrap()).unwrap();
        assert_eq!(marker["schema"], 1);
    }

    #[test]
    fn runtime_waits_for_exclusive_maintenance_lock_before_opening_storage() {
        let (_temporary, host, storage) = fixture();
        let paths = CleanPaths::new(&storage).unwrap();
        let lock = open_project_lock(&paths).unwrap();
        let runtime = host.build_runtime.clone();
        let worker = std::thread::spawn(move || runtime.write_blob(b"waited"));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!storage.ownership_root().exists());
        drop(lock);
        assert!(worker.join().unwrap().is_ok());
        assert!(storage.layout_marker().is_file());
    }

    #[test]
    fn valid_layout_runtimes_acquire_shared_maintenance_leases_concurrently() {
        let (_temporary, first, storage) = fixture();
        first.build_runtime.write_blob(b"initialize").unwrap();
        let second = fixture_host(first.project_root(), &storage);
        let runtime = second.build_runtime.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sent.send(runtime.acquire_maintenance()).unwrap();
        });

        received
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("a valid layout must not wait for an exclusive maintenance lease")
            .unwrap();
        worker.join().unwrap();
        assert!(first.build_runtime.has_maintenance_lease().unwrap());
        assert!(second.build_runtime.has_maintenance_lease().unwrap());
    }

    #[test]
    fn runtime_cancels_while_waiting_behind_exclusive_maintenance_lock() {
        let (temporary, _unused, storage) = fixture();
        let root = temporary.path().join("project");
        let cancellation = squish_kernel::CancellationToken::default();
        let host = fixture_host_with_cancellation(&root, &storage, cancellation.clone());
        let paths = CleanPaths::new(&storage).unwrap();
        let lock = open_project_lock(&paths).unwrap();
        let runtime = host.build_runtime.clone();
        let worker = std::thread::spawn(move || runtime.write_blob(b"must-not-open-storage"));

        std::thread::sleep(std::time::Duration::from_millis(75));
        cancellation.cancel();
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.code(), "host_maintenance_cancelled");
        assert!(!storage.ownership_root().exists());
        drop(lock);
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_mixed_case_target_directory_supports_build_and_clean() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp");
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(scratch).unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("squish-host-mixed-case-")
            .tempdir_in(scratch)
            .unwrap();
        let root = temporary.path().join("project");
        std::fs::create_dir_all(root.join("TARGET")).unwrap();
        let storage = ProjectBuildLayout::new(&root, "target/xmlsquish").unwrap();

        {
            let host = fixture_host(&root, &storage);
            host.build_runtime
                .write_blob(b"ordinary-directory")
                .unwrap();
            assert!(storage.layout_marker().is_file());
        }

        let host = fixture_host(&root, &storage);
        let status = Services::clean_project(
            &host,
            host.project_root(),
            squish_kernel::CancellationToken::default(),
        )
        .unwrap();
        assert!(matches!(status, ProjectCleanStatus::Cleaned(_)));
        assert!(!storage.ownership_root().exists());
    }

    #[cfg(unix)]
    #[test]
    fn project_root_replacement_while_clean_waits_never_redirects_deletion() {
        let (temporary, host, storage) = fixture();
        std::fs::create_dir_all(storage.ownership_root()).unwrap();
        let paths = CleanPaths::new(&storage).unwrap();
        let lock = open_project_lock(&paths).unwrap();
        let root = host.project_root().to_path_buf();
        let host = Arc::new(host);
        let worker_host = Arc::clone(&host);
        let worker_root = root.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            Services::clean_project(
                worker_host.as_ref(),
                &worker_root,
                squish_kernel::CancellationToken::default(),
            )
        });
        started_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(75));

        let moved = temporary.path().join("moved-project");
        std::fs::rename(&root, &moved).unwrap();
        let replacement = temporary.path().join("replacement-project");
        let replacement_build = replacement.join("target/xmlsquish");
        std::fs::create_dir_all(&replacement_build).unwrap();
        let sentinel = replacement_build.join("authoritative.txt");
        std::fs::write(&sentinel, b"must survive redirected clean").unwrap();
        create_test_directory_alias(&replacement, &root);
        drop(lock);

        let clean_error = worker.join().unwrap().unwrap_err();
        assert_eq!(clean_error.code(), "project_clean_failed");
        let build_error = host
            .build_runtime
            .write_blob(b"must-not-enter-replacement")
            .unwrap_err();
        assert_eq!(build_error.code(), "host_maintenance_lock");
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"must survive redirected clean"
        );
        assert!(!replacement_build.join("cache").exists());
    }

    #[cfg(windows)]
    #[test]
    fn project_root_junction_replacement_never_redirects_build_or_clean() {
        let (temporary, host, _storage) = fixture();
        let root = host.project_root().to_path_buf();
        let moved = temporary.path().join("moved-project");
        std::fs::rename(&root, &moved).unwrap();
        let replacement = temporary.path().join("replacement-project");
        let replacement_build = replacement.join("target/xmlsquish");
        std::fs::create_dir_all(&replacement_build).unwrap();
        let sentinel = replacement_build.join("authoritative.txt");
        std::fs::write(&sentinel, b"must survive redirected operations").unwrap();
        create_test_directory_alias(&replacement, &root);

        let clean_error =
            Services::clean_project(&host, &root, squish_kernel::CancellationToken::default())
                .unwrap_err();
        assert_eq!(clean_error.code(), "project_root_mismatch");
        let build_error = host
            .build_runtime
            .write_blob(b"must-not-enter-replacement")
            .unwrap_err();
        assert_eq!(build_error.code(), "host_maintenance_lock");
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"must survive redirected operations"
        );
        assert!(!replacement_build.join("cache").exists());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_rejects_alias_introduced_after_layout_construction() {
        let (temporary, host, storage) = fixture();
        let redirected = temporary.path().join("redirected-target");
        std::fs::create_dir_all(&redirected).unwrap();
        std::os::unix::fs::symlink(&redirected, host.project_root().join("target")).unwrap();

        let error = host
            .build_runtime
            .write_blob(b"must-not-escape")
            .unwrap_err();
        assert_eq!(error.code(), "host_maintenance_lock");
        assert!(std::fs::read_dir(&redirected).unwrap().next().is_none());
        assert!(!storage.layout_marker().exists());
    }
}
