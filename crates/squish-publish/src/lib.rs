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

/// 文件发布失败。 / Filesystem publication failure.
#[derive(Debug)]
pub enum PublishError<E> {
    /// 目标不是可移植的、根目录内的相对路径。 / Destination is not a portable root-relative path.
    InvalidDestination(String),
    /// 目标与已有大小写拼写或物理身份冲突。 / Destination conflicts by case spelling or physical identity.
    AliasConflict(PathBuf),
    /// 路径经过符号链接。 / A path traverses a symbolic link.
    Symlink(PathBuf),
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
    store: S,
    observer: Arc<dyn PublishObserver>,
}

impl<S: BlobStore> FileArtifactPublisher<S> {
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
        create_directory_tree(root.as_ref(), observer.as_ref())?;
        let root = fs::canonicalize(root.as_ref())?;
        let state = root.join(STATE_DIR);
        create_directory_tree(&state, observer.as_ref())?;
        reject_symlink(&state)?;
        let publisher = Self {
            root,
            state,
            store,
            observer,
        };
        publisher.with_lock(|this| {
            this.recover_locked()?;
            this.recover_generation_locked()
        })?;
        Ok(publisher)
    }

    /// 返回规范化发布根目录。 / Returns the canonical publication root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 发布并返回稳定协议产物。 / Publishes and returns the stable protocol artifact.
    pub fn publish_artifact(
        &self,
        publication: &Publication,
    ) -> Result<GenerationArtifact, PublishError<S::Error>> {
        self.with_lock(|this| {
            this.recover_locked()?;
            let relative = PathBuf::from(publication.destination.as_str());
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
            .open(self.state.join(LOCK))?;
        lock.lock_exclusive()?;
        let result = operation(self);
        let unlock = FileExt::unlock(&lock);
        result.and_then(|value| unlock.map(|()| value).map_err(PublishError::Io))
    }

    fn recover_locked(&self) -> Result<(), PublishError<S::Error>> {
        let path = self.state.join(JOURNAL);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes).map_err(PublishError::Journal)?;
        let relative =
            parse_destination(&journal.destination).map_err(PublishError::InvalidDestination)?;
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
    if path.exists() {
        return Ok(());
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
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
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
