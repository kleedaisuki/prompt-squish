//! 可恢复文件事务的操作系统适配器。 / Operating-system adapter for recoverable file transactions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use squish_project::{
    CommitPreparation, JournalRecord, MutationKind, MutationPlan, MutationPlanner, TransactionId,
};
use unicode_normalization::UnicodeNormalization;

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
    /// 新项目候选树与恢复日志均已同步。 / A new-project candidate tree and recovery journal are durable.
    CreationPrepared,
    /// 新项目树已原子发布。 / A new-project tree has been atomically published.
    CreationPublished,
    /// 包含工作区清单已持久替换。 / The enclosing workspace manifest has been durably replaced.
    CreationWorkspaceReplaced,
    /// 新项目事务已完成。 / A new-project transaction has completed.
    CreationCompleted,
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

/// One owned `.agents/skills/<name>` replacement or removal accompanying a mutation plan.
/// `expected_digest` is `None` only when the destination must not exist. `files = None`
/// removes an owned installation; otherwise the map is the complete replacement tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillDirectoryUpdate {
    /// Validated, single-component skill name.
    pub name: String,
    /// Previously observed content digest, excluding the ownership marker.
    pub expected_digest: Option<String>,
    /// Complete skill-relative regular-file contents, including the ownership marker.
    pub files: Option<BTreeMap<PathBuf, Vec<u8>>>,
}

const SKILL_MARKER: &str = ".xmlsquish-managed";
const SKILL_MARKER_PREFIX: &[u8] = b"xmlsquish skill v1\n";

/// Return the complete ownership marker for a skill name and normalized content digest.
/// The marker is stored inside the projection but excluded from that digest.
pub fn skill_marker(name: &str, digest: &str) -> Vec<u8> {
    let mut marker = SKILL_MARKER_PREFIX.to_vec();
    marker.extend_from_slice(name.as_bytes());
    marker.push(b'\n');
    marker.extend_from_slice(digest.as_bytes());
    marker.push(b'\n');
    marker
}
const MAX_SKILL_FILES: usize = 50_000;
const MAX_SKILL_BYTES: u64 = 512 << 20;
const MAX_SKILL_FILE_BYTES: u64 = 128 << 20;
const MAX_SKILL_PATH_BYTES: usize = 1024;
const MAX_SKILL_SEGMENT_BYTES: usize = 255;
const MAX_SKILL_MARKER_BYTES: u64 = 256;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillStaged {
    name: String,
    expected_digest: Option<String>,
    candidate_digest: Option<String>,
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
    commit_plan_inner(root, plan, None, faults)
}

pub(crate) fn commit_plan_with_skill(
    root: &Path,
    plan: &MutationPlan,
    update: &SkillDirectoryUpdate,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    commit_plan_inner(root, plan, Some(update), faults)
}

pub(crate) fn sync_skill(
    root: &Path,
    name: &str,
    expected_lock_digest: &str,
    expected_manifest_digest: &str,
    files: &BTreeMap<PathBuf, Vec<u8>>,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    let update = SkillDirectoryUpdate {
        name: name.into(),
        expected_digest: None,
        files: Some(files.clone()),
    };
    let candidate = validate_skill_update(&update)?;
    if candidate.as_deref() != Some(expected_lock_digest) {
        return Err(RepositoryError::Layout(format!(
            "skill `{name}` candidate differs from lock digest"
        )));
    }
    let manifest_path = root.join(squish_project::MANIFEST_FILE_NAME);
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|e| RepositoryError::io(&manifest_path, e))?;
    let manifest_relative = PathBuf::from(squish_project::MANIFEST_FILE_NAME);
    if digest_for_path(&manifest_relative, &manifest_bytes) != expected_manifest_digest {
        return Err(RepositoryError::Contended(vec![manifest_relative]));
    }
    let manifest = squish_project::Manifest::parse(
        std::str::from_utf8(&manifest_bytes)
            .map_err(|_| RepositoryError::Layout("xmlsquish.toml is not UTF-8".into()))?,
    )?;
    if !manifest.skills.contains_key(name) {
        return Err(RepositoryError::Layout(format!(
            "skill `{name}` is not declared in root manifest"
        )));
    }
    let lock_path = root.join(squish_project::LOCK_FILE_NAME);
    let lock_bytes = fs::read(&lock_path).map_err(|e| RepositoryError::io(&lock_path, e))?;
    let lock = squish_project::Lockfile::parse(
        std::str::from_utf8(&lock_bytes)
            .map_err(|_| RepositoryError::Layout("xmlsquish.lock is not UTF-8".into()))?,
    )?;
    if lock
        .skills
        .iter()
        .find(|skill| skill.name == name)
        .map(|skill| skill.digest.as_str())
        != Some(expected_lock_digest)
    {
        return Err(RepositoryError::Layout(format!(
            "skill `{name}` is not pinned to requested digest"
        )));
    }
    let already_current = with_locked_repository(root, faults, || {
        let actual_lock = fs::read(&lock_path).map_err(|e| RepositoryError::io(&lock_path, e))?;
        if actual_lock != lock_bytes {
            return Err(RepositoryError::Contended(vec![PathBuf::from(
                squish_project::LOCK_FILE_NAME,
            )]));
        }
        let actual_manifest =
            fs::read(&manifest_path).map_err(|e| RepositoryError::io(&manifest_path, e))?;
        if actual_manifest != manifest_bytes {
            return Err(RepositoryError::Contended(vec![PathBuf::from(
                squish_project::MANIFEST_FILE_NAME,
            )]));
        }
        let current = skill_directory_digest(root, name)?;
        if current.is_some() && current.as_deref() != Some(expected_lock_digest) {
            return Err(RepositoryError::Contended(vec![skill_relative(name)]));
        }
        Ok(current.is_some())
    })?;
    if already_current {
        return Ok(());
    }
    let lock_relative = PathBuf::from(squish_project::LOCK_FILE_NAME);
    let plan = MutationPlan {
        id: TransactionId(format!("sync-skill-{name}")),
        kind: MutationKind::AddSkill { name: name.into() },
        manifest_digest: lock.manifest_digest,
        observations: BTreeMap::from([
            (
                lock_relative.clone(),
                digest_for_path(&lock_relative, &lock_bytes),
            ),
            (
                manifest_relative.clone(),
                digest_for_path(&manifest_relative, &manifest_bytes),
            ),
        ]),
        files: Vec::new(),
        retry_limit: 0,
    };
    commit_plan_inner(root, &plan, Some(&update), faults)
}

