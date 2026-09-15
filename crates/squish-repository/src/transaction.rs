//! 可恢复文件事务的操作系统适配器。 / Operating-system adapter for recoverable file transactions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use squish_project::{CommitPreparation, JournalRecord, MutationPlan, MutationPlanner};

use crate::{
    RepositoryError,
    repository::{digest, missing_digest, workspace_members_digest, workspace_members_observation},
};

const STATE_DIR: &str = ".xmlsquish";
const TRANSACTIONS_DIR: &str = "transactions";
const LOCK_NAME: &str = "repository.lock";
const JOURNAL_NAME: &str = "journal.jsonl";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// 可注入的持久化边界。 / Injectable durability boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    /// 快照已持锁完成恢复，即将读取权威状态。 / A snapshot recovered under lock and is about to read authoritative state.
    SnapshotRead,
    /// 候选文件已同步。 / Candidate files have been synchronized.
    CandidatesStaged,
    /// 持久提交决定已同步。 / The durable commit decision has been synchronized.
    CommitDecided,
    /// 一个目标已替换。 / One target has been replaced.
    TargetReplaced,
    /// 完成记录已同步。 / The completion record has been synchronized.
    Completed,
    /// 恢复即将替换一个目标。 / Recovery is about to replace a target.
    RecoveryReplace,
}

/// 测试和宿主可实现的确定性故障注入器。 / Deterministic fault injector implemented by tests or hosts.
pub trait FaultInjector: Send + Sync {
    /// 在命名持久化边界返回错误以模拟中断。 / Returns an error at a named durability boundary to simulate interruption.
    fn check(&self, point: FaultPoint) -> io::Result<()>;
}

/// 永不注入故障的默认实现。 / Default implementation that never injects a fault.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFault;

impl FaultInjector for NoFault {
    fn check(&self, _point: FaultPoint) -> io::Result<()> {
        Ok(())
    }
}

/// 一次 formatter 文件替换。 / One formatter file replacement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatUpdate {
    /// 工作区相对路径。 / Workspace-relative path.
    pub path: PathBuf,
    /// formatter 读取时观察到的摘要。 / Digest observed by the formatter.
    pub expected_digest: String,
    /// 完整候选内容。 / Complete candidate contents.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct FormatStaged {
    files: Vec<(PathBuf, String)>,
}

struct WriterLock(File);

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

pub(crate) fn recover_transactions(
    root: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    let _lock = writer_lock(root)?;
    recover_locked(root, faults)
}

pub(crate) fn with_locked_repository<T>(
    root: &Path,
    faults: &dyn FaultInjector,
    operation: impl FnOnce() -> Result<T, RepositoryError>,
) -> Result<T, RepositoryError> {
    let _lock = writer_lock(root)?;
    recover_locked(root, faults)?;
    operation()
}

pub(crate) fn commit_plan(
    root: &Path,
    plan: &MutationPlan,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    validate_plan_paths(plan)?;
    let _lock = writer_lock(root)?;
    recover_locked(root, faults)?;
    let observed = observe(root, plan.observations.keys())?;
    let generation = generation();
    match MutationPlanner::prepare_commit(plan, &observed, generation, plan.retry_limit) {
        CommitPreparation::Ready { .. } => {}
        CommitPreparation::Retry { changed } | CommitPreparation::Contended { changed, .. } => {
            return Err(RepositoryError::Contended(changed));
        }
    }
    for file in &plan.files {
        let actual = digest_for_path(&file.path, &file.candidate);
        if actual != file.candidate_digest {
            return Err(RepositoryError::Layout(format!(
                "candidate digest for `{}` is invalid",
                file.path.display()
            )));
        }
    }

    let dir = transaction_dir(root, &format!("{}-{generation}", safe_id(&plan.id.0)));
    fs::create_dir_all(&dir).map_err(|e| RepositoryError::io(&dir, e))?;
    sync_parent(&dir)?;
    for (index, file) in plan.files.iter().enumerate() {
        stage(&dir, index, &file.candidate)?;
    }
    sync_directory(&dir)?;
    let journal = dir.join(JOURNAL_NAME);
    append(
        &journal,
        &JournalRecord::Staged {
            id: plan.id.clone(),
            generation,
            files: plan
                .files
                .iter()
                .map(|f| (f.path.clone(), f.candidate_digest.clone()))
                .collect(),
        },
    )?;
    sync_directory(&dir)?;
    faults
        .check(FaultPoint::CandidatesStaged)
        .map_err(|e| RepositoryError::io(&journal, e))?;
    append(
        &journal,
        &JournalRecord::CommitDecided {
            id: plan.id.clone(),
            generation,
        },
    )?;
    faults
        .check(FaultPoint::CommitDecided)
        .map_err(|e| RepositoryError::io(&journal, e))?;
    for (index, file) in plan.files.iter().enumerate() {
        replace_staged(root, &dir, index, &file.path, &file.candidate_digest)?;
        append(
            &journal,
            &JournalRecord::Replaced {
                id: plan.id.clone(),
                generation,
                path: file.path.clone(),
            },
        )?;
        faults
            .check(FaultPoint::TargetReplaced)
            .map_err(|e| RepositoryError::io(&journal, e))?;
    }
    append(
        &journal,
        &JournalRecord::Completed {
            id: plan.id.clone(),
            generation,
        },
    )?;
    faults
        .check(FaultPoint::Completed)
        .map_err(|e| RepositoryError::io(&journal, e))?;
    remove_transaction(&dir)
}

