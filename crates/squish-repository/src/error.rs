use std::{io, path::PathBuf};

use thiserror::Error;

/// 文件系统项目适配器错误。 / Filesystem project-adapter error.
#[derive(Debug, Error)]
pub enum RepositoryError {
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
}
