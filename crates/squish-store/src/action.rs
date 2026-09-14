use std::{
    collections::HashMap,
    fmt,
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::BlobDigest;

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
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS actions (
                 action_key BLOB PRIMARY KEY CHECK(length(action_key) = 32),
                 last_used_unix_ms INTEGER NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS action_results (
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
        )?;
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
            "INSERT INTO actions(action_key, last_used_unix_ms) VALUES (?1, ?2)
             ON CONFLICT(action_key) DO UPDATE SET last_used_unix_ms=excluded.last_used_unix_ms",
            params![key.digest().as_bytes().as_slice(), entry.last_used_unix_ms],
        )?;
        transaction.execute(
            "DELETE FROM action_results WHERE action_key=?1",
            [key.digest().as_bytes().as_slice()],
        )?;
        for (ordinal, digest) in entry.results.iter().enumerate() {
            transaction.execute(
                "INSERT INTO action_results(action_key, ordinal, digest) VALUES (?1, ?2, ?3)",
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
                "SELECT 1 FROM actions WHERE action_key=?1",
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
                "SELECT digest FROM action_results WHERE action_key=?1 ORDER BY ordinal",
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
            "BEGIN; DELETE FROM action_results; DELETE FROM actions; DELETE FROM run_events; COMMIT;",
        )?;
        Ok(())
    }
}

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
}
