use std::{
    collections::HashMap,
    fmt,
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use squish_build::{
    ActionIndex as BuildIndex, ActionKey as BuildKey, ActionRecord, ContentDigest, OutputName,
    ProducedOutput,
};
use squish_protocol::{ArtifactKind, DigestAlgorithm};

use crate::{BlobDigest, Cas, CasError};

static REBUILD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 一个已规范化动作描述的 BLAKE3 键。 / A BLAKE3 key of a canonical action description.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionKey(BlobDigest);

impl ActionKey {
    /// 从规范化动作字节计算键。 / Computes a key from canonical action bytes.
    #[must_use]
    pub fn of(canonical_action: &[u8]) -> Self {
        Self(BlobDigest::of(canonical_action))
    }
    /// 从已经验证的摘要构造动作键。 / Constructs an action key from an already validated digest.
    #[must_use]
    pub const fn from_digest(digest: BlobDigest) -> Self {
        Self(digest)
    }
    /// 返回底层摘要。 / Returns the underlying digest.
    #[must_use]
    pub const fn digest(self) -> BlobDigest {
        self.0
    }
}

/// 动作索引中的轻量结果引用。 / Lightweight result references in the action index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionEntry {
    /// 按稳定语义顺序排列的结果 blob。 / Result blobs in stable semantic order.
    pub results: Vec<BlobDigest>,
    /// 最近访问时间，Unix epoch 毫秒。 / Last access time in Unix-epoch milliseconds.
    pub last_used_unix_ms: i64,
}

/// 单个可检查的完整动作清单。 / One inspectable, complete action manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionManifest {
    /// 构建边界使用的完整类型化记录。 / Fully typed record used at the build boundary.
    pub record: ActionRecord,
    /// 快照中记录的最近使用时间。 / Last-use time recorded in the snapshot.
    pub last_used_unix_ms: i64,
}

/// 清单目录中被省略候选项的原因。 / Reason a catalog candidate was omitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogIssueKind {
    /// 旧式轻量摘要行不是完整清单。 / A legacy digest-only row is not a complete manifest.
    DigestOnly,
    /// SQLite 行不能解码为领域清单。 / SQLite rows could not be decoded as a domain manifest.
    InvalidManifest(String),
    /// 至少一个声明的 CAS blob 缺失或摘要校验失败。 / A declared CAS blob was missing or failed digest verification.
    OutputUnavailable,
    /// CAS blob 的长度与清单不符。 / A CAS blob length disagreed with the manifest.
    OutputSizeMismatch,
}

/// 一个被省略的目录候选项。 / One omitted catalog candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogIssue {
    /// 候选动作键。 / Candidate action key.
    pub key: ActionKey,
    /// 省略原因。 / Omission reason.
    pub kind: CatalogIssueKind,
}

/// 有界、稳定排序的动作清单页。 / A bounded, stably ordered page of action manifests.
///
/// `next_after` 是本页最后扫描的键，而不一定是最后返回的记录；这保证损坏或旧式行
/// 不会令调用方分页停滞。每次调用来自一个 SQLite 读快照，因此并发 `clear` 或写入
/// 不会产生撕裂清单。跨页写入可能自然出现在后续页中；需要全局冻结视图的调用方应
/// 在上层持有维护租约。 / `next_after` is the last scanned key, not necessarily the
/// last returned record, so corrupt or legacy rows cannot stall pagination. Each call
/// comes from one SQLite read snapshot, preventing concurrent clears or writes from
/// producing torn manifests. Writes between pages may naturally appear on later pages;
/// callers needing a globally frozen view should hold an upper-layer maintenance lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionManifestPage {
    /// 已解码的完整清单，按动作键升序；验证索引还会完成 CAS 校验。 / Decoded complete manifests in ascending action-key order; the verified index also completes CAS validation.
    pub manifests: Vec<ActionManifest>,
    /// 下一页的排他游标；`None` 表示快照中已无更多候选。 / Exclusive cursor for the next page; `None` means the snapshot had no more candidates.
    pub next_after: Option<ActionKey>,
    /// 本页检查的候选数。 / Number of candidates inspected by this page.
    pub scanned: usize,
    /// 被安全省略的候选及原因。 / Candidates safely omitted, with reasons.
    pub issues: Vec<CatalogIssue>,
    /// 验证层从可重建索引删除的失效行数。 / Invalid rebuildable rows removed by the verification layer.
    pub repaired: usize,
}

/// 单页最多扫描的动作数。 / Maximum number of actions scanned by one page.
pub const MAX_MANIFEST_PAGE_SIZE: usize = 4096;

/// 一条轻量运行事件索引；详细负载应放入 CAS。 / A lightweight run-event index; detailed payload belongs in the CAS.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEvent {
    /// 单次运行的稳定标识。 / Stable identifier of one run.
    pub run_id: String,
    /// 运行内严格递增的序号。 / Strictly increasing sequence within the run.
    pub sequence: u64,
    /// 小型稳定事件类别。 / Small stable event kind.
    pub kind: String,
    /// 可选详细负载的 CAS 摘要。 / Optional CAS digest of a detailed payload.
    pub payload: Option<BlobDigest>,
    /// 事件发生时间，Unix epoch 毫秒。 / Event occurrence time in Unix-epoch milliseconds.
    pub occurred_unix_ms: i64,
}

/// 可丢弃动作索引失败。 / Disposable action-index failure.
#[derive(Debug)]
pub struct IndexError(String);
impl fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for IndexError {}
impl From<rusqlite::Error> for IndexError {
    fn from(error: rusqlite::Error) -> Self {
        Self(format!("action index failed: {error}"))
    }
}
impl From<CasError> for IndexError {
    fn from(error: CasError) -> Self {
        Self(format!("action record blob validation failed: {error}"))
    }
}

#[derive(Default)]
struct MemoryState {
    actions: HashMap<ActionKey, ActionEntry>,
    events: Vec<RunEvent>,
}

/// 用于测试和短生命周期进程的内存动作索引。 / In-memory action index for tests and short-lived processes.
#[derive(Default)]
pub struct MemoryActionIndex(Mutex<MemoryState>);

