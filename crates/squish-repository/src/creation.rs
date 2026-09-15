//! 新项目目录的可恢复发布。 / Recoverable publication of new project directories.
//!
//! Publication has one irreversible boundary: renaming the first missing path component into
//! place. A marker travels inside that tree, allowing recovery to distinguish a prepared staging
//! tree from a published tree even if the process dies before its next journal write.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use squish_project::{MANIFEST_FILE_NAME, Manifest};
use toml_edit::{Array, DocumentMut, Item, Value};

use crate::{FaultInjector, FaultPoint, RepositoryError};

const STATE_DIR: &str = ".xmlsquish";
const CREATIONS_DIR: &str = "new-transactions";
const LOCK_NAME: &str = "creation.lock";
const JOURNAL_NAME: &str = "journal.json";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// 脚手架中的一个项目相对文件。 / One project-relative file in a scaffold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectFile {
    /// 不含 `..` 的相对路径。 / Relative path without `..`.
    pub path: PathBuf,
    /// 要原样写入的完整内容。 / Complete bytes to write verbatim.
    pub bytes: Vec<u8>,
}

/// 对候选树生效的版本控制处置。 / VCS disposition effective for the candidate tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectVcs {
    /// 不执行版本控制集成。 / No VCS integration is performed.
    None,
    /// 已由外层 Git 工作树覆盖；不创建嵌套仓库。 / Covered by an enclosing Git worktree; no nested repository.
    InheritedGit,
    /// 候选树发布前应初始化为 Git 仓库。 / Initialize the candidate tree as a Git repository before publication.
    InitializeGit,
}

/// 包含工作区的语义成员请求。 / Semantic membership request for an enclosing workspace.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMembership {
    /// 包含 `xmlsquish.toml` 的规范工作区根。 / Canonical workspace root containing `xmlsquish.toml`.
    #[serde(with = "native_path_serde")]
    pub root: PathBuf,
    /// 使用 `/` 语义的工作区相对成员路径。 / Workspace-relative member path with `/` semantics.
    pub member: String,
}

/// 一次完整新项目发布请求。 / One complete new-project publication request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateProjectRequest {
    /// 必须尚不存在的目标目录。 / Destination directory that must not exist.
    pub destination: PathBuf,
    /// 完整且确定的脚手架文件。 / Complete deterministic scaffold files.
    pub files: Vec<ProjectFile>,
    /// 新项目的包名，用于工作区重复检测。 / New package name used for workspace duplicate detection.
    pub package_name: String,
    /// 有效 VCS 处置。 / Effective VCS disposition.
    pub vcs: ProjectVcs,
    /// 可选包含工作区。 / Optional enclosing workspace.
    pub workspace: Option<WorkspaceMembership>,
}

/// 成功发布的新项目。 / A successfully published new project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedProject {
    /// 发布后的绝对目标。 / Absolute destination after publication.
    pub destination: PathBuf,
    /// 是否修改了包含工作区清单。 / Whether the enclosing workspace manifest was modified.
    pub workspace_updated: bool,
}

/// 无副作用的新项目位置解析结果。 / Side-effect-free new-project location resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewProjectLocation {
    /// 绝对、正规且尚不存在的目标。 / Absolute, normalized, non-existing destination.
    pub destination: PathBuf,
    /// 唯一包含工作区及其成员路径。 / Unique enclosing workspace and member path.
    pub workspace: Option<WorkspaceMembership>,
}

/// 把用户目标解析为绝对、词法正规且现有前缀已规范化的路径。 / Resolves a user destination to an absolute, lexically normalized path with a canonical existing prefix.
pub fn normalize_new_destination(path: &Path) -> Result<PathBuf, RepositoryError> {
    let lexical = absolute_lexical(path)?;
    let (anchor, tail) = nearest_existing(&lexical)?;
    let anchor = anchor
        .canonicalize()
        .map_err(|error| RepositoryError::io(&anchor, error))?;
    Ok(anchor.join(tail))
}

/// 解析目标及唯一包含工作区，不创建锁、目录或日志。 / Resolves a destination and its unique enclosing workspace without creating locks, directories, or journals.
pub fn inspect_new_destination(path: &Path) -> Result<NewProjectLocation, RepositoryError> {
    let destination = normalize_new_destination(path)?;
    let (mut cursor, _) = nearest_existing(&destination)?;
    cursor = cursor
        .canonicalize()
        .map_err(|error| RepositoryError::io(&cursor, error))?;
    let mut workspaces = Vec::new();
    loop {
        let manifest_path = cursor.join(MANIFEST_FILE_NAME);
        if manifest_path.is_file() {
            let source = fs::read_to_string(&manifest_path)
                .map_err(|error| RepositoryError::io(&manifest_path, error))?;
            let manifest = Manifest::parse(&source)?;
            if manifest.workspace.is_some() {
                let relative = destination.strip_prefix(&cursor).map_err(|_| {
                    RepositoryError::WorkspaceConflict(format!(
                        "destination `{}` escapes candidate workspace `{}`",
                        destination.display(),
                        cursor.display()
                    ))
                })?;
                validate_relative(relative)?;
                workspaces.push(WorkspaceMembership {
                    root: cursor.clone(),
                    member: path_text(relative),
                });
            }
        }
        if !cursor.pop() {
            break;
        }
    }
    if workspaces.len() > 1 {
        return Err(RepositoryError::WorkspaceConflict(format!(
            "destination `{}` is enclosed by multiple workspaces",
            destination.display()
        )));
    }
    Ok(NewProjectLocation {
        destination,
        workspace: workspaces.pop(),
    })
}

/// 候选树发布前的宿主准备端口。 / Host preparation port invoked before candidate publication.
pub trait StagePreparer {
    /// 在候选项目根执行外部准备，例如 `git init`。 / Performs external preparation such as `git init` in the candidate project root.
    fn prepare(&self, candidate_root: &Path, vcs: ProjectVcs) -> io::Result<()>;

    /// 返回提交前是否已请求取消。 / Returns whether cancellation was requested before commit.
    ///
    /// The repository samples this once at the final pre-publication boundary. Cancellation after
    /// that sample races with an irreversible rename and is therefore a deferred, post-commit
    /// concern rather than grounds for rollback.
    fn cancellation_requested(&self) -> bool {
        false
    }
}

/// 不执行外部准备的默认端口。 / Default port that performs no external preparation.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoStagePreparation;

impl StagePreparer for NoStagePreparation {
    fn prepare(&self, _candidate_root: &Path, _vcs: ProjectVcs) -> io::Result<()> {
        Ok(())
    }
}

