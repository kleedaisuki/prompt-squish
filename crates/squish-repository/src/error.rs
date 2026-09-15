use std::{io, path::PathBuf};

use thiserror::Error;

/// 文件系统项目适配器错误。 / Filesystem project-adapter error.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// 新项目目标已被其它文件系统对象占用。 / A new-project destination is already occupied.
    #[error("project destination already exists: {0}")]
    DestinationExists(PathBuf),
    /// 工作区不能无歧义地接纳新成员。 / A workspace cannot accept the new member unambiguously.
    #[error("workspace membership conflict: {0}")]
    WorkspaceConflict(String),
    /// 项目创建在不可逆发布前被取消。 / Project creation was cancelled before irreversible publication.
    #[error("project creation cancelled before publication: {0}")]
    CreationCancelled(PathBuf),
    /// 平台未能证明排他发布失败发生在移动之前。 / The platform could not prove that an exclusive-publication error preceded the move.
    #[error("project publication outcome is unknown at {destination}: {source}")]
    PublicationOutcomeUnknown {
        /// 请求的项目目标。 / Requested project destination.
        destination: PathBuf,
        /// 平台发布错误。 / Platform publication error.
        #[source]
        source: io::Error,
    },
    /// 项目目录已不可逆发布，但提交后收尾尚需恢复。 / The project directory was irreversibly published but post-commit completion still needs recovery.
    #[error("project creation committed at {destination}, but completion failed: {source}")]
    CreationCommitted {
        /// 已发布的项目目标。 / Published project destination.
        destination: PathBuf,
        /// 提交后的底层故障。 / Underlying post-commit failure.
        #[source]
        source: Box<RepositoryError>,
    },
    /// 文件系统操作失败。 / A filesystem operation failed.
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// 未发现项目。 / No project was discovered.
    #[error("no xmlsquish.toml found from {0}")]
    NotFound(PathBuf),
    /// 路径或工作区归属无效。 / A path or workspace ownership is invalid.
    #[error("invalid project layout: {0}")]
    Layout(String),
    /// 项目领域文档无效。 / A project-domain document is invalid.
    #[error(transparent)]
    Project(#[from] squish_project::ProjectError),
    /// Journal 数据无效。 / Journal data is invalid.
    #[error("invalid transaction journal at {path}: {message}")]
    Journal { path: PathBuf, message: String },
    /// 乐观提交检测到并发修改。 / Optimistic commit detected concurrent modification.
    #[error("project changed concurrently: {0:?}")]
    Contended(Vec<PathBuf>),
    /// 锁包缺少物理检出位置。 / A locked package lacks a physical checkout location.
    #[error("no checkout location supplied for locked package `{0}`")]
    MissingPackageLocation(String),
    /// 请求的包、目标或 profile 不存在。 / A requested package, target, or profile is absent.
    #[error("unknown project item: {0}")]
    Unknown(String),
}

impl RepositoryError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// 返回已经越过项目创建提交边界的目标（若有）。 / Returns the destination whose project-creation commit boundary was crossed, if any.
    #[must_use]
    pub fn committed_creation(&self) -> Option<&std::path::Path> {
        match self {
            Self::CreationCommitted { destination, .. } => Some(destination),
            _ => None,
        }
    }
}
