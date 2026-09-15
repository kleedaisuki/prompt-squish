use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// SHA-256 摘要的强类型包装。 / Strongly typed SHA-256 digest.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Sha256Digest(pub String);

impl Sha256Digest {
    /// 校验并构造小写的 64 位十六进制摘要。 / Validates a lowercase 64-digit digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, FetchError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(FetchError::Integrity(
                "SHA-256 must be 64 lowercase hex digits".into(),
            ));
        }
        Ok(Self(value))
    }
}

/// 精确归档字节摘要。 / Digest of exact archive bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArchiveDigest(pub Sha256Digest);
/// 规范逻辑文件树摘要。 / Digest of a canonical logical file tree.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContentDigest(pub Sha256Digest);
/// 精确 manifest 字节摘要。 / Digest of exact manifest bytes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManifestDigest(pub Sha256Digest);

/// Git 对象格式。 / Git object format.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

/// 带算法的完整 Git 对象 ID。 / Full algorithm-tagged Git object ID.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GitOid {
    pub format: GitObjectFormat,
    pub hex: String,
}

impl GitOid {
    /// 校验 OID 长度和十六进制格式。 / Validates OID length and hexadecimal form.
    pub fn new(format: GitObjectFormat, hex: impl Into<String>) -> Result<Self, FetchError> {
        let hex = hex.into();
        let len = match format {
            GitObjectFormat::Sha1 => 40,
            GitObjectFormat::Sha256 => 64,
        };
        if hex.len() != len || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(FetchError::Git("invalid full Git object ID".into()));
        }
        Ok(Self {
            format,
            hex: hex.to_ascii_lowercase(),
        })
    }
}

/// 外部访问策略；`LocalOnly` 保证零网络。 / External access policy; `LocalOnly` guarantees zero network.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Online,
    LocalOnly,
}

/// 获取和物化限制。 / Acquisition and materialization limits.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_metadata_bytes: u64,
    pub max_metadata_line: usize,
    pub max_archive_bytes: u64,
    pub max_expanded_bytes: u64,
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_path_bytes: usize,
    pub max_segment_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_metadata_bytes: 8 << 20,
            max_metadata_line: 1 << 20,
            max_archive_bytes: 256 << 20,
            max_expanded_bytes: 512 << 20,
            max_file_bytes: 128 << 20,
            max_files: 50_000,
            max_path_bytes: 1024,
            max_segment_bytes: 255,
        }
    }
}

/// 来源宿主的结构化事件。 / Structured source-host event.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum SourceEvent {
    CacheLookupFinished {
        key: String,
        layer: String,
        status: String,
    },
    NetworkRequestStarted {
        request_id: u64,
        origin: String,
        purpose: String,
    },
    NetworkRequestFinished {
        request_id: u64,
        status: String,
        received_bytes: u64,
        validator_used: bool,
    },
    SourceRetryScheduled {
        request_id: u64,
        reason: String,
        attempt: u8,
        delay_ms: u64,
    },
    MetadataRevalidated {
        registry_id: String,
        resource: String,
        modified: bool,
    },
    IntegrityVerified {
        identity: String,
        content_digest: String,
        files: usize,
        bytes: u64,
    },
    MaterializationStarted {
        identity: String,
        content_digest: String,
    },
    MaterializationCommitted {
        content_digest: String,
        files: usize,
        bytes: u64,
        cache_status: String,
    },
    CacheFaultRecovered {
        layer: String,
        expected_digest: String,
        recovery: String,
    },
    SourceUnavailable {
        identity: String,
        mode: String,
        reason: String,
    },
}

/// 事件接收器；实现不得获得 credential 或响应正文。 / Event sink; implementations never receive credentials or response bodies.
pub trait Observer: Send + Sync {
    fn emit(&self, event: SourceEvent);
}
impl<F: Fn(SourceEvent) + Send + Sync> Observer for F {
    fn emit(&self, event: SourceEvent) {
        self(event);
    }
}

/// 不记录事件的 observer。 / Observer which discards events.
#[derive(Default)]
pub struct NoopObserver;
impl Observer for NoopObserver {
    fn emit(&self, _: SourceEvent) {}
}

/// 完成的不可变包目录。 / Complete immutable package directory.
#[derive(Clone, Debug)]
pub struct MaterializedPackage {
    pub content_digest: ContentDigest,
    pub manifest_digest: ManifestDigest,
    pub manifest: squish_project::Manifest,
    pub root: PathBuf,
    pub files: Arc<[FileRecord]>,
}

/// Marker 中的逻辑文件记录。 / Logical file record stored in a completion marker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub length: u64,
    pub sha256: String,
}

/// 来源宿主错误。 / Source-host failure.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("configuration invalid: {0}")]
    Config(String),
    #[error("offline cache miss: {0}")]
    OfflineMiss(String),
    #[error("metadata invalid: {0}")]
    Metadata(String),
    #[error("integrity verification failed: {0}")]
    Integrity(String),
    #[error("logical path invalid: {0}")]
    Path(String),
    #[error("unsupported tree entry: {0}")]
    Unsupported(String),
    #[error("Git source failed: {0}")]
    Git(String),
    #[error("I/O unavailable: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP unavailable: {0}")]
    Http(String),
    #[error("JSON invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manifest invalid: {0}")]
    Manifest(#[from] squish_project::ProjectError),
}

/// 共享宿主配置。 / Shared host configuration.
#[derive(Clone)]
pub struct HostContext {
    pub cache: PathBuf,
    pub limits: Limits,
    pub observer: Arc<dyn Observer>,
}
impl HostContext {
    /// 建立缓存目录。 / Creates cache directories.
    pub fn new(cache: PathBuf) -> Result<Self, FetchError> {
        std::fs::create_dir_all(cache.join("v1/stage"))?;
        Ok(Self {
            cache,
            limits: Limits::default(),
            observer: Arc::new(NoopObserver),
        })
    }
}