/// 目录发布的原子、不覆盖平台原语。 / Atomic, no-overwrite platform primitive for directory publication.
pub trait DirectoryPublisher {
    /// 把 `source` 原子移动到尚不存在的 `destination`。 / Atomically moves `source` to a non-existing `destination`.
    ///
    /// An error may be outcome-unknown on remote filesystems. The repository reconciles its
    /// durable marker and both paths rather than assuming the move did not happen.
    fn publish_exclusive(&self, source: &Path, destination: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
struct PlatformDirectoryPublisher;

impl DirectoryPublisher for PlatformDirectoryPublisher {
    fn publish_exclusive(&self, source: &Path, destination: &Path) -> io::Result<()> {
        squish_platform_fs::rename_exclusive(source, destination)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Prepared,
    Publishing,
    Published,
    WorkspaceReplaced,
    Completed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    phase: Phase,
    #[serde(with = "native_path_serde")]
    destination: PathBuf,
    #[serde(with = "native_path_serde")]
    publish_path: PathBuf,
    #[serde(with = "native_path_serde")]
    stage_root: PathBuf,
    #[serde(with = "native_path_serde")]
    missing_tail: PathBuf,
    marker_name: String,
    workspace: Option<WorkspaceMembership>,
    package_name: String,
}

struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// 原子发布项目并在需要时把它加入包含工作区。 / Atomically publishes a project and joins an enclosing workspace when needed.
///
/// All fallible validation and external preparation occurs before directory publication. Once the
/// directory rename succeeds, errors leave a durable journal and later recovery rolls forward.
pub fn create_project(
    request: &CreateProjectRequest,
    preparer: &dyn StagePreparer,
    faults: &dyn FaultInjector,
) -> Result<CreatedProject, RepositoryError> {
    create_project_with_publisher(request, preparer, faults, &PlatformDirectoryPublisher)
}

/// 使用可注入发布原语创建项目，供平台竞争测试使用。 / Creates a project with an injectable publication primitive for platform-race tests.
#[doc(hidden)]
pub fn create_project_with_publisher(
    request: &CreateProjectRequest,
    preparer: &dyn StagePreparer,
    faults: &dyn FaultInjector,
    publisher: &dyn DirectoryPublisher,
) -> Result<CreatedProject, RepositoryError> {
    validate_request(request)?;
    if !request.destination.is_absolute() {
        return Err(RepositoryError::Layout(
            "new-project destination must be absolute and normalized".into(),
        ));
    }
    let receipt_destination = request.destination.clone();
    let destination = normalize_new_destination(&request.destination)?;
    let workspace = request
        .workspace
        .as_ref()
        .map(normalize_workspace)
        .transpose()?;
    if let Some(workspace) = &workspace {
        if Path::new(&workspace.member) == Path::new(STATE_DIR) {
            return Err(RepositoryError::WorkspaceConflict(format!(
                "top-level member `{STATE_DIR}` is reserved for workspace manager state"
            )));
        }
        let expected = normalize_new_destination(&workspace.root.join(&workspace.member))?;
        if expected != destination {
            return Err(RepositoryError::WorkspaceConflict(format!(
                "member `{}` does not name destination `{}`",
                workspace.member,
                destination.display()
            )));
        }
    }
    let transaction_anchor = workspace.as_ref().map_or_else(
        || nearest_existing(&destination).map(|value| value.0),
        |value| Ok(value.root.clone()),
    )?;
    let transaction_relative = destination.strip_prefix(&transaction_anchor).map_err(|_| {
        RepositoryError::Layout("transaction anchor does not contain destination".into())
    })?;
    let state_dir = state_dir_for(transaction_relative)?;
    let _lock = creation_lock(&transaction_anchor, state_dir)?;
    recover_locked(&transaction_anchor, state_dir, faults)?;
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(RepositoryError::DestinationExists(destination));
    }
    let _workspace_lock = workspace
        .as_ref()
        .map(|workspace| workspace_lock(&workspace.root))
        .transpose()?;
    if let Some(workspace) = &workspace {
        // Creation→workspace is the global lock order. Reconcile an older file transaction while
        // already holding that workspace lock so validation never observes half-committed policy.
        crate::transaction::recover_locked(&workspace.root, faults)?;
    }
    let workspace_updated = workspace.as_ref().is_some_and(|workspace| {
        !membership_effective(workspace, &request.package_name).unwrap_or(false)
    });
    if let Some(workspace) = &workspace {
        preflight_membership(workspace, &request.package_name, &destination)?;
    }

    // A sibling creator may have published part of our formerly missing prefix while we waited.
    // Replanning turns that race into the ordinary nearest-existing-ancestor case.
    let (publish_anchor, missing_tail) = nearest_existing(&destination)?;
    let id = generation();
    let tx = transaction_anchor
        .join(state_dir)
        .join(CREATIONS_DIR)
        .join(&id);
    let stage_root = publish_anchor.join(format!(".xmlsquish-new-stage-{id}"));
    let staged_publish = stage_root.join(first_component(&missing_tail)?);
    let staged_destination = stage_root.join(&missing_tail);
    let marker_name = format!(".xmlsquish-published-{id}");
    let publish_path = publish_anchor.join(first_component(&missing_tail)?);
    let mut journal = Journal {
        version: 1,
        phase: Phase::Prepared,
        destination: destination.clone(),
        publish_path: publish_path.clone(),
        stage_root: stage_root.clone(),
        missing_tail,
        marker_name,
        workspace,
        package_name: request.package_name.clone(),
    };
    // The journal precedes every staging side effect. A crash can therefore always identify and
    // remove the exact candidate root without guessing from directory names.
    if let Err(error) = store_journal(&tx, &journal) {
        let _ = cleanup_predecision(&tx);
        return Err(error);
    }
    faults
        .check(FaultPoint::CreationPrepared)
        .map_err(|e| RepositoryError::io(&tx, e))?;
    let preparation = (|| {
        fs::create_dir_all(&staged_destination)
            .map_err(|e| RepositoryError::io(&staged_destination, e))?;
        for file in &request.files {
            let target = staged_destination.join(&file.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| RepositoryError::io(parent, e))?;
            }
            write_new(&target, &file.bytes)?;
        }
        // The initializer must never observe half-durable public scaffold state. It may add its
        // own private metadata (for example `.git`), which is synchronized by the second walk.
        sync_tree(&staged_publish)?;
        preparer
            .prepare(&staged_destination, request.vcs)
            .map_err(|e| RepositoryError::io(&staged_destination, e))?;
        write_new(&staged_publish.join(&journal.marker_name), id.as_bytes())?;
        sync_tree(&staged_publish)?;
        Ok(())
    })();
    if let Err(error) = preparation {
        let _ = cleanup_predecision(&stage_root);
        let _ = cleanup_predecision(&tx);
        return Err(error);
    }

    // The anchor lock serializes cooperative creators. The second check prevents ordinary races;
    // platform directory rename then provides the single publication boundary.
    if fs::symlink_metadata(&publish_path).is_ok() {
        cleanup_predecision(&stage_root)?;
        cleanup_predecision(&tx)?;
        return Err(RepositoryError::DestinationExists(destination));
    }
    if preparer.cancellation_requested() {
        cleanup_predecision(&stage_root)?;
        cleanup_predecision(&tx)?;
        return Err(RepositoryError::CreationCancelled(receipt_destination));
    }
    journal.phase = Phase::Publishing;
    store_journal(&tx, &journal)?;
    if let Err(error) = publisher.publish_exclusive(&staged_publish, &publish_path) {
        let published_marker = publish_path.join(&journal.marker_name);
        if !published_marker.is_file() {
            if staged_publish.exists() {
                let collision = fs::symlink_metadata(&publish_path).is_ok();
                cleanup_predecision(&stage_root)?;
                cleanup_predecision(&tx)?;
                return if collision {
                    Err(RepositoryError::DestinationExists(destination))
                } else {
                    Err(RepositoryError::io(&publish_path, error))
                };
            }
            return Err(RepositoryError::PublicationOutcomeUnknown {
                destination: receipt_destination,
                source: error,
            });
        }
    }
    let completion = (|| {
        sync_directory(&stage_root)?;
        sync_directory(&publish_anchor)?;
        // Deliberately inject before the phase write: the in-tree marker closes this exact
        // rename→journal window and recovery must infer publication from it.
        faults
            .check(FaultPoint::CreationPublished)
            .map_err(|e| RepositoryError::io(&publish_path, e))?;
        journal.phase = Phase::Published;
        store_journal(&tx, &journal)?;
        finish_published(&tx, &mut journal, faults, _workspace_lock.is_some())
    })();
    if let Err(source) = completion {
        return Err(RepositoryError::CreationCommitted {
            destination: receipt_destination,
            source: Box::new(source),
        });
    }
    Ok(CreatedProject {
        destination: receipt_destination,
        workspace_updated,
    })
}

/// 恢复某个事务锚点下所有项目创建。 / Recovers every project creation below one transaction anchor.
pub fn recover_project_creations(
    anchor: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    let anchor = anchor
        .canonicalize()
        .map_err(|error| RepositoryError::io(anchor, error))?;
    for state_dir in [STATE_DIR, ".xmlsquish-state"] {
        if !anchor.join(state_dir).join(CREATIONS_DIR).is_dir() {
            continue;
        }
        let _lock = creation_lock(&anchor, state_dir)?;
        recover_locked(&anchor, state_dir, faults)?;
    }
    Ok(())
}

pub(crate) fn recover_creation_ancestors(
    root: &Path,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    for ancestor in root.ancestors() {
        recover_project_creations(ancestor, faults)?;
    }
    Ok(())
}

fn recover_locked(
    anchor: &Path,
    state_dir: &str,
    faults: &dyn FaultInjector,
) -> Result<(), RepositoryError> {
    let root = anchor.join(state_dir).join(CREATIONS_DIR);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(RepositoryError::io(&root, error)),
    };
    let mut dirs = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RepositoryError::io(&root, e))?;
    dirs.sort_unstable_by_key(|entry| entry.file_name());
    for entry in dirs {
        if !entry
            .file_type()
            .map_err(|e| RepositoryError::io(entry.path(), e))?
            .is_dir()
        {
            continue;
        }
        let tx = entry.path();
        if !tx.join(JOURNAL_NAME).is_file() {
            // The journal is created before staging, so an unjournaled directory is provably
            // predecision and owns no external candidate path.
            cleanup_predecision(&tx)?;
            continue;
        }
        let mut journal = load_journal(&tx)?;
        if journal.phase == Phase::Completed {
            let marker = journal.publish_path.join(&journal.marker_name);
            if marker.is_file() {
                fs::remove_file(&marker).map_err(|error| RepositoryError::io(&marker, error))?;
                sync_directory(&journal.publish_path)?;
            }
            cleanup_predecision(&tx)?;
            cleanup_predecision(&journal.stage_root)?;
            continue;
        }
        let marker = journal.publish_path.join(&journal.marker_name);
        if journal.phase == Phase::Prepared && !marker.is_file() {
            cleanup_predecision(&journal.stage_root)?;
            cleanup_predecision(&tx)?;
            continue;
        }
        if journal.phase == Phase::Publishing && !marker.is_file() {
            let staged_publish = journal
                .stage_root
                .join(first_component(&journal.missing_tail)?);
            if staged_publish.exists() {
                cleanup_predecision(&journal.stage_root)?;
                cleanup_predecision(&tx)?;
                continue;
            }
            return Err(RepositoryError::Journal {
                path: tx.join(JOURNAL_NAME),
                message: "exclusive publication outcome is unknown: source and marker are absent"
                    .into(),
            });
        }
        if !marker.is_file() {
            return Err(RepositoryError::Journal {
                path: tx.join(JOURNAL_NAME),
                message: "published project marker is missing".into(),
            });
        }
        journal.phase = Phase::Published;
        finish_published(&tx, &mut journal, faults, false)?;
    }
    Ok(())
}

