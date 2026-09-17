//! 可崩溃恢复的本地文件产物发布器。 / Crash-recoverable local-file artifact publisher.
//!
//! 发布器从 [`BlobStore`] 流式读取不可变内容。单文件兼容 API 在目标目录内校验临时
//! 文件后原子替换；generation API 先完整暂存产物集合，再以一次 current manifest
//! 替换提交。持久 journal 让下一次打开自动 roll-forward 被中断的提交。
//! The publisher streams immutable content from a [`BlobStore`]. The compatible single-file
//! API verifies a destination-local temporary before atomic replacement; the generation API
//! stages a complete artifact set and commits it with one current-manifest replacement. A
//! durable journal automatically rolls an interrupted commit forward on the next open.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use squish_build::{
    ArtifactDescriptor, ArtifactPublisher, ArtifactRead, BlobStore, CommittedGeneration,
    GenerationArtifact, GenerationId, GenerationRef, GenerationRepository, LogicalArtifactName,
    Publication, PublicationPath, PublicationTargetId,
};
use squish_protocol::{ArtifactId, Digest, DigestAlgorithm};
use std::{
    ffi::OsStr,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tempfile::NamedTempFile;

const STATE_DIR: &str = ".squish-publish";
const JOURNAL: &str = "journal.json";
const GENERATION_JOURNAL: &str = "generation-journal.json";
const GENERATIONS: &str = "generations";
const TARGETS: &str = "targets";
const LOCK: &str = "lock";
const LEGACY_MIGRATION: &str = "legacy-migrated";

/// 推导项目发布与 clean 共用、且位于 catalog 外部的稳定锁路径。 /
/// Derives the stable project lock shared by publishers and clean, outside the catalog.
///
/// 锁位于 catalog 的同级 `.locks` 目录；文件名由完整 catalog 路径派生，因此同一
/// 父目录下的多个项目不会互相串行化。调用者只需向 publish 与 clean 传入相同的
/// catalog root，无需公开 publisher 的私有状态布局。
/// The lock lives in a sibling `.locks` directory. Its name derives from the complete catalog
/// path, so projects sharing a parent do not serialize each other.
#[must_use]
pub fn project_lock_path(catalog_root: impl AsRef<Path>) -> PathBuf {
    let catalog_root = catalog_root.as_ref();
    let parent = catalog_root.parent().unwrap_or_else(|| Path::new("."));
    let mut identity = catalog_root.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    parent.join(".locks").join(format!(
        "{}.lock",
        hex_bytes(&Sha256::digest(identity.as_bytes()))
    ))
}

/// 返回项目 clean epoch 文件的稳定路径。 / Returns the stable project-clean epoch path.
#[must_use]
pub fn project_epoch_path(catalog_root: impl AsRef<Path>) -> PathBuf {
    project_lock_path(catalog_root).with_extension("epoch")
}

/// clean epoch 的不透明快照。 / Opaque snapshot of a clean epoch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectEpoch(u64);

impl ProjectEpoch {
    /// 返回下一 epoch；计数器耗尽时返回 `None`。 / Returns the next epoch, or `None` on counter exhaustion.
    #[must_use]
    pub fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// 读取当前项目 clean epoch；尚无 epoch 文件也是一个稳定状态。 /
/// Reads the current project-clean epoch; an absent epoch file is also a stable state.
pub fn read_project_epoch(catalog_root: impl AsRef<Path>) -> io::Result<ProjectEpoch> {
    read_project_epoch_path(&project_epoch_path(catalog_root))
}

/// 原子持久写入 canonical epoch；调用者必须已经持有项目锁。 /
/// Atomically persists a canonical epoch; the caller must already hold the project lock.
pub fn write_project_epoch(path: impl AsRef<Path>, epoch: &ProjectEpoch) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "epoch path has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    writeln!(temporary, "{}", epoch.0)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    sync_epoch_parent(parent)
}

fn read_project_epoch_path(path: &Path) -> io::Result<ProjectEpoch> {
    match fs::read(path) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let digits = text.strip_suffix('\n').ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "epoch lacks canonical newline")
            })?;
            if digits.is_empty()
                || (digits.len() > 1 && digits.starts_with('0'))
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "epoch is not canonical unsigned decimal",
                ));
            }
            digits
                .parse::<u64>()
                .map(ProjectEpoch)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ProjectEpoch(0)),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_epoch_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_epoch_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

/// 文件发布失败。 / Filesystem publication failure.
#[derive(Debug)]
pub enum PublishError<E> {
    /// 目标不是可移植的、根目录内的相对路径。 / Destination is not a portable root-relative path.
    InvalidDestination(String),
    /// 目标与已有大小写拼写或物理身份冲突。 / Destination conflicts by case spelling or physical identity.
    AliasConflict(PathBuf),
    /// 路径经过符号链接。 / A path traverses a symbolic link.
    Symlink(PathBuf),
    /// clean 事务尚未完成；发布必须等待其恢复。 / A clean transaction is pending and must recover before publication.
    MaintenancePending(PathBuf),
    /// publisher 属于已被 clean 取代的旧 runtime。 / The publisher belongs to a runtime superseded by clean.
    Superseded(PathBuf),
    /// Blob 后端失败。 / Blob backend failed.
    Store(E),
    /// Blob 不存在。 / Blob is absent.
    MissingBlob,
    /// Blob 的大小或完整摘要不符合声明。 / Blob size or complete digest differs from its declaration.
    IntegrityMismatch,
    /// 摘要算法不受发布器支持。 / Digest algorithm is unsupported by this publisher.
    UnsupportedDigest(String),
    /// 文件系统或 journal 操作失败。 / Filesystem or journal operation failed.
    Io(io::Error),
    /// journal 无法解码；保留原始状态供诊断。 / Journal cannot be decoded; original state is retained for diagnosis.
    Journal(serde_json::Error),
}

impl<E: fmt::Display> fmt::Display for PublishError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDestination(value) => {
                write!(f, "invalid publication destination: {value}")
            }
            Self::AliasConflict(path) => {
                write!(f, "publication destination aliases {}", path.display())
            }
            Self::Symlink(path) => {
                write!(f, "publication path traverses symlink {}", path.display())
            }
            Self::MaintenancePending(path) => {
                write!(f, "project maintenance is pending at {}", path.display())
            }
            Self::Superseded(path) => {
                write!(
                    f,
                    "publication runtime was superseded at {}",
                    path.display()
                )
            }
            Self::Store(error) => write!(f, "blob store failed: {error}"),
            Self::MissingBlob => f.write_str("publication blob is missing"),
            Self::IntegrityMismatch => {
                f.write_str("publication blob failed size or digest verification")
            }
            Self::UnsupportedDigest(name) => write!(f, "unsupported publication digest: {name}"),
            Self::Io(error) => write!(f, "publication I/O failed: {error}"),
            Self::Journal(error) => write!(f, "publication journal is malformed: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for PublishError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Journal(error) => Some(error),
            _ => None,
        }
    }
}

