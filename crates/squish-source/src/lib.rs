//! prompt-squish 的源码身份与加载领域。 / Source identity and loading domain for prompt-squish.
//!
//! 本 crate 严格区分可复现的逻辑身份和只供适配器读取的物理位置。一次构建先经
//! [`SnapshotBuilder`] 冻结所有输入；后续阶段只能观察 [`SourceSnapshot`]，因此宿主
//! 文件系统在构建期间发生变化也不会改变本次构建。
//! This crate strictly separates reproducible logical identity from physical locations used
//! only by adapters. A build first freezes all inputs through [`SnapshotBuilder`]; later phases
//! can observe only [`SourceSnapshot`], so host filesystem changes cannot alter that build.

#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use squish_protocol::{Digest, DigestAlgorithm, OpaqueSourceId};

const SOURCE_HASH_DOMAIN: &[u8] = b"xmlsquish\0source\0";

/// 包解析完成后的稳定身份。 / Stable identity of a resolved package.
///
/// 标识符只接受 ASCII 字母数字、`-`、`_` 和 `.`，从而可无歧义地放入逻辑 URI
/// 的 authority 部分。它不是依赖别名，也不包含检出目录。
/// The identifier accepts only ASCII alphanumerics, `-`, `_`, and `.`, making it unambiguous
/// in a logical URI authority. It is neither a dependency alias nor a checkout directory.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageId(String);

/// 非法包身份。 / Invalid package identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidPackageId(String);

impl fmt::Display for InvalidPackageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid package id {:?}", self.0)
    }
}

impl std::error::Error for InvalidPackageId {}

impl PackageId {
    /// 验证并构造包身份。 / Validates and creates a package identity.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPackageId> {
        let value = value.into();
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid || value == "." || value == ".." {
            return Err(InvalidPackageId(value));
        }
        Ok(Self(value))
    }

    /// 返回稳定文本身份。 / Returns the stable textual identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 调用包清单中的依赖别名。 / Dependency alias in an importing package manifest.
///
/// 别名仅用于解析 [`SourceRef`]，绝不进入解析后的 [`SourceId`]。
/// An alias is used only to resolve a [`SourceRef`] and never enters the resolved [`SourceId`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DependencyAlias(String);

impl DependencyAlias {
    /// 验证并构造依赖别名。 / Validates and creates a dependency alias.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPackageId> {
        PackageId::new(value).map(|value| Self(value.0))
    }

    /// 返回清单中的别名。 / Returns the alias as written in the manifest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 非法逻辑源码路径。 / Invalid logical source path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLogicalPath {
    /// 路径为空或规范化后为空。 / Path is empty before or after normalization.
    Empty,
    /// 路径是绝对路径或带 Windows 盘符。 / Path is absolute or has a Windows drive prefix.
    Absolute,
    /// `..` 会越过包根目录。 / `..` would escape the package root.
    EscapesRoot,
    /// 路径包含 NUL。 / Path contains NUL.
    Nul,
}

impl fmt::Display for InvalidLogicalPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "logical source path must not be empty",
            Self::Absolute => "logical source path must be relative",
            Self::EscapesRoot => "logical source path escapes its package root",
            Self::Nul => "logical source path contains NUL",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for InvalidLogicalPath {}

/// 与宿主平台无关的包内源码路径。 / Host-platform-independent source path within a package.
///
/// `/` 与 `\\` 均视作分隔符；空段和 `.` 被移除，`..` 被词法折叠且不得越过根。
/// 结果始终使用 `/`、区分大小写且不做 Unicode 规范化。符号链接不会被解析。
/// Both `/` and `\\` are separators; empty segments and `.` are removed, while `..` is
/// lexically folded and may not escape the root. Results always use `/`, remain
/// case-sensitive, and are not Unicode-normalized. Symbolic links are never resolved.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LogicalPath(String);

impl LogicalPath {
    /// 规范化相对逻辑路径。 / Normalizes a relative logical path.
    pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidLogicalPath> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(InvalidLogicalPath::Empty);
        }
        if value.contains('\0') {
            return Err(InvalidLogicalPath::Nul);
        }
        let bytes = value.as_bytes();
        if matches!(bytes.first(), Some(b'/' | b'\\'))
            || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        {
            return Err(InvalidLogicalPath::Absolute);
        }

        let mut segments = Vec::new();
        for segment in value.split(['/', '\\']) {
            match segment {
                "" | "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return Err(InvalidLogicalPath::EscapesRoot);
                    }
                }
                other => segments.push(other),
            }
        }
        if segments.is_empty() {
            return Err(InvalidLogicalPath::Empty);
        }
        Ok(Self(segments.join("/")))
    }

    /// 返回使用 `/` 的规范路径。 / Returns the normalized slash-separated path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 尚未解析的依赖源码引用。 / Unresolved source reference through a dependency alias.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceRef {
    alias: DependencyAlias,
    path: LogicalPath,
}

