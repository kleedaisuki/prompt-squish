use std::{fmt, ops::Range, path::PathBuf};

use thiserror::Error;

/// 配置值所属的合并层。 / Merge layer that supplied a configuration value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigLayer {
    /// 编译期默认值。 / Compiled-in defaults.
    Defaults,
    /// 用户配置文件。 / User configuration file.
    User(PathBuf),
    /// 工作区配置文件。 / Workspace configuration file.
    Workspace(PathBuf),
    /// 按命令行顺序应用的覆盖项。 / CLI override applied in argv order.
    Cli { index: usize },
}

impl fmt::Display for ConfigLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Defaults => f.write_str("compiled defaults"),
            Self::User(path) => write!(f, "user config {}", path.display()),
            Self::Workspace(path) => write!(f, "workspace config {}", path.display()),
            Self::Cli { index } => write!(f, "CLI override #{}", index + 1),
        }
    }
}

/// 指向配置来源中字节范围的位置。 / Byte-range location in a configuration source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    /// 产生问题的层。 / Layer containing the problem.
    pub layer: ConfigLayer,
    /// UTF-8 源文本中的字节范围。 / Byte range in the UTF-8 source text.
    pub span: Option<Range<usize>>,
}

/// 配置加载或验证错误。 / Configuration loading or validation error.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// 配置文件无法读取。 / A selected configuration file could not be read.
    #[error("cannot read {location}: {source}")]
    Read {
        /// 文件层。 / File layer.
        location: ConfigLayer,
        /// 底层 I/O 错误。 / Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// TOML 文本无效。 / TOML text is invalid.
    #[error("invalid TOML in {location:?}: {message}")]
    Parse {
        /// 来源位置。 / Source location.
        location: SourceLocation,
        /// 解析器消息。 / Parser message.
        message: String,
    },
    /// 出现未声明或拼错的键。 / An undeclared or misspelled key was found.
    #[error("unknown configuration key `{key}` in {location:?}")]
    UnknownKey {
        /// 完整点分键。 / Full dotted key.
        key: String,
        /// 来源位置。 / Source location.
        location: SourceLocation,
    },
    /// 配置值违反领域约束。 / A value violates a domain constraint.
    #[error("invalid value for `{key}` in {location:?}: {message}")]
    InvalidValue {
        /// 完整点分键。 / Full dotted key.
        key: String,
        /// 来源位置。 / Source location.
        location: SourceLocation,
        /// 约束说明。 / Constraint explanation.
        message: String,
    },
    /// 两个注册表身份发生碰撞。 / Two registry identities collide.
    #[error("registry collision between `{first}` and `{second}` for {kind} `{value}`")]
    RegistryCollision {
        /// 第一个别名。 / First alias.
        first: String,
        /// 第二个别名。 / Second alias.
        second: String,
        /// 碰撞类型。 / Collision kind.
        kind: &'static str,
        /// 规范化后的值。 / Canonical colliding value.
        value: String,
        /// 较高优先级定义的位置。 / Location of the later definition.
        location: Box<SourceLocation>,
    },
}