impl<E> From<io::Error> for PublishError<E> {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Journal {
    destination: String,
    temporary: String,
    size: u64,
    digest: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GenerationManifest {
    target_id: String,
    generation_id: String,
    artifacts: Vec<ManifestArtifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ManifestArtifact {
    id: String,
    name: String,
    kind: squish_protocol::ArtifactKind,
    destination: String,
    size: u64,
    digest: Digest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GenerationJournal {
    target_key: String,
    generation_id: String,
}

/// generation 协议中的持久化边界。 / Durable boundaries in the generation protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurablePoint {
    /// 一个完整校验的产物文件已同步。 / One fully verified artifact file was synchronized.
    ArtifactStaged,
    /// generation manifest 已同步。 / The generation manifest was synchronized.
    ManifestStaged,
    /// 不可变 generation 目录项已持久化。 / The immutable generation directory entry was persisted.
    GenerationStaged,
    /// commit-decision journal 已持久化。 / The commit-decision journal was persisted.
    CommitDecision,
    /// stable current manifest 已原子切换。 / The stable current manifest was atomically switched.
    CurrentSwitched,
    /// commit-decision journal 已清除。 / The commit-decision journal was cleared.
    JournalCleared,
}

/// 目录元数据持久化结果。 / Directory-metadata durability result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryDurability {
    /// 平台成功同步了目录句柄。 / The platform successfully synchronized a directory handle.
    Synced,
    /// 安全标准库不提供目录句柄同步；已同步文件并使用原子替换。 / Safe standard-library APIs cannot synchronize directory handles; files are synced and replacement remains atomic.
    Unsupported,
}

/// 发布器的结构化持久性事件。 / Structured publisher durability event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishEvent {
    /// 新目录项已经创建。 / A new directory entry was created.
    DirectoryCreated(PathBuf),
    /// 已尝试将包含目录项持久化。 / Persistence of the containing directory was attempted.
    DirectoryDurability {
        /// 被同步的目录。 / Directory synchronized.
        path: PathBuf,
        /// 平台提供的保证。 / Guarantee supplied by the platform.
        guarantee: DirectoryDurability,
    },
    /// 一个 generation 持久化边界已完成；可用于确定性 crash fault injection。
    /// A generation durability boundary completed; useful for deterministic crash fault injection.
    DurablePoint(DurablePoint),
}

/// 接收持久性事件而不耦合日志框架。 / Receives durability events without coupling to a logging framework.
pub trait PublishObserver: Send + Sync {
    /// 观察一个已完成的持久性步骤。 / Observes a completed durability step.
    fn observe(&self, event: &PublishEvent);
}

/// 默认无操作观察者。 / Default no-op observer.
#[derive(Debug, Default)]
pub struct NoopObserver;
impl PublishObserver for NoopObserver {
    fn observe(&self, _event: &PublishEvent) {}
}

/// 由 [`BlobStore`] 支撑的生产文件发布适配器。 / Production file-publisher adapter backed by a [`BlobStore`].
///
/// 根路径会在打开时规范化；所有目标都是相对于该根的可移植路径。单一进程间锁串行化
/// journal 与发布，使恢复成为正常启动路径而不是交给用户的特殊故障处理。
/// The root is canonicalized at open time; every destination is a portable path
/// relative to it. A cross-process lock serializes the journal and publication,
/// making recovery an ordinary open path rather than user-owned repair.
///
/// # 示例 / Example
/// ```no_run
/// # use squish_publish::FileArtifactPublisher;
/// # fn open<S: squish_build::BlobStore>(store: S) -> Result<(), squish_publish::PublishError<S::Error>> {
/// let publisher = FileArtifactPublisher::open("dist", store)?;
/// // squish_build::ArtifactPublisher::publish(&publisher, &publication)?;
/// # Ok(()) }
/// ```
pub struct FileArtifactPublisher<S> {
    root: PathBuf,
    state: PathBuf,
    lock: PathBuf,
    maintenance_marker: Option<PathBuf>,
    epoch_fence: Option<(PathBuf, ProjectEpoch)>,
    logical_prefix: PathBuf,
    store: S,
    observer: Arc<dyn PublishObserver>,
}

/// 显式项目锁发布器的文件系统布局。 / Filesystem layout for an explicit project-lock publisher.
pub struct ExternalPublisherLayout {
    artifact_root: PathBuf,
    state_root: PathBuf,
    logical_prefix: PathBuf,
    lock_path: PathBuf,
    maintenance_marker: Option<PathBuf>,
    legacy_state: Option<PathBuf>,
    epoch_fence: Option<(PathBuf, ProjectEpoch)>,
}

impl ExternalPublisherLayout {
    /// 创建会检查 clean marker 的显式锁布局。 / Creates an explicit-lock layout guarded by the clean marker.
    #[must_use]
    pub fn new(
        artifact_root: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        logical_prefix: impl Into<PathBuf>,
        lock_path: impl Into<PathBuf>,
    ) -> Self {
        let lock_path = lock_path.into();
        Self {
            artifact_root: artifact_root.into(),
            state_root: state_root.into(),
            logical_prefix: logical_prefix.into(),
            maintenance_marker: Some(lock_path.with_extension("clean.json")),
            lock_path,
            legacy_state: None,
            epoch_fence: None,
        }
    }

    /// 配置一次性 legacy state 迁移。 / Configures one-time legacy-state migration.
    #[must_use]
    pub fn with_legacy_state(mut self, path: impl Into<PathBuf>) -> Self {
        self.legacy_state = Some(path.into());
        self
    }

    /// 配置 clean epoch 栅栏。 / Configures the clean-epoch fence.
    #[must_use]
    pub fn with_epoch(mut self, path: impl Into<PathBuf>, expected: ProjectEpoch) -> Self {
        self.epoch_fence = Some((path.into(), expected));
        self
    }
}

impl<S: BlobStore> FileArtifactPublisher<S> {
    /// 打开完全配置的显式项目锁布局。 / Opens a fully configured explicit project-lock layout.
    pub fn with_external_layout(
        layout: ExternalPublisherLayout,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        Self::open_layout(layout, store, observer)
    }

    /// 打开发布根目录并自动恢复未完成的 journal。 / Opens a publication root and automatically recovers an unfinished journal.
    pub fn open(root: impl AsRef<Path>, store: S) -> Result<Self, PublishError<S::Error>> {
        Self::with_observer(root, store, Arc::new(NoopObserver))
    }

    /// 打开发布根目录、安装观察者并自动恢复。 / Opens the publication root, installs an observer, and recovers automatically.
    pub fn with_observer(
        root: impl AsRef<Path>,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        let root = root.as_ref().to_path_buf();
        let state = root.join(STATE_DIR);
        // 兼容 API 把锁放在尚不存在的 state 内，因此必须先引导该目录；production
        // 的显式 external-lock API 不走这个锁外初始化分支。
        // The compatible API stores its lock inside an initially absent state directory and
        // therefore must bootstrap it; the production external-lock API skips this branch.
        create_directory_tree(&root, observer.as_ref())?;
        create_directory_tree(&state, observer.as_ref())?;
        let lock = state.join(LOCK);
        Self::open_layout(
            ExternalPublisherLayout {
                artifact_root: root,
                state_root: state,
                logical_prefix: PathBuf::new(),
                lock_path: lock,
                maintenance_marker: None,
                legacy_state: None,
                epoch_fence: None,
            },
            store,
            observer,
        )
    }

    /// 使用彼此分离的用户产物根和私有状态根打开发布器。 /
    /// Opens a publisher with separate user-artifact and private-state roots.
    ///
    /// `logical_prefix` 是公开 locator 中不应在 `artifact_root` 下重复的前缀。例如，
    /// artifact root 为 `target/xmlsquish` 时，前缀也可设为 `target/xmlsquish`，使 locator
    /// `target/xmlsquish/app.prompt` 物化为 `artifact_root/app.prompt`。
    /// `logical_prefix` is the public-locator prefix that must not be repeated below
    /// `artifact_root`.
    pub fn open_with_layout(
        artifact_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        logical_prefix: impl AsRef<Path>,
        store: S,
    ) -> Result<Self, PublishError<S::Error>> {
        Self::with_layout_and_observer(
            artifact_root,
            state_root,
            logical_prefix,
            store,
            Arc::new(NoopObserver),
        )
    }

    /// 使用分离布局和观察者打开发布器并自动恢复。 /
    /// Opens a separated-layout publisher with an observer and recovers automatically.
    pub fn with_layout_and_observer(
        artifact_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        logical_prefix: impl AsRef<Path>,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        let artifact_root = artifact_root.as_ref().to_path_buf();
        let state_root = state_root.as_ref().to_path_buf();
        // This overload intentionally preserves the historical state-local lock contract.
        create_directory_tree(&artifact_root, observer.as_ref())?;
        create_directory_tree(&state_root, observer.as_ref())?;
        let lock = state_root.join(LOCK);
        Self::open_layout(
            ExternalPublisherLayout {
                artifact_root,
                state_root,
                logical_prefix: logical_prefix.as_ref().to_path_buf(),
                lock_path: lock,
                maintenance_marker: None,
                legacy_state: None,
                epoch_fence: None,
            },
            store,
            observer,
        )
    }

