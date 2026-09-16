use std::{
    fmt,
    io::{Read, Write},
};

use crate::{ActionKey, ContentDigest, ProducedOutput};
use serde::{Deserialize, Serialize};
use squish_protocol::{ArtifactId, ArtifactKind, Digest};

/// 非法发布身份或逻辑路径。 / Invalid publication identity or logical path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPublicationIdentity;

impl fmt::Display for InvalidPublicationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("publication identity or path is not canonical")
    }
}

impl std::error::Error for InvalidPublicationIdentity {}

/// 一个声明构建目标的稳定发布身份。 / Stable publication identity of one declared build target.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicationTargetId(String);

impl PublicationTargetId {
    /// 验证非空且不含控制字符的目标身份。 / Validates a non-empty target identity without control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPublicationIdentity> {
        let value = value.into();
        if value.is_empty() || value.chars().any(char::is_control) {
            Err(InvalidPublicationIdentity)
        } else {
            Ok(Self(value))
        }
    }

    /// 返回领域身份文本。 / Returns the domain identity text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PublicationTargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for PublicationTargetId {
    type Error = InvalidPublicationIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<PublicationTargetId> for String {
    fn from(value: PublicationTargetId) -> Self {
        value.0
    }
}

/// target 内可查询的稳定逻辑产物名。 / Stable target-local logical artifact name.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct LogicalArtifactName(String);

impl LogicalArtifactName {
    /// 验证非空且不含控制字符的逻辑名。 / Validates a non-empty logical name without control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPublicationIdentity> {
        let value = value.into();
        if value.is_empty() || value.chars().any(char::is_control) {
            Err(InvalidPublicationIdentity)
        } else {
            Ok(Self(value))
        }
    }

    /// 返回逻辑名。 / Returns the logical name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LogicalArtifactName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for LogicalArtifactName {
    type Error = InvalidPublicationIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<LogicalArtifactName> for String {
    fn from(value: LogicalArtifactName) -> Self {
        value.0
    }
}

/// 非法 generation 十六进制编码。 / Invalid hexadecimal generation encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidGenerationId;

impl fmt::Display for InvalidGenerationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("generation ID must be 64 lowercase hexadecimal characters")
    }
}

impl std::error::Error for InvalidGenerationId {}

/// 发布器生成的 256-bit 不可变 generation 身份。 / Publisher-issued 256-bit immutable generation identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GenerationId([u8; 32]);

impl GenerationId {
    /// 从发布器的完整 256-bit 身份构造。 / Constructs from the publisher's complete 256-bit identity.
    pub const fn from_bytes(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// 解码规范小写十六进制边界表示。 / Decodes the canonical lowercase hexadecimal boundary representation.
    pub fn from_hex(value: &str) -> Result<Self, InvalidGenerationId> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InvalidGenerationId);
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = (hex_nibble(value.as_bytes()[offset])? << 4)
                | hex_nibble(value.as_bytes()[offset + 1])?;
        }
        Ok(Self(bytes))
    }

    /// 返回原始 256-bit 身份。 / Returns the raw 256-bit identity.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 返回规范小写十六进制。 / Returns canonical lowercase hexadecimal.
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut result = String::with_capacity(64);
        for byte in self.0 {
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 0xf) as usize] as char);
        }
        result
    }
}

impl fmt::Display for GenerationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_hex().fmt(formatter)
    }
}
impl Serialize for GenerationId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}
impl<'de> Deserialize<'de> for GenerationId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_hex(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn hex_nibble(value: u8) -> Result<u8, InvalidGenerationId> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(InvalidGenerationId),
    }
}

/// 用户可见、与物理发布布局无关的规范相对路径。 / Canonical user-visible relative path independent of physical publication layout.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct PublicationPath(String);

impl PublicationPath {
    /// 验证使用 `/` 分隔的非空规范相对路径。 / Validates a non-empty canonical `/`-separated relative path.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPublicationIdentity> {
        let value = value.into();
        let valid = !value.is_empty()
            && !value.starts_with('/')
            && !value.contains('\\')
            && value.split('/').all(portable_path_component)
            && value.split('/').next() != Some(".squish-publish");
        if valid {
            Ok(Self(value))
        } else {
            Err(InvalidPublicationIdentity)
        }
    }

    /// 返回规范逻辑路径。 / Returns the canonical logical path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn portable_path_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && !value.chars().any(|character| {
            character.is_control() || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        && value == value.trim_end_matches([' ', '.'])
        && !is_windows_device_name(value)
}

fn is_windows_device_name(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or_default();
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

impl fmt::Display for PublicationPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for PublicationPath {
    type Error = InvalidPublicationIdentity;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<PublicationPath> for String {
    fn from(value: PublicationPath) -> Self {
        value.0
    }
}

/// 一个 target 的不可变 generation 引用。 / Immutable generation reference for one target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GenerationRef {
    /// 目标领域身份。 / Target domain identity.
    pub target: PublicationTargetId,
    /// 发布器生成的 generation 身份。 / Publisher-issued generation identity.
    pub generation: GenerationId,
}

/// 与位置无关、可完整验证的产物描述符。 / Location-independent, fully verifiable artifact descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactDescriptor {
    /// generation 内稳定产物 ID。 / Stable artifact ID within the generation.
    pub id: ArtifactId,
    /// target 内可查询逻辑名。 / Target-local query name.
    pub name: LogicalArtifactName,
    /// 语义类别。 / Semantic kind.
    pub kind: ArtifactKind,
    /// 内容摘要。 / Content digest.
    pub digest: Digest,
    /// 内容字节数。 / Content byte length.
    pub size: u64,
}