fn finish_published(
    tx: &Path,
    journal: &mut Journal,
    faults: &dyn FaultInjector,
    workspace_already_locked: bool,
) -> Result<(), RepositoryError> {
    if journal.phase != Phase::WorkspaceReplaced && journal.phase != Phase::Completed {
        if let Some(workspace) = &journal.workspace {
            ensure_membership(workspace, &journal.package_name, workspace_already_locked)?;
        }
        // Ensure-member is semantic and idempotent, so fault before recording the phase tests
        // the workspace-replace→journal recovery boundary rather than the easy side of it.
        faults
            .check(FaultPoint::CreationWorkspaceReplaced)
            .map_err(|e| RepositoryError::io(tx, e))?;
        journal.phase = Phase::WorkspaceReplaced;
        store_journal(tx, journal)?;
    }
    journal.phase = Phase::Completed;
    store_journal(tx, journal)?;
    fs::remove_file(journal.publish_path.join(&journal.marker_name))
        .map_err(|e| RepositoryError::io(&journal.publish_path, e))?;
    sync_directory(&journal.publish_path)?;
    faults
        .check(FaultPoint::CreationCompleted)
        .map_err(|e| RepositoryError::io(tx, e))?;
    cleanup_predecision(&journal.stage_root)?;
    fs::remove_dir_all(tx).map_err(|e| RepositoryError::io(tx, e))?;
    Ok(())
}

