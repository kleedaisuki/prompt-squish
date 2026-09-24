use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const FORMAT_VERSION: &str = "v1";
const ALGORITHM: &str = "blake3";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 一个 BLAKE3 内容摘要。 / A BLAKE3 content digest.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobDigest([u8; 32]);

impl BlobDigest {
    /// 计算字节内容的摘要。 / Computes the digest of the byte content.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// 从原始 32 字节构造摘要。 / Constructs a digest from its raw 32 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 返回原始摘要字节。 / Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 返回小写十六进制编码。 / Returns the lowercase hexadecimal encoding.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl fmt::Debug for BlobDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{ALGORITHM}:{}", self.to_hex())
    }
}

impl fmt::Display for BlobDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

impl FromStr for BlobDigest {
    type Err = CasError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hash = blake3::Hash::from_hex(value).map_err(|_| CasError::InvalidDigest)?;
        Ok(Self(*hash.as_bytes()))
    }
}

/// CAS 中可供观测的事件类别。 / Observable CAS event category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CasEventKind {
    /// 写入了一个此前不存在或已损坏的 blob。 / A missing or corrupt blob was written.
    Written,
    /// 并发或重复写入复用了已有 blob。 / A concurrent or duplicate write reused an existing blob.
    Reused,
    /// 读取发现内容与路径摘要不符，并将其视为缓存未命中。 / A read found a digest mismatch and treated it as a miss.
    CorruptMiss,
}

/// CAS 观测事件。 / An observable CAS event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CasEvent {
    /// 事件类别。 / Event category.
    pub kind: CasEventKind,
    /// 被访问的逻辑摘要。 / Logical digest being accessed.
    pub digest: BlobDigest,
    /// 损坏时实际读到的摘要。 / Actual digest read when corruption was detected.
    pub actual: Option<BlobDigest>,
}

/// 接收存储事件而不耦合到终端或日志框架。 / Receives storage events without coupling to a terminal or logging framework.
pub trait CasObserver: Send + Sync {
    /// 记录一个已完成的存储事件。 / Records one completed storage event.
    fn observe(&self, event: &CasEvent);
}

/// 默认的无操作观察者。 / Default no-op observer.
#[derive(Debug, Default)]
pub struct NoopObserver;
impl CasObserver for NoopObserver {
    fn observe(&self, _event: &CasEvent) {}
}

/// 文件系统 CAS 失败。 / Filesystem CAS failure.
#[derive(Debug)]
pub enum CasError {
    /// 文件系统操作失败。 / A filesystem operation failed.
    Io(io::Error),
    /// 摘要文本不是 64 位十六进制 BLAKE3 值。 / Digest text was not a 64-digit hexadecimal BLAKE3 value.
    InvalidDigest,
    /// 构建端口请求了 CAS 不支持的摘要算法。 / The build port requested a digest algorithm unsupported by this CAS.
    UnsupportedAlgorithm,
}

impl fmt::Display for CasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "content store I/O failed: {error}"),
            Self::InvalidDigest => formatter.write_str("invalid BLAKE3 digest"),
            Self::UnsupportedAlgorithm => {
                formatter.write_str("content store supports only BLAKE3 digests")
            }
        }
    }
}
impl std::error::Error for CasError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidDigest | Self::UnsupportedAlgorithm => None,
        }
    }
}
impl From<io::Error> for CasError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// 不可变、分桶的文件系统内容寻址存储。 / Immutable, bucketed filesystem content-addressed store.
///
/// 布局为 `blobs/v1/blake3/aa/<remaining-hex>`。写入先落到同目录临时文件，
/// `sync_all` 后通过原子重命名发布；读取总会重新计算摘要。
/// The layout is `blobs/v1/blake3/aa/<remaining-hex>`. Writes first reach a
/// same-directory temporary file, are `sync_all`ed, and are published with an
/// atomic rename; reads always recompute the digest.
///
/// # Example / 示例
/// ```
/// use squish_store::Cas;
/// # let root = std::env::temp_dir().join("squish-store-doc-example");
/// let store = Cas::open(&root)?;
/// let digest = store.put(b"hello")?;
/// assert_eq!(store.get(digest)?, Some(b"hello".to_vec()));
/// # std::fs::remove_dir_all(root).ok();
/// # Ok::<(), squish_store::CasError>(())
/// ```
pub struct Cas {
    root: PathBuf,
    observer: Arc<dyn CasObserver>,
}