pub(crate) fn atomic_write(
    root: &Path,
    relative: &Path,
    expected_digest: &str,
    bytes: &[u8],
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    atomic_write_many(
        root,
        &[FormatUpdate {
            path: relative.to_path_buf(),
            expected_digest: expected_digest.into(),
            bytes: bytes.to_vec(),
        }],
        faults,
    )
}

pub(crate) fn atomic_write_many(
    root: &Path,
    updates: &[FormatUpdate],
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut unique = BTreeSet::new();
    for update in updates {
        validate_relative(&update.path)?;
        if !unique.insert(&update.path) {
            return Err(RepositoryError::Layout(format!(
                "duplicate format path `{}`",
                update.path.display()
            )));
        }
    }
    let _lock = writer_lock(root)?;
    recover_locked(root, faults)?;
    let observed = observe(root, updates.iter().map(|u| &u.path))?;
    let changed = updates
        .iter()
        .filter(|u| observed.get(&u.path) != Some(&u.expected_digest))
        .map(|u| u.path.clone())
        .collect::<Vec<_>>();
    if !changed.is_empty() {
        return Err(RepositoryError::Contended(changed));
    }

    let generation = generation();
    let dir = transaction_dir(root, &format!("fmt-{generation}"));
    fs::create_dir_all(&dir).map_err(|e| RepositoryError::io(&dir, e))?;
    sync_parent(&dir)?;
    let files = updates
        .iter()
        .map(|u| (u.path.clone(), digest_for_path(&u.path, &u.bytes)))
        .collect::<Vec<_>>();
    for (index, update) in updates.iter().enumerate() {
        stage(&dir, index, &update.bytes)?;
    }
    sync_directory(&dir)?;
    let metadata = dir.join("format.json");
    write_synced(
        &metadata,
        &serde_json::to_vec(&FormatStaged {
            files: files.clone(),
        })
        .map_err(|e| RepositoryError::Journal {
            path: metadata.clone(),
            message: e.to_string(),
        })?,
    )?;
    sync_directory(&dir)?;
    let decision = dir.join("commit");
    write_synced(&decision, b"commit\n")?;
    sync_directory(&dir)?;
    faults
        .check(FaultPoint::CommitDecided)
        .map_err(|e| RepositoryError::io(&decision, e))?;
    for (index, (path, expected)) in files.iter().enumerate() {
        replace_staged(root, &dir, index, path, expected)?;
        faults
            .check(FaultPoint::TargetReplaced)
            .map_err(|e| RepositoryError::io(path, e))?;
    }
    remove_transaction(&dir)
}

fn recover_locked(root: &Path, faults: &dyn FaultInjector) -> Result<(), RepositoryError> {
    let base = root.join(STATE_DIR).join(TRANSACTIONS_DIR);
    let entries = match fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(RepositoryError::io(&base, e)),
    };
    let mut dirs = entries
        .map(|entry| {
            entry
                .map(|e| e.path())
                .map_err(|e| RepositoryError::io(&base, e))
        })
        .collect::<Result<Vec<_>, _>>()?;
    dirs.sort();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let format = dir.join("format.json");
        if format.exists() {
            recover_format(root, &dir, faults)?;
            continue;
        }
        recover_mutation(root, &dir, faults)?;
    }
    Ok(())
}