fn validate_request(request: &CreateProjectRequest) -> Result<(), RepositoryError> {
    if request.destination.as_os_str().is_empty() {
        return Err(RepositoryError::Layout(
            "new-project destination is empty".into(),
        ));
    }
    if request.files.is_empty() {
        return Err(RepositoryError::Layout(
            "new-project scaffold is empty".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for file in &request.files {
        validate_relative(&file.path)?;
        if !seen.insert(&file.path) {
            return Err(RepositoryError::Layout(format!(
                "duplicate scaffold path `{}`",
                file.path.display()
            )));
        }
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<(), RepositoryError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(RepositoryError::Layout(format!(
            "invalid project-relative path `{}`",
            path.display()
        )));
    }
    Ok(())
}

fn preflight_membership(
    workspace: &WorkspaceMembership,
    package: &str,
    destination: &Path,
) -> Result<(), RepositoryError> {
    validate_member(workspace)?;
    let manifest_path = workspace.root.join(MANIFEST_FILE_NAME);
    let source =
        fs::read_to_string(&manifest_path).map_err(|e| RepositoryError::io(&manifest_path, e))?;
    let manifest = Manifest::parse(&source)?;
    let policy = manifest.workspace.as_ref().ok_or_else(|| {
        RepositoryError::WorkspaceConflict(format!(
            "`{}` is not a workspace root",
            workspace.root.display()
        ))
    })?;
    if policy
        .exclude
        .iter()
        .any(|pattern| pattern_matches(pattern, &workspace.member).unwrap_or(false))
    {
        return Err(RepositoryError::WorkspaceConflict(format!(
            "member `{}` is excluded",
            workspace.member
        )));
    }
    if package_occurrences(&workspace.root, &manifest, package, Some(destination))? != 0 {
        return Err(RepositoryError::WorkspaceConflict(format!(
            "duplicate workspace package name `{package}`"
        )));
    }
    Ok(())
}

fn membership_effective(
    workspace: &WorkspaceMembership,
    _package: &str,
) -> Result<bool, RepositoryError> {
    let path = workspace.root.join(MANIFEST_FILE_NAME);
    let source = fs::read_to_string(&path).map_err(|e| RepositoryError::io(&path, e))?;
    let manifest = Manifest::parse(&source)?;
    Ok(manifest.workspace.as_ref().is_some_and(|policy| {
        policy
            .members
            .iter()
            .any(|p| pattern_matches(&path_text(p), &workspace.member).unwrap_or(false))
    }))
}

fn ensure_membership(
    workspace: &WorkspaceMembership,
    package: &str,
    already_locked: bool,
) -> Result<bool, RepositoryError> {
    let _lock = if already_locked {
        None
    } else {
        Some(workspace_lock(&workspace.root)?)
    };
    preflight_membership_after_publish(workspace, package)?;
    if membership_effective(workspace, package)? {
        return Ok(false);
    }
    let path = workspace.root.join(MANIFEST_FILE_NAME);
    let source = fs::read_to_string(&path).map_err(|e| RepositoryError::io(&path, e))?;
    let mut doc = source.parse::<DocumentMut>().map_err(|e| {
        RepositoryError::WorkspaceConflict(format!("cannot edit workspace manifest: {e}"))
    })?;
    let table = doc
        .get_mut("workspace")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| RepositoryError::WorkspaceConflict("workspace table disappeared".into()))?;
    match table.get_mut("members") {
        Some(Item::Value(Value::Array(array))) => array.push(workspace.member.clone()),
        Some(_) => {
            return Err(RepositoryError::WorkspaceConflict(
                "workspace.members is not an array".into(),
            ));
        }
        None => {
            let mut array = Array::new();
            array.push(workspace.member.clone());
            table.insert("members", Item::Value(Value::Array(array)));
        }
    }
    atomic_replace(&path, doc.to_string().as_bytes())?;
    Ok(true)
}

fn preflight_membership_after_publish(
    workspace: &WorkspaceMembership,
    package: &str,
) -> Result<(), RepositoryError> {
    validate_member(workspace)?;
    let path = workspace.root.join(MANIFEST_FILE_NAME);
    let source = fs::read_to_string(&path).map_err(|e| RepositoryError::io(&path, e))?;
    let manifest = Manifest::parse(&source)?;
    let policy = manifest.workspace.as_ref().ok_or_else(|| {
        RepositoryError::WorkspaceConflict("workspace declaration disappeared".into())
    })?;
    if policy
        .exclude
        .iter()
        .any(|p| pattern_matches(p, &workspace.member).unwrap_or(false))
    {
        return Err(RepositoryError::WorkspaceConflict(format!(
            "member `{}` is excluded",
            workspace.member
        )));
    }
    let destination = workspace.root.join(&workspace.member);
    let duplicates = other_package_occurrences(&workspace.root, &manifest, package, &destination)?;
    if duplicates != 0 {
        return Err(RepositoryError::WorkspaceConflict(format!(
            "duplicate workspace package name `{package}`"
        )));
    }
    Ok(())
}

fn other_package_occurrences(
    root: &Path,
    manifest: &Manifest,
    package: &str,
    created_destination: &Path,
) -> Result<usize, RepositoryError> {
    let mut count = usize::from(
        manifest
            .package
            .as_ref()
            .is_some_and(|candidate| candidate.name == package),
    );
    let policy = manifest.workspace.as_ref().ok_or_else(|| {
        RepositoryError::WorkspaceConflict("workspace declaration disappeared".into())
    })?;
    let created = created_destination
        .canonicalize()
        .map_err(|error| RepositoryError::io(created_destination, error))?;
    for directory in crate::repository::workspace_member_dirs(root, policy)? {
        if directory == created {
            continue;
        }
        let path = directory.join(MANIFEST_FILE_NAME);
        let source =
            fs::read_to_string(&path).map_err(|error| RepositoryError::io(&path, error))?;
        let member = Manifest::parse(&source)?;
        count += usize::from(
            member
                .package
                .as_ref()
                .is_some_and(|candidate| candidate.name == package),
        );
    }
    Ok(count)
}

fn validate_member(workspace: &WorkspaceMembership) -> Result<(), RepositoryError> {
    let member = Path::new(&workspace.member);
    validate_relative(member)?;
    let expected = absolute_lexical(&workspace.root.join(member))?;
    if !expected.starts_with(absolute_lexical(&workspace.root)?) {
        return Err(RepositoryError::WorkspaceConflict(
            "member escapes workspace root".into(),
        ));
    }
    Ok(())
}

fn normalize_workspace(
    workspace: &WorkspaceMembership,
) -> Result<WorkspaceMembership, RepositoryError> {
    let root = workspace
        .root
        .canonicalize()
        .map_err(|error| RepositoryError::io(&workspace.root, error))?;
    Ok(WorkspaceMembership {
        root,
        member: workspace.member.clone(),
    })
}

fn pattern_matches(pattern: &str, member: &str) -> Result<bool, RepositoryError> {
    Pattern::new(pattern)
        .map(|p| p.matches(member))
        .map_err(|e| {
            RepositoryError::WorkspaceConflict(format!(
                "invalid workspace pattern `{pattern}`: {e}"
            ))
        })
}

fn package_occurrences(
    root: &Path,
    manifest: &Manifest,
    package: &str,
    allowed_missing: Option<&Path>,
) -> Result<usize, RepositoryError> {
    let mut count = usize::from(
        manifest
            .package
            .as_ref()
            .is_some_and(|candidate| candidate.name == package),
    );
    let policy = manifest.workspace.as_ref().ok_or_else(|| {
        RepositoryError::WorkspaceConflict("workspace declaration disappeared".into())
    })?;
    for directory in workspace_dirs_allowing(root, policy, allowed_missing)? {
        let path = directory.join(MANIFEST_FILE_NAME);
        let source = fs::read_to_string(&path).map_err(|e| RepositoryError::io(&path, e))?;
        let member = Manifest::parse(&source)?;
        count += usize::from(
            member
                .package
                .as_ref()
                .is_some_and(|candidate| candidate.name == package),
        );
    }
    Ok(count)
}

fn workspace_dirs_allowing(
    root: &Path,
    policy: &squish_project::Workspace,
    allowed_missing: Option<&Path>,
) -> Result<Vec<PathBuf>, RepositoryError> {
    let mut effective = policy.clone();
    if let Some(allowed) = allowed_missing.filter(|path| !path.exists()) {
        let relative = allowed.strip_prefix(root).map_err(|_| {
            RepositoryError::WorkspaceConflict("prospective member escapes workspace".into())
        })?;
        effective.members.retain(|member| member != relative);
    }
    crate::repository::workspace_member_dirs(root, &effective)
}

fn path_text(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn nearest_existing(destination: &Path) -> Result<(PathBuf, PathBuf), RepositoryError> {
    let mut cursor = destination.to_path_buf();
    let mut tail = PathBuf::new();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(_) => {
                if tail.as_os_str().is_empty() {
                    return Err(RepositoryError::DestinationExists(
                        destination.to_path_buf(),
                    ));
                }
                if !cursor.is_dir() {
                    return Err(RepositoryError::Layout(format!(
                        "ancestor `{}` is not a directory",
                        cursor.display()
                    )));
                }
                return Ok((cursor, tail));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    RepositoryError::Layout("destination has no existing ancestor".into())
                })?;
                tail = PathBuf::from(name).join(tail);
                cursor.pop();
            }
            Err(error) => return Err(RepositoryError::io(&cursor, error)),
        }
    }
}