impl MemoryActionIndex {
    /// 创建空索引。 / Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// 插入或完整替换一个动作结果。 / Inserts or fully replaces an action result.
    pub fn put(&self, key: ActionKey, entry: ActionEntry) {
        self.0
            .lock()
            .expect("memory action index poisoned")
            .actions
            .insert(key, entry);
    }
    /// 查找动作，并用调用方提供的时间更新 LRU。 / Looks up an action and updates LRU with the caller-provided time.
    #[must_use]
    pub fn get(&self, key: ActionKey, accessed_unix_ms: i64) -> Option<ActionEntry> {
        let mut state = self.0.lock().expect("memory action index poisoned");
        let entry = state.actions.get_mut(&key)?;
        entry.last_used_unix_ms = accessed_unix_ms;
        Some(entry.clone())
    }
    /// 返回从最久未使用到最近使用的键。 / Returns keys from least to most recently used.
    #[must_use]
    pub fn lru(&self, limit: usize) -> Vec<ActionKey> {
        let state = self.0.lock().expect("memory action index poisoned");
        let mut values: Vec<_> = state.actions.iter().collect();
        values.sort_by_key(|(key, entry)| (entry.last_used_unix_ms, **key));
        values
            .into_iter()
            .take(limit)
            .map(|(key, _)| *key)
            .collect()
    }
    /// 追加一条运行事件。 / Appends one run event.
    pub fn record_event(&self, event: RunEvent) {
        self.0
            .lock()
            .expect("memory action index poisoned")
            .events
            .push(event);
    }
    /// 按序号返回指定运行的事件。 / Returns one run's events ordered by sequence.
    #[must_use]
    pub fn events(&self, run_id: &str) -> Vec<RunEvent> {
        let state = self.0.lock().expect("memory action index poisoned");
        let mut events: Vec<_> = state
            .events
            .iter()
            .filter(|event| event.run_id == run_id)
            .cloned()
            .collect();
        events.sort_by_key(|event| event.sequence);
        events
    }
    /// 清空所有可重建状态。 / Clears all rebuildable state.
    pub fn clear(&self) {
        *self.0.lock().expect("memory action index poisoned") = MemoryState::default();
    }
}