fn recover_format(
    root: &Path,
    dir: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    if !dir.join("commit").exists() {
        return remove_transaction(dir);
    }
    let path = dir.join("format.json");
    let bytes = fs::read(&path).map_err(|e| RepositoryError::io(&path, e))?;
    let state: FormatStaged =
        serde_json::from_slice(&bytes).map_err(|e| RepositoryError::Journal {
            path: path.clone(),
            message: e.to_string(),
        })?;
    for (index, (target, digest)) in state.files.iter().enumerate() {
        faults
            .check(FaultPoint::RecoveryReplace)
            .map_err(|e| RepositoryError::io(target, e))?;
        replace_staged(root, dir, index, target, digest)?;
    }
    remove_transaction(dir)
}

fn recover_mutation(
    root: &Path,
    dir: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    let journal = dir.join(JOURNAL_NAME);
    if !journal.exists() {
        return remove_transaction(dir);
    }
    let records = read_records(&journal)?;
    let Some(JournalRecord::Staged { files, .. }) = records.first() else {
        return remove_transaction(dir);
    };
    let decided = records
        .iter()
        .any(|r| matches!(r, JournalRecord::CommitDecided { .. }));
    if !decided {
        return remove_transaction(dir);
    }
    for (index, (path, expected)) in files.iter().enumerate() {
        faults
            .check(FaultPoint::RecoveryReplace)
            .map_err(|e| RepositoryError::io(path, e))?;
        replace_staged(root, dir, index, path, expected)?;
    }
    remove_transaction(dir)
}

fn read_records(path: &Path) -> Result<Vec<JournalRecord>, RepositoryError> {
    let bytes = fs::read(path).map_err(|e| RepositoryError::io(path, e))?;
    let mut records = Vec::new();
    // Only a newline publishes a complete append; a crash may leave one torn trailing record.
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.ends_with(b"\n") {
            break;
        }
        let line = &line[..line.len() - 1];
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        records.push(
            serde_json::from_slice(line).map_err(|e| RepositoryError::Journal {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?,
        );
    }
    Ok(records)
}

fn append(path: &Path, record: &JournalRecord) -> Result<(), RepositoryError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| RepositoryError::io(path, e))?;
    serde_json::to_writer(&mut file, record).map_err(|e| RepositoryError::Journal {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    file.write_all(b"\n")
        .map_err(|e| RepositoryError::io(path, e))?;
    file.sync_all().map_err(|e| RepositoryError::io(path, e))
}

fn stage(dir: &Path, index: usize, bytes: &[u8]) -> Result<(), RepositoryError> {
    write_synced(&dir.join(format!("{index}.candidate")), bytes)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), RepositoryError> {
    let mut file = File::create(path).map_err(|e| RepositoryError::io(path, e))?;
    file.write_all(bytes)
        .map_err(|e| RepositoryError::io(path, e))?;
    file.sync_all().map_err(|e| RepositoryError::io(path, e))
}

fn replace_staged(
    root: &Path,
    dir: &Path,
    index: usize,
    relative: &Path,
    expected: &str,
) -> Result<(), RepositoryError> {
    validate_relative(relative)?;
    let target = root.join(relative);
    if let Ok(bytes) = fs::read(&target)
        && digest_for_path(relative, &bytes) == expected
    {
        return Ok(());
    }
    let staged = dir.join(format!("{index}.candidate"));
    let bytes = fs::read(&staged).map_err(|e| RepositoryError::io(&staged, e))?;
    if digest_for_path(relative, &bytes) != expected {
        return Err(RepositoryError::Journal {
            path: staged,
            message: "staged candidate digest mismatch".into(),
        });
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| RepositoryError::io(parent, e))?;
    }
    // Each recovery attempt needs its own rename source because a successful rename consumes it.
    let replacement = dir.join(format!("{index}.replacement"));
    write_synced(&replacement, &bytes)?;
    fs::rename(&replacement, &target).map_err(|e| RepositoryError::io(&target, e))?;
    sync_parent(&target)
}