/// Enumerate only dependency-owned projections, excluding bundled and foreign trees.
/// The marker prefix identifies the product; its embedded digest is validated before return.
pub(crate) fn managed_skill_names(root: &Path) -> Result<Vec<(String, String)>, RepositoryError> {
    let parent = safe_skill_parent(root, false)?;
    if parent != root.join(".agents").join("skills") || !parent.exists() {
        return Ok(Vec::new());
    }
    let mut managed = Vec::new();
    for entry in fs::read_dir(&parent).map_err(|e| RepositoryError::io(&parent, e))? {
        let entry = entry.map_err(|e| RepositoryError::io(&parent, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_skill_name(&name) {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| RepositoryError::io(&path, e))?;
        if is_alias(&meta) || !meta.is_dir() {
            continue;
        }
        let marker = path.join(SKILL_MARKER);
        let meta = match fs::symlink_metadata(&marker) {
            Ok(meta)
                if meta.is_file() && !is_alias(&meta) && meta.len() <= MAX_SKILL_MARKER_BYTES =>
            {
                meta
            }
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(RepositoryError::io(&marker, e)),
        };
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        File::open(&marker)
            .map_err(|e| RepositoryError::io(&marker, e))?
            .take(MAX_SKILL_MARKER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| RepositoryError::io(&marker, e))?;
        if bytes.starts_with(SKILL_MARKER_PREFIX) {
            let digest = skill_directory_digest(root, &name)?.expect("enumerated directory exists");
            managed.push((name, digest));
        }
    }
    managed.sort();
    Ok(managed)
}

pub(crate) fn prune_skill(
    root: &Path,
    name: &str,
    expected_digest: &str,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    if !valid_skill_name(name) {
        return Err(RepositoryError::Layout(format!(
            "invalid skill name `{name}`"
        )));
    }
    let manifest_path = root.join(squish_project::MANIFEST_FILE_NAME);
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|e| RepositoryError::io(&manifest_path, e))?;
    let manifest = squish_project::Manifest::parse(
        std::str::from_utf8(&manifest_bytes)
            .map_err(|_| RepositoryError::Layout("xmlsquish.toml is not UTF-8".into()))?,
    )?;
    if manifest.skills.contains_key(name) {
        return Err(RepositoryError::Layout(format!(
            "skill `{name}` remains declared in manifest"
        )));
    }
    let lock_path = root.join(squish_project::LOCK_FILE_NAME);
    let lock_bytes = match fs::read(&lock_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(RepositoryError::io(&lock_path, e)),
    };
    if let Some(bytes) = &lock_bytes {
        let lock = squish_project::Lockfile::parse(
            std::str::from_utf8(bytes)
                .map_err(|_| RepositoryError::Layout("xmlsquish.lock is not UTF-8".into()))?,
        )?;
        if lock.skills.iter().any(|skill| skill.name == name) {
            return Err(RepositoryError::Layout(format!(
                "skill `{name}` remains pinned in lock"
            )));
        }
    }
    let manifest_rel = PathBuf::from(squish_project::MANIFEST_FILE_NAME);
    let lock_rel = PathBuf::from(squish_project::LOCK_FILE_NAME);
    let observations = BTreeMap::from([
        (
            manifest_rel.clone(),
            digest_for_path(&manifest_rel, &manifest_bytes),
        ),
        (
            lock_rel.clone(),
            lock_bytes
                .as_ref()
                .map_or_else(missing_digest, |bytes| digest_for_path(&lock_rel, bytes)),
        ),
    ]);
    let plan = MutationPlan {
        id: TransactionId(format!("prune-skill-{name}")),
        kind: MutationKind::RemoveSkill { name: name.into() },
        manifest_digest: "prune-existing-authority".into(),
        observations,
        files: Vec::new(),
        retry_limit: 0,
    };
    let update = SkillDirectoryUpdate {
        name: name.into(),
        expected_digest: Some(expected_digest.into()),
        files: None,
    };
    commit_plan_inner(root, &plan, Some(&update), faults)
}

fn commit_plan_inner(
    root: &Path,
    plan: &MutationPlan,
    update: Option<&SkillDirectoryUpdate>,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    validate_plan_paths(plan)?;
    if let Some(update) = update {
        validate_skill_intent(plan, update)?;
    }
    let candidate_digest = update.map(validate_skill_update).transpose()?;
    let _lock = writer_lock(root)?;
    recover_locked(root, faults)?;
    if let Some(update) = update {
        let actual = skill_directory_digest(root, &update.name)?;
        if actual != update.expected_digest {
            return Err(RepositoryError::Contended(vec![skill_relative(
                &update.name,
            )]));
        }
    }
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
    if let Some(update) = update {
        stage_skill_tree(&dir, update)?;
        let state = SkillStaged {
            name: update.name.clone(),
            expected_digest: update.expected_digest.clone(),
            candidate_digest: candidate_digest
                .clone()
                .expect("validated update has digest option"),
        };
        let path = dir.join("skill.json");
        write_synced(
            &path,
            &serde_json::to_vec(&state).map_err(|e| RepositoryError::Journal {
                path: path.clone(),
                message: e.to_string(),
            })?,
        )?;
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
    if let Some(update) = update {
        replace_skill_tree(
            root,
            &dir,
            &SkillStaged {
                name: update.name.clone(),
                expected_digest: update.expected_digest.clone(),
                candidate_digest: candidate_digest
                    .clone()
                    .expect("validated update has digest option"),
            },
        )?;
        faults
            .check(FaultPoint::TargetReplaced)
            .map_err(|e| RepositoryError::io(skill_relative(&update.name), e))?;
    }
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

pub(crate) fn recover_locked(
    root: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
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
    let skill_path = dir.join("skill.json");
    if skill_path.exists() {
        let bytes = fs::read(&skill_path).map_err(|e| RepositoryError::io(&skill_path, e))?;
        let state: SkillStaged =
            serde_json::from_slice(&bytes).map_err(|e| RepositoryError::Journal {
                path: skill_path.clone(),
                message: e.to_string(),
            })?;
        faults
            .check(FaultPoint::RecoveryReplace)
            .map_err(|e| RepositoryError::io(skill_relative(&state.name), e))?;
        replace_skill_tree(root, dir, &state)?;
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

fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn skill_relative(name: &str) -> PathBuf {
    Path::new(".agents").join("skills").join(name)
}

fn validate_skill_update(update: &SkillDirectoryUpdate) -> Result<Option<String>, RepositoryError> {
    if !valid_skill_name(&update.name) {
        return Err(RepositoryError::Layout(format!(
            "invalid skill name `{}`",
            update.name
        )));
    }
    match &update.files {
        Some(files) => {
            let digest = skill_tree_digest(files)?;
            if files.get(Path::new(SKILL_MARKER)) != Some(&skill_marker(&update.name, &digest)) {
                return Err(RepositoryError::Layout(
                    "skill tree lacks a matching xmlsquish ownership marker".into(),
                ));
            }
            if !files.contains_key(Path::new("SKILL.md")) {
                return Err(RepositoryError::Layout("skill tree lacks SKILL.md".into()));
            }
            Ok(Some(digest))
        }
        None if update.expected_digest.is_some() => Ok(None),
        None => Err(RepositoryError::Layout(
            "cannot remove a skill without an expected owned tree".into(),
        )),
    }
}

fn validate_skill_intent(
    plan: &MutationPlan,
    update: &SkillDirectoryUpdate,
) -> Result<(), RepositoryError> {
    let matches = match (&plan.kind, &update.files) {
        (MutationKind::AddSkill { name }, Some(_)) => name == &update.name,
        (MutationKind::RemoveSkill { name }, None) => name == &update.name,
        _ => false,
    };
    if !matches {
        return Err(RepositoryError::Layout(
            "skill transaction kind, name, and tree operation disagree".into(),
        ));
    }
    Ok(())
}

/// Digest a complete skill tree, excluding the fixed ownership marker. Paths must be
/// normalized, Unicode, non-overlapping relative regular-file names. The binary framing
/// includes each path and file length so independent trees cannot be ambiguously encoded.
///
/// ```
/// use std::{collections::BTreeMap, path::PathBuf};
/// use squish_repository::skill_tree_digest;
/// let source = BTreeMap::from([(PathBuf::from("SKILL.md"), b"example".to_vec())]);
/// let digest = skill_tree_digest(&source).unwrap();
/// assert!(digest.starts_with("blake3:"));
/// ```
pub fn skill_tree_digest(files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<String, RepositoryError> {
    if files.len() - usize::from(files.contains_key(Path::new(SKILL_MARKER))) > MAX_SKILL_FILES {
        return Err(RepositoryError::Layout(
            "skill tree exceeds file-count limit".into(),
        ));
    }
    validate_portable_paths(files.keys())?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"xmlsquish\0skill-tree\0v1\0");
    let mut total = 0u64;
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(path, _)| {
        path.to_str()
            .expect("validated Unicode path")
            .replace('\\', "/")
    });
    for (path, bytes) in ordered {
        if path == Path::new(SKILL_MARKER) {
            if bytes.len() as u64 > MAX_SKILL_MARKER_BYTES {
                return Err(RepositoryError::Layout(
                    "invalid skill ownership marker".into(),
                ));
            }
            continue;
        }
        let len = bytes.len() as u64;
        total = total
            .checked_add(len)
            .ok_or_else(|| RepositoryError::Layout("skill tree length overflow".into()))?;
        if len > MAX_SKILL_FILE_BYTES || total > MAX_SKILL_BYTES {
            return Err(RepositoryError::Layout(
                "skill tree exceeds byte limit".into(),
            ));
        }
        let text = path.to_str().ok_or_else(|| {
            RepositoryError::Layout(format!("non-Unicode skill file `{}`", path.display()))
        })?;
        let portable = text.replace('\\', "/");
        hasher.update(&(portable.len() as u64).to_le_bytes());
        hasher.update(portable.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn validate_skill_file_path(path: &Path) -> Result<(), RepositoryError> {
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(RepositoryError::Layout(format!(
            "unsafe skill file path `{}`",
            path.display()
        )));
    }
    let text = path.to_str().ok_or_else(|| {
        RepositoryError::Layout(format!("non-Unicode skill file path `{}`", path.display()))
    })?;
    if text.len() > MAX_SKILL_PATH_BYTES {
        return Err(RepositoryError::Layout("skill path exceeds limit".into()));
    }
    for component in &components {
        let segment = component.as_os_str().to_str().expect("whole path is UTF-8");
        let base = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .to_ascii_uppercase();
        if segment.len() > MAX_SKILL_SEGMENT_BYTES
            || segment.ends_with([' ', '.'])
            || segment.contains(':')
            || segment.contains(['<', '>', '"', '|', '?', '*'])
            || segment.chars().any(char::is_control)
            || segment.eq_ignore_ascii_case(".git")
            || matches!(
                base.as_str(),
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
        {
            return Err(RepositoryError::Layout(format!(
                "non-portable skill file path `{}`",
                path.display()
            )));
        }
    }
    if components.len() > 1 && components[0].as_os_str() == SKILL_MARKER {
        return Err(RepositoryError::Layout(
            "ownership marker cannot be a directory".into(),
        ));
    }
    Ok(())
}

fn portable_key(path: &Path) -> String {
    path.to_str()
        .expect("validated Unicode path")
        .replace('\\', "/")
        .split('/')
        .map(|segment| {
            segment
                .nfc()
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_portable_paths<'a>(
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> Result<(), RepositoryError> {
    let mut files = BTreeSet::new();
    let mut dirs = BTreeMap::<String, PathBuf>::new();
    for path in paths {
        validate_skill_file_path(path)?;
        let key = portable_key(path);
        if !files.insert(key.clone()) || dirs.contains_key(&key) {
            return Err(RepositoryError::Layout(format!(
                "portable skill path collision at `{}`",
                path.display()
            )));
        }
        for ancestor in path
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            let parent = ancestor.to_path_buf();
            let key = portable_key(&parent);
            if files.contains(&key) || dirs.get(&key).is_some_and(|existing| existing != &parent) {
                return Err(RepositoryError::Layout(format!(
                    "portable skill directory collision at `{}`",
                    path.display()
                )));
            }
            dirs.insert(key, parent);
        }
    }
    Ok(())
}

fn safe_skill_parent(root: &Path, create: bool) -> Result<PathBuf, RepositoryError> {
    let mut path = root.to_path_buf();
    for component in [".agents", "skills"] {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() && !is_alias(&meta) => {}
            Ok(_) => {
                return Err(RepositoryError::Layout(format!(
                    "skill parent `{}` is not an ordinary directory",
                    path.display()
                )));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
                fs::create_dir(&path).map_err(|e| RepositoryError::io(&path, e))?;
                sync_parent(&path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(path),
            Err(e) => return Err(RepositoryError::io(&path, e)),
        }
    }
    Ok(path)
}

pub(crate) fn skill_directory_digest(
    root: &Path,
    name: &str,
) -> Result<Option<String>, RepositoryError> {
    if !valid_skill_name(name) {
        return Err(RepositoryError::Layout(format!(
            "invalid skill name `{name}`"
        )));
    }
    let parent = safe_skill_parent(root, false)?;
    if parent != root.join(".agents").join("skills") {
        return Ok(None);
    }
    let target = parent.join(name);
    let meta = match fs::symlink_metadata(&target) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepositoryError::io(&target, e)),
    };
    if !meta.is_dir() || is_alias(&meta) {
        return Err(RepositoryError::Layout(format!(
            "skill destination `{}` is not an ordinary directory",
            target.display()
        )));
    }
    Ok(Some(digest_skill_disk_tree(&target, name)?))
}

fn is_alias(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn digest_skill_disk_tree(root: &Path, name: &str) -> Result<String, RepositoryError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::<(PathBuf, u64)>::new();
    let mut dirs = 0usize;
    let mut total = 0u64;
    while let Some(dir) = pending.pop() {
        dirs += 1;
        if dirs > MAX_SKILL_FILES + 1 {
            return Err(RepositoryError::Layout(
                "skill tree exceeds directory-count limit".into(),
            ));
        }
        for entry in fs::read_dir(&dir).map_err(|e| RepositoryError::io(&dir, e))? {
            let entry = entry.map_err(|e| RepositoryError::io(&dir, e))?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| RepositoryError::io(&path, e))?;
            if is_alias(&meta) || (!meta.is_dir() && !meta.is_file()) {
                return Err(RepositoryError::Layout(format!(
                    "skill tree contains non-regular entry `{}`",
                    path.display()
                )));
            }
            if meta.is_dir() {
                pending.push(path);
                continue;
            }
            if files.len() > MAX_SKILL_FILES || meta.len() > MAX_SKILL_FILE_BYTES {
                return Err(RepositoryError::Layout(
                    "skill tree exceeds file limit".into(),
                ));
            }
            let relative = path
                .strip_prefix(root)
                .expect("descendant path")
                .to_path_buf();
            validate_skill_file_path(&relative)?;
            if relative != Path::new(SKILL_MARKER) {
                total = total
                    .checked_add(meta.len())
                    .ok_or_else(|| RepositoryError::Layout("skill tree length overflow".into()))?;
                if total > MAX_SKILL_BYTES {
                    return Err(RepositoryError::Layout(
                        "skill tree exceeds byte limit".into(),
                    ));
                }
            }
            files.push((relative, meta.len()));
        }
    }
    if files
        .iter()
        .filter(|(path, _)| path != Path::new(SKILL_MARKER))
        .count()
        > MAX_SKILL_FILES
    {
        return Err(RepositoryError::Layout(
            "skill tree exceeds file-count limit".into(),
        ));
    }
    validate_portable_paths(files.iter().map(|(path, _)| path))?;
    files.sort_by_key(|(path, _)| {
        path.to_str()
            .expect("validated Unicode path")
            .replace('\\', "/")
    });
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"xmlsquish\0skill-tree\0v1\0");
    let mut marker = None;
    let mut buffer = [0u8; 64 * 1024];
    for (path, expected_len) in files {
        let full = root.join(&path);
        let mut file = File::open(&full).map_err(|e| RepositoryError::io(&full, e))?;
        if path == Path::new(SKILL_MARKER) {
            if expected_len > MAX_SKILL_MARKER_BYTES {
                return Err(RepositoryError::Layout(
                    "invalid skill ownership marker".into(),
                ));
            }
            let mut bytes = Vec::new();
            file.take(MAX_SKILL_MARKER_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| RepositoryError::io(&full, e))?;
            marker = Some(bytes);
            continue;
        }
        let name = path
            .to_str()
            .expect("validated Unicode path")
            .replace('\\', "/");
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&expected_len.to_le_bytes());
        let mut remaining = expected_len;
        while remaining > 0 {
            let chunk = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
            let count = file
                .read(&mut buffer[..chunk])
                .map_err(|e| RepositoryError::io(&full, e))?;
            if count == 0 {
                return Err(RepositoryError::Layout(format!(
                    "skill file shrank while reading `{}`",
                    full.display()
                )));
            }
            hasher.update(&buffer[..count]);
            remaining -= count as u64;
        }
        if file
            .read(&mut buffer[..1])
            .map_err(|e| RepositoryError::io(&full, e))?
            != 0
        {
            return Err(RepositoryError::Layout(format!(
                "skill file grew while reading `{}`",
                full.display()
            )));
        }
    }
    let digest = format!("blake3:{}", hasher.finalize().to_hex());
    if marker.as_deref() != Some(skill_marker(name, &digest).as_slice()) {
        return Err(RepositoryError::Layout(format!(
            "skill destination `{}` is unowned or modified",
            root.display()
        )));
    }
    Ok(digest)
}

fn stage_skill_tree(dir: &Path, update: &SkillDirectoryUpdate) -> Result<(), RepositoryError> {
    let Some(files) = &update.files else {
        return Ok(());
    };
    let base = dir.join("skill.candidate");
    fs::create_dir(&base).map_err(|e| RepositoryError::io(&base, e))?;
    for (relative, bytes) in files {
        let path = base.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| RepositoryError::io(parent, e))?;
        }
        write_synced(&path, bytes)?;
    }
    sync_staged_dirs(&base)
}

fn sync_staged_dirs(dir: &Path) -> Result<(), RepositoryError> {
    for entry in fs::read_dir(dir).map_err(|e| RepositoryError::io(dir, e))? {
        let entry = entry.map_err(|e| RepositoryError::io(dir, e))?;
        if entry
            .file_type()
            .map_err(|e| RepositoryError::io(entry.path(), e))?
            .is_dir()
        {
            sync_staged_dirs(&entry.path())?;
        }
    }
    sync_directory(dir)
}

fn replace_skill_tree(root: &Path, dir: &Path, state: &SkillStaged) -> Result<(), RepositoryError> {
    if !valid_skill_name(&state.name) {
        return Err(RepositoryError::Journal {
            path: dir.join("skill.json"),
            message: "invalid skill name".into(),
        });
    }
    let target = root.join(skill_relative(&state.name));
    let parent = safe_skill_parent(root, state.candidate_digest.is_some())?;
    if parent != root.join(".agents").join("skills") && state.candidate_digest.is_none() {
        return Ok(());
    }
    let actual = skill_directory_digest(root, &state.name)?;
    if actual == state.candidate_digest {
        return Ok(());
    }
    let backup = dir.join("skill.previous");
    if actual.is_some() {
        if actual != state.expected_digest || backup.exists() {
            return Err(RepositoryError::Layout(format!(
                "skill `{}` changed during committed transaction",
                state.name
            )));
        }
        fs::rename(&target, &backup).map_err(|e| RepositoryError::io(&target, e))?;
        sync_parent(&target)?;
    } else if state.expected_digest.is_none() && backup.exists() {
        return Err(RepositoryError::Journal {
            path: backup,
            message: "unexpected skill backup".into(),
        });
    }
    if let Some(expected) = &state.candidate_digest {
        let staged = dir.join("skill.candidate");
        if digest_skill_disk_tree(&staged, &state.name)? != *expected {
            return Err(RepositoryError::Journal {
                path: staged,
                message: "staged skill tree digest mismatch".into(),
            });
        }
        fs::rename(&staged, &target).map_err(|e| RepositoryError::io(&target, e))?;
        sync_parent(&target)?;
    }
    Ok(())
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
    use squish_project::{
        LockedSkill, LockedSkillSource, MutationFile, MutationKind, TransactionId,
    };

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

    fn skill_files() -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::from([
            (
                PathBuf::from("SKILL.md"),
                b"---\nname: demo\n---\nDemo\n".to_vec(),
            ),
            (PathBuf::from("references/usage.md"), b"usage".to_vec()),
        ]);
        let digest = skill_tree_digest(&files).unwrap();
        files.insert(PathBuf::from(SKILL_MARKER), skill_marker("demo", &digest));
        files
    }

    fn skill_test_dir() -> TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".temp");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    fn skill_plan(root: &Path) -> MutationPlan {
        let manifest = PathBuf::from("xmlsquish.toml");
        let lock = PathBuf::from("xmlsquish.lock");
        let old_manifest = fs::read(root.join(&manifest)).unwrap();
        let old_lock = fs::read(root.join(&lock)).unwrap();
        let observations = BTreeMap::from([
            (manifest.clone(), digest_for_path(&manifest, &old_manifest)),
            (lock.clone(), digest_for_path(&lock, &old_lock)),
        ]);
        let files = [
            (manifest, b"new manifest".to_vec()),
            (lock, b"new lock".to_vec()),
        ]
        .into_iter()
        .map(|(path, candidate)| MutationFile {
            expected_digest: observations[&path].clone(),
            candidate_digest: digest_for_path(&path, &candidate),
            path,
            candidate,
        })
        .collect();
        MutationPlan {
            id: TransactionId("skill-test".into()),
            kind: MutationKind::AddSkill {
                name: "demo".into(),
            },
            manifest_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            observations,
            files,
            retry_limit: 0,
        }
    }

    #[test]
    fn skill_publish_and_files_roll_forward_after_commit_decision() {
        let temp = skill_test_dir();
        fs::write(temp.path().join("xmlsquish.toml"), b"old manifest").unwrap();
        fs::write(temp.path().join("xmlsquish.lock"), b"old lock").unwrap();
        let plan = skill_plan(temp.path());
        let update = SkillDirectoryUpdate {
            name: "demo".into(),
            expected_digest: None,
            files: Some(skill_files()),
        };
        let fault = OnceAt {
            point: FaultPoint::CommitDecided,
            fired: AtomicBool::new(false),
        };
        assert!(commit_plan_with_skill(temp.path(), &plan, &update, &fault).is_err());
        assert!(!temp.path().join(skill_relative("demo")).exists());
        assert_eq!(
            fs::read(temp.path().join("xmlsquish.toml")).unwrap(),
            b"old manifest"
        );
        recover_transactions(temp.path(), &NoFault).unwrap();
        assert_eq!(
            skill_directory_digest(temp.path(), "demo").unwrap(),
            Some(skill_tree_digest(&skill_files()).unwrap())
        );
        assert_eq!(
            fs::read(temp.path().join("xmlsquish.toml")).unwrap(),
            b"new manifest"
        );
        assert_eq!(
            fs::read(temp.path().join("xmlsquish.lock")).unwrap(),
            b"new lock"
        );
    }

    #[test]
    fn skill_removal_recovery_does_not_delete_unowned_or_modified_tree() {
        let temp = skill_test_dir();
        fs::write(temp.path().join("xmlsquish.toml"), b"old manifest").unwrap();
        fs::write(temp.path().join("xmlsquish.lock"), b"old lock").unwrap();
        let target = temp.path().join(skill_relative("demo"));
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), b"user-owned").unwrap();
        let files = skill_files();
        let old = skill_tree_digest(&files).unwrap();
        let update = SkillDirectoryUpdate {
            name: "demo".into(),
            expected_digest: Some(old.clone()),
            files: None,
        };
        let mut plan = skill_plan(temp.path());
        plan.kind = MutationKind::RemoveSkill {
            name: "demo".into(),
        };
        assert!(matches!(
            commit_plan_with_skill(temp.path(), &plan, &update, &NoFault),
            Err(RepositoryError::Layout(_))
        ));
        assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), b"user-owned");

        fs::remove_dir_all(&target).unwrap();
        fs::create_dir_all(&target).unwrap();
        for (path, bytes) in files {
            let full = target.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, bytes).unwrap();
        }
        fs::write(target.join("SKILL.md"), b"edited").unwrap();
        assert!(matches!(
            commit_plan_with_skill(temp.path(), &plan, &update, &NoFault),
            Err(RepositoryError::Layout(_))
        ));
        assert_eq!(fs::read(target.join("SKILL.md")).unwrap(), b"edited");
    }

    #[test]
    fn removal_after_detach_rolls_forward_idempotently() {
        let temp = skill_test_dir();
        fs::write(temp.path().join("xmlsquish.toml"), b"old manifest").unwrap();
        fs::write(temp.path().join("xmlsquish.lock"), b"old lock").unwrap();
        let target = temp.path().join(skill_relative("demo"));
        for (path, bytes) in skill_files() {
            let full = target.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, bytes).unwrap();
        }
        let update = SkillDirectoryUpdate {
            name: "demo".into(),
            expected_digest: skill_directory_digest(temp.path(), "demo").unwrap(),
            files: None,
        };
        let fault = OnceAt {
            point: FaultPoint::TargetReplaced,
            fired: AtomicBool::new(false),
        };
        let mut plan = skill_plan(temp.path());
        plan.kind = MutationKind::RemoveSkill {
            name: "demo".into(),
        };
        assert!(commit_plan_with_skill(temp.path(), &plan, &update, &fault).is_err());
        assert!(!target.exists());
        recover_transactions(temp.path(), &NoFault).unwrap();
        recover_transactions(temp.path(), &NoFault).unwrap();
        assert!(!target.exists());
        assert_eq!(
            fs::read(temp.path().join("xmlsquish.lock")).unwrap(),
            b"new lock"
        );
    }

    #[test]
    fn sync_skill_publishes_only_the_pinned_missing_tree() {
        let temp = skill_test_dir();
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\n[skills.demo]\npath = \"local/demo\"\n",
        )
        .unwrap();
        let files = skill_files();
        let expected = skill_tree_digest(&files).unwrap();
        let lock = squish_project::Lockfile {
            lock_version: squish_project::LOCK_VERSION,
            resolver_version: "test-v1".into(),
            manifest_digest:
                "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            packages: Vec::new(),
            skills: vec![LockedSkill {
                name: "demo".into(),
                source: LockedSkillSource::Path {
                    path: "local/demo".into(),
                    mutable: true,
                },
                digest: expected.clone(),
            }],
        };
        fs::write(temp.path().join("xmlsquish.lock"), lock.to_toml().unwrap()).unwrap();
        let manifest_bytes = fs::read(temp.path().join("xmlsquish.toml")).unwrap();
        let manifest_digest = digest_for_path(Path::new("xmlsquish.toml"), &manifest_bytes);
        sync_skill(
            temp.path(),
            "demo",
            &expected,
            &manifest_digest,
            &files,
            &NoFault,
        )
        .unwrap();
        sync_skill(
            temp.path(),
            "demo",
            &expected,
            &manifest_digest,
            &files,
            &NoFault,
        )
        .unwrap();
        assert_eq!(
            skill_directory_digest(temp.path(), "demo").unwrap(),
            Some(expected.clone())
        );
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\n",
        )
        .unwrap();
        assert!(matches!(
            sync_skill(
                temp.path(),
                "demo",
                &expected,
                &manifest_digest,
                &files,
                &NoFault
            ),
            Err(RepositoryError::Contended(_))
        ));
        fs::write(temp.path().join("xmlsquish.toml"), &manifest_bytes).unwrap();
        fs::write(
            temp.path().join(skill_relative("demo")).join("SKILL.md"),
            b"edited",
        )
        .unwrap();
        assert!(matches!(
            sync_skill(
                temp.path(),
                "demo",
                &expected,
                &manifest_digest,
                &files,
                &NoFault
            ),
            Err(RepositoryError::Layout(_))
        ));
    }

    #[test]
    fn prune_removes_only_verified_stale_dependency_projection() {
        let temp = skill_test_dir();
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\n",
        )
        .unwrap();
        let target = temp.path().join(skill_relative("demo"));
        for (path, bytes) in skill_files() {
            let full = target.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, bytes).unwrap();
        }
        let bundled = temp.path().join(skill_relative("bundled"));
        fs::create_dir_all(&bundled).unwrap();
        fs::write(
            bundled.join(SKILL_MARKER),
            b"xmlsquish bundled skill v1\nblake3:other\n",
        )
        .unwrap();
        let names = managed_skill_names(temp.path()).unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].0, "demo");
        let renamed = temp.path().join(skill_relative("renamed"));
        fs::rename(&target, &renamed).unwrap();
        assert!(matches!(
            skill_directory_digest(temp.path(), "renamed"),
            Err(RepositoryError::Layout(_))
        ));
        fs::rename(&renamed, &target).unwrap();
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\n[skills.demo]\npath = \"local/demo\"\n",
        )
        .unwrap();
        assert!(matches!(
            prune_skill(temp.path(), "demo", &names[0].1, &NoFault),
            Err(RepositoryError::Layout(_))
        ));
        assert!(target.exists());
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n[workspace]\n",
        )
        .unwrap();
        prune_skill(temp.path(), "demo", &names[0].1, &NoFault).unwrap();
        assert!(!target.exists());
        assert!(bundled.exists());
    }

    #[test]
    fn skill_paths_reject_windows_devices_and_unicode_aliases() {
        let devices = BTreeMap::from([(PathBuf::from("CON.txt"), Vec::new())]);
        assert!(skill_tree_digest(&devices).is_err());
        let dots = BTreeMap::from([(PathBuf::from("notes. "), Vec::new())]);
        assert!(skill_tree_digest(&dots).is_err());
        let aliases = BTreeMap::from([
            (PathBuf::from("Caf\u{e9}.txt"), Vec::new()),
            (PathBuf::from("Cafe\u{301}.txt"), Vec::new()),
        ]);
        assert!(skill_tree_digest(&aliases).is_err());
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