impl SourceRef {
    /// 创建依赖别名下的源码引用。 / Creates a source reference under a dependency alias.
    pub fn new(alias: DependencyAlias, path: LogicalPath) -> Self {
        Self { alias, path }
    }

    /// 返回依赖别名。 / Returns the dependency alias.
    pub fn alias(&self) -> &DependencyAlias {
        &self.alias
    }

    /// 返回包内逻辑路径。 / Returns the logical path within the package.
    pub fn path(&self) -> &LogicalPath {
        &self.path
    }

    /// 用解析器选出的真实包身份绑定此引用。 / Binds this reference to the package identity selected by resolution.
    pub fn resolve(&self, package: PackageId) -> SourceId {
        SourceId::new(package, self.path.clone())
    }
}

/// 与位置和内容无关的已解析逻辑源码身份。 / Resolved logical source identity independent of location and content.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceIdentity {
    package: PackageId,
    path: LogicalPath,
    uri: String,
}

/// 不透明协议源码身份未满足领域 URI 规范。 / Opaque protocol source identity did not satisfy the domain URI grammar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidSourceIdentity {
    /// scheme 或 URI 结构错误。 / Scheme or URI structure is invalid.
    Structure,
    /// 包 authority 非法。 / Package authority is invalid.
    Package(InvalidPackageId),
    /// 路径百分号编码或 UTF-8 非法。 / Path percent encoding or UTF-8 is invalid.
    Encoding,
    /// 解码后的逻辑路径非法。 / Decoded logical path is invalid.
    Path(InvalidLogicalPath),
    /// URI 可解析但不是唯一规范拼写。 / URI is parseable but not in its unique canonical spelling.
    NonCanonical,
}

impl fmt::Display for InvalidSourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structure => {
                formatter.write_str("source identity must be an xmlsquish package URI")
            }
            Self::Package(error) => error.fmt(formatter),
            Self::Encoding => {
                formatter.write_str("source identity contains invalid percent encoding")
            }
            Self::Path(error) => error.fmt(formatter),
            Self::NonCanonical => formatter.write_str("source identity URI is not canonical"),
        }
    }
}

impl std::error::Error for InvalidSourceIdentity {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Package(error) => Some(error),
            Self::Path(error) => Some(error),
            Self::Structure | Self::Encoding | Self::NonCanonical => None,
        }
    }
}

impl SourceIdentity {
    /// 从已解析包和规范路径构造身份。 / Creates an identity from a resolved package and normalized path.
    pub fn new(package: PackageId, path: LogicalPath) -> Self {
        let uri = format!(
            "xmlsquish://{}/{}",
            package.as_str(),
            encode_uri_path(path.as_str())
        );
        Self { package, path, uri }
    }

    /// 返回解析后的包身份。 / Returns the resolved package identity.
    pub fn package(&self) -> &PackageId {
        &self.package
    }

    /// 返回规范包内路径。 / Returns the normalized in-package path.
    pub fn path(&self) -> &LogicalPath {
        &self.path
    }

    /// 返回可用于协议和诊断的规范 URI。 / Returns the canonical URI used by protocols and diagnostics.
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// 在展示/进程边界创建不透明协议身份。 / Creates an opaque protocol identity at a presentation/process boundary.
    pub fn to_protocol(&self) -> OpaqueSourceId {
        OpaqueSourceId::new(self.uri.clone()).expect("a logical source URI is never empty")
    }

    /// 验证并解析来自协议边界的不透明身份。 / Validates and parses an opaque identity received at a protocol boundary.
    pub fn try_from_protocol(value: &OpaqueSourceId) -> Result<Self, InvalidSourceIdentity> {
        let raw = value.as_str();
        let remainder = raw
            .strip_prefix("xmlsquish://")
            .ok_or(InvalidSourceIdentity::Structure)?;
        let (package, encoded_path) = remainder
            .split_once('/')
            .ok_or(InvalidSourceIdentity::Structure)?;
        let package = PackageId::new(package).map_err(InvalidSourceIdentity::Package)?;
        let decoded_path = decode_uri_path(encoded_path)?;
        let path = LogicalPath::new(decoded_path).map_err(InvalidSourceIdentity::Path)?;
        let identity = Self::new(package, path);
        if identity.uri() != raw {
            return Err(InvalidSourceIdentity::NonCanonical);
        }
        Ok(identity)
    }
}