/// generation 中一个具有稳定逻辑路径的产物。 / One generation artifact with a stable logical path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GenerationArtifact {
    /// 与位置无关的描述符。 / Location-independent descriptor.
    pub descriptor: ArtifactDescriptor,
    /// 用户可见逻辑路径。 / User-visible logical path.
    pub path: PublicationPath,
}

/// 一次完整原子提交的 generation。 / One complete atomically committed generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommittedGeneration {
    /// target 与 generation 的不可变身份。 / Immutable target and generation identity.
    pub identity: GenerationRef,
    /// 按发布请求顺序排列的完整成员。 / Complete membership in publication-request order.
    pub artifacts: Vec<GenerationArtifact>,
}

/// 内容寻址二进制存储端口；具体 CAS 属于 store 领域。 / Port for content-addressed blobs; the concrete CAS belongs to the store domain.
pub trait BlobStore {
    /// 后端错误。 / Backend error.
    type Error;

    /// 将 blob 流式复制到 sink；缺失返回 `false`。 / Streams a blob into a sink; returns `false` when absent.
    fn copy_to(&self, digest: &ContentDigest, sink: &mut dyn Write) -> Result<bool, Self::Error>;

    /// 流式、幂等写入并返回由后端验证的摘要。 / Streams an idempotent write and returns the backend-verified digest.
    fn write_from(&self, source: &mut dyn Read) -> Result<ContentDigest, Self::Error>;
}

/// 可复用动作结果的持久记录。 / Persistent record of a reusable action result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRecord {
    /// 最终物化键。 / Final materialized key.
    pub key: ActionKey,
    /// 按动作声明顺序排列的输出。 / Outputs in action declaration order.
    pub outputs: Vec<ProducedOutput>,
}

/// `ActionKey -> outputs` 索引端口。 / Port for an `ActionKey -> outputs` index.
pub trait ActionIndex {
    /// 后端错误。 / Backend error.
    type Error;

    /// 查找已验证记录。 / Looks up a verified record.
    fn lookup(&self, key: &ActionKey) -> Result<Option<ActionRecord>, Self::Error>;

    /// 幂等记录成功动作。 / Idempotently records a successful action.
    fn record(&self, record: &ActionRecord) -> Result<(), Self::Error>;
}

/// 原子发布请求。 / Atomic publication request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publication {
    /// 已物化到 blob store 的输出。 / Output already materialized in the blob store.
    pub output: ProducedOutput,
    /// target 内稳定、可查询的逻辑产物名。 / Stable queryable logical artifact name within the target.
    pub name: LogicalArtifactName,
    /// 用户可见目标 URI；它不属于纯动作 key。 / User-visible destination URI; it is not part of the pure action key.
    pub destination: PublicationPath,
}

/// 将不可变 blob 发布到用户可见位置的端口。 / Port publishing immutable blobs to user-visible locations.
pub trait ArtifactPublisher {
    /// 后端错误。 / Backend error.
    type Error;

    /// 原子发布或替换一个目标。 / Atomically publishes or replaces one destination.
    fn publish(&self, publication: &Publication) -> Result<(), Self::Error>;
}

/// generation 产物读取结果；缺失不会伪装为空内容。 / Generation-artifact read result; absence never masquerades as empty content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactRead {
    /// 逻辑成员不存在于该 generation。 / The logical member is absent from the generation.
    NotFound,
    /// 完整字节已写入 sink 且摘要验证成功。 / Complete bytes were written to the sink and digest verification succeeded.
    Verified(ArtifactDescriptor),
}

/// 原子 generation 发布、查询与验证读取端口。 / Port for atomic generation publication, query, and verified reads.
///
/// 实现独占 journal、current pointer、hash 与物理目录约定。调用者只能交换领域身份、
/// 逻辑路径和可验证描述符。 / Implementations exclusively own journals, current pointers,
/// hashes, and physical directory conventions. Callers exchange only domain identities, logical
/// paths, and verifiable descriptors.
pub trait GenerationRepository: Send + Sync {
    /// 适配器错误。 / Adapter error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// 原子发布一个完整 generation。 / Atomically publishes one complete generation.
    fn publish_generation(
        &self,
        target: &PublicationTargetId,
        publications: &[Publication],
    ) -> Result<CommittedGeneration, Self::Error>;

    /// 返回最近完整提交且已验证的 generation。 / Returns the latest completely committed and verified generation.
    fn current_generation(
        &self,
        target: &PublicationTargetId,
    ) -> Result<Option<CommittedGeneration>, Self::Error>;

    /// 解析 generation 成员、写入完整字节并验证 manifest、大小和摘要。 / Resolves a generation member, writes all bytes, and verifies membership, size, and digest.
    fn read_generation_artifact(
        &self,
        generation: &GenerationRef,
        destination: &PublicationPath,
        sink: &mut dyn Write,
    ) -> Result<ArtifactRead, Self::Error>;
}