impl fmt::Debug for Cas {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cas")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Cas {
    /// 打开或创建 CAS 根目录。 / Opens or creates a CAS root directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, CasError> {
        Self::with_observer(root, Arc::new(NoopObserver))
    }

    /// 打开 CAS 并安装结构化事件观察者。 / Opens a CAS with a structured event observer.
    pub fn with_observer(
        root: impl AsRef<Path>,
        observer: Arc<dyn CasObserver>,
    ) -> Result<Self, CasError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("blobs").join(FORMAT_VERSION).join(ALGORITHM))?;
        Ok(Self { root, observer })
    }

    /// 返回 CAS 根目录。 / Returns the CAS root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 返回摘要的物理路径；调用者不得直接修改它。 / Returns a digest's physical path; callers must not modify it directly.
    #[must_use]
    pub fn path_for(&self, digest: BlobDigest) -> PathBuf {
        let hex = digest.to_hex();
        self.root
            .join("blobs")
            .join(FORMAT_VERSION)
            .join(ALGORITHM)
            .join(&hex[..2])
            .join(&hex[2..])
    }

    /// 原子写入内容并返回其摘要；相同内容的并发写入收敛到一个文件。 / Atomically stores content and returns its digest; concurrent identical writes converge on one file.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobDigest, CasError> {
        let digest = BlobDigest::of(bytes);
        let destination = self.path_for(digest);
        fs::create_dir_all(destination.parent().expect("CAS path has a bucket parent"))?;
        if self.valid_file(&destination, digest)? {
            self.emit(CasEventKind::Reused, digest, None);
            return Ok(digest);
        }
        let temporary = temporary_path(&destination);
        let result: Result<(), CasError> = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            // 两种写入 API 共用同一提交与事件策略。 / Both write APIs share one
            // publication and event policy, including the Windows EFS fallback.
            self.publish_temporary(&temporary, digest)
        })();
        if result.is_err() {
            fs::remove_file(&temporary).ok();
        }
        result?;
        Ok(digest)
    }

    /// 流式写入内容并返回摘要，不把完整 blob 保存在内存中。 / Streams content into the CAS without retaining the complete blob in memory.
    pub fn put_reader(&self, source: &mut dyn Read) -> Result<BlobDigest, CasError> {
        let staging = self.root.join("blobs").join(FORMAT_VERSION).join(ALGORITHM);
        let temporary = staging.join(format!(
            ".incoming-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let mut hash = blake3::Hasher::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
                file.write_all(&buffer[..count])?;
            }
            file.sync_all()?;
            drop(file);
            let digest = BlobDigest::from_bytes(*hash.finalize().as_bytes());
            self.publish_temporary(&temporary, digest)?;
            Ok(digest)
        })();
        if result.is_err() {
            fs::remove_file(&temporary).ok();
        }
        result
    }

    /// 读取并校验 blob；缺失或损坏均返回 `None`，损坏另有观测事件。 / Reads and verifies a blob; missing or corrupt content returns `None`, with a separate corruption event.
    pub fn get(&self, digest: BlobDigest) -> Result<Option<Vec<u8>>, CasError> {
        let bytes = match fs::read(self.path_for(digest)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let actual = BlobDigest::of(&bytes);
        if actual != digest {
            self.emit(CasEventKind::CorruptMiss, digest, Some(actual));
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// 提交已同步、且内容与摘要一致的暂存 blob；负责观察事件与目录持久化。
    /// Publishes a synced staging blob matching its digest, including events and directory sync.
    fn publish_temporary(&self, temporary: &Path, digest: BlobDigest) -> Result<(), CasError> {
        self.publish_temporary_with(
            temporary,
            digest,
            |source, destination| fs::rename(source, destination),
            is_cross_device,
        )
    }

    fn publish_temporary_with(
        &self,
        temporary: &Path,
        digest: BlobDigest,
        mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
        cross_device: impl Fn(&io::Error) -> bool,
    ) -> Result<(), CasError> {
        let destination = self.path_for(digest);
        fs::create_dir_all(destination.parent().expect("CAS path has a bucket parent"))?;
        if self.valid_file(&destination, digest)? {
            fs::remove_file(temporary).ok();
            self.emit(CasEventKind::Reused, digest, None);
            return Ok(());
        }
        let kind = match rename(temporary, &destination) {
            Ok(()) => CasEventKind::Written,
            Err(_error) if destination.exists() && self.valid_file(&destination, digest)? => {
                fs::remove_file(temporary).ok();
                CasEventKind::Reused
            }
            Err(error) if cross_device(&error) => {
                self.publish_cross_device(temporary, &destination, digest)?
            }
            Err(error) => return Err(error.into()),
        };
        if kind == CasEventKind::Written {
            sync_parent(&destination)?;
        }
        self.emit(kind, digest, None);
        Ok(())
    }

    #[cfg(not(windows))]
    fn copy_and_publish_bucket_local(
        &self,
        source: &Path,
        destination: &Path,
        digest: BlobDigest,
        rename: &mut impl FnMut(&Path, &Path) -> io::Result<()>,
    ) -> Result<CasEventKind, CasError> {
        let bucket_temporary = temporary_path(destination);
        let result = (|| {
            let mut input = fs::File::open(source)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&bucket_temporary)?;
            let mut hash = blake3::Hasher::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
                output.write_all(&buffer[..count])?;
            }
            output.sync_all()?;
            drop(output);
            if hash.finalize().as_bytes() != digest.as_bytes() {
                return Err(CasError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "staged CAS blob changed before bucket-local publication",
                )));
            }
            let kind = match rename(&bucket_temporary, destination) {
                Ok(()) => CasEventKind::Written,
                Err(_error) if destination.exists() && self.valid_file(destination, digest)? => {
                    fs::remove_file(&bucket_temporary).ok();
                    CasEventKind::Reused
                }
                Err(error) => return Err(error.into()),
            };
            fs::remove_file(source)?;
            Ok(kind)
        })();
        if result.is_err() {
            fs::remove_file(&bucket_temporary).ok();
        }
        result
    }

    #[cfg(windows)]
    fn publish_cross_device(
        &self,
        temporary: &Path,
        destination: &Path,
        digest: BlobDigest,
    ) -> Result<CasEventKind, CasError> {
        // Rust 的 fs::rename 在 EFS 上可能报跨设备错误；共享平台原语在 Windows
        // 使用 MoveFileW，保留经验证的不覆盖提交语义。 / Rust's fs::rename can
        // report a cross-device error on EFS. The shared platform primitive uses
        // MoveFileW on Windows, retaining the verified no-replace semantics.
        for attempt in 0..2 {
            match squish_platform_fs::rename_exclusive(temporary, destination) {
                Ok(()) => return Ok(CasEventKind::Written),
                Err(error) => {
                    if destination.exists() && self.valid_file(destination, digest)? {
                        fs::remove_file(temporary).ok();
                        return Ok(CasEventKind::Reused);
                    }
                    if destination.exists() && attempt == 0 {
                        fs::remove_file(destination)?;
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }
        unreachable!("the second no-clobber publication attempt always returns")
    }

    #[cfg(not(windows))]
    fn publish_cross_device(
        &self,
        temporary: &Path,
        destination: &Path,
        digest: BlobDigest,
    ) -> Result<CasEventKind, CasError> {
        self.copy_and_publish_bucket_local(
            temporary,
            destination,
            digest,
            &mut |source: &Path, destination: &Path| fs::rename(source, destination),
        )
    }

    fn copy_verified(&self, digest: BlobDigest, sink: &mut dyn Write) -> Result<bool, CasError> {
        let mut file = match fs::File::open(self.path_for(digest)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let (actual, _size) = digest_reader(&mut file)?;
        if actual != digest {
            self.emit(CasEventKind::CorruptMiss, digest, Some(actual));
            return Ok(false);
        }
        file.seek(SeekFrom::Start(0))?;
        io::copy(&mut file, sink)?;
        Ok(true)
    }

    fn valid_file(&self, path: &Path, digest: BlobDigest) -> Result<bool, CasError> {
        match fs::File::open(path) {
            Ok(mut file) => {
                let (actual, _size) = digest_reader(&mut file)?;
                Ok(actual == digest)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    fn emit(&self, kind: CasEventKind, digest: BlobDigest, actual: Option<BlobDigest>) {
        self.observer.observe(&CasEvent {
            kind,
            digest,
            actual,
        });
    }
}

impl squish_build::BlobStore for Cas {
    type Error = CasError;

    fn copy_to(
        &self,
        digest: &squish_build::ContentDigest,
        sink: &mut dyn Write,
    ) -> Result<bool, Self::Error> {
        self.copy_verified(build_digest(digest)?, sink)
    }

    fn write_from(
        &self,
        source: &mut dyn Read,
    ) -> Result<squish_build::ContentDigest, Self::Error> {
        let digest = self.put_reader(source)?;
        Ok(squish_build::ContentDigest::new(
            squish_protocol::DigestAlgorithm::Blake3,
            digest.as_bytes().to_vec(),
        )
        .expect("BLAKE3 has the protocol's canonical digest length"))
    }
}

fn build_digest(digest: &squish_build::ContentDigest) -> Result<BlobDigest, CasError> {
    if digest.algorithm() != &squish_protocol::DigestAlgorithm::Blake3 {
        return Err(CasError::UnsupportedAlgorithm);
    }
    let bytes: [u8; 32] = digest
        .bytes()
        .try_into()
        .map_err(|_| CasError::InvalidDigest)?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn digest_reader(source: &mut dyn Read) -> Result<(BlobDigest, u64), CasError> {
    let mut hash = blake3::Hasher::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("blob size exceeds u64"))?;
    }
    Ok((BlobDigest::from_bytes(*hash.finalize().as_bytes()), size))
}

fn temporary_path(destination: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    destination.with_extension(format!("tmp-{}-{sequence}", std::process::id()))
}

fn is_cross_device(error: &io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE is 17 on Windows. `CrossesDevices` also covers
    // platform-native EXDEV without hard-coding Unix errno values.
    // Windows 的 ERROR_NOT_SAME_DEVICE 为 17；`CrossesDevices` 同时覆盖其他
    // 平台原生 EXDEV，无需硬编码 Unix errno。
    error.kind() == io::ErrorKind::CrossesDevices
        || cfg!(windows) && error.raw_os_error() == Some(17)
}

#[cfg(unix)]
fn sync_parent(destination: &Path) -> Result<(), CasError> {
    // 持久化目录项，使断电后的可见性与已同步内容一致。
    // Persist the directory entry so crash visibility matches synced content.
    std::fs::File::open(destination.parent().expect("CAS path has a parent"))?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_destination: &Path) -> Result<(), CasError> {
    // Windows 的稳定 Rust API 无法以可移植方式打开目录并 flush；文件自身已同步。
    // Stable Rust cannot portably open and flush a directory on Windows; the file itself is synced.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_build::BlobStore;
    use std::sync::{Barrier, Mutex};

    #[derive(Default)]
    struct Events(Mutex<Vec<CasEvent>>);
    impl CasObserver for Events {
        fn observe(&self, event: &CasEvent) {
            self.0.lock().expect("event lock").push(event.clone());
        }
    }
    fn test_dir() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp/squish-store-tests");
        fs::create_dir_all(&root).expect("create project-local test root");
        tempfile::Builder::new()
            .prefix("cas-")
            .tempdir_in(root)
            .expect("test directory")
    }

    #[test]
    fn bucketed_round_trip_and_reuse() {
        let directory = test_dir();
        let store = Cas::open(directory.path()).unwrap();
        let digest = store.put(b"stable bytes").unwrap();
        assert_eq!(store.get(digest).unwrap().unwrap(), b"stable bytes");
        assert!(
            store
                .path_for(digest)
                .starts_with(directory.path().join("blobs/v1/blake3"))
        );
        assert_eq!(store.put(b"stable bytes").unwrap(), digest);
    }

    #[test]
    fn byte_and_stream_writers_share_publication_events() {
        let directory = test_dir();
        let events = Arc::new(Events::default());
        let store = Cas::with_observer(directory.path(), events.clone()).unwrap();
        let bytes = b"same object via both writer forms";

        let digest = store.put(bytes).unwrap();
        assert_eq!(
            store.put_reader(&mut std::io::Cursor::new(bytes)).unwrap(),
            digest
        );
        assert_eq!(store.put(bytes).unwrap(), digest);
        assert_eq!(store.get(digest).unwrap(), Some(bytes.to_vec()));
        assert_eq!(
            events
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [
                CasEventKind::Written,
                CasEventKind::Reused,
                CasEventKind::Reused
            ]
        );
    }

    #[test]
    fn corruption_is_an_observable_miss_and_can_be_repaired() {
        let directory = test_dir();
        let events = Arc::new(Events::default());
        let store = Cas::with_observer(directory.path(), events.clone()).unwrap();
        let digest = store.put(b"good").unwrap();
        fs::write(store.path_for(digest), b"bad").unwrap();
        assert_eq!(store.get(digest).unwrap(), None);
        assert_eq!(
            events.0.lock().unwrap().last().unwrap().kind,
            CasEventKind::CorruptMiss
        );
        store.put(b"good").unwrap();
        assert_eq!(store.get(digest).unwrap().unwrap(), b"good");
    }

    #[test]
    fn concurrent_identical_writers_converge() {
        let directory = test_dir();
        let store = Arc::new(Cas::open(directory.path()).unwrap());
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.put(b"one immutable object").unwrap()
                })
            })
            .collect();
        let digests: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(digests.iter().all(|digest| *digest == digests[0]));
        assert_eq!(
            store.get(digests[0]).unwrap().unwrap(),
            b"one immutable object"
        );
    }

    #[test]
    fn build_port_streams_and_enforces_digest_algorithm() {
        let directory = test_dir();
        let store = Cas::open(directory.path()).unwrap();
        let mut source = std::io::Cursor::new(b"port bytes");
        let digest = BlobStore::write_from(&store, &mut source).unwrap();
        let mut sink = Vec::new();

        assert!(BlobStore::copy_to(&store, &digest, &mut sink).unwrap());
        assert_eq!(sink, b"port bytes");

        let wrong =
            squish_build::ContentDigest::new(squish_protocol::DigestAlgorithm::Sha256, vec![0; 32])
                .unwrap();
        assert!(BlobStore::copy_to(&store, &wrong, &mut sink).is_err());
    }

    #[test]
    fn large_reader_reuse_is_streamed_and_observable() {
        let directory = test_dir();
        let events = Arc::new(Events::default());
        let store = Cas::with_observer(directory.path(), events.clone()).unwrap();
        let bytes: Vec<u8> = (0..8 * 1024 * 1024)
            .map(|index| (index % 251) as u8)
            .collect();

        let first = store.put_reader(&mut std::io::Cursor::new(&bytes)).unwrap();
        let second = store.put_reader(&mut std::io::Cursor::new(&bytes)).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            events
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![CasEventKind::Written, CasEventKind::Reused]
        );
    }

    #[test]
    fn cross_device_publish_falls_back_to_atomic_platform_publication() {
        let directory = test_dir();
        let events = Arc::new(Events::default());
        let store = Cas::with_observer(directory.path(), events.clone()).unwrap();
        let bytes = b"streamed across an EFS rename boundary";
        let digest = BlobDigest::of(bytes);
        let staging = store
            .root()
            .join("blobs")
            .join(FORMAT_VERSION)
            .join(ALGORITHM)
            .join(".incoming-deterministic-test");
        fs::write(&staging, bytes).unwrap();
        let calls = std::cell::Cell::new(0);

        store
            .publish_temporary_with(
                &staging,
                digest,
                |source, destination| {
                    let call = calls.get();
                    calls.set(call + 1);
                    if call == 0 {
                        Err(io::Error::other("injected cross-device rename"))
                    } else {
                        fs::rename(source, destination)
                    }
                },
                |_| true,
            )
            .unwrap();

        assert_eq!(
            calls.get(),
            1,
            "the injected rename handles only the initial staging publication"
        );
        assert!(!staging.exists());
        assert_eq!(store.get(digest).unwrap().unwrap(), bytes);
        assert_eq!(
            events.0.lock().unwrap().as_slice(),
            &[CasEvent {
                kind: CasEventKind::Written,
                digest,
                actual: None,
            }]
        );
    }

    #[cfg(windows)]
    #[test]
    fn platform_publication_never_clobbers_an_existing_target() {
        let directory = test_dir();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"existing").unwrap();

        let error = squish_platform_fs::rename_exclusive(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
    }

    #[test]
    fn failed_stream_removes_unpublished_staging_file() {
        struct FailedReader(bool);
        impl Read for FailedReader {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("injected source failure"));
                }
                self.0 = true;
                buffer[..4].copy_from_slice(b"part");
                Ok(4)
            }
        }

        let directory = test_dir();
        let store = Cas::open(directory.path()).unwrap();
        assert!(store.put_reader(&mut FailedReader(false)).is_err());
        let staging = store
            .root()
            .join("blobs")
            .join(FORMAT_VERSION)
            .join(ALGORITHM);
        assert!(
            fs::read_dir(staging)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".incoming-"))
        );
    }
}
