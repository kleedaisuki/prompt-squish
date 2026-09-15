//! 远程依赖的具体来源宿主。 / Concrete source host for remote dependencies.
//!
//! Registry 与 Git 在获取后都进入同一个 [`LogicalTree`]，因此路径规则、内容摘要与
//! 原子物化只有一份实现。 / Registry and Git acquisition converge on one [`LogicalTree`],
//! leaving one implementation of path policy, content identity, and atomic publication.

#![forbid(unsafe_code)]

mod filesystem;
mod git;
mod materialize;
mod registry;
mod types;

pub use filesystem::FilesystemHost;
pub use git::{GitHost, GitInvocation, GitRunOutput, GitRunner, GitSelector, SystemGitRunner};
pub use materialize::{LogicalFile, LogicalTree, Materializer};
pub use registry::{
    AuthorizationValue, CredentialError, CredentialLookup, CredentialPort, HttpRequest,
    HttpResponse, HttpTransport, NoCredentials, RegistryConfig, ReqwestTransport, SparseRegistry,
};
pub use types::*;
