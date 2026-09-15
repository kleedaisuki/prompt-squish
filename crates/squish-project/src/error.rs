//! 项目领域错误。 / Project-domain errors.

use thiserror::Error;

/// 可定位的语义校验问题。 / A path-addressable semantic validation issue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationIssue {
    /// TOML/domain path. / TOML/领域路径。
    pub path: String,
    /// Human-readable explanation. / 人类可读说明。
    pub message: String,
}

impl ValidationIssue {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// 项目模型的有类型失败。 / Typed project-model failure.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// TOML syntax or decoding failed. / TOML 语法或解码失败。
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// Editable TOML syntax failed. / 可编辑 TOML 语法失败。
    #[error("invalid editable TOML: {0}")]
    EditableToml(#[from] toml_edit::TomlError),
    /// The document decoded but violates domain invariants. / 文档解码成功但违反领域不变式。
    #[error("validation failed: {0:?}")]
    Validation(Vec<ValidationIssue>),
    /// A requested dependency does not exist. / 请求的依赖不存在。
    #[error("dependency `{0}` does not exist")]
    MissingDependency(String),
    /// An edit would silently overwrite intent. / 编辑会静默覆盖意图。
    #[error("dependency `{0}` already exists; use replace explicitly")]
    DuplicateDependency(String),
    /// Serialization failed. / 序列化失败。
    #[error("serialization failed: {0}")]
    Serialization(#[from] toml_edit::ser::Error),
    /// Internal scaffold generation violated a deterministic domain invariant. / 内部脚手架生成违反确定性领域不变式。
    #[error("project scaffold invariant failed: {0}")]
    ScaffoldInvariant(String),
}