    /// 使用显式共享锁路径打开分离布局发布器。 /
    /// Opens a separated-layout publisher with an explicit shared lock path.
    pub fn open_with_layout_and_lock(
        artifact_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        logical_prefix: impl AsRef<Path>,
        lock_path: impl AsRef<Path>,
        store: S,
    ) -> Result<Self, PublishError<S::Error>> {
        Self::with_layout_observer_and_lock(
            artifact_root,
            state_root,
            logical_prefix,
            lock_path,
            store,
            Arc::new(NoopObserver),
        )
    }

    /// 使用显式共享锁路径和观察者打开分离布局发布器。 /
    /// Opens a separated-layout publisher with an explicit shared lock path and observer.
    pub fn with_layout_observer_and_lock(
        artifact_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        logical_prefix: impl AsRef<Path>,
        lock_path: impl AsRef<Path>,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        let lock_path = lock_path.as_ref().to_path_buf();
        let marker = lock_path.with_extension("clean.json");
        Self::open_layout(
            ExternalPublisherLayout {
                artifact_root: artifact_root.as_ref().to_path_buf(),
                state_root: state_root.as_ref().to_path_buf(),
                logical_prefix: logical_prefix.as_ref().to_path_buf(),
                lock_path,
                maintenance_marker: Some(marker),
                legacy_state: None,
                epoch_fence: None,
            },
            store,
            observer,
        )
    }

    /// 在同一显式锁临界区内初始化分离布局并迁移 legacy state。 /
    /// Initializes a separated layout and migrates legacy state in one explicit-lock section.
    pub fn with_layout_observer_lock_and_legacy(
        artifact_root: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
        logical_prefix: impl AsRef<Path>,
        lock_path: impl AsRef<Path>,
        legacy_state: impl AsRef<Path>,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        let lock_path = lock_path.as_ref().to_path_buf();
        let marker = lock_path.with_extension("clean.json");
        Self::open_layout(
            ExternalPublisherLayout {
                artifact_root: artifact_root.as_ref().to_path_buf(),
                state_root: state_root.as_ref().to_path_buf(),
                logical_prefix: logical_prefix.as_ref().to_path_buf(),
                lock_path,
                maintenance_marker: Some(marker),
                legacy_state: Some(legacy_state.as_ref().to_path_buf()),
                epoch_fence: None,
            },
            store,
            observer,
        )
    }

    fn open_layout(
        layout: ExternalPublisherLayout,
        store: S,
        observer: Arc<dyn PublishObserver>,
    ) -> Result<Self, PublishError<S::Error>> {
        let logical_prefix = validate_logical_prefix(&layout.logical_prefix)?;
        let artifact_root = layout.artifact_root;
        let state_root = layout.state_root;
        let maintenance_marker = layout.maintenance_marker;
        let legacy_state = layout.legacy_state;
        let epoch_fence = layout.epoch_fence;
        let lock_path = layout.lock_path;
        let lock_parent = lock_path.parent().ok_or_else(|| {
            PublishError::InvalidDestination("shared lock path has no parent".into())
        })?;
        let lock_name = lock_path.file_name().ok_or_else(|| {
            PublishError::InvalidDestination("shared lock path has no file name".into())
        })?;
        // 只引导锁本身的 sibling 目录；用户 artifact/state 根必须等拿到项目锁后创建。
        // Bootstrap only the sibling lock directory. User artifact/state roots are created
        // only after the project lock is held, so clean cannot race with initialization.
        create_directory_tree(lock_parent, observer.as_ref())?;
        let lock = fs::canonicalize(lock_parent)?.join(lock_name);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)?;
        lock_file.lock_exclusive()?;
        let result = (|| {
            validate_fences(&maintenance_marker, &epoch_fence)?;
            create_directory_tree(&artifact_root, observer.as_ref())?;
            let root = fs::canonicalize(&artifact_root)?;
            create_directory_tree(&state_root, observer.as_ref())?;
            let state = fs::canonicalize(&state_root)?;
            reject_symlink(&state)?;
            let publisher = Self {
                root,
                state,
                lock,
                maintenance_marker,
                epoch_fence,
                logical_prefix,
                store,
                observer,
            };
            publisher.recover_locked()?;
            publisher.recover_generation_locked()?;
            if let Some(legacy_state) = legacy_state.as_deref() {
                publisher.migrate_legacy_state_locked(legacy_state)?;
            }
            Ok(publisher)
        })();
        let unlock = FileExt::unlock(&lock_file);
        result.and_then(|publisher| unlock.map(|()| publisher).map_err(PublishError::Io))
    }