impl fmt::Display for SourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.uri().fmt(formatter)
    }
}

/// 领域源码身份的简写。 / Short name for the domain source identity.
pub type SourceId = SourceIdentity;

fn encode_uri_path(path: &str) -> String {
    let mut output = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            output.push(char::from(byte));
        } else {
            use fmt::Write as _;
            write!(output, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    output
}

fn decode_uri_path(path: &str) -> Result<String, InvalidSourceIdentity> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let pair = bytes
            .get(index + 1..index + 3)
            .ok_or(InvalidSourceIdentity::Encoding)?;
        decoded.push((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| InvalidSourceIdentity::Encoding)
}

fn hex_nibble(byte: u8) -> Result<u8, InvalidSourceIdentity> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(InvalidSourceIdentity::Encoding),
    }
}

/// 仅用于读取与诊断的宿主文件位置。 / Host file location used only for reading and diagnostics.
///
/// 该类型有意保持宿主原生 [`PathBuf`]，不进入 [`SourceId`]、摘要或缓存键；调用者负责
/// 从项目根解析相对路径。当前端口不表示 URL，也不进行网络访问。
/// This type deliberately retains a host-native [`PathBuf`] and never enters [`SourceId`], a
/// digest, or a cache key; callers resolve relative paths against a project root. This port
/// does not represent URLs and performs no network access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocator(PathBuf);

impl SourceLocator {
    /// 包装一个宿主文件路径而不影响逻辑身份。 / Wraps a host file path without affecting logical identity.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// 返回宿主文件路径。 / Returns the host file path.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// 精确源码字节的带算法摘要。 / Algorithm-tagged digest of exact source bytes.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SourceDigest([u8; 32]);

impl SourceDigest {
    /// 以域分离 BLAKE3 计算摘要。 / Computes a domain-separated BLAKE3 digest.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(SOURCE_HASH_DOMAIN);
        hasher.update(bytes);
        Self(*hasher.finalize().as_bytes())
    }

    /// 返回算法名称。 / Returns the algorithm name.
    pub const fn algorithm(&self) -> &'static str {
        "blake3"
    }

    /// 返回原始 32 字节摘要。 / Returns the raw 32-byte digest.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 在持久化/进程边界创建带类型的协议摘要。 / Creates a typed protocol digest at a persistence/process boundary.
    pub fn to_protocol(self) -> Digest {
        Digest::new(DigestAlgorithm::Blake3, self.0.to_vec())
            .expect("a BLAKE3 digest always contains 32 bytes")
    }
}

impl fmt::Debug for SourceDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for SourceDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("blake3:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// 已冻结的源码字节及其非语义读取位置。 / Frozen source bytes and their non-semantic read location.
#[derive(Clone, Debug)]
pub struct SourceBlob {
    id: SourceId,
    locator: SourceLocator,
    digest: SourceDigest,
    bytes: Arc<[u8]>,
}

impl SourceBlob {
    /// 返回逻辑源码身份。 / Returns the logical source identity.
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// 返回首次读取使用的物理位置。 / Returns the physical location used for the first read.
    pub fn locator(&self) -> &SourceLocator {
        &self.locator
    }

    /// 返回精确字节摘要。 / Returns the digest of the exact bytes.
    pub const fn digest(&self) -> SourceDigest {
        self.digest
    }

    /// 返回本次快照冻结的精确字节。 / Returns exact bytes frozen for this snapshot.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// 源码读取失败；可克隆以稳定重放快照中的失败。 / Source read failure, cloneable for stable replay from a snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLoadError {
    id: SourceId,
    locator: SourceLocator,
    kind: io::ErrorKind,
    message: String,
}

impl SourceLoadError {
    /// 返回失败源码身份。 / Returns the identity whose load failed.
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// 返回首次尝试的位置。 / Returns the location of the first attempt.
    pub fn locator(&self) -> &SourceLocator {
        &self.locator
    }

    /// 返回可分类的 I/O 错误种类。 / Returns the classifiable I/O error kind.
    pub const fn kind(&self) -> io::ErrorKind {
        self.kind
    }
}

impl fmt::Display for SourceLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not read {}: {}", self.id, self.message)
    }
}

