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
mod scaffold;
mod transaction;

pub use edit::{CandidateManifest, DependencyChange, EditPlan};
pub use error::{ProjectError, ValidationIssue};
pub use lock::{LockedPackage, LockedSource, Lockfile};
pub use manifest::Manifest;
pub use model::{
    DependencyDetail, DependencySpec, GitReference, Limits, Package, Profile, Target, Workspace,
};
pub use resolver::{DependencyResolver, ResolutionInput, ResolutionMode};
pub use scaffold::{NewProjectSpec, ProjectScaffold, STARTER_SOURCE, ScaffoldFile, ScaffoldVcs};
pub use squish_protocol::{InvalidPackageName, PackageName};
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

/// 根据协议与清单共享的唯一包名文法验证文本。 / Validates text with the single package-name grammar shared by protocol and manifests.
///
/// This helper lets manifest validation retain its path-addressable [`ValidationIssue`] mapping
/// while all package identities use [`PackageName`] as the authoritative implementation.
///
/// # Errors
///
/// Returns [`InvalidPackageName`] for an empty name or for any byte outside ASCII letters,
/// digits, `-`, and `_`. Input is never normalized.
pub fn validate_package_name(value: &str) -> Result<(), InvalidPackageName> {
    PackageName::new(value).map(|_| ())
}

#[cfg(test)]
mod package_name_tests {
    use super::*;

    #[test]
    fn manifest_helper_delegates_to_protocol_identity() {
        for valid in ["a", "Agent42", "support-agent", "support_agent", "0"] {
            validate_package_name(valid).unwrap();
            assert_eq!(PackageName::new(valid).unwrap().as_str(), valid);
        }
        for invalid in ["", "a b", "agent.", "代理", "a/b", "a\\b", "a:b"] {
            assert_eq!(validate_package_name(invalid), Err(InvalidPackageName));
            assert_eq!(PackageName::new(invalid), Err(InvalidPackageName));
        }
    }
}