    /// 返回规范化发布根目录。 / Returns the canonical publication root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 返回规范化私有状态根。 / Returns the canonical private-state root.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state
    }

    /// 返回用于协调发布和清理的锁文件路径。 /
    /// Returns the lock-file path coordinating publication and cleanup.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock
    }

    /// 在共享项目锁下把旧版内嵌状态复制到当前私有状态根。 /
    /// Copies legacy embedded state into the current private state root under the shared lock.
    ///
    /// 仅当新状态根为空时执行首次迁移。文件树先复制到新状态根同卷的临时目录，随后
    /// 切换、恢复并验证所有 current generation；验证成功前绝不删除 legacy tree。
    /// Migration starts only with an empty new state root. The tree is copied to a same-volume
    /// staging directory, switched, recovered, and fully validated before the legacy tree is
    /// removed.
    pub fn migrate_legacy_state(
        &self,
        legacy_state: impl AsRef<Path>,
    ) -> Result<bool, PublishError<S::Error>> {
        let legacy_state = legacy_state.as_ref();
        self.with_lock(|this| this.migrate_legacy_state_locked(legacy_state))
    }

    /// 发布并返回稳定协议产物。 / Publishes and returns the stable protocol artifact.
    pub fn publish_artifact(
        &self,
        publication: &Publication,
    ) -> Result<GenerationArtifact, PublishError<S::Error>> {
        self.with_lock(|this| {
            this.recover_locked()?;
            let relative = this.artifact_relative(&publication.destination)?;
            let destination = this.root.join(&relative);
            this.prepare_path(&relative)?;
            this.reject_physical_alias(&destination)?;
            if valid_file(
                &destination,
                publication.output.size,
                &publication.output.digest,
            )? {
                return artifact(publication);
            }

            let parent = destination
                .parent()
                .expect("validated destination has a parent");
            let temporary = NamedTempFile::new_in(parent)?;
            let temporary_name = temporary
                .path()
                .file_name()
                .expect("temporary file is named")
                .to_string_lossy()
                .into_owned();
            let journal = Journal {
                destination: publication.destination.as_str().to_owned(),
                temporary: temporary_name,
                size: publication.output.size,
                digest: publication.output.digest.clone(),
            };
            this.fill_and_verify(temporary.as_file(), &journal)?;
            // 仅在暂存字节通过完整大小与摘要校验后提交持久恢复意图；请求或来源错误
            // 因而不会污染之后的打开或发布。
            // Commit durable recovery intent only after the staged bytes pass complete
            // size and digest verification, so request/source errors cannot poison later work.
            this.write_journal(&journal)?;
            persist_replace(temporary, &destination)?;
            sync_parent(&destination, this.observer.as_ref())?;
            this.finish_journal()?;
            artifact(publication)
        })
    }

    /// 将一个 target 的全部产物作为单一 generation 发布。 / Publishes every artifact of one target as a single generation.
    ///
    /// 此方法先从 CAS 完整读取、校验并同步**全部**字节和 manifest，之后才写入持久
    /// commit decision。current manifest 的一次原子替换是唯一提交点；所以通过
    /// [`Self::current_generation`] 读取的调用者不会把不同 generation 的文件混合起来。
    /// The method completely reads, verifies, and synchronizes **all** CAS bytes and the
    /// manifest before recording a durable commit decision. One atomic replacement of the
    /// current manifest is the sole commit point, so [`Self::current_generation`] readers
    /// cannot combine files from different generations.
    ///
    /// # 示例 / Example
    /// ```no_run
    /// # use squish_publish::FileArtifactPublisher;
    /// # fn publish<S: squish_build::BlobStore>(p: &FileArtifactPublisher<S>, files: &[squish_build::Publication]) -> Result<(), squish_publish::PublishError<S::Error>> {
    /// let target = squish_build::PublicationTargetId::new("app").unwrap();
    /// let generation = p.publish_generation(&target, files)?;
    /// assert_eq!(generation.identity.target, target);
    /// # Ok(()) }
    /// ```
    pub fn publish_generation(
        &self,
        target_id: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, PublishError<S::Error>> {
        for attempt in 0..3 {
            let result = self.with_lock(|this| {
                this.recover_locked()?;
                this.recover_generation_locked()?;
                this.publish_generation_locked(target_id, publications)
            });
            match result {
                Err(PublishError::Io(error))
                    if attempt < 2 && error.kind() == io::ErrorKind::PermissionDenied =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                other => return other,
            }
        }
        unreachable!("bounded retry loop always returns")
    }

    /// 读取一个 target 最近完整提交的 generation。 / Reads the most recently committed complete generation for a target.
    pub fn current_generation(
        &self,
        target_id: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, PublishError<S::Error>> {
        let key = target_key(target_id);
        self.with_lock(|this| {
            this.recover_generation_locked()?;
            let path = this.current_manifest_path(&key);
            match fs::read(path) {
                Ok(bytes) => {
                    let manifest: GenerationManifest =
                        serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
                    this.validate_manifest(&key, Some(target_id), &manifest)
                        .map(Some)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            }
        })
    }

    /// 按类型化 generation 与逻辑路径读取并完整验证一个成员。 / Reads and fully verifies one member by typed generation and logical path.
    pub fn read_generation_artifact(
        &self,
        generation: &GenerationRef,
        destination: &PublicationPath,
        sink: &mut dyn Write,
    ) -> Result<ArtifactRead, PublishError<S::Error>> {
        let key = target_key(&generation.target);
        self.with_lock(|this| {
            this.recover_generation_locked()?;
            let manifest_path = this.generation_manifest_path(&key, generation.generation);
            let bytes = match fs::read(manifest_path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(ArtifactRead::NotFound);
                }
                Err(error) => return Err(error.into()),
            };
            let manifest: GenerationManifest =
                serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
            // 成员读取只校验 manifest 的身份与集合结构，再读取并校验所选字节。
            // Member reads validate manifest identity and membership structure, then read and
            // verify only the selected bytes. `current_generation` already performs the one
            // full-generation validation required for snapshot discovery.
            let committed = this.decode_manifest(&key, Some(&generation.target), &manifest)?;
            if committed.identity != *generation {
                return Err(PublishError::IntegrityMismatch);
            }
            let Some(member) = committed
                .artifacts
                .into_iter()
                .find(|item| &item.path == destination)
            else {
                return Ok(ArtifactRead::NotFound);
            };
            let path = this.generation_artifact_path(&key, generation.generation, destination);
            let mut file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(PublishError::IntegrityMismatch);
                }
                Err(error) => return Err(error.into()),
            };
            let mut verifier = VerifyingWriter::new(sink, &member.descriptor.digest)
                .map_err(PublishError::UnsupportedDigest)?;
            io::copy(&mut file, &mut verifier)?;
            let (size, matches) = verifier.finish();
            if size != member.descriptor.size || !matches {
                return Err(PublishError::IntegrityMismatch);
            }
            Ok(ArtifactRead::Verified(member.descriptor))
        })
    }

    fn generation_manifest_path(&self, key: &str, generation: GenerationId) -> PathBuf {
        self.state
            .join(GENERATIONS)
            .join(key)
            .join(generation.to_hex())
            .join("manifest.json")
    }

    fn generation_artifact_path(
        &self,
        key: &str,
        generation: GenerationId,
        destination: &PublicationPath,
    ) -> PathBuf {
        self.state
            .join(GENERATIONS)
            .join(key)
            .join(generation.to_hex())
            .join("artifacts")
            .join(destination.as_str())
    }

    fn validate_manifest(
        &self,
        key: &str,
        expected_target: Option<&PublicationTargetId>,
        manifest: &GenerationManifest,
    ) -> Result<CommittedGeneration, PublishError<S::Error>> {
        let committed = self.decode_manifest(key, expected_target, manifest)?;
        for artifact in &committed.artifacts {
            if !valid_file(
                &self.generation_artifact_path(key, committed.identity.generation, &artifact.path),
                artifact.descriptor.size,
                &artifact.descriptor.digest,
            )? {
                return Err(PublishError::IntegrityMismatch);
            }
        }
        Ok(committed)
    }

    fn decode_manifest(
        &self,
        key: &str,
        expected_target: Option<&PublicationTargetId>,
        manifest: &GenerationManifest,
    ) -> Result<CommittedGeneration, PublishError<S::Error>> {
        let target = PublicationTargetId::new(manifest.target_id.clone())
            .map_err(|_| PublishError::IntegrityMismatch)?;
        if expected_target.is_some_and(|expected| expected != &target) || target_key(&target) != key
        {
            return Err(PublishError::IntegrityMismatch);
        }
        let generation = GenerationId::from_hex(&manifest.generation_id)
            .map_err(|_| PublishError::IntegrityMismatch)?;
        let mut artifacts = Vec::with_capacity(manifest.artifacts.len());
        for stored in &manifest.artifacts {
            let path = PublicationPath::new(stored.destination.clone())
                .map_err(|_| PublishError::IntegrityMismatch)?;
            let descriptor = ArtifactDescriptor {
                id: ArtifactId::new(stored.id.clone())
                    .map_err(|_| PublishError::IntegrityMismatch)?,
                name: LogicalArtifactName::new(stored.name.clone())
                    .map_err(|_| PublishError::IntegrityMismatch)?,
                kind: stored.kind.clone(),
                digest: stored.digest.clone(),
                size: stored.size,
            };
            artifacts.push(GenerationArtifact { descriptor, path });
        }
        if !artifacts.windows(2).all(|pair| {
            (&pair[0].path, &pair[0].descriptor.id) < (&pair[1].path, &pair[1].descriptor.id)
        }) {
            return Err(PublishError::IntegrityMismatch);
        }
        validate_artifact_members(&artifacts)?;
        let computed = generation_id_from_artifacts(&target, &artifacts);
        if computed != generation {
            return Err(PublishError::IntegrityMismatch);
        }
        Ok(CommittedGeneration {
            identity: GenerationRef { target, generation },
            artifacts,
        })
    }

    fn publish_generation_locked(
        &self,
        target_id: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, PublishError<S::Error>> {
        let key = target_key(target_id);
        let mut publications = publications.to_vec();
        publications.sort_by(|left, right| {
            left.destination
                .cmp(&right.destination)
                .then_with(|| left.output.name.cmp(&right.output.name))
        });
        let destinations = validate_generation_destinations(&publications)?;
        validate_generation_members(&publications)?;
        let generation_id = generation_id(target_id, &publications);
        let target_dir = self.state.join(GENERATIONS).join(&key);
        create_directory_tree(&target_dir, self.observer.as_ref())?;

        let temporary = tempfile::Builder::new()
            .prefix(".stage-")
            .tempdir_in(&target_dir)?;
        let artifact_root = temporary.path().join("artifacts");
        fs::create_dir(&artifact_root)?;
        let mut artifacts = Vec::with_capacity(publications.len());
        let mut manifest_artifacts = Vec::with_capacity(publications.len());
        for (publication, relative) in publications.iter().zip(&destinations) {
            let path = artifact_root.join(relative);
            create_directory_tree(
                path.parent().expect("validated artifact path has parent"),
                self.observer.as_ref(),
            )?;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            let verification = Journal {
                destination: publication.destination.as_str().to_owned(),
                temporary: String::new(),
                size: publication.output.size,
                digest: publication.output.digest.clone(),
            };
            self.fill_and_verify(&file, &verification)?;
            self.observer
                .observe(&PublishEvent::DurablePoint(DurablePoint::ArtifactStaged));
            let published = artifact(publication)?;
            manifest_artifacts.push(manifest_artifact(&published));
            artifacts.push(published);
        }

        let manifest = GenerationManifest {
            target_id: target_id.as_str().to_owned(),
            generation_id: generation_id.to_hex(),
            artifacts: manifest_artifacts,
        };
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(PublishError::Journal)?;
        let manifest_path = temporary.path().join("manifest.json");
        write_new_synced(&manifest_path, &manifest_bytes)?;
        self.observer
            .observe(&PublishEvent::DurablePoint(DurablePoint::ManifestStaged));
        sync_tree_directories(temporary.path(), self.observer.as_ref())?;

        let final_dir = target_dir.join(generation_id.to_hex());
        if final_dir.exists() {
            let existing = fs::read(final_dir.join("manifest.json"))?;
            if existing != manifest_bytes {
                return Err(PublishError::IntegrityMismatch);
            }
            for (publication, path) in publications.iter().zip(&destinations) {
                if !valid_file(
                    &final_dir.join("artifacts").join(path),
                    publication.output.size,
                    &publication.output.digest,
                )? {
                    return Err(PublishError::IntegrityMismatch);
                }
            }
        } else {
            let staged = temporary.keep();
            fs::rename(&staged, &final_dir)?;
            sync_directory(&target_dir, self.observer.as_ref())?;
        }
        self.observer
            .observe(&PublishEvent::DurablePoint(DurablePoint::GenerationStaged));

        self.write_generation_journal(&GenerationJournal {
            target_key: key.clone(),
            generation_id: generation_id.to_hex(),
        })?;
        self.observer
            .observe(&PublishEvent::DurablePoint(DurablePoint::CommitDecision));
        self.materialize_generation(&key, &manifest)?;
        self.commit_current(&key, &manifest_bytes)?;
        self.observer
            .observe(&PublishEvent::DurablePoint(DurablePoint::CurrentSwitched));
        self.finish_generation_journal()?;
        self.observer
            .observe(&PublishEvent::DurablePoint(DurablePoint::JournalCleared));
        Ok(CommittedGeneration {
            identity: GenerationRef {
                target: target_id.clone(),
                generation: generation_id,
            },
            artifacts,
        })
    }

    fn with_lock<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, PublishError<S::Error>>,
    ) -> Result<T, PublishError<S::Error>> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock)?;
        lock.lock_exclusive()?;
        // clean 可以在 publisher 实例仍被缓存时删除 root/state；每次操作都在同一项目
        // 锁内恢复这些基础目录，既不与 clean 竞态，也不要求上层丢弃运行时对象。
        // Clean may remove root/state while this publisher remains cached. Re-establish the
        // base directories under the same project lock so operations neither race clean nor
        // require the caller to discard its runtime object.
        let result = validate_fences(&self.maintenance_marker, &self.epoch_fence)
            .and_then(|()| self.ensure_roots())
            .and_then(|()| operation(self));
        let unlock = FileExt::unlock(&lock);
        result.and_then(|value| unlock.map(|()| value).map_err(PublishError::Io))
    }

    fn ensure_roots(&self) -> Result<(), PublishError<S::Error>> {
        create_directory_tree(&self.root, self.observer.as_ref())?;
        create_directory_tree(&self.state, self.observer.as_ref())?;
        reject_symlink(&self.root)?;
        reject_symlink(&self.state)
    }

    fn migrate_legacy_state_locked(
        &self,
        legacy_state: &Path,
    ) -> Result<bool, PublishError<S::Error>> {
        let marker = self.state.join(LEGACY_MIGRATION);
        if marker.exists() {
            let validation = (|| {
                self.recover_locked()?;
                self.recover_generation_locked()?;
                self.validate_current_generations()
            })();
            if let Err(error) = validation {
                fs::remove_dir_all(&self.state).ok();
                create_directory_tree(&self.state, self.observer.as_ref())?;
                return Err(error);
            }
            remove_legacy_tree(legacy_state)?;
            sync_parent(legacy_state, self.observer.as_ref())?;
            fs::remove_file(&marker)?;
            sync_directory(&self.state, self.observer.as_ref())?;
            return Ok(true);
        }
        if !legacy_state.exists() || fs::read_dir(&self.state)?.next().is_some() {
            return Ok(false);
        }
        reject_symlink(legacy_state)?;
        let parent = self.state.parent().expect("state root has a parent");
        let staging = tempfile::Builder::new()
            .prefix(".legacy-state-")
            .tempdir_in(parent)?;
        copy_directory_tree(legacy_state, staging.path())?;
        write_new_synced(&staging.path().join(LEGACY_MIGRATION), b"copied\n")?;
        sync_tree_directories(staging.path(), self.observer.as_ref())?;
        let staged = staging.keep();
        fs::remove_dir(&self.state)?;
        fs::rename(&staged, &self.state)?;
        sync_directory(parent, self.observer.as_ref())?;
        self.migrate_legacy_state_locked(legacy_state)
    }

    fn validate_current_generations(&self) -> Result<(), PublishError<S::Error>> {
        let targets = self.state.join(TARGETS);
        let entries = match fs::read_dir(targets) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(PublishError::IntegrityMismatch);
            }
            let key = entry.file_name().to_string_lossy().into_owned();
            if !is_key(&key) {
                return Err(PublishError::IntegrityMismatch);
            }
            let bytes = fs::read(entry.path().join("current.json"))?;
            let manifest: GenerationManifest =
                serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
            self.validate_manifest(&key, None, &manifest)?;
            self.materialize_generation(&key, &manifest)?;
        }
        Ok(())
    }

    fn recover_locked(&self) -> Result<(), PublishError<S::Error>> {
        let path = self.state.join(JOURNAL);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
        let locator = PublicationPath::new(journal.destination.clone())
            .map_err(|_| PublishError::InvalidDestination(journal.destination.clone()))?;
        let relative = self.artifact_relative(&locator)?;
        if !is_file_name(&journal.temporary) {
            return Err(PublishError::InvalidDestination(journal.temporary));
        }
        let destination = self.root.join(&relative);
        self.prepare_path(&relative)?;
        self.reject_physical_alias(&destination)?;
        if valid_file(&destination, journal.size, &journal.digest)? {
            self.remove_temporary(&destination, &journal);
            return self.finish_journal();
        }
        let temporary_path = destination
            .parent()
            .expect("validated destination has a parent")
            .join(&journal.temporary);
        if !valid_file(&temporary_path, journal.size, &journal.digest)? {
            let temporary = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temporary_path)?;
            self.fill_and_verify(&temporary, &journal)?;
        }
        // NamedTempFile 无法接管恢复出的名字；经由同目录的新临时文件发布，确保 Windows
        // 也使用原子覆盖语义。
        // NamedTempFile cannot adopt a recovered name; publish through a fresh
        // same-directory temporary so Windows also receives atomic replacement semantics.
        let mut staged =
            NamedTempFile::new_in(destination.parent().expect("destination has parent"))?;
        let mut source = File::open(&temporary_path)?;
        io::copy(&mut source, staged.as_file_mut())?;
        staged.as_file().sync_all()?;
        persist_replace(staged, &destination)?;
        fs::remove_file(temporary_path).ok();
        sync_parent(&destination, self.observer.as_ref())?;
        self.finish_journal()
    }

    fn recover_generation_locked(&self) -> Result<(), PublishError<S::Error>> {
        let path = self.state.join(GENERATION_JOURNAL);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let journal: GenerationJournal =
            serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
        if !is_key(&journal.target_key) || !is_key(&journal.generation_id) {
            return Err(PublishError::InvalidDestination(
                "malformed generation journal identity".into(),
            ));
        }
        let manifest = self
            .state
            .join(GENERATIONS)
            .join(&journal.target_key)
            .join(&journal.generation_id)
            .join("manifest.json");
        let bytes = fs::read(manifest)?;
        let parsed: GenerationManifest =
            serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
        let target = PublicationTargetId::new(parsed.target_id.clone())
            .map_err(|_| PublishError::IntegrityMismatch)?;
        if parsed.generation_id != journal.generation_id
            || target_key(&target) != journal.target_key
        {
            return Err(PublishError::IntegrityMismatch);
        }
        self.validate_manifest(&journal.target_key, Some(&target), &parsed)?;
        self.materialize_generation(&journal.target_key, &parsed)?;
        self.commit_current(&journal.target_key, &bytes)?;
        self.finish_generation_journal()
    }

    fn write_generation_journal(
        &self,
        journal: &GenerationJournal,
    ) -> Result<(), PublishError<S::Error>> {
        let mut temporary = NamedTempFile::new_in(&self.state)?;
        serde_json::to_writer(&mut temporary, journal).map_err(PublishError::Journal)?;
        temporary.as_file_mut().flush()?;
        temporary.as_file().sync_all()?;
        persist_replace(temporary, &self.state.join(GENERATION_JOURNAL))?;
        sync_directory(&self.state, self.observer.as_ref())
    }

    fn finish_generation_journal(&self) -> Result<(), PublishError<S::Error>> {
        match fs::remove_file(self.state.join(GENERATION_JOURNAL)) {
            Ok(()) => sync_directory(&self.state, self.observer.as_ref()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn current_manifest_path(&self, key: &str) -> PathBuf {
        self.state.join(TARGETS).join(key).join("current.json")
    }

    fn commit_current(&self, key: &str, bytes: &[u8]) -> Result<(), PublishError<S::Error>> {
        let target = self.state.join(TARGETS).join(key);
        create_directory_tree(&target, self.observer.as_ref())?;
        let mut temporary = NamedTempFile::new_in(&target)?;
        temporary.write_all(bytes)?;
        temporary.as_file_mut().flush()?;
        temporary.as_file().sync_all()?;
        persist_replace(temporary, &target.join("current.json"))?;
        sync_directory(&target, self.observer.as_ref())
    }

    /// 将私有 generation 的声明成员投影到稳定用户路径，并移除上一 generation
    /// 独有的目标。journal 在调用此函数前已经持久化，因此任一逐文件替换中断后都可
    /// 在下次打开时幂等续作。
    /// Projects a private generation onto stable user paths and removes destinations owned
    /// only by the previous generation. The durable journal makes every per-file step
    /// idempotently recoverable.
    fn materialize_generation(
        &self,
        key: &str,
        manifest: &GenerationManifest,
    ) -> Result<(), PublishError<S::Error>> {
        let generation = GenerationId::from_hex(&manifest.generation_id)
            .map_err(|_| PublishError::IntegrityMismatch)?;
        let previous = match fs::read(self.current_manifest_path(key)) {
            Ok(bytes) => Some(
                serde_json::from_slice::<GenerationManifest>(&bytes)
                    .map_err(PublishError::Journal)?,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };

        for artifact in manifest
            .artifacts
            .iter()
            .filter(|artifact| is_user_artifact(&artifact.kind))
        {
            let locator = PublicationPath::new(artifact.destination.clone())
                .map_err(|_| PublishError::IntegrityMismatch)?;
            let relative = self.artifact_relative(&locator)?;
            let destination = self.root.join(&relative);
            self.prepare_path(&relative)?;
            self.reject_physical_alias(&destination)?;
            if valid_file(&destination, artifact.size, &artifact.digest)? {
                continue;
            }
            let source = self
                .state
                .join(GENERATIONS)
                .join(key)
                .join(generation.to_hex())
                .join("artifacts")
                .join(locator.as_str());
            let parent = destination
                .parent()
                .expect("validated destination has a parent");
            let mut staged = NamedTempFile::new_in(parent)?;
            let mut source = File::open(source)?;
            io::copy(&mut source, staged.as_file_mut())?;
            staged.as_file_mut().flush()?;
            staged.as_file().sync_all()?;
            if !valid_file(staged.path(), artifact.size, &artifact.digest)? {
                return Err(PublishError::IntegrityMismatch);
            }
            persist_replace(staged, &destination)?;
            sync_parent(&destination, self.observer.as_ref())?;
        }

        if let Some(previous) = previous {
            let retained = manifest
                .artifacts
                .iter()
                .filter(|artifact| is_user_artifact(&artifact.kind))
                .map(|artifact| artifact.destination.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            for artifact in previous
                .artifacts
                .into_iter()
                .filter(|artifact| is_user_artifact(&artifact.kind))
            {
                if retained.contains(artifact.destination.as_str()) {
                    continue;
                }
                let locator = PublicationPath::new(artifact.destination)
                    .map_err(|_| PublishError::IntegrityMismatch)?;
                let path = self.root.join(self.artifact_relative(&locator)?);
                match fs::remove_file(&path) {
                    Ok(()) => {
                        sync_parent(&path, self.observer.as_ref())?;
                        remove_empty_parents(path.parent(), &self.root)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    fn artifact_relative(
        &self,
        destination: &PublicationPath,
    ) -> Result<PathBuf, PublishError<S::Error>> {
        let path = Path::new(destination.as_str());
        let relative = if self.logical_prefix.as_os_str().is_empty() {
            path
        } else {
            path.strip_prefix(&self.logical_prefix).map_err(|_| {
                PublishError::InvalidDestination(format!(
                    "{} is outside logical prefix {}",
                    destination.as_str(),
                    self.logical_prefix.display()
                ))
            })?
        };
        if relative.as_os_str().is_empty() {
            return Err(PublishError::InvalidDestination(
                destination.as_str().to_owned(),
            ));
        }
        Ok(relative.to_owned())
    }

    fn prepare_path(&self, relative: &Path) -> Result<(), PublishError<S::Error>> {
        let mut current = self.root.clone();
        let count = relative.components().count();
        for (index, component) in relative.components().enumerate() {
            let Component::Normal(name) = component else {
                unreachable!("destination was validated")
            };
            reject_case_collision(&current, name)?;
            current.push(name);
            if current.exists() {
                reject_symlink(&current)?;
                if index + 1 < count && !current.is_dir() {
                    return Err(PublishError::Io(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        current.display().to_string(),
                    )));
                }
            } else if index + 1 < count {
                fs::create_dir(&current)?;
                self.observer
                    .observe(&PublishEvent::DirectoryCreated(current.clone()));
                sync_directory(
                    current.parent().expect("created directory has parent"),
                    self.observer.as_ref(),
                )?;
                reject_symlink(&current)?;
            }
        }
        Ok(())
    }

    fn reject_physical_alias(&self, destination: &Path) -> Result<(), PublishError<S::Error>> {
        if !destination.exists() {
            return Ok(());
        }
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory)? {
                let path = entry?.path();
                if path == destination {
                    continue;
                }
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    pending.push(path);
                    continue;
                }
                if same_file::is_same_file(&path, destination).unwrap_or(false) {
                    return Err(PublishError::AliasConflict(path));
                }
            }
        }
        Ok(())
    }

    fn fill_and_verify(
        &self,
        file: &File,
        journal: &Journal,
    ) -> Result<(), PublishError<S::Error>> {
        file.set_len(0)?;
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        let mut verifier = VerifyingWriter::new(&mut file, &journal.digest)
            .map_err(PublishError::UnsupportedDigest)?;
        if !self
            .store
            .copy_to(&journal.digest, &mut verifier)
            .map_err(PublishError::Store)?
        {
            return Err(PublishError::MissingBlob);
        }
        let (size, matches) = verifier.finish();
        if size != journal.size || !matches {
            return Err(PublishError::IntegrityMismatch);
        }
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }

    fn write_journal(&self, journal: &Journal) -> Result<(), PublishError<S::Error>> {
        let mut temporary = NamedTempFile::new_in(&self.state)?;
        serde_json::to_writer(&mut temporary, journal).map_err(PublishError::Journal)?;
        temporary.as_file_mut().flush()?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(self.state.join(JOURNAL))
            .map_err(|error| PublishError::Io(error.error))?;
        sync_directory(&self.state, self.observer.as_ref())
    }

    fn finish_journal(&self) -> Result<(), PublishError<S::Error>> {
        match fs::remove_file(self.state.join(JOURNAL)) {
            Ok(()) => sync_directory(&self.state, self.observer.as_ref()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn remove_temporary(&self, destination: &Path, journal: &Journal) {
        if let Some(parent) = destination.parent() {
            fs::remove_file(parent.join(&journal.temporary)).ok();
        }
    }
}

impl<S: BlobStore> ArtifactPublisher for FileArtifactPublisher<S> {
    type Error = PublishError<S::Error>;
    fn publish(&self, publication: &Publication) -> Result<(), Self::Error> {
        self.publish_artifact(publication).map(|_| ())
    }
}

impl<S> GenerationRepository for FileArtifactPublisher<S>
where
    S: BlobStore + Send + Sync,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    type Error = PublishError<S::Error>;

    fn publish_generation(
        &self,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, Self::Error> {
        FileArtifactPublisher::publish_generation(self, target, publications)
    }

    fn current_generation(
        &self,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, Self::Error> {
        FileArtifactPublisher::current_generation(self, target)
    }

    fn read_generation_artifact(
        &self,
        generation: &GenerationRef,
        destination: &PublicationPath,
        sink: &mut dyn Write,
    ) -> Result<ArtifactRead, Self::Error> {
        FileArtifactPublisher::read_generation_artifact(self, generation, destination, sink)
    }
}

fn artifact<E>(publication: &Publication) -> Result<GenerationArtifact, PublishError<E>> {
    Ok(GenerationArtifact {
        descriptor: ArtifactDescriptor {
            id: ArtifactId::new(publication.output.name.as_str()).expect("OutputName is non-empty"),
            name: publication.name.clone(),
            kind: publication.output.kind.clone(),
            digest: publication.output.digest.clone(),
            size: publication.output.size,
        },
        path: publication.destination.clone(),
    })
}

fn manifest_artifact(artifact: &GenerationArtifact) -> ManifestArtifact {
    ManifestArtifact {
        id: artifact.descriptor.id.as_str().into(),
        name: artifact.descriptor.name.as_str().into(),
        kind: artifact.descriptor.kind.clone(),
        destination: artifact.path.as_str().into(),
        size: artifact.descriptor.size,
        digest: artifact.descriptor.digest.clone(),
    }
}

fn is_user_artifact(kind: &squish_protocol::ArtifactKind) -> bool {
    matches!(
        kind,
        squish_protocol::ArtifactKind::Prompt
            | squish_protocol::ArtifactKind::BinaryIr
            | squish_protocol::ArtifactKind::DebugInfo
    )
}

fn target_key(target_id: &PublicationTargetId) -> String {
    hex_bytes(&Sha256::digest(target_id.as_str().as_bytes()))
}

fn generation_id(target_id: &PublicationTargetId, publications: &[Publication]) -> GenerationId {
    let artifacts = publications
        .iter()
        .map(|publication| {
            artifact::<std::convert::Infallible>(publication).expect("typed publication is valid")
        })
        .collect::<Vec<_>>();
    generation_id_from_artifacts(target_id, &artifacts)
}

fn generation_id_from_artifacts(
    target_id: &PublicationTargetId,
    artifacts: &[GenerationArtifact],
) -> GenerationId {
    let mut hasher = Sha256::new();
    feed_field(&mut hasher, b"xmlsquish-published-generation-v1");
    feed_field(&mut hasher, target_id.as_str().as_bytes());
    hasher.update((artifacts.len() as u64).to_le_bytes());
    for artifact in artifacts {
        feed_field(&mut hasher, artifact.path.as_str().as_bytes());
        feed_field(&mut hasher, artifact.descriptor.id.as_str().as_bytes());
        feed_field(&mut hasher, artifact.descriptor.name.as_str().as_bytes());
        feed_field(
            &mut hasher,
            serde_json::to_string(&artifact.descriptor.kind)
                .expect("ArtifactKind serialization is infallible")
                .as_bytes(),
        );
        feed_field(
            &mut hasher,
            serde_json::to_string(&artifact.descriptor.digest)
                .expect("Digest serialization is infallible")
                .as_bytes(),
        );
        hasher.update(artifact.descriptor.size.to_le_bytes());
    }
    GenerationId::from_bytes(hasher.finalize().into())
}

fn feed_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0xf) as usize] as char);
    }
    result
}

fn is_key(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_generation_destinations<E>(
    publications: &[Publication],
) -> Result<Vec<PathBuf>, PublishError<E>> {
    let mut destinations = Vec::with_capacity(publications.len());
    let mut normalized = Vec::<(String, PathBuf)>::with_capacity(publications.len());
    for publication in publications {
        let path = PathBuf::from(publication.destination.as_str());
        let folded = path
            .iter()
            .map(portable_component_key)
            .collect::<Vec<_>>()
            .join("/");
        for (prior, prior_path) in &normalized {
            if folded == *prior
                || folded
                    .strip_prefix(prior)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || prior
                    .strip_prefix(&folded)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
                return Err(PublishError::AliasConflict(prior_path.clone()));
            }
        }
        normalized.push((folded, path.clone()));
        destinations.push(path);
    }
    Ok(destinations)
}

fn validate_generation_members<E>(publications: &[Publication]) -> Result<(), PublishError<E>> {
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for publication in publications {
        if !ids.insert(publication.output.name.as_str()) || !names.insert(publication.name.as_str())
        {
            return Err(PublishError::InvalidDestination(
                "generation artifact IDs and logical names must be unique".into(),
            ));
        }
    }
    Ok(())
}

fn validate_artifact_members<E>(artifacts: &[GenerationArtifact]) -> Result<(), PublishError<E>> {
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut paths = Vec::<String>::new();
    for artifact in artifacts {
        if !ids.insert(artifact.descriptor.id.as_str())
            || !names.insert(artifact.descriptor.name.as_str())
        {
            return Err(PublishError::IntegrityMismatch);
        }
        let folded = Path::new(artifact.path.as_str())
            .iter()
            .map(portable_component_key)
            .collect::<Vec<_>>()
            .join("/");
        if paths.iter().any(|prior| {
            folded == *prior
                || folded
                    .strip_prefix(prior)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || prior
                    .strip_prefix(&folded)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            return Err(PublishError::IntegrityMismatch);
        }
        paths.push(folded);
    }
    Ok(())
}

fn write_new_synced<E>(path: &Path, bytes: &[u8]) -> Result<(), PublishError<E>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn sync_tree_directories<E>(
    root: &Path,
    observer: &dyn PublishObserver,
) -> Result<(), PublishError<E>> {
    let mut directories = vec![root.to_path_buf()];
    let mut index = 0;
    while index < directories.len() {
        for entry in fs::read_dir(&directories[index])? {
            let path = entry?.path();
            if path.is_dir() {
                directories.push(path);
            }
        }
        index += 1;
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory, observer)?;
    }
    Ok(())
}

fn parse_destination(value: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.starts_with(['/', '\\']) {
        return Err(value.to_owned());
    }
    let pieces: Vec<_> = value.split(['/', '\\']).collect();
    if pieces.iter().any(|piece| {
        piece.is_empty()
            || *piece == "."
            || *piece == ".."
            || piece.chars().any(|character| {
                character.is_control()
                    || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            })
            || piece.trim_end_matches([' ', '.']).is_empty()
            || is_windows_device_name(piece)
    }) || pieces.first().is_some_and(|piece| piece.ends_with(':'))
        || pieces.first() == Some(&STATE_DIR)
    {
        return Err(value.to_owned());
    }
    let path = pieces.iter().collect::<PathBuf>();
    if path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        Ok(path)
    } else {
        Err(value.to_owned())
    }
}

fn validate_logical_prefix<E>(value: &Path) -> Result<PathBuf, PublishError<E>> {
    if value.as_os_str().is_empty() {
        return Ok(PathBuf::new());
    }
    let portable = value.to_string_lossy().replace('\\', "/");
    parse_destination(&portable).map_err(PublishError::InvalidDestination)
}

fn remove_empty_parents<E>(
    mut directory: Option<&Path>,
    root: &Path,
) -> Result<(), PublishError<E>> {
    while let Some(path) = directory {
        if path == root {
            break;
        }
        match fs::remove_dir(path) {
            Ok(()) => directory = path.parent(),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
                ) =>
            {
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn copy_directory_tree<E>(source: &Path, destination: &Path) -> Result<(), PublishError<E>> {
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let target = destination.join(entry.file_name());
            if file_type.is_symlink() {
                return Err(PublishError::Symlink(entry.path()));
            }
            if file_type.is_dir() {
                fs::create_dir(&target)?;
                pending.push((entry.path(), target));
                continue;
            }
            if !file_type.is_file() {
                return Err(PublishError::IntegrityMismatch);
            }
            let mut input = File::open(entry.path())?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            io::copy(&mut input, &mut output)?;
            output.flush()?;
            output.sync_all()?;
        }
    }
    Ok(())
}

fn remove_legacy_tree<E>(path: &Path) -> Result<(), PublishError<E>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(PublishError::Symlink(path.to_path_buf()))
        }
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path).map_err(PublishError::Io),
        Ok(_) => Err(PublishError::IntegrityMismatch),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn reject_pending_maintenance<E>(marker: &Option<PathBuf>) -> Result<(), PublishError<E>> {
    let Some(marker) = marker else {
        return Ok(());
    };
    match fs::symlink_metadata(marker) {
        Ok(_) => Err(PublishError::MaintenancePending(marker.clone())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_fences<E>(
    marker: &Option<PathBuf>,
    epoch_fence: &Option<(PathBuf, ProjectEpoch)>,
) -> Result<(), PublishError<E>> {
    reject_pending_maintenance(marker)?;
    let Some((path, expected)) = epoch_fence else {
        return Ok(());
    };
    let observed = read_project_epoch_path(path)?;
    if &observed != expected {
        return Err(PublishError::Superseded(path.clone()));
    }
    Ok(())
}

fn portable_component_key(value: &OsStr) -> String {
    value
        .to_string_lossy()
        .trim_end_matches([' ', '.'])
        .to_lowercase()
}

fn is_windows_device_name(value: &str) -> bool {
    let stem = value
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or_default();
    matches!(
        stem.to_ascii_uppercase().as_str(),
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
    )
}

fn is_file_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['/', '\\'])
        && !value.ends_with(':')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && Path::new(value).components().count() == 1
}

fn reject_case_collision<E>(directory: &Path, requested: &OsStr) -> Result<(), PublishError<E>> {
    let requested_text = requested.to_string_lossy();
    for entry in match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    } {
        let name = entry?.file_name();
        if name != requested
            && name.to_string_lossy().to_lowercase() == requested_text.to_lowercase()
        {
            return Err(PublishError::AliasConflict(directory.join(name)));
        }
    }
    Ok(())
}

fn reject_symlink<E>(path: &Path) -> Result<(), PublishError<E>> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        Err(PublishError::Symlink(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn valid_file<E>(path: &Path, size: u64, digest: &Digest) -> Result<bool, PublishError<E>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() != size {
        return Ok(false);
    }
    let mut verifier =
        VerifyingWriter::new(io::sink(), digest).map_err(PublishError::UnsupportedDigest)?;
    io::copy(&mut file, &mut verifier)?;
    Ok(verifier.finish().1)
}

enum Hasher {
    Blake3(Box<blake3::Hasher>),
    Sha256(Sha256),
}
struct VerifyingWriter<W> {
    inner: W,
    hasher: Hasher,
    expected: Vec<u8>,
    size: u64,
}
impl<W: Write> VerifyingWriter<W> {
    fn new(inner: W, digest: &Digest) -> Result<Self, String> {
        let hasher = match digest.algorithm() {
            DigestAlgorithm::Blake3 => Hasher::Blake3(Box::default()),
            DigestAlgorithm::Sha256 => Hasher::Sha256(Sha256::new()),
            DigestAlgorithm::Other(name) => return Err(name.clone()),
        };
        Ok(Self {
            inner,
            hasher,
            expected: digest.bytes().to_vec(),
            size: 0,
        })
    }
    fn finish(self) -> (u64, bool) {
        let actual = match self.hasher {
            Hasher::Blake3(value) => value.finalize().as_bytes().to_vec(),
            Hasher::Sha256(value) => value.finalize().to_vec(),
        };
        (self.size, actual == self.expected)
    }
}
impl<W: Write> Write for VerifyingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        match &mut self.hasher {
            Hasher::Blake3(value) => {
                value.update(&bytes[..written]);
            }
            Hasher::Sha256(value) => {
                value.update(&bytes[..written]);
            }
        }
        self.size = self
            .size
            .checked_add(written as u64)
            .ok_or_else(|| io::Error::other("publication size overflow"))?;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn create_directory_tree<E>(
    path: &Path,
    observer: &dyn PublishObserver,
) -> Result<(), PublishError<E>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => return validate_directory_component(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_directory_tree(parent, observer)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {
            observer.observe(&PublishEvent::DirectoryCreated(path.to_path_buf()));
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                sync_directory(parent, observer)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            validate_directory_component(path, &metadata)
        }
        Err(error) => Err(error.into()),
    }
}

fn validate_directory_component<E>(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), PublishError<E>> {
    if metadata.file_type().is_symlink() || is_reparse_point(metadata) {
        return Err(PublishError::Symlink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(PublishError::Io(io::Error::new(
            io::ErrorKind::NotADirectory,
            path.display().to_string(),
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn sync_parent<E>(path: &Path, observer: &dyn PublishObserver) -> Result<(), PublishError<E>> {
    sync_directory(path.parent().expect("published path has parent"), observer)
}

/// 以同目录原子覆盖提交临时文件。 / Commits a same-directory temporary by atomic replacement.
///
/// `NamedTempFile::persist` 在 Windows 使用允许覆盖的 `MoveFileExW`，而不是不能覆盖既有
/// 目标的 `std::fs::rename`；Unix 则使用同文件系统的 rename。
/// On Windows, `NamedTempFile::persist` uses replacement-capable `MoveFileExW` rather than
/// `std::fs::rename`, which cannot overwrite an existing target; Unix uses same-filesystem rename.
fn persist_replace<E>(temporary: NamedTempFile, destination: &Path) -> Result<(), PublishError<E>> {
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| PublishError::Io(error.error))
}

#[cfg(unix)]
fn sync_directory<E>(path: &Path, observer: &dyn PublishObserver) -> Result<(), PublishError<E>> {
    File::open(path)?.sync_all()?;
    observer.observe(&PublishEvent::DirectoryDurability {
        path: path.to_path_buf(),
        guarantee: DirectoryDurability::Synced,
    });
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory<E>(path: &Path, observer: &dyn PublishObserver) -> Result<(), PublishError<E>> {
    // Rust 的安全 Windows 标准库 API 不能用 FILE_FLAG_BACKUP_SEMANTICS 打开目录供
    // FlushFileBuffers 使用。文件本身在原子覆盖前已同步；显式报告较弱的元数据保证。
    // Rust's safe Windows standard-library API cannot open a directory with
    // FILE_FLAG_BACKUP_SEMANTICS for FlushFileBuffers. Files themselves are synced
    // before atomic replacement; expose the weaker metadata guarantee explicitly.
    observer.observe(&PublishEvent::DirectoryDurability {
        path: path.to_path_buf(),
        guarantee: DirectoryDurability::Unsupported,
    });
    Ok(())
}

#[cfg(test)]
mod tests;