fn first_component(path: &Path) -> Result<&std::ffi::OsStr, RepositoryError> {
    match path.components().next() {
        Some(Component::Normal(value)) => Ok(value),
        _ => Err(RepositoryError::Layout(
            "missing path has no normal first component".into(),
        )),
    }
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, RepositoryError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| RepositoryError::io(path, e))?
            .join(path)
    };
    let mut output = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    return Err(RepositoryError::Layout(
                        "path escapes filesystem root".into(),
                    ));
                }
            }
            other => output.push(other.as_os_str()),
        }
    }
    Ok(output)
}

fn creation_lock(anchor: &Path, state_dir: &str) -> Result<Lock, RepositoryError> {
    lock_file(&anchor.join(state_dir).join(LOCK_NAME))
}
fn workspace_lock(root: &Path) -> Result<Lock, RepositoryError> {
    lock_file(&root.join(STATE_DIR).join("repository.lock"))
}

fn lock_file(path: &Path) -> Result<Lock, RepositoryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| RepositoryError::io(parent, e))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| RepositoryError::io(path, e))?;
    file.lock_exclusive()
        .map_err(|e| RepositoryError::io(path, e))?;
    Ok(Lock(file))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), RepositoryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| RepositoryError::io(path, e))?;
    file.write_all(bytes)
        .map_err(|e| RepositoryError::io(path, e))?;
    file.sync_all().map_err(|e| RepositoryError::io(path, e))
}

fn store_journal(tx: &Path, journal: &Journal) -> Result<(), RepositoryError> {
    fs::create_dir_all(tx).map_err(|e| RepositoryError::io(tx, e))?;
    let bytes = serde_json::to_vec(journal).map_err(|e| RepositoryError::Journal {
        path: tx.join(JOURNAL_NAME),
        message: e.to_string(),
    })?;
    atomic_replace(&tx.join(JOURNAL_NAME), &bytes)?;
    sync_directory(tx)?;
    if let Some(parent) = tx.parent() {
        sync_directory(parent)?;
        if let Some(state) = parent.parent() {
            sync_directory(state)?;
            if let Some(anchor) = state.parent() {
                // `create_dir_all` may have created state, transaction collection, and id in one
                // call. Syncing every parent persists each directory entry up to the pre-existing
                // transaction anchor rather than only the journal file itself.
                sync_directory(anchor)?;
            }
        }
    }
    Ok(())
}

fn load_journal(tx: &Path) -> Result<Journal, RepositoryError> {
    let path = tx.join(JOURNAL_NAME);
    let bytes = fs::read(&path).map_err(|e| RepositoryError::io(&path, e))?;
    serde_json::from_slice(&bytes).map_err(|e| RepositoryError::Journal {
        path,
        message: e.to_string(),
    })
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), RepositoryError> {
    let temp = path.with_extension(format!("new-{}", generation()));
    write_new(&temp, bytes)?;
    fs::rename(&temp, path).map_err(|e| RepositoryError::io(path, e))?;
    sync_directory(path.parent().expect("file has parent"))
}

fn cleanup_predecision(tx: &Path) -> Result<(), RepositoryError> {
    match fs::remove_dir_all(tx) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RepositoryError::io(tx, error)),
    }
}

fn sync_tree(path: &Path) -> Result<(), RepositoryError> {
    for entry in fs::read_dir(path).map_err(|e| RepositoryError::io(path, e))? {
        let entry = entry.map_err(|e| RepositoryError::io(path, e))?;
        let kind = entry
            .file_type()
            .map_err(|e| RepositoryError::io(entry.path(), e))?;
        if kind.is_dir() {
            sync_tree(&entry.path())?;
        } else if kind.is_file() {
            match File::open(entry.path()).and_then(|file| file.sync_all()) {
                Ok(()) => {}
                Err(error)
                    if cfg!(windows)
                        && matches!(
                            error.kind(),
                            io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput
                        ) =>
                {
                    // Windows may reject FlushFileBuffers on a read-only handle. Scaffold files
                    // were synced by their writer; host-created metadata receives best effort.
                }
                Err(error) => return Err(RepositoryError::io(entry.path(), error)),
            }
        }
    }
    sync_directory(path)
}

fn sync_directory(path: &Path) -> Result<(), RepositoryError> {
    match File::open(path).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(error)
            if cfg!(windows)
                && matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput
                ) =>
        {
            Ok(())
        }
        Err(error) => Err(RepositoryError::io(path, error)),
    }
}