impl std::error::Error for SourceLoadError {}

/// 读取源码字节的端口。 / Port for reading source bytes.
///
/// 实现必须只执行一次同步读取，不得自行缓存、解析或联网。一次身份只读一次的约束由
/// [`SnapshotBuilder`] 统一执行。
/// Implementations perform one synchronous read and must not cache, parse, or use the network.
/// [`SnapshotBuilder`] centrally enforces one read per identity.
pub trait SourceProvider {
    /// 从物理位置读取全部字节。 / Reads all bytes from a physical location.
    fn read(&self, locator: &SourceLocator) -> io::Result<Vec<u8>>;
}

/// 直接使用宿主文件系统的 provider 适配器。 / Provider adapter backed directly by the host filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileSourceProvider;

impl SourceProvider for FileSourceProvider {
    fn read(&self, locator: &SourceLocator) -> io::Result<Vec<u8>> {
        std::fs::read(locator.as_path())
    }
}

#[derive(Clone, Debug)]
enum SnapshotEntry {
    Loaded(Arc<SourceBlob>),
    Failed(Arc<SourceLoadError>),
}

/// 构建源码快照时的失败。 / Failure while building a source snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotBuildError {
    /// builder 已经 seal，不能再增加身份。 / The builder is sealed and cannot accept identities.
    Sealed,
    /// 首次读取或缓存的读取失败。 / A first-read or cached read failure.
    Load(Arc<SourceLoadError>),
}

impl fmt::Display for SnapshotBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sealed => formatter.write_str("source snapshot builder is sealed"),
            Self::Load(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SnapshotBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sealed => None,
            Self::Load(error) => Some(error),
        }
    }
}

/// 读取、哈希并驻留一次调用全部源码的 builder。 / Builder that reads, hashes, and interns all sources for one invocation.
pub struct SnapshotBuilder<P> {
    provider: P,
    entries: Option<HashMap<SourceId, SnapshotEntry>>,
    interned: HashMap<SourceDigest, Vec<Arc<[u8]>>>,
}

impl<P: SourceProvider> SnapshotBuilder<P> {
    /// 使用注入的读取端口创建空 builder。 / Creates an empty builder with an injected read port.
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            entries: Some(HashMap::new()),
            interned: HashMap::new(),
        }
    }

    /// 首次读取身份，或稳定重放此前成功/失败结果。 / Reads an identity once or stably replays its prior success/failure.
    pub fn load(
        &mut self,
        id: SourceId,
        locator: SourceLocator,
    ) -> Result<Arc<SourceBlob>, SnapshotBuildError> {
        let entries = self.entries.as_ref().ok_or(SnapshotBuildError::Sealed)?;
        if let Some(entry) = entries.get(&id) {
            return match entry {
                SnapshotEntry::Loaded(blob) => Ok(Arc::clone(blob)),
                SnapshotEntry::Failed(error) => Err(SnapshotBuildError::Load(Arc::clone(error))),
            };
        }

        let bytes = match self.provider.read(&locator) {
            Ok(bytes) => bytes,
            Err(error) => {
                let error = Arc::new(SourceLoadError {
                    id: id.clone(),
                    locator,
                    kind: error.kind(),
                    message: error.to_string(),
                });
                self.entries
                    .as_mut()
                    .expect("builder seal state cannot change during a load")
                    .insert(id, SnapshotEntry::Failed(Arc::clone(&error)));
                return Err(SnapshotBuildError::Load(error));
            }
        };
        let digest = SourceDigest::of(&bytes);
        let bytes = self.intern_bytes(digest, bytes);
        let blob = Arc::new(SourceBlob {
            id: id.clone(),
            locator,
            digest,
            bytes,
        });
        self.entries
            .as_mut()
            .expect("builder seal state cannot change during a load")
            .insert(id, SnapshotEntry::Loaded(Arc::clone(&blob)));
        Ok(blob)
    }

    fn intern_bytes(&mut self, digest: SourceDigest, bytes: Vec<u8>) -> Arc<[u8]> {
        let candidates = self.interned.entry(digest).or_default();
        if let Some(existing) = candidates
            .iter()
            .find(|existing| existing.as_ref() == bytes)
        {
            return Arc::clone(existing);
        }
        let bytes: Arc<[u8]> = bytes.into();
        candidates.push(Arc::clone(&bytes));
        bytes
    }

    /// 封闭 builder 并返回不可变快照。 / Seals the builder and returns an immutable snapshot.
    ///
    /// 调用后 [`Self::load`] 必定返回 [`SnapshotBuildError::Sealed`]。通常每次调用只 seal
    /// 一次；重复 seal 会明确返回 `Sealed`，不会伪造第二份空快照。
    /// After this call [`Self::load`] always returns [`SnapshotBuildError::Sealed`]. Normally an
    /// invocation seals once; repeated sealing explicitly returns `Sealed` rather than fabricating
    /// a second empty snapshot.
    pub fn seal(&mut self) -> Result<SourceSnapshot, SnapshotBuildError> {
        let entries = self.entries.take().ok_or(SnapshotBuildError::Sealed)?;
        Ok(SourceSnapshot { entries })
    }
}