fn observe<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> Result<BTreeMap<PathBuf, String>, RepositoryError> {
    paths
        .into_iter()
        .map(|path| {
            validate_relative(path)?;
            if path == &workspace_members_observation() {
                return Ok((path.clone(), workspace_members_digest(root)?));
            }
            let full = root.join(path);
            match fs::read(&full) {
                Ok(bytes) => Ok((path.clone(), digest_for_path(path, &bytes))),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    Ok((path.clone(), missing_digest()))
                }
                Err(e) => Err(RepositoryError::io(full, e)),
            }
        })
        .collect()
}

fn digest_for_path(path: &Path, bytes: &[u8]) -> String {
    if path
        .file_name()
        .is_some_and(|name| name == squish_project::MANIFEST_FILE_NAME)
    {
        digest(b"manifest", bytes)
    } else if path
        .file_name()
        .is_some_and(|name| name == squish_project::LOCK_FILE_NAME)
    {
        digest(b"lock", bytes)
    } else {
        digest(b"file", bytes)
    }
}

pub(crate) fn candidate_digest(path: &Path, bytes: &[u8]) -> Result<String, RepositoryError> {
    validate_relative(path)?;
    Ok(digest_for_path(path, bytes))
}

fn validate_plan_paths(plan: &MutationPlan) -> Result<(), RepositoryError> {
    for path in plan.observations.keys() {
        validate_relative(path)?;
    }
    for file in &plan.files {
        validate_relative(&file.path)?;
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<(), RepositoryError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(RepositoryError::Layout(format!(
            "transaction path `{}` is not a normalized workspace-relative path",
            path.display()
        )));
    }
    Ok(())
}

fn writer_lock(root: &Path) -> Result<WriterLock, RepositoryError> {
    let state = root.join(STATE_DIR);
    fs::create_dir_all(state.join(TRANSACTIONS_DIR)).map_err(|e| RepositoryError::io(&state, e))?;
    let path = state.join(LOCK_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| RepositoryError::io(&path, e))?;
    file.lock_exclusive()
        .map_err(|e| RepositoryError::io(&path, e))?;
    Ok(WriterLock(file))
}