fn generation() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{time:032x}-{sequence:016x}-{}", std::process::id())
}

/// 为持久日志编码无损原生路径。 / Losslessly encodes native paths in durable journals.
///
/// Legacy string values remain readable so transactions written by the initial v1 implementation
/// can still recover. New writes always carry explicit native units and never call a lossy string
/// conversion.
mod native_path_serde {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
    };

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    enum NativePath {
        UnixBytes(Vec<u8>),
        WindowsUtf16(Vec<u16>),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CompatiblePath {
        Legacy(String),
        Native(NativePath),
    }

    pub(super) fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            return NativePath::UnixBytes(path.as_os_str().as_bytes().to_vec())
                .serialize(serializer);
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            return NativePath::WindowsUtf16(path.as_os_str().encode_wide().collect())
                .serialize(serializer);
        }
        #[allow(unreachable_code)]
        Err(serde::ser::Error::custom(
            "native path journal encoding is unsupported on this platform",
        ))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        match CompatiblePath::deserialize(deserializer)? {
            CompatiblePath::Legacy(value) => Ok(PathBuf::from(value)),
            CompatiblePath::Native(NativePath::UnixBytes(bytes)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    Ok(PathBuf::from(OsString::from_vec(bytes)))
                }
                #[cfg(not(unix))]
                {
                    let _ = bytes;
                    Err(de::Error::custom(
                        "Unix-byte journal path cannot be decoded on this platform",
                    ))
                }
            }
            CompatiblePath::Native(NativePath::WindowsUtf16(units)) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStringExt;
                    Ok(PathBuf::from(OsString::from_wide(&units)))
                }
                #[cfg(not(windows))]
                {
                    let _ = units;
                    Err(de::Error::custom(
                        "Windows UTF-16 journal path cannot be decoded on this platform",
                    ))
                }
            }
        }
    }
}

