use std::io::{Read, Write};

use crate::{ActionKey, ContentDigest, ProducedOutput};

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
    /// 用户可见目标 URI；它不属于纯动作 key。 / User-visible destination URI; it is not part of the pure action key.
    pub destination: String,
}

/// 将不可变 blob 发布到用户可见位置的端口。 / Port publishing immutable blobs to user-visible locations.
pub trait ArtifactPublisher {
    /// 后端错误。 / Backend error.
    type Error;

    /// 原子发布或替换一个目标。 / Atomically publishes or replaces one destination.
    fn publish(&self, publication: &Publication) -> Result<(), Self::Error>;
}