/// 已封闭、不可变且包含成功与失败的源码快照。 / Sealed immutable source snapshot containing successes and failures.
#[derive(Clone, Debug, Default)]
pub struct SourceSnapshot {
    entries: HashMap<SourceId, SnapshotEntry>,
}

impl SourceSnapshot {
    /// 返回已驻留身份数，包括失败身份。 / Returns the number of interned identities, including failures.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 判断快照是否为空。 / Returns whether the snapshot is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 查找成功加载的源码；失败或未知身份返回 `None`。 / Looks up a loaded source; failed or unknown identities return `None`.
    pub fn get(&self, id: &SourceId) -> Option<&SourceBlob> {
        match self.entries.get(id) {
            Some(SnapshotEntry::Loaded(blob)) => Some(blob),
            Some(SnapshotEntry::Failed(_)) | None => None,
        }
    }

    /// 查找已缓存的失败。 / Looks up a cached failure.
    pub fn failure(&self, id: &SourceId) -> Option<&SourceLoadError> {
        match self.entries.get(id) {
            Some(SnapshotEntry::Failed(error)) => Some(error.as_ref()),
            Some(SnapshotEntry::Loaded(_)) | None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    #[derive(Default)]
    struct MemoryProvider {
        reads: RefCell<HashMap<PathBuf, usize>>,
        files: RefCell<HashMap<PathBuf, io::Result<Vec<u8>>>>,
    }

    impl MemoryProvider {
        fn insert(&self, path: &str, bytes: &[u8]) {
            self.files
                .borrow_mut()
                .insert(path.into(), Ok(bytes.to_vec()));
        }

        fn fail(&self, path: &str) {
            self.files.borrow_mut().insert(
                path.into(),
                Err(io::Error::new(io::ErrorKind::NotFound, "fixture missing")),
            );
        }

        fn reads(&self, path: &str) -> usize {
            self.reads
                .borrow()
                .get(Path::new(path))
                .copied()
                .unwrap_or(0)
        }
    }

    impl SourceProvider for &MemoryProvider {
        fn read(&self, locator: &SourceLocator) -> io::Result<Vec<u8>> {
            *self
                .reads
                .borrow_mut()
                .entry(locator.as_path().to_owned())
                .or_default() += 1;
            self.files
                .borrow_mut()
                .remove(locator.as_path())
                .unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::NotFound, "unknown fixture")))
        }
    }

    fn id(package: &str, path: &str) -> SourceId {
        SourceId::new(
            PackageId::new(package).unwrap(),
            LogicalPath::new(path).unwrap(),
        )
    }

    #[test]
    fn alias_resolves_to_package_identity_and_portable_uri() {
        let reference = SourceRef::new(
            DependencyAlias::new("common").unwrap(),
            LogicalPath::new(r"src\\space name.xml").unwrap(),
        );
        let resolved = reference.resolve(PackageId::new("shared-prompts").unwrap());

        assert_eq!(reference.alias().as_str(), "common");
        assert_eq!(resolved.package().as_str(), "shared-prompts");
        assert_eq!(
            resolved.uri(),
            "xmlsquish://shared-prompts/src/space%20name.xml"
        );
        assert_eq!(resolved.to_protocol().as_str(), resolved.uri());
        assert_eq!(
            SourceIdentity::try_from_protocol(&resolved.to_protocol()).unwrap(),
            resolved
        );
        assert!(!resolved.uri().contains("common"));
        let noncanonical =
            OpaqueSourceId::new("xmlsquish://shared-prompts/src/%73pace.xml").unwrap();
        assert_eq!(
            SourceIdentity::try_from_protocol(&noncanonical),
            Err(InvalidSourceIdentity::NonCanonical)
        );
    }

    #[test]
    fn logical_paths_have_explicit_cross_platform_normalization() {
        assert_eq!(
            LogicalPath::new(r"src\\generated\..\main.xml")
                .unwrap()
                .as_str(),
            "src/main.xml"
        );
        assert_eq!(
            LogicalPath::new("../outside.xml"),
            Err(InvalidLogicalPath::EscapesRoot)
        );
        assert_eq!(
            LogicalPath::new(r"C:\\project\\main.xml"),
            Err(InvalidLogicalPath::Absolute)
        );
    }

    #[test]
    fn one_identity_is_read_once_even_with_a_different_locator() {
        let provider = MemoryProvider::default();
        provider.insert("first", b"first bytes");
        provider.insert("second", b"second bytes");
        let mut builder = SnapshotBuilder::new(&provider);
        let source = id("agent", "src/main.xml");

        let first = builder
            .load(source.clone(), SourceLocator::file("first"))
            .unwrap();
        let second = builder.load(source, SourceLocator::file("second")).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.bytes(), b"first bytes");
        assert_eq!(provider.reads("first"), 1);
        assert_eq!(provider.reads("second"), 0);
    }

    #[test]
    fn failures_are_read_once_and_retained_by_snapshot() {
        let provider = MemoryProvider::default();
        provider.fail("missing");
        let mut builder = SnapshotBuilder::new(&provider);
        let source = id("agent", "src/missing.xml");

        let first = builder
            .load(source.clone(), SourceLocator::file("missing"))
            .unwrap_err();
        provider.insert("missing", b"appeared later");
        let second = builder
            .load(source.clone(), SourceLocator::file("missing"))
            .unwrap_err();
        let snapshot = builder.seal().unwrap();

        assert_eq!(first, second);
        assert_eq!(provider.reads("missing"), 1);
        assert_eq!(
            snapshot.failure(&source).unwrap().kind(),
            io::ErrorKind::NotFound
        );
        assert!(snapshot.get(&source).is_none());
    }

    #[test]
    fn seal_prevents_new_loads() {
        let provider = MemoryProvider::default();
        provider.insert("main", b"main");
        let mut builder = SnapshotBuilder::new(&provider);
        let snapshot = builder.seal().unwrap();

        let result = builder.load(id("agent", "src/main.xml"), SourceLocator::file("main"));

        assert_eq!(result.unwrap_err(), SnapshotBuildError::Sealed);
        assert_eq!(builder.seal().unwrap_err(), SnapshotBuildError::Sealed);
        assert!(snapshot.is_empty());
        assert_eq!(provider.reads("main"), 0);
    }

    #[test]
    fn snapshot_is_not_changed_by_physical_source_mutation() {
        let provider = MemoryProvider::default();
        provider.insert("main", b"version one");
        let mut builder = SnapshotBuilder::new(&provider);
        let source = id("agent", "src/main.xml");
        let loaded = builder
            .load(source.clone(), SourceLocator::file("main"))
            .unwrap();
        let original_digest = loaded.digest();
        let wire_digest = original_digest.to_protocol();
        let snapshot = builder.seal().unwrap();
        provider.insert("main", b"version two");

        let frozen = snapshot.get(&source).unwrap();
        assert_eq!(frozen.bytes(), b"version one");
        assert_eq!(frozen.digest(), original_digest);
        assert_eq!(wire_digest.algorithm(), &DigestAlgorithm::Blake3);
        assert_eq!(wire_digest.bytes(), original_digest.as_bytes());
        assert_eq!(provider.reads("main"), 1);
    }

    #[test]
    fn equal_content_is_interned_without_merging_identity() {
        let provider = MemoryProvider::default();
        provider.insert("a", b"same");
        provider.insert("b", b"same");
        let mut builder = SnapshotBuilder::new(&provider);
        let a = builder
            .load(id("a", "src/main.xml"), SourceLocator::file("a"))
            .unwrap();
        let b = builder
            .load(id("b", "src/main.xml"), SourceLocator::file("b"))
            .unwrap();

        assert_ne!(a.id(), b.id());
        assert!(Arc::ptr_eq(&a.bytes, &b.bytes));
    }
}