fn state_dir_for(missing_tail: &Path) -> Result<&'static str, RepositoryError> {
    if first_component(missing_tail)? == std::ffi::OsStr::new(STATE_DIR) {
        Ok(".xmlsquish-state")
    } else {
        Ok(STATE_DIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoFault;
    use std::sync::{Arc, Barrier};
    use tempfile::tempdir;

    struct Fail(FaultPoint);
    impl FaultInjector for Fail {
        fn check(&self, point: FaultPoint) -> io::Result<()> {
            if point == self.0 {
                Err(io::Error::other("injected"))
            } else {
                Ok(())
            }
        }
    }

    struct PreparationFailure;
    impl StagePreparer for PreparationFailure {
        fn prepare(&self, _candidate_root: &Path, _vcs: ProjectVcs) -> io::Result<()> {
            Err(io::Error::other("initializer failed"))
        }
    }

    struct CancelBeforePublication;
    impl StagePreparer for CancelBeforePublication {
        fn prepare(&self, _candidate_root: &Path, _vcs: ProjectVcs) -> io::Result<()> {
            Ok(())
        }

        fn cancellation_requested(&self) -> bool {
            true
        }
    }

    struct ExternalRacer;
    impl DirectoryPublisher for ExternalRacer {
        fn publish_exclusive(&self, _source: &Path, destination: &Path) -> io::Result<()> {
            fs::create_dir(destination)?;
            fs::write(destination.join("racer-owned"), b"keep")?;
            Err(io::Error::new(io::ErrorKind::AlreadyExists, "racer won"))
        }
    }

    struct MoveThenReportError;
    impl DirectoryPublisher for MoveThenReportError {
        fn publish_exclusive(&self, source: &Path, destination: &Path) -> io::Result<()> {
            fs::rename(source, destination)?;
            Err(io::Error::other("remote filesystem lost the success reply"))
        }
    }

    struct LoseSourceAndPublishForeign;
    impl DirectoryPublisher for LoseSourceAndPublishForeign {
        fn publish_exclusive(&self, source: &Path, destination: &Path) -> io::Result<()> {
            fs::remove_dir_all(source)?;
            fs::create_dir(destination)?;
            fs::write(destination.join("foreign-owned"), b"keep")?;
            Err(io::Error::other("publication outcome unavailable"))
        }
    }

    fn request(destination: PathBuf) -> CreateProjectRequest {
        CreateProjectRequest {
            destination: normalize_new_destination(&destination).unwrap(),
            files: vec![ProjectFile { path: PathBuf::from("xmlsquish.toml"), bytes: b"manifest-version = 1\n[package]\nname='new'\nversion='0.1.0'\ndialect='xmlsquish/1'\nsource-root='src'\n".to_vec() }],
            package_name: "new".into(), vcs: ProjectVcs::None, workspace: None,
        }
    }

    #[test]
    fn publishes_deep_missing_subtree_and_leaves_no_marker() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("a/b/new")).unwrap();
        let made =
            create_project(&request(destination.clone()), &NoStagePreparation, &NoFault).unwrap();
        assert_eq!(made.destination, destination);
        assert!(destination.join("xmlsquish.toml").is_file());
        assert_eq!(
            fs::read_dir(temp.path().join("a"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".xmlsquish-published"))
                .count(),
            0
        );
    }

    #[test]
    fn relative_request_is_rejected_without_writes() {
        let mut req = request(normalize_new_destination(Path::new("relative-new")).unwrap());
        req.destination = PathBuf::from("relative-new");
        assert!(matches!(
            create_project(&req, &NoStagePreparation, &NoFault),
            Err(RepositoryError::Layout(_))
        ));
        assert!(!Path::new("relative-new").exists());
    }

    #[test]
    fn state_directory_name_remains_a_valid_destination() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join(STATE_DIR)).unwrap();
        create_project(&request(destination.clone()), &NoStagePreparation, &NoFault).unwrap();
        assert!(destination.join(MANIFEST_FILE_NAME).is_file());
    }

    #[test]
    fn initializer_failure_removes_unjournaled_candidate() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        assert!(
            create_project(&request(destination.clone()), &PreparationFailure, &NoFault).is_err()
        );
        assert!(!destination.exists());
        recover_project_creations(temp.path(), &NoFault).unwrap();
    }

    #[test]
    fn final_prepublication_cancellation_removes_stage_and_journal() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        let error = create_project(
            &request(destination.clone()),
            &CancelBeforePublication,
            &NoFault,
        )
        .unwrap_err();
        assert!(matches!(error, RepositoryError::CreationCancelled(path) if path == destination));
        assert!(!destination.exists());
        recover_project_creations(temp.path(), &NoFault).unwrap();
        let stages = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".xmlsquish-new-stage-")
            })
            .count();
        assert_eq!(stages, 0);
    }

    #[test]
    fn inspection_finds_workspace_without_mutating_it() {
        let temp = tempdir().unwrap();
        let manifest = temp.path().join(MANIFEST_FILE_NAME);
        fs::write(
            &manifest,
            "manifest-version = 1\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        let before = fs::read(&manifest).unwrap();
        let location = inspect_new_destination(&temp.path().join("packages/new")).unwrap();
        assert_eq!(location.workspace.unwrap().member, "packages/new");
        assert_eq!(fs::read(&manifest).unwrap(), before);
        assert!(!temp.path().join(STATE_DIR).exists());
    }

    #[test]
    fn prepared_failure_recovers_by_removing_only_staging() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("deep/new")).unwrap();
        assert!(
            create_project(
                &request(destination.clone()),
                &NoStagePreparation,
                &Fail(FaultPoint::CreationPrepared)
            )
            .is_err()
        );
        assert!(!destination.exists());
        recover_project_creations(temp.path(), &NoFault).unwrap();
        assert!(!destination.exists());
    }

    #[test]
    fn every_postpublication_fault_rolls_forward() {
        for point in [
            FaultPoint::CreationPublished,
            FaultPoint::CreationWorkspaceReplaced,
            FaultPoint::CreationCompleted,
        ] {
            let temp = tempdir().unwrap();
            let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
            let error = create_project(
                &request(destination.clone()),
                &NoStagePreparation,
                &Fail(point),
            )
            .unwrap_err();
            assert_eq!(error.committed_creation(), Some(destination.as_path()));
            recover_project_creations(temp.path(), &NoFault).unwrap();
            assert!(destination.join("xmlsquish.toml").is_file(), "{point:?}");
            assert!(!destination.join(".xmlsquish-published").exists());
        }
    }

    #[test]
    fn concurrent_creators_have_exactly_one_winner() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let destination = destination.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    create_project(&request(destination), &NoStagePreparation, &NoFault)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(RepositoryError::DestinationExists(_))))
                .count(),
            1
        );
    }

    #[test]
    fn exclusive_publication_never_overwrites_an_external_race_winner() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        let error = create_project_with_publisher(
            &request(destination.clone()),
            &NoStagePreparation,
            &NoFault,
            &ExternalRacer,
        )
        .unwrap_err();
        assert!(matches!(error, RepositoryError::DestinationExists(path) if path == destination));
        assert_eq!(fs::read(destination.join("racer-owned")).unwrap(), b"keep");
        recover_project_creations(temp.path(), &NoFault).unwrap();
    }

    #[test]
    fn publication_error_after_move_is_reconciled_by_marker() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        let created = create_project_with_publisher(
            &request(destination.clone()),
            &NoStagePreparation,
            &NoFault,
            &MoveThenReportError,
        )
        .unwrap();
        assert_eq!(created.destination, destination);
        assert!(created.destination.join(MANIFEST_FILE_NAME).is_file());
    }

    #[test]
    fn indeterminate_publication_preserves_journal_and_foreign_destination() {
        let temp = tempdir().unwrap();
        let destination = normalize_new_destination(&temp.path().join("new")).unwrap();
        let error = create_project_with_publisher(
            &request(destination.clone()),
            &NoStagePreparation,
            &NoFault,
            &LoseSourceAndPublishForeign,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RepositoryError::PublicationOutcomeUnknown { destination: path, .. }
                if path == destination
        ));
        assert_eq!(
            fs::read(destination.join("foreign-owned")).unwrap(),
            b"keep"
        );
        let journals = temp.path().join(STATE_DIR).join(CREATIONS_DIR);
        assert_eq!(fs::read_dir(&journals).unwrap().count(), 1);

        assert!(matches!(
            recover_project_creations(temp.path(), &NoFault),
            Err(RepositoryError::Journal { message, .. }) if message.contains("outcome is unknown")
        ));
        assert_eq!(
            fs::read(destination.join("foreign-owned")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::read_dir(journals).unwrap().count(), 1);
    }

    #[test]
    fn sibling_creators_replan_a_shared_missing_parent() {
        let temp = tempdir().unwrap();
        let left = normalize_new_destination(&temp.path().join("a/left")).unwrap();
        let right = normalize_new_destination(&temp.path().join("a/right")).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [left.clone(), right.clone()]
            .into_iter()
            .map(|destination| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    create_project(&request(destination), &NoStagePreparation, &NoFault)
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert!(left.join(MANIFEST_FILE_NAME).is_file());
        assert!(right.join(MANIFEST_FILE_NAME).is_file());
    }

    #[test]
    fn missing_exact_member_is_allowed_and_not_duplicated() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("packages")).unwrap();
        let manifest = temp.path().join(MANIFEST_FILE_NAME);
        fs::write(
            &manifest,
            "manifest-version = 1\n[workspace]\nmembers = [\"packages/new\"]\n",
        )
        .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let destination = normalize_new_destination(&root.join("packages/new")).unwrap();
        let mut req = request(destination.clone());
        req.workspace = Some(WorkspaceMembership {
            root,
            member: "packages/new".into(),
        });
        create_project(&req, &NoStagePreparation, &NoFault).unwrap();
        assert!(destination.is_dir());
        assert_eq!(
            fs::read_to_string(manifest)
                .unwrap()
                .matches("packages/new")
                .count(),
            1
        );
    }

    #[test]
    fn workspace_root_recovers_when_packages_parent_preexisted() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("packages")).unwrap();
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let destination = normalize_new_destination(&root.join("packages/new")).unwrap();
        let mut req = request(destination.clone());
        req.workspace = Some(WorkspaceMembership {
            root: root.clone(),
            member: "packages/new".into(),
        });
        assert!(
            create_project(
                &req,
                &NoStagePreparation,
                &Fail(FaultPoint::CreationPublished)
            )
            .is_err()
        );
        assert!(root.join(STATE_DIR).join(CREATIONS_DIR).is_dir());
        assert!(!root.join("packages").join(STATE_DIR).exists());
        recover_project_creations(&root, &NoFault).unwrap();
        assert!(destination.is_dir());
        assert!(
            fs::read_to_string(root.join(MANIFEST_FILE_NAME))
                .unwrap()
                .contains("packages/new")
        );
    }

    #[test]
    fn recovery_rejects_a_racing_duplicate_outside_created_destination() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("packages")).unwrap();
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let destination = normalize_new_destination(&root.join("packages/new")).unwrap();
        let mut req = request(destination.clone());
        req.workspace = Some(WorkspaceMembership {
            root: root.clone(),
            member: "packages/new".into(),
        });
        assert!(
            create_project(
                &req,
                &NoStagePreparation,
                &Fail(FaultPoint::CreationPublished)
            )
            .is_err()
        );

        let racer = root.join("packages/racer");
        fs::create_dir(&racer).unwrap();
        fs::write(
            racer.join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[package]\nname='new'\nversion='0.1.0'\ndialect='xmlsquish/1'\nsource-root='src'\n",
        )
        .unwrap();
        assert!(matches!(
            recover_project_creations(&root, &NoFault),
            Err(RepositoryError::WorkspaceConflict(message)) if message.contains("duplicate")
        ));
        assert!(destination.is_dir());
    }

    #[test]
    fn recovery_rejects_duplicate_in_an_explicitly_listed_other_member() {
        let temp = tempdir().unwrap();
        let existing = temp.path().join("packages/existing");
        fs::create_dir_all(&existing).unwrap();
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = [\"packages/existing\"]\n",
        )
        .unwrap();
        fs::write(
            existing.join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[package]\nname='existing'\nversion='0.1.0'\ndialect='xmlsquish/1'\nsource-root='src'\n",
        )
        .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let root_manifest = root.join(MANIFEST_FILE_NAME);
        let destination = normalize_new_destination(&root.join("packages/new")).unwrap();
        let mut req = request(destination.clone());
        req.workspace = Some(WorkspaceMembership {
            root: root.clone(),
            member: "packages/new".into(),
        });
        assert!(
            create_project(
                &req,
                &NoStagePreparation,
                &Fail(FaultPoint::CreationPublished)
            )
            .is_err()
        );

        fs::write(
            existing.join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[package]\nname='new'\nversion='0.1.0'\ndialect='xmlsquish/1'\nsource-root='src'\n",
        )
        .unwrap();
        assert!(matches!(
            recover_project_creations(&root, &NoFault),
            Err(RepositoryError::WorkspaceConflict(message)) if message.contains("duplicate")
        ));
        assert!(
            !fs::read_to_string(root_manifest)
                .unwrap()
                .contains("packages/new")
        );
        assert!(destination.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_destination_round_trips_through_recovery_journal() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let temp = tempdir().unwrap();
        let leaf = OsString::from_vec(b"new-\xff".to_vec());
        let destination = normalize_new_destination(&temp.path().join(leaf)).unwrap();
        let error = create_project(
            &request(destination.clone()),
            &NoStagePreparation,
            &Fail(FaultPoint::CreationPublished),
        )
        .unwrap_err();
        assert_eq!(error.committed_creation(), Some(destination.as_path()));
        recover_project_creations(temp.path(), &NoFault).unwrap();
        assert!(destination.join(MANIFEST_FILE_NAME).is_file());
    }

    #[test]
    fn recovery_discards_unjournaled_predecision_directory() {
        let temp = tempdir().unwrap();
        let orphan = temp
            .path()
            .join(STATE_DIR)
            .join(CREATIONS_DIR)
            .join("orphan");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("partial"), b"partial").unwrap();
        recover_project_creations(temp.path(), &NoFault).unwrap();
        assert!(!orphan.exists());
    }

    #[test]
    fn durable_v1_reader_accepts_legacy_string_paths() {
        let value = serde_json::json!({
            "version": 1,
            "phase": "prepared",
            "destination": "legacy/new",
            "publish_path": "legacy/new",
            "stage_root": "legacy/stage",
            "missing_tail": "new",
            "marker_name": ".xmlsquish-published-legacy",
            "workspace": null,
            "package_name": "new"
        });
        let journal: Journal = serde_json::from_value(value).unwrap();
        assert_eq!(journal.destination, PathBuf::from("legacy/new"));
        assert_eq!(journal.stage_root, PathBuf::from("legacy/stage"));
    }

    #[test]
    fn recovery_removes_a_partially_staged_journaled_tree() {
        let temp = tempdir().unwrap();
        let anchor = temp.path().canonicalize().unwrap();
        let destination = normalize_new_destination(&anchor.join("new")).unwrap();
        let stage_root = anchor.join(".xmlsquish-new-stage-crashed");
        let tx = anchor.join(STATE_DIR).join(CREATIONS_DIR).join("crashed");
        let journal = Journal {
            version: 1,
            phase: Phase::Prepared,
            destination: destination.clone(),
            publish_path: destination,
            stage_root: stage_root.clone(),
            missing_tail: PathBuf::from("new"),
            marker_name: ".xmlsquish-published-crashed".into(),
            workspace: None,
            package_name: "new".into(),
        };
        store_journal(&tx, &journal).unwrap();
        fs::create_dir_all(stage_root.join("new/.git/objects")).unwrap();
        fs::write(stage_root.join("new/.git/HEAD"), b"partial").unwrap();
        recover_project_creations(&anchor, &NoFault).unwrap();
        assert!(!tx.exists());
        assert!(!stage_root.exists());
    }

    #[test]
    fn workspace_rejects_reserved_state_member_without_changes() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let manifest = root.join(MANIFEST_FILE_NAME);
        let before = fs::read(&manifest).unwrap();
        let destination = normalize_new_destination(&root.join(STATE_DIR)).unwrap();
        let mut req = request(destination.clone());
        req.workspace = Some(WorkspaceMembership {
            root: root.clone(),
            member: STATE_DIR.into(),
        });
        assert!(matches!(
            create_project(&req, &NoStagePreparation, &NoFault),
            Err(RepositoryError::WorkspaceConflict(message)) if message.contains("reserved")
        ));
        assert!(!destination.exists());
        assert!(!root.join(".xmlsquish-state").exists());
        assert_eq!(fs::read(manifest).unwrap(), before);
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_destination_matches_canonical_workspace_root() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join(MANIFEST_FILE_NAME),
            "manifest-version = 1\n[workspace]\nmembers = []\n",
        )
        .unwrap();
        let native_destination = temp.path().join("packages/new");
        let mut req = request(normalize_new_destination(&native_destination).unwrap());
        req.destination = native_destination;
        req.workspace = Some(WorkspaceMembership {
            root: temp.path().canonicalize().unwrap(),
            member: "packages/new".into(),
        });
        create_project(&req, &NoStagePreparation, &NoFault).unwrap();
    }

    #[test]
    fn workspace_edit_preserves_comments_and_recovery_is_idempotent() {
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("xmlsquish.toml"),
            "manifest-version = 1\n# keep me\n[workspace]\nmembers = [] # tail\n",
        )
        .unwrap();
        let mut req = request(temp.path().join("packages/new"));
        req.workspace = Some(WorkspaceMembership {
            root: temp.path().canonicalize().unwrap(),
            member: "packages/new".into(),
        });
        assert!(
            create_project(
                &req,
                &NoStagePreparation,
                &Fail(FaultPoint::CreationWorkspaceReplaced),
            )
            .is_err()
        );
        recover_project_creations(temp.path(), &NoFault).unwrap();
        recover_project_creations(temp.path(), &NoFault).unwrap();
        let source = fs::read_to_string(temp.path().join("xmlsquish.toml")).unwrap();
        assert!(source.contains("# keep me"));
        assert!(source.contains("# tail"));
        assert_eq!(source.matches("packages/new").count(), 1);
    }
}