fn transaction_dir(root: &Path, name: &str) -> PathBuf {
    root.join(STATE_DIR).join(TRANSACTIONS_DIR).join(name)
}
fn safe_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
fn generation() -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    time ^ NEXT_ID.fetch_add(1, Ordering::Relaxed) ^ u64::from(std::process::id())
}
fn remove_transaction(dir: &Path) -> Result<(), RepositoryError> {
    match fs::remove_dir_all(dir) {
        Ok(()) => sync_parent(dir),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(RepositoryError::io(dir, e)),
    }
}
fn sync_parent(path: &Path) -> Result<(), RepositoryError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), RepositoryError> {
    match File::open(path).and_then(|f| f.sync_all()) {
        Ok(()) => Ok(()),
        Err(e)
            if cfg!(windows)
                && matches!(
                    e.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput
                ) =>
        {
            Ok(())
        }
        Err(e) => Err(RepositoryError::io(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::atomic::{AtomicBool, Ordering},
    };

    use tempfile::TempDir;

    use super::*;
    use squish_project::{MutationFile, MutationKind, TransactionId};

    struct OnceAt {
        point: FaultPoint,
        fired: AtomicBool,
    }

    impl FaultInjector for OnceAt {
        fn check(&self, point: FaultPoint) -> io::Result<()> {
            if point == self.point && !self.fired.swap(true, Ordering::SeqCst) {
                Err(io::Error::new(io::ErrorKind::Interrupted, "injected"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn format_preflight_changes_nothing_when_any_input_is_stale() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("a.xml"), b"old-a").unwrap();
        fs::write(temp.path().join("b.xml"), b"old-b").unwrap();
        let updates = [
            FormatUpdate {
                path: "a.xml".into(),
                expected_digest: digest_for_path(Path::new("a.xml"), b"old-a"),
                bytes: b"new-a".to_vec(),
            },
            FormatUpdate {
                path: "b.xml".into(),
                expected_digest: digest_for_path(Path::new("b.xml"), b"stale"),
                bytes: b"new-b".to_vec(),
            },
        ];

        let result = atomic_write_many(temp.path(), &updates, &NoFault);

        assert!(matches!(result, Err(RepositoryError::Contended(_))));
        assert_eq!(fs::read(temp.path().join("a.xml")).unwrap(), b"old-a");
        assert_eq!(fs::read(temp.path().join("b.xml")).unwrap(), b"old-b");
    }

    #[test]
    fn recovery_rolls_forward_all_format_files_after_commit_decision() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("a.xml"), b"old-a").unwrap();
        fs::write(temp.path().join("b.xml"), b"old-b").unwrap();
        let updates = [
            FormatUpdate {
                path: "a.xml".into(),
                expected_digest: digest_for_path(Path::new("a.xml"), b"old-a"),
                bytes: b"new-a".to_vec(),
            },
            FormatUpdate {
                path: "b.xml".into(),
                expected_digest: digest_for_path(Path::new("b.xml"), b"old-b"),
                bytes: b"new-b".to_vec(),
            },
        ];
        let fault = OnceAt {
            point: FaultPoint::TargetReplaced,
            fired: AtomicBool::new(false),
        };
        assert!(atomic_write_many(temp.path(), &updates, &fault).is_err());

        recover_transactions(temp.path(), &NoFault).unwrap();

        assert_eq!(fs::read(temp.path().join("a.xml")).unwrap(), b"new-a");
        assert_eq!(fs::read(temp.path().join("b.xml")).unwrap(), b"new-b");
        assert_eq!(
            fs::read_dir(temp.path().join(STATE_DIR).join(TRANSACTIONS_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn predecision_interruption_is_discarded_without_replacing_input() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("a.xml"), b"old").unwrap();
        let update = FormatUpdate {
            path: "a.xml".into(),
            expected_digest: digest_for_path(Path::new("a.xml"), b"old"),
            bytes: b"new".to_vec(),
        };
        // A format transaction makes its decision only after staging; simulate an abandoned
        // staging directory directly to cover the recovery side of that boundary.
        let dir = transaction_dir(temp.path(), "fmt-abandoned");
        fs::create_dir_all(&dir).unwrap();
        stage(&dir, 0, &update.bytes).unwrap();
        write_synced(
            &dir.join("format.json"),
            &serde_json::to_vec(&FormatStaged {
                files: vec![(update.path, digest_for_path(Path::new("a.xml"), b"new"))],
            })
            .unwrap(),
        )
        .unwrap();

        recover_transactions(temp.path(), &NoFault).unwrap();

        assert_eq!(fs::read(temp.path().join("a.xml")).unwrap(), b"old");
        assert!(!dir.exists());
    }

    #[test]
    fn manifest_and_lock_roll_forward_together_after_interruption() {
        let temp = TempDir::new().unwrap();
        let old_manifest = b"old manifest";
        let old_lock = b"old lock";
        let new_manifest = b"new manifest";
        let new_lock = b"new lock";
        fs::write(temp.path().join("xmlsquish.toml"), old_manifest).unwrap();
        fs::write(temp.path().join("xmlsquish.lock"), old_lock).unwrap();
        let observations = BTreeMap::from([
            (
                PathBuf::from("xmlsquish.toml"),
                digest_for_path(Path::new("xmlsquish.toml"), old_manifest),
            ),
            (
                PathBuf::from("xmlsquish.lock"),
                digest_for_path(Path::new("xmlsquish.lock"), old_lock),
            ),
        ]);
        let files = [
            ("xmlsquish.toml", new_manifest.as_slice()),
            ("xmlsquish.lock", new_lock.as_slice()),
        ]
        .into_iter()
        .map(|(path, bytes)| MutationFile {
            path: path.into(),
            expected_digest: observations[Path::new(path)].clone(),
            candidate: bytes.to_vec(),
            candidate_digest: digest_for_path(Path::new(path), bytes),
        })
        .collect();
        let plan = MutationPlan {
            id: TransactionId("replace-both".into()),
            kind: MutationKind::AddDependency {
                alias: "peer".into(),
            },
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            observations,
            files,
            retry_limit: 4,
        };
        let fault = OnceAt {
            point: FaultPoint::TargetReplaced,
            fired: AtomicBool::new(false),
        };

        assert!(commit_plan(temp.path(), &plan, &fault).is_err());
        recover_transactions(temp.path(), &NoFault).unwrap();

        assert_eq!(
            fs::read(temp.path().join("xmlsquish.toml")).unwrap(),
            new_manifest
        );
        assert_eq!(
            fs::read(temp.path().join("xmlsquish.lock")).unwrap(),
            new_lock
        );
    }
}
