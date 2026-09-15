//! `xmlsquish` 项目的具体文件系统仓库。 / Concrete filesystem repository for `xmlsquish` projects.
//!
//! 适配器冻结所有权威项目输入，并执行带持久提交点的向前恢复事务。它不进行依赖求解。
//! The adapter freezes every authoritative project input and executes roll-forward transactions
//! with a durable commit point. It deliberately does not resolve dependencies.

#![forbid(unsafe_code)]

mod creation;
mod error;
mod repository;
mod snapshot;
mod transaction;

pub use creation::{
    CreateProjectRequest, CreatedProject, DirectoryPublisher, NewProjectLocation,
    NoStagePreparation, ProjectFile, ProjectVcs, StagePreparer, WorkspaceMembership,
    create_project, create_project_with_publisher, inspect_new_destination,
    normalize_new_destination, recover_project_creations,
};
pub use error::RepositoryError;
pub use repository::{Discovery, ProjectRepository};
pub use snapshot::{
    LockedManifestSnapshot, ManifestSnapshot, PackageLocation, ProjectSnapshot, ProjectSource,
    ResolvedPackage, ResolvedTarget,
};
pub use transaction::{FaultInjector, FaultPoint, FormatUpdate, NoFault};