/// SQLite WAL 动作索引；数据库可删除并通过重新构建逐步填充。 / SQLite-WAL action index; the database may be deleted and repopulated by clean builds.
///
/// 表中仅保存动作键、blob 摘要、LRU 时间和小型事件元数据。blob 始终在 CAS 中。
/// Tables contain only action keys, blob digests, LRU timestamps, and small event
/// metadata. Blobs always remain in the CAS.
pub struct SqliteActionIndex {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl SqliteActionIndex {
    /// 打开索引、启用 WAL 并迁移可重建 schema。 / Opens the index, enables WAL, and migrates the rebuildable schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| IndexError(format!("create index directory: {error}")))?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        retry_locked(|| connection.pragma_update(None, "journal_mode", "WAL"))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        retry_locked(|| {
            let result = connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS actions (
                 action_key BLOB PRIMARY KEY CHECK(length(action_key) = 32),
                 last_used_unix_ms INTEGER NOT NULL,
                 record_kind TEXT NOT NULL DEFAULT 'digest-only'
                     CHECK(record_kind IN ('digest-only', 'manifest'))
             ) STRICT;
             CREATE TABLE IF NOT EXISTS action_results (
                 action_key BLOB NOT NULL REFERENCES actions(action_key) ON DELETE CASCADE,
                 ordinal INTEGER NOT NULL,
                 digest BLOB NOT NULL CHECK(length(digest) = 32),
                 output_name TEXT NOT NULL CHECK(output_name <> ''),
                 artifact_kind TEXT NOT NULL CHECK(artifact_kind IN ('binary-ir', 'prompt', 'debug-info', 'metadata', 'other')),
                 artifact_other TEXT,
                 size INTEGER NOT NULL CHECK(size >= 0),
                 CHECK((artifact_kind = 'other') = (artifact_other IS NOT NULL)),
                 PRIMARY KEY(action_key, ordinal)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS digest_results (
                 action_key BLOB NOT NULL REFERENCES actions(action_key) ON DELETE CASCADE,
                 ordinal INTEGER NOT NULL,
                 digest BLOB NOT NULL CHECK(length(digest) = 32),
                 PRIMARY KEY(action_key, ordinal)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS run_events (
                 run_id TEXT NOT NULL,
                 sequence INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 payload_digest BLOB CHECK(payload_digest IS NULL OR length(payload_digest) = 32),
                 occurred_unix_ms INTEGER NOT NULL,
                 PRIMARY KEY(run_id, sequence)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS actions_lru ON actions(last_used_unix_ms, action_key);
             COMMIT;",
            );
            // execute_batch may encounter a schema lock after BEGIN and leave the
            // connection inside that transaction. Roll it back before retrying so
            // the next attempt (or migrate_schema) starts from a clean state.
            // execute_batch 可能在 BEGIN 后遇到 schema 锁并遗留活动事务；重试前
            // 回滚，确保下一次尝试（或 migrate_schema）从干净连接状态开始。
            if result.is_err() {
                connection.execute_batch("ROLLBACK").ok();
            }
            result
        })?;
        migrate_schema(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            path: path.to_path_buf(),
        })
    }

    /// 在旧文件旁创建独立空索引，绝不删除或替换可能仍活动的数据库。 / Creates an independent empty index beside the old file, never deleting or replacing a possibly active database.
    ///
    /// SQLite/WAL 数据库不能仅凭路径判断是否仍被另一进程使用。在没有上层维护租约
    /// （maintenance lease）的情况下，本方法采用新的唯一 generation，而不是 unlink
    /// `db`、`db-wal` 或 `db-shm`。调用方可立即使用返回的索引；旧 generation 留待拥有
    /// 全局排他维护权的上层清理。映射、LRU 和运行历史由干净构建重新填充。
    /// A pathname cannot prove that another process no longer uses a SQLite/WAL
    /// database. Without an upper-layer maintenance lease, this method therefore
    /// allocates a unique generation instead of unlinking `db`, `db-wal`, or
    /// `db-shm`. The returned index is immediately usable; an upper layer holding
    /// global exclusive maintenance authority may later collect old generations.
    pub fn recreate(path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let fresh = reserve_generation(path.as_ref())?;
        Self::open(fresh)
    }

    /// 返回此连接实际使用的 generation 路径。 / Returns the generation path actually used by this connection.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 以单个事务插入或替换动作及其结果摘要。 / Inserts or replaces an action and its result digests in one transaction.
    pub fn put(&self, key: ActionKey, entry: &ActionEntry) -> Result<(), IndexError> {
        let mut connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO actions(action_key, last_used_unix_ms, record_kind)
             VALUES (?1, ?2, 'digest-only')
             ON CONFLICT(action_key) DO UPDATE SET
                 last_used_unix_ms=excluded.last_used_unix_ms,
                 record_kind='digest-only'",
            params![key.digest().as_bytes().as_slice(), entry.last_used_unix_ms],
        )?;
        transaction.execute(
            "DELETE FROM action_results WHERE action_key=?1",
            [key.digest().as_bytes().as_slice()],
        )?;
        transaction.execute(
            "DELETE FROM digest_results WHERE action_key=?1",
            [key.digest().as_bytes().as_slice()],
        )?;
        for (ordinal, digest) in entry.results.iter().enumerate() {
            transaction.execute(
                "INSERT INTO digest_results(action_key, ordinal, digest) VALUES (?1, ?2, ?3)",
                params![
                    key.digest().as_bytes().as_slice(),
                    ordinal as i64,
                    digest.as_bytes().as_slice()
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 查找动作，并在同一事务中更新 LRU 时间。 / Looks up an action and updates its LRU time in the same transaction.
    pub fn get(
        &self,
        key: ActionKey,
        accessed_unix_ms: i64,
    ) -> Result<Option<ActionEntry>, IndexError> {
        let mut connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM actions
                 WHERE action_key=?1 AND record_kind='digest-only'",
                [key.digest().as_bytes().as_slice()],
                |_| Ok(()),
            )
            .optional()?;
        if exists.is_none() {
            transaction.commit()?;
            return Ok(None);
        }
        transaction.execute(
            "UPDATE actions SET last_used_unix_ms=?2 WHERE action_key=?1",
            params![key.digest().as_bytes().as_slice(), accessed_unix_ms],
        )?;
        let results = {
            let mut statement = transaction.prepare(
                "SELECT digest FROM digest_results WHERE action_key=?1 ORDER BY ordinal",
            )?;
            statement
                .query_map([key.digest().as_bytes().as_slice()], |row| {
                    row.get::<_, Vec<u8>>(0)
                })?
                .map(|row| row.and_then(|bytes| digest_from_sql(&bytes)))
                .collect::<Result<Vec<_>, _>>()?
        };
        transaction.commit()?;
        Ok(Some(ActionEntry {
            results,
            last_used_unix_ms: accessed_unix_ms,
        }))
    }

    /// 返回从最久未使用到最近使用的动作键。 / Returns action keys from least to most recently used.
    pub fn lru(&self, limit: usize) -> Result<Vec<ActionKey>, IndexError> {
        let connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let mut statement = connection.prepare(
            "SELECT action_key FROM actions ORDER BY last_used_unix_ms, action_key LIMIT ?1",
        )?;
        let rows = statement.query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        Ok(rows
            .map(|row| {
                row.and_then(|bytes| digest_from_sql(&bytes))
                    .map(ActionKey::from_digest)
            })
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// 从一个一致的读快照枚举完整清单。 / Enumerates complete manifests from one consistent read snapshot.
    ///
    /// `after` 是排他动作键游标。`limit` 必须非零，并会被限制为
    /// [`MAX_MANIFEST_PAGE_SIZE`]，从而避免检查接口意外分配无界内存。旧式
    /// digest-only 行和无法解码的清单不会作为有效记录返回，而会出现在
    /// [`ActionManifestPage::issues`] 中。 / `after` is an exclusive action-key cursor.
    /// `limit` must be non-zero and is capped at [`MAX_MANIFEST_PAGE_SIZE`] to keep
    /// inspection memory bounded. Legacy digest-only rows and undecodable manifests
    /// are reported in [`ActionManifestPage::issues`] rather than returned as valid.
    ///
    /// # Example / 示例
    /// ```no_run
    /// # use squish_store::{ActionKey, IndexError, VerifiedActionIndex};
    /// # fn inspect(index: &VerifiedActionIndex) -> Result<(), IndexError> {
    /// let mut after: Option<ActionKey> = None;
    /// loop {
    ///     let page = index.manifest_page(after, 256)?;
    ///     for manifest in page.manifests {
    ///         println!("{}", manifest.record.key.as_str());
    ///     }
    ///     let Some(next) = page.next_after else { break };
    ///     after = Some(next);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn manifest_page(
        &self,
        after: Option<ActionKey>,
        limit: usize,
    ) -> Result<ActionManifestPage, IndexError> {
        if limit == 0 {
            return Err(IndexError("manifest page limit must be non-zero".into()));
        }
        let limit = limit.min(MAX_MANIFEST_PAGE_SIZE);
        let fetch = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        let mut connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let transaction = connection.transaction()?;
        let candidates = {
            let mut statement = transaction.prepare(
                "SELECT action_key, record_kind, last_used_unix_ms
                 FROM actions
                 WHERE (?1 IS NULL OR action_key > ?1)
                 ORDER BY action_key
                 LIMIT ?2",
            )?;
            let after = after.map(|key| key.digest().as_bytes().to_vec());
            statement
                .query_map(params![after, fetch], |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let has_more = candidates.len() > limit;
        let candidates = &candidates[..candidates.len().min(limit)];
        let mut manifests = Vec::with_capacity(candidates.len());
        let mut issues = Vec::new();
        for (key_bytes, record_kind, last_used_unix_ms) in candidates {
            let key = ActionKey::from_digest(digest_from_sql(key_bytes)?);
            if record_kind != "manifest" {
                issues.push(CatalogIssue {
                    key,
                    kind: CatalogIssueKind::DigestOnly,
                });
                continue;
            }
            match load_manifest_from(&transaction, key) {
                Ok(Some(outputs)) => manifests.push(ActionManifest {
                    record: ActionRecord {
                        key: build_key(key),
                        outputs,
                    },
                    last_used_unix_ms: *last_used_unix_ms,
                }),
                Ok(None) => issues.push(CatalogIssue {
                    key,
                    kind: CatalogIssueKind::InvalidManifest(
                        "manifest disappeared inside its read snapshot".into(),
                    ),
                }),
                Err(error) => issues.push(CatalogIssue {
                    key,
                    kind: CatalogIssueKind::InvalidManifest(error.to_string()),
                }),
            }
        }
        transaction.commit()?;
        Ok(ActionManifestPage {
            manifests,
            next_after: has_more
                .then(|| {
                    candidates
                        .last()
                        .expect("a full page has a final candidate")
                })
                .map(|(bytes, _, _)| digest_from_sql(bytes).map(ActionKey::from_digest))
                .transpose()?,
            scanned: candidates.len(),
            issues,
            repaired: 0,
        })
    }

    /// 插入或替换一条运行事件索引。 / Inserts or replaces one run-event index row.
    pub fn record_event(&self, event: &RunEvent) -> Result<(), IndexError> {
        let connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let sequence = i64::try_from(event.sequence)
            .map_err(|_| IndexError("run event sequence exceeds SQLite INTEGER".into()))?;
        connection.execute(
            "INSERT OR REPLACE INTO run_events(run_id, sequence, kind, payload_digest, occurred_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![event.run_id, sequence, event.kind, event.payload.map(|value| value.as_bytes().to_vec()), event.occurred_unix_ms],
        )?;
        Ok(())
    }

    /// 按序号返回指定运行的事件。 / Returns one run's events ordered by sequence.
    pub fn events(&self, run_id: &str) -> Result<Vec<RunEvent>, IndexError> {
        let connection = self
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let mut statement = connection.prepare(
            "SELECT sequence, kind, payload_digest, occurred_unix_ms FROM run_events WHERE run_id=?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([run_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        Ok(rows
            .map(|row| {
                let (sequence, kind, payload, occurred_unix_ms) = row?;
                let sequence = u64::try_from(sequence)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, sequence))?;
                let payload = payload.map(|bytes| digest_from_sql(&bytes)).transpose()?;
                Ok(RunEvent {
                    run_id: run_id.to_owned(),
                    sequence,
                    kind,
                    payload,
                    occurred_unix_ms,
                })
            })
            .collect::<Result<Vec<_>, rusqlite::Error>>()?)
    }

    /// 清空所有可重建协调状态，但不接触 CAS。 / Clears all rebuildable coordination state without touching the CAS.
    pub fn clear(&self) -> Result<(), IndexError> {
        self.connection.lock().expect("SQLite action index poisoned").execute_batch(
            "BEGIN; DELETE FROM action_results; DELETE FROM digest_results; DELETE FROM actions; DELETE FROM run_events; COMMIT;",
        )?;
        Ok(())
    }
}

/// 为构建端口绑定 CAS 的动作索引适配器。 / Action-index adapter binding the build port to a CAS.
///
/// 构造时强制注入 CAS，因此每次命中都能验证完整输出清单，而不会存在“忘记配置
/// 验证器”的运行时模式。缺失、摘要损坏或大小不符的输出会使整个动作成为 miss，并
/// 丢弃可重建索引行；随后成功执行可用 [`BuildIndex::record`] 正常修复。
/// Construction requires a CAS, so every hit validates its complete output manifest
/// without an optional verifier mode. A missing, corrupt, or incorrectly-sized output
/// turns the whole action into a miss and drops the rebuildable row; a subsequent
/// successful execution repairs it through [`BuildIndex::record`].
pub struct VerifiedActionIndex {
    index: SqliteActionIndex,
    cas: Arc<Cas>,
}

impl VerifiedActionIndex {
    /// 打开 SQLite 索引并绑定权威 CAS。 / Opens a SQLite index and binds its authoritative CAS.
    pub fn open(path: impl AsRef<Path>, cas: Arc<Cas>) -> Result<Self, IndexError> {
        Ok(Self {
            index: SqliteActionIndex::open(path)?,
            cas,
        })
    }

    /// 从已打开索引与 CAS 构造适配器。 / Constructs the adapter from an open index and CAS.
    #[must_use]
    pub fn from_parts(index: SqliteActionIndex, cas: Arc<Cas>) -> Self {
        Self { index, cas }
    }

    /// 返回底层领域索引。 / Returns the underlying domain index.
    #[must_use]
    pub fn index(&self) -> &SqliteActionIndex {
        &self.index
    }

    /// 枚举并逐输出验证一页完整清单。 / Enumerates and validates every output in one page of complete manifests.
    ///
    /// SQLite 快照先产生类型化候选，随后每个输出通过 CAS 重新计算摘要并检查长度。
    /// 失效记录不会返回；若记录在并发修复期间未改变，验证器会删除旧索引行并增加
    /// `repaired`。 / A SQLite snapshot first produces typed candidates, after which
    /// every output is rehashed by the CAS and length-checked. Invalid records are
    /// omitted; if a record did not change during a concurrent repair, its stale index
    /// row is deleted and `repaired` is incremented.
    pub fn manifest_page(
        &self,
        after: Option<ActionKey>,
        limit: usize,
    ) -> Result<ActionManifestPage, IndexError> {
        let mut page = self.index.manifest_page(after, limit)?;
        let candidates = std::mem::take(&mut page.manifests);
        for manifest in candidates {
            let key = parse_build_key(&manifest.record.key)?;
            let mut failure = None;
            for output in &manifest.record.outputs {
                let digest = match blob_digest(&output.digest) {
                    Ok(digest) => digest,
                    Err(error) => {
                        failure = Some(CatalogIssueKind::InvalidManifest(error.to_string()));
                        break;
                    }
                };
                let Some(bytes) = self.cas.get(digest)? else {
                    failure = Some(CatalogIssueKind::OutputUnavailable);
                    break;
                };
                if u64::try_from(bytes.len()).ok() != Some(output.size) {
                    failure = Some(CatalogIssueKind::OutputSizeMismatch);
                    break;
                }
            }
            if let Some(kind) = failure {
                page.issues.push(CatalogIssue { key, kind });
                if self.remove_manifest_if_unchanged(key, &manifest)? {
                    page.repaired += 1;
                }
            } else {
                page.manifests.push(manifest);
            }
        }
        Ok(page)
    }
}

impl BuildIndex for VerifiedActionIndex {
    type Error = IndexError;

    fn lookup(&self, key: &BuildKey) -> Result<Option<ActionRecord>, Self::Error> {
        let stored_key = parse_build_key(key)?;
        let Some(outputs) = self.load_manifest(stored_key)? else {
            return Ok(None);
        };
        for output in &outputs {
            let digest = blob_digest(&output.digest)?;
            let Some(bytes) = self.cas.get(digest)? else {
                self.remove(stored_key)?;
                return Ok(None);
            };
            if u64::try_from(bytes.len()).ok() != Some(output.size) {
                self.remove(stored_key)?;
                return Ok(None);
            }
        }
        self.touch(stored_key, now_unix_ms())?;
        Ok(Some(ActionRecord {
            key: key.clone(),
            outputs,
        }))
    }

    fn record(&self, record: &ActionRecord) -> Result<(), Self::Error> {
        let key = parse_build_key(&record.key)?;
        let mut connection = self
            .index
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO actions(action_key, last_used_unix_ms, record_kind)
             VALUES (?1, ?2, 'manifest')
             ON CONFLICT(action_key) DO UPDATE SET
                 last_used_unix_ms=excluded.last_used_unix_ms,
                 record_kind='manifest'",
            params![key.digest().as_bytes().as_slice(), now_unix_ms()],
        )?;
        transaction.execute(
            "DELETE FROM action_results WHERE action_key=?1",
            [key.digest().as_bytes().as_slice()],
        )?;
        transaction.execute(
            "DELETE FROM digest_results WHERE action_key=?1",
            [key.digest().as_bytes().as_slice()],
        )?;
        for (ordinal, output) in record.outputs.iter().enumerate() {
            let digest = blob_digest(&output.digest)?;
            let (kind, other) = encode_kind(&output.kind)?;
            let size = i64::try_from(output.size)
                .map_err(|_| IndexError("output size exceeds SQLite INTEGER".into()))?;
            transaction.execute(
                "INSERT INTO action_results(action_key, ordinal, digest, output_name, artifact_kind, artifact_other, size)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    key.digest().as_bytes().as_slice(),
                    i64::try_from(ordinal).map_err(|_| IndexError("too many action outputs".into()))?,
                    digest.as_bytes().as_slice(),
                    output.name.as_str(),
                    kind,
                    other,
                    size,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

impl VerifiedActionIndex {
    fn load_manifest(&self, key: ActionKey) -> Result<Option<Vec<ProducedOutput>>, IndexError> {
        let connection = self
            .index
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        load_manifest_from(&connection, key)
    }

    fn remove_manifest_if_unchanged(
        &self,
        key: ActionKey,
        expected: &ActionManifest,
    ) -> Result<bool, IndexError> {
        let mut connection = self
            .index
            .connection
            .lock()
            .expect("SQLite action index poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_time = transaction
            .query_row(
                "SELECT last_used_unix_ms FROM actions
                 WHERE action_key=?1 AND record_kind='manifest'",
                [key.digest().as_bytes().as_slice()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let unchanged = current_time == Some(expected.last_used_unix_ms)
            && load_manifest_from(&transaction, key)?.as_deref()
                == Some(expected.record.outputs.as_slice());
        // Recheck the authoritative CAS while holding the index write transaction.
        // A concurrent producer may have repaired the blob after the first check but
        // before publishing its index record; deleting then would discard good work.
        // 持有索引写事务时再次检查权威 CAS。并发生产者可能在首次检查后修复 blob、
        // 尚未来得及发布索引记录；此时删除会误丢弃有效工作。
        let still_invalid = unchanged && !self.outputs_available(&expected.record.outputs)?;
        let removed = if still_invalid {
            transaction.execute(
                "DELETE FROM actions WHERE action_key=?1 AND record_kind='manifest'",
                [key.digest().as_bytes().as_slice()],
            )?
        } else {
            0
        };
        transaction.commit()?;
        Ok(removed != 0)
    }

    fn outputs_available(&self, outputs: &[ProducedOutput]) -> Result<bool, IndexError> {
        for output in outputs {
            let digest = blob_digest(&output.digest)?;
            let Some(bytes) = self.cas.get(digest)? else {
                return Ok(false);
            };
            if u64::try_from(bytes.len()).ok() != Some(output.size) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn load_manifest_from(
    connection: &Connection,
    key: ActionKey,
) -> Result<Option<Vec<ProducedOutput>>, IndexError> {
    // 单条 LEFT JOIN 在一个 SQLite statement snapshot 中同时判定记录类型与读取
    // 清单。这样并发 clear 不可能落在“存在性检查”和结果读取之间制造假空清单。
    // One LEFT JOIN determines the record kind and reads the manifest from one
    // SQLite statement snapshot. A concurrent clear therefore cannot fabricate
    // an empty manifest between a separate existence check and result query.
    let mut statement = connection.prepare(
        "SELECT a.record_kind, r.digest, r.output_name, r.artifact_kind,
                    r.artifact_other, r.size
             FROM actions AS a
             LEFT JOIN action_results AS r ON r.action_key = a.action_key
             WHERE a.action_key=?1
             ORDER BY r.ordinal",
    )?;
    let rows = statement.query_map([key.digest().as_bytes().as_slice()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<Vec<u8>>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut outputs = Vec::new();
    let mut saw_action = false;
    for row in rows {
        saw_action = true;
        let (record_kind, digest, name, kind, other, size) = row?;
        if record_kind != "manifest" {
            return Ok(None);
        }
        let Some(digest) = digest else {
            // LEFT JOIN 的全 NULL 右侧唯一表示一个合法的零输出 manifest。
            // An all-NULL right side is the sole representation of a valid
            // zero-output manifest produced by the LEFT JOIN.
            if name.is_none() && kind.is_none() && other.is_none() && size.is_none() {
                continue;
            }
            return Err(IndexError("partial output manifest row".into()));
        };
        let digest = digest_from_sql(&digest)?;
        let name = OutputName::new(
            name.ok_or_else(|| IndexError("missing output name in manifest".into()))?,
        )
        .map_err(|_| IndexError("empty output name in rebuildable index".into()))?;
        let kind = decode_kind(
            &kind.ok_or_else(|| IndexError("missing artifact kind in manifest".into()))?,
            other,
        )?;
        let size = u64::try_from(
            size.ok_or_else(|| IndexError("missing output size in manifest".into()))?,
        )
        .map_err(|_| IndexError("negative output size in rebuildable index".into()))?;
        outputs.push(ProducedOutput {
            name,
            kind,
            digest: content_digest(digest),
            size,
        });
    }
    if !saw_action {
        return Ok(None);
    }
    Ok(Some(outputs))
}

impl VerifiedActionIndex {
    fn touch(&self, key: ActionKey, time: i64) -> Result<(), IndexError> {
        self.index
            .connection
            .lock()
            .expect("SQLite action index poisoned")
            .execute(
                "UPDATE actions SET last_used_unix_ms=?2 WHERE action_key=?1",
                params![key.digest().as_bytes().as_slice(), time],
            )?;
        Ok(())
    }

    fn remove(&self, key: ActionKey) -> Result<(), IndexError> {
        self.index
            .connection
            .lock()
            .expect("SQLite action index poisoned")
            .execute(
                "DELETE FROM actions WHERE action_key=?1",
                [key.digest().as_bytes().as_slice()],
            )?;
        Ok(())
    }
}

fn parse_build_key(key: &BuildKey) -> Result<ActionKey, IndexError> {
    let hex = key
        .as_str()
        .strip_prefix("blake3:")
        .ok_or_else(|| IndexError("action key must use canonical blake3:<hex> form".into()))?;
    let digest = hex
        .parse::<BlobDigest>()
        .map_err(|_| IndexError("action key must use canonical blake3:<hex> form".into()))?;
    if key.as_str() != format!("blake3:{}", digest.to_hex()) {
        return Err(IndexError(
            "action key must use canonical lowercase blake3:<hex> form".into(),
        ));
    }
    Ok(ActionKey::from_digest(digest))
}

fn build_key(key: ActionKey) -> BuildKey {
    BuildKey::new(format!("blake3:{}", key.digest().to_hex()))
        .expect("a stored BLAKE3 action key is canonical")
}

fn blob_digest(digest: &ContentDigest) -> Result<BlobDigest, IndexError> {
    if digest.algorithm() != &DigestAlgorithm::Blake3 {
        return Err(IndexError("CAS action output must use BLAKE3".into()));
    }
    let bytes: [u8; 32] = digest
        .bytes()
        .try_into()
        .map_err(|_| IndexError("invalid BLAKE3 output digest length".into()))?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn content_digest(digest: BlobDigest) -> ContentDigest {
    ContentDigest::new(DigestAlgorithm::Blake3, digest.as_bytes().to_vec())
        .expect("BLAKE3 has the protocol's canonical digest length")
}

fn encode_kind(kind: &ArtifactKind) -> Result<(&'static str, Option<&str>), IndexError> {
    Ok(match kind {
        ArtifactKind::BinaryIr => ("binary-ir", None),
        ArtifactKind::Prompt => ("prompt", None),
        ArtifactKind::DebugInfo => ("debug-info", None),
        ArtifactKind::Metadata => ("metadata", None),
        ArtifactKind::Other(name) if !name.is_empty() => ("other", Some(name)),
        ArtifactKind::Other(_) => {
            return Err(IndexError("custom artifact kind must not be empty".into()));
        }
    })
}

fn decode_kind(kind: &str, other: Option<String>) -> Result<ArtifactKind, IndexError> {
    match (kind, other) {
        ("binary-ir", None) => Ok(ArtifactKind::BinaryIr),
        ("prompt", None) => Ok(ArtifactKind::Prompt),
        ("debug-info", None) => Ok(ArtifactKind::DebugInfo),
        ("metadata", None) => Ok(ArtifactKind::Metadata),
        ("other", Some(name)) if !name.is_empty() => Ok(ArtifactKind::Other(name)),
        _ => Err(IndexError(
            "invalid artifact kind in rebuildable index".into(),
        )),
    }
}

fn now_unix_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// 重试 SQLite 不受 busy timeout 约束的 schema-lock 竞态。 / Retries SQLite schema-lock races not covered by the busy timeout.
fn retry_locked<T>(mut operation: impl FnMut() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    const ATTEMPTS: usize = 250;
    for attempt in 0..ATTEMPTS {
        match operation() {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) && attempt + 1 < ATTEMPTS =>
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            result => return result,
        }
    }
    unreachable!("the final retry always returns")
}

fn migrate_schema(connection: &mut Connection) -> Result<(), IndexError> {
    // BEGIN IMMEDIATE serializes the inspect-and-alter sequence across processes.
    // Without it, two first-open callers can both observe a missing column and race ALTER.
    // BEGIN IMMEDIATE 会跨进程序列化“检查后修改”；否则两个首次打开者可能同时
    // 看到缺列并竞态执行 ALTER。
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let action_columns = transaction
        .prepare("PRAGMA table_info(actions)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !action_columns.iter().any(|column| column == "record_kind") {
        transaction.execute_batch(
            "ALTER TABLE actions ADD COLUMN record_kind TEXT NOT NULL
                 DEFAULT 'digest-only'
                 CHECK(record_kind IN ('digest-only', 'manifest'));",
        )?;
    }
    let columns = transaction
        .prepare("PRAGMA table_info(action_results)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    let additions = [
        ("output_name", "TEXT NOT NULL DEFAULT ''"),
        ("artifact_kind", "TEXT NOT NULL DEFAULT ''"),
        ("artifact_other", "TEXT"),
        ("size", "INTEGER NOT NULL DEFAULT 0 CHECK(size >= 0)"),
    ];
    for (name, declaration) in additions {
        if !columns.iter().any(|column| column == name) {
            transaction.execute_batch(&format!(
                "ALTER TABLE action_results ADD COLUMN {name} {declaration}"
            ))?;
        }
    }
    // 旧 schema 只有摘要，没有重建完整输出清单所需的字段。它是可丢弃缓存，
    // 因此迁移时删除这些不完整动作，避免把伪造的默认元数据暴露为命中。
    // The old schema held digests only. Drop those incomplete rebuildable actions
    // during migration rather than exposing fabricated default metadata as hits.
    transaction.execute_batch(
        "DROP TABLE IF EXISTS temp.incomplete_manifest_keys;
         CREATE TEMP TABLE incomplete_manifest_keys AS
             SELECT DISTINCT action_key FROM action_results
             WHERE output_name = '' OR artifact_kind = '';
         DELETE FROM action_results WHERE action_key IN (
             SELECT action_key FROM incomplete_manifest_keys
         );
         DELETE FROM digest_results WHERE action_key IN (
             SELECT action_key FROM incomplete_manifest_keys
         );
         DELETE FROM actions WHERE action_key IN (
             SELECT action_key FROM incomplete_manifest_keys
         );
         DROP TABLE incomplete_manifest_keys;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// 旧名称的兼容别名。 / Compatibility alias for the former name.
pub type BuildActionIndex = VerifiedActionIndex;

fn digest_from_sql(bytes: &[u8]) -> rusqlite::Result<BlobDigest> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            bytes.len(),
            rusqlite::types::Type::Blob,
            Box::new(IndexError(
                "invalid digest length in rebuildable index".into(),
            )),
        )
    })?;
    Ok(BlobDigest::from_bytes(bytes))
}

fn reserve_generation(base: &Path) -> Result<PathBuf, IndexError> {
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| IndexError(format!("create index directory: {error}")))?;
    }
    loop {
        let sequence = REBUILD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = base.as_os_str().to_os_string();
        name.push(format!(".rebuild-{}-{sequence}", std::process::id()));
        let candidate = PathBuf::from(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(IndexError(format!(
                    "reserve fresh index generation: {error}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn database_path() -> (tempfile::TempDir, PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.temp/squish-store-tests");
        fs::create_dir_all(&root).expect("create project-local test root");
        let directory = tempfile::Builder::new()
            .prefix("index-")
            .tempdir_in(root)
            .unwrap();
        let path = directory.path().join("actions.sqlite3");
        (directory, path)
    }

    #[test]
    fn memory_index_tracks_lru() {
        let index = MemoryActionIndex::new();
        let older = ActionKey::of(b"older");
        let newer = ActionKey::of(b"newer");
        index.put(
            older,
            ActionEntry {
                results: vec![BlobDigest::of(b"a")],
                last_used_unix_ms: 1,
            },
        );
        index.put(
            newer,
            ActionEntry {
                results: vec![BlobDigest::of(b"b")],
                last_used_unix_ms: 2,
            },
        );
        assert_eq!(index.lru(2), vec![older, newer]);
        assert_eq!(index.get(older, 3).unwrap().last_used_unix_ms, 3);
        assert_eq!(index.lru(2), vec![newer, older]);
    }

    #[test]
    fn sqlite_index_is_wal_transactional_and_reopenable() {
        let (_directory, path) = database_path();
        let key = ActionKey::of(b"compile module");
        let outputs = vec![BlobDigest::of(b"ir"), BlobDigest::of(b"debug")];
        {
            let index = SqliteActionIndex::open(&path).unwrap();
            index
                .put(
                    key,
                    &ActionEntry {
                        results: outputs.clone(),
                        last_used_unix_ms: 10,
                    },
                )
                .unwrap();
            let event = RunEvent {
                run_id: "run-1".into(),
                sequence: 0,
                kind: "action-finished".into(),
                payload: Some(outputs[0]),
                occurred_unix_ms: 11,
            };
            index.record_event(&event).unwrap();
            assert_eq!(index.events("run-1").unwrap(), vec![event]);
            let connection = index.connection.lock().unwrap();
            assert_eq!(
                connection
                    .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                    .unwrap()
                    .to_ascii_lowercase(),
                "wal"
            );
        }
        let index = SqliteActionIndex::open(&path).unwrap();
        assert_eq!(
            index.get(key, 20).unwrap().unwrap(),
            ActionEntry {
                results: outputs,
                last_used_unix_ms: 20
            }
        );
    }

    #[test]
    fn clear_discards_index_state() {
        let (_directory, path) = database_path();
        let index = SqliteActionIndex::open(path).unwrap();
        let key = ActionKey::of(b"action");
        index
            .put(
                key,
                &ActionEntry {
                    results: vec![BlobDigest::of(b"blob")],
                    last_used_unix_ms: 1,
                },
            )
            .unwrap();
        index.clear().unwrap();
        assert_eq!(index.get(key, 2).unwrap(), None);
    }

    #[test]
    fn corrupt_database_can_be_explicitly_recreated() {
        let (_directory, path) = database_path();
        fs::write(&path, b"not a sqlite database").unwrap();
        assert!(SqliteActionIndex::open(&path).is_err());
        let index = SqliteActionIndex::recreate(&path).unwrap();
        assert_ne!(index.path(), path);
        assert_eq!(fs::read(&path).unwrap(), b"not a sqlite database");
        assert!(index.lru(1).unwrap().is_empty());
    }

    #[test]
    fn recreate_never_unlinks_an_active_database_or_its_wal() {
        let (_directory, path) = database_path();
        let active = std::sync::Arc::new(SqliteActionIndex::open(&path).unwrap());
        let original = ActionKey::of(b"original");
        active
            .put(
                original,
                &ActionEntry {
                    results: vec![BlobDigest::of(b"old")],
                    last_used_unix_ms: 1,
                },
            )
            .unwrap();

        let active_writer = std::sync::Arc::clone(&active);
        let writer = std::thread::spawn(move || {
            for time in 2..20 {
                active_writer.get(original, time).unwrap().unwrap();
            }
        });
        let fresh = SqliteActionIndex::recreate(&path).unwrap();
        writer.join().unwrap();

        assert_ne!(fresh.path(), active.path());
        assert!(path.exists());
        assert!(active.get(original, 21).unwrap().is_some());
        assert!(fresh.get(original, 21).unwrap().is_none());
    }

    #[test]
    fn verified_index_validates_every_blob_and_repairs_a_miss() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let bytes = b"verified output";
        let digest = cas.put(bytes).unwrap();
        let key = BuildKey::new(format!("blake3:{}", BlobDigest::of(b"action").to_hex())).unwrap();
        let record = ActionRecord {
            key: key.clone(),
            outputs: vec![ProducedOutput {
                name: OutputName::new("main").unwrap(),
                kind: ArtifactKind::Prompt,
                digest: content_digest(digest),
                size: bytes.len() as u64,
            }],
        };
        let index = VerifiedActionIndex::open(path, Arc::clone(&cas)).unwrap();

        BuildIndex::record(&index, &record).unwrap();
        assert_eq!(
            BuildIndex::lookup(&index, &key).unwrap(),
            Some(record.clone())
        );

        fs::write(cas.path_for(digest), b"corrupt").unwrap();
        assert_eq!(BuildIndex::lookup(&index, &key).unwrap(), None);
        assert_eq!(BuildIndex::lookup(&index, &key).unwrap(), None);

        cas.put(bytes).unwrap();
        BuildIndex::record(&index, &record).unwrap();
        assert_eq!(
            BuildIndex::lookup(&index, &key).unwrap(),
            Some(record.clone())
        );

        fs::remove_file(cas.path_for(digest)).unwrap();
        assert_eq!(BuildIndex::lookup(&index, &key).unwrap(), None);
        cas.put(bytes).unwrap();
        BuildIndex::record(&index, &record).unwrap();
        assert_eq!(BuildIndex::lookup(&index, &key).unwrap(), Some(record));
    }

    #[test]
    fn build_boundary_rejects_noncanonical_action_keys() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let index = VerifiedActionIndex::open(path, cas).unwrap();
        let uppercase = BuildKey::new(format!(
            "blake3:{}",
            BlobDigest::of(b"action").to_hex().to_uppercase()
        ))
        .unwrap();

        assert!(BuildIndex::lookup(&index, &uppercase).is_err());
    }

    #[test]
    fn digest_only_schema_migrates_to_an_explicit_miss() {
        let (_directory, path) = database_path();
        let key = ActionKey::of(b"legacy action");
        let digest = BlobDigest::of(b"legacy output");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE actions (
                     action_key BLOB PRIMARY KEY,
                     last_used_unix_ms INTEGER NOT NULL
                 );
                 CREATE TABLE action_results (
                     action_key BLOB NOT NULL REFERENCES actions(action_key),
                     ordinal INTEGER NOT NULL,
                     digest BLOB NOT NULL,
                     PRIMARY KEY(action_key, ordinal)
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions VALUES (?1, 1)",
                [key.digest().as_bytes().as_slice()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO action_results VALUES (?1, 0, ?2)",
                params![
                    key.digest().as_bytes().as_slice(),
                    digest.as_bytes().as_slice()
                ],
            )
            .unwrap();
        drop(connection);

        let migrated = SqliteActionIndex::open(path).unwrap();
        assert_eq!(migrated.get(key, 2).unwrap(), None);
    }

    #[test]
    fn digest_only_record_is_never_a_verified_manifest_hit() {
        let (directory, path) = database_path();
        let stored_key = ActionKey::of(b"same key");
        let build_key = BuildKey::new(format!("blake3:{}", stored_key.digest().to_hex())).unwrap();
        let raw = SqliteActionIndex::open(path).unwrap();
        raw.put(
            stored_key,
            &ActionEntry {
                results: vec![BlobDigest::of(b"digest only")],
                last_used_unix_ms: 1,
            },
        )
        .unwrap();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let verified = VerifiedActionIndex::from_parts(raw, cas);

        assert_eq!(BuildIndex::lookup(&verified, &build_key).unwrap(), None);
    }

    #[test]
    fn zero_output_manifest_is_distinct_from_an_absent_action() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let key = BuildKey::new(format!(
            "blake3:{}",
            BlobDigest::of(b"zero output").to_hex()
        ))
        .unwrap();
        let record = ActionRecord {
            key: key.clone(),
            outputs: Vec::new(),
        };
        let verified = VerifiedActionIndex::open(path, cas).unwrap();

        assert_eq!(BuildIndex::lookup(&verified, &key).unwrap(), None);
        BuildIndex::record(&verified, &record).unwrap();
        assert_eq!(BuildIndex::lookup(&verified, &key).unwrap(), Some(record));
    }

    #[test]
    fn concurrent_clear_never_fabricates_an_empty_manifest() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let bytes = b"one output";
        let digest = cas.put(bytes).unwrap();
        let key = BuildKey::new(format!("blake3:{}", BlobDigest::of(b"race").to_hex())).unwrap();
        let record = ActionRecord {
            key: key.clone(),
            outputs: vec![ProducedOutput {
                name: OutputName::new("main").unwrap(),
                kind: ArtifactKind::Prompt,
                digest: content_digest(digest),
                size: bytes.len() as u64,
            }],
        };
        let reader = VerifiedActionIndex::open(&path, Arc::clone(&cas)).unwrap();
        BuildIndex::record(&reader, &record).unwrap();
        let clearer = VerifiedActionIndex::open(&path, cas).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let clear_barrier = Arc::clone(&barrier);
        let clear_thread = std::thread::spawn(move || {
            clear_barrier.wait();
            clearer.index().clear().unwrap();
        });

        barrier.wait();
        let observed = BuildIndex::lookup(&reader, &key).unwrap();
        clear_thread.join().unwrap();
        assert!(observed.is_none() || observed == Some(record));
    }

    #[test]
    fn concurrent_first_open_serializes_legacy_migration() {
        let (_directory, path) = database_path();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE actions (
                     action_key BLOB PRIMARY KEY,
                     last_used_unix_ms INTEGER NOT NULL
                 );
                 CREATE TABLE action_results (
                     action_key BLOB NOT NULL REFERENCES actions(action_key),
                     ordinal INTEGER NOT NULL,
                     digest BLOB NOT NULL,
                     PRIMARY KEY(action_key, ordinal)
                 );",
            )
            .unwrap();
        drop(connection);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    SqliteActionIndex::open(path)
                })
            })
            .collect();

        for worker in workers {
            worker.join().unwrap().unwrap();
        }
    }

    fn catalog_record(cas: &Cas, label: &str, bytes: &[u8]) -> ActionRecord {
        let digest = cas.put(bytes).unwrap();
        let key = ActionKey::of(label.as_bytes());
        ActionRecord {
            key: build_key(key),
            outputs: vec![ProducedOutput {
                name: OutputName::new("main").unwrap(),
                kind: ArtifactKind::Prompt,
                digest: content_digest(digest),
                size: bytes.len() as u64,
            }],
        }
    }

    #[test]
    fn catalog_returns_complete_and_zero_output_manifests_but_not_digest_only_rows() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let index = VerifiedActionIndex::open(path, Arc::clone(&cas)).unwrap();
        let complete = catalog_record(&cas, "complete", b"contents");
        let zero = ActionRecord {
            key: build_key(ActionKey::of(b"zero")),
            outputs: Vec::new(),
        };
        BuildIndex::record(&index, &complete).unwrap();
        BuildIndex::record(&index, &zero).unwrap();
        index
            .index()
            .put(
                ActionKey::of(b"legacy"),
                &ActionEntry {
                    results: vec![BlobDigest::of(b"digest")],
                    last_used_unix_ms: 1,
                },
            )
            .unwrap();

        let page = index.manifest_page(None, 10).unwrap();
        let records: Vec<_> = page
            .manifests
            .iter()
            .map(|manifest| manifest.record.clone())
            .collect();
        assert!(records.contains(&complete));
        assert!(records.contains(&zero));
        assert_eq!(
            page.issues
                .iter()
                .filter(|issue| issue.kind == CatalogIssueKind::DigestOnly)
                .count(),
            1
        );
    }

    #[test]
    fn verified_catalog_omits_and_repairs_missing_or_corrupt_outputs() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let index = VerifiedActionIndex::open(path, Arc::clone(&cas)).unwrap();
        let missing = catalog_record(&cas, "missing", b"missing bytes");
        let corrupt = catalog_record(&cas, "corrupt", b"correct bytes");
        BuildIndex::record(&index, &missing).unwrap();
        BuildIndex::record(&index, &corrupt).unwrap();
        fs::remove_file(cas.path_for(blob_digest(&missing.outputs[0].digest).unwrap())).unwrap();
        fs::write(
            cas.path_for(blob_digest(&corrupt.outputs[0].digest).unwrap()),
            b"wrong bytes",
        )
        .unwrap();

        let page = index.manifest_page(None, 10).unwrap();
        assert!(page.manifests.is_empty());
        assert_eq!(page.repaired, 2);
        assert_eq!(
            page.issues
                .iter()
                .filter(|issue| issue.kind == CatalogIssueKind::OutputUnavailable)
                .count(),
            2
        );
        assert!(
            index
                .index()
                .manifest_page(None, 10)
                .unwrap()
                .manifests
                .is_empty()
        );
    }

    #[test]
    fn catalog_paging_is_bounded_and_ordered_by_action_key() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let index = VerifiedActionIndex::open(path, Arc::clone(&cas)).unwrap();
        for label in ["delta", "alpha", "charlie", "bravo", "echo"] {
            BuildIndex::record(&index, &catalog_record(&cas, label, label.as_bytes())).unwrap();
        }

        let mut after = None;
        let mut keys = Vec::new();
        loop {
            let page = index.manifest_page(after, 2).unwrap();
            assert!(page.scanned <= 2);
            keys.extend(
                page.manifests
                    .iter()
                    .map(|manifest| parse_build_key(&manifest.record.key).unwrap()),
            );
            let Some(next) = page.next_after else { break };
            after = Some(next);
        }
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
        assert_eq!(keys.len(), 5);
        assert!(index.manifest_page(None, 0).is_err());
    }

    #[test]
    fn catalog_snapshot_never_observes_a_torn_manifest_during_clear_and_write() {
        let (directory, path) = database_path();
        let cas = Arc::new(Cas::open(directory.path().join("cas")).unwrap());
        let reader = VerifiedActionIndex::open(&path, Arc::clone(&cas)).unwrap();
        let writer = VerifiedActionIndex::open(&path, Arc::clone(&cas)).unwrap();
        let record = catalog_record(&cas, "racing record", b"stable output");
        BuildIndex::record(&writer, &record).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writer_barrier = Arc::clone(&barrier);
        let expected = record.clone();
        let worker = std::thread::spawn(move || {
            writer_barrier.wait();
            for _ in 0..50 {
                writer.index().clear().unwrap();
                BuildIndex::record(&writer, &expected).unwrap();
            }
        });
        barrier.wait();
        for _ in 0..50 {
            let page = reader.manifest_page(None, 10).unwrap();
            assert!(page.issues.is_empty());
            assert!(
                page.manifests.is_empty()
                    || page.manifests
                        == vec![ActionManifest {
                            record: record.clone(),
                            last_used_unix_ms: page.manifests[0].last_used_unix_ms,
                        }]
            );
        }
        worker.join().unwrap();
    }
}
