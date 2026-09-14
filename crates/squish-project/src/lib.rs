//! prompt-squish 的项目模型与依赖领域。 / Project-model and dependency domain for prompt-squish.
//!
//! This crate owns human-authored manifest intent, exact resolved lock state,
//! comment-preserving dependency edits, and recoverable mutation plans. It has
//! no filesystem or network adapter and therefore cannot download or execute code.

#![forbid(unsafe_code)]

mod edit;
mod error;
mod lock;
mod manifest;
mod model;
mod resolver;
mod transaction;

pub use edit::{CandidateManifest, DependencyChange, EditPlan};
pub use error::{ProjectError, ValidationIssue};
pub use lock::{LockedPackage, LockedSource, Lockfile};
pub use manifest::Manifest;
pub use model::{
    DependencyDetail, DependencySpec, GitReference, Limits, Package, Profile, Target, Workspace,
};
pub use resolver::{DependencyResolver, ResolutionInput, ResolutionMode};
pub use transaction::{
    CommitPreparation, JournalRecord, MutationFile, MutationKind, MutationPlan, MutationPlanner,
    TransactionId,
};

/// 当前支持的清单架构版本。 / Current supported manifest schema version.
pub const MANIFEST_VERSION: u32 = 1;
/// 当前支持的锁文件架构版本。 / Current supported lock schema version.
pub const LOCK_VERSION: u32 = 1;
/// 项目清单的标准文件名。 / Canonical project manifest filename.
pub const MANIFEST_FILE_NAME: &str = "xmlsquish.toml";
/// 工作区锁的标准文件名。 / Canonical workspace lock filename.
pub const LOCK_FILE_NAME: &str = "xmlsquish.lock";
