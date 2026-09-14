//! 清单的有类型数据模型。 / Typed manifest data model.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

/// 工作区统一策略。 / Workspace-wide policy and membership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Workspace {
    /// Member manifest directories. / 成员清单目录。
    #[serde(default)]
    pub members: Vec<PathBuf>,
    /// Glob-like exclusions interpreted by the repository adapter. / 由仓库适配器解释的排除项。
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Shared output root. / 共享输出根目录。
    #[serde(default = "default_target_dir")]
    pub target_dir: PathBuf,
    /// Dependencies inherited with `{ workspace = true }`. / 由 `{ workspace = true }` 继承的依赖。
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            exclude: Vec::new(),
            target_dir: default_target_dir(),
            dependencies: BTreeMap::new(),
        }
    }
}

fn default_target_dir() -> PathBuf {
    PathBuf::from("target/xmlsquish")
}

/// 可发布包的身份与前端默认值。 / Package identity and frontend defaults.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Package {
    /// Stable package name. / 稳定包名。
    pub name: String,
    /// Semantic package version. / 语义化包版本。
    pub version: Version,
    /// Frontend dialect identifier. / 前端方言标识符。
    #[serde(default = "default_dialect")]
    pub dialect: String,
    /// Source root relative to this manifest. / 相对清单的源根目录。
    #[serde(default = "default_source_root")]
    pub source_root: PathBuf,
}

fn default_dialect() -> String {
    "xmlsquish/1".into()
}
fn default_source_root() -> PathBuf {
    PathBuf::from("src")
}

/// 可构建的链接入口。 / A buildable link entry point.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Target {
    /// Entry source relative to the package. / 相对包的入口源。
    pub entry: PathBuf,
    /// Backend plugin/capability identifier. / 后端插件或能力标识符。
    #[serde(default = "default_backend")]
    pub backend: String,
    /// Published path; defaults to `<target>.prompt`. / 发布路径，默认 `<target>.prompt`。
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Link-time scalar arguments. / 链接时标量参数。
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    /// Target resource limits. / 目标资源限制。
    #[serde(default)]
    pub limits: Limits,
    /// Enabled frontend/backend features. / 启用的前后端特性。
    #[serde(default)]
    pub features: BTreeSet<String>,
}

fn default_backend() -> String {
    "squish".into()
}

impl Target {
    /// 返回显式输出，否则生成 `<target>.prompt`。 / Returns the explicit output or derives `<target>.prompt`.
    pub fn output_path(&self, target_name: &str) -> PathBuf {
        self.output
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{target_name}.prompt")))
    }
}

/// 构建或格式化策略的命名覆盖。 / Named build/format policy overrides.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Profile {
    /// Optional parent profile. / 可选父 profile。
    #[serde(default)]
    pub inherits: Option<String>,
    /// Override whether to emit a debug companion; `None` inherits. / 覆盖是否生成调试伴随文件；`None` 表示继承。
    #[serde(default)]
    pub debug_info: Option<bool>,
    /// Stable backend-specific optimization label. / 稳定的后端优化等级标签。
    #[serde(default)]
    pub optimization: Option<String>,
    /// Profile argument overrides. / Profile 参数覆盖。
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    /// Profile resource limits. / Profile 资源限制。
    #[serde(default)]
    pub limits: Limits,
    /// Features added by the profile. / Profile 增加的特性。
    #[serde(default)]
    pub features: BTreeSet<String>,
}

/// 执行预算；`None` 表示由上层或后端决定。 / Execution budgets; `None` delegates upward or to the backend.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Limits {
    /// Maximum macro depth. / 最大宏深度。
    pub max_depth: Option<u64>,
    /// Maximum expansions. / 最大展开数。
    pub max_expansions: Option<u64>,
    /// Maximum output bytes. / 最大输出字节数。
    pub max_output_bytes: Option<u64>,
}

/// 依赖声明的简写或完整形式。 / Shorthand or detailed dependency declaration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Registry version requirement shorthand. / Registry 版本要求简写。
    Version(VersionReq),
    /// Fully typed declaration. / 完整有类型声明。
    Detail(Box<DependencyDetail>),
}

impl DependencySpec {
    /// 构造 registry 依赖。 / Constructs a registry dependency.
    pub fn registry(requirement: VersionReq, registry: Option<String>) -> Self {
        Self::Detail(Box::new(DependencyDetail {
            version: Some(requirement),
            registry,
            ..DependencyDetail::default()
        }))
    }

    /// 构造 Git 依赖；resolver 将选择器锁定为 commit。 / Constructs a Git dependency whose selector the resolver pins to a commit.
    pub fn git(repository: impl Into<String>, reference: GitReference) -> Self {
        Self::Detail(Box::new(DependencyDetail {
            git: Some(repository.into()),
            git_reference: reference,
            ..DependencyDetail::default()
        }))
    }

    /// 构造相对本地 path 依赖。 / Constructs a relative local path dependency.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Detail(Box::new(DependencyDetail {
            path: Some(path.into()),
            ..DependencyDetail::default()
        }))
    }

    /// 继承同别名 workspace 依赖。 / Inherits the same-alias workspace dependency.
    pub fn workspace() -> Self {
        Self::Detail(Box::new(DependencyDetail {
            workspace: true,
            ..DependencyDetail::default()
        }))
    }
}

/// 依赖来源与选择器。 / Dependency source and selectors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct DependencyDetail {
    /// Registry semantic-version requirement. / Registry 语义版本要求。
    pub version: Option<VersionReq>,
    /// Non-default registry URL/name. / 非默认 registry URL/名称。
    pub registry: Option<String>,
    /// Git repository URL. / Git 仓库 URL。
    pub git: Option<String>,
    /// Git selector; resolution must pin it to a commit. / Git 选择器；解析必须锁定 commit。
    #[serde(flatten)]
    pub git_reference: GitReference,
    /// Local dependency directory. / 本地依赖目录。
    pub path: Option<PathBuf>,
    /// Inherit a workspace dependency of the same alias. / 继承同别名工作区依赖。
    #[serde(default)]
    pub workspace: bool,
    /// Rename the resolved package. / 重命名解析后的包。
    pub package: Option<String>,
    /// Optional dependency flag. / 可选依赖标志。
    #[serde(default)]
    pub optional: bool,
    /// Requested dependency features. / 请求的依赖特性。
    #[serde(default)]
    pub features: BTreeSet<String>,
    /// Enable dependency default features. / 启用依赖默认特性。
    #[serde(default = "default_true")]
    pub default_features: bool,
}

impl Default for DependencyDetail {
    fn default() -> Self {
        Self {
            version: None,
            registry: None,
            git: None,
            git_reference: GitReference::default(),
            path: None,
            workspace: false,
            package: None,
            optional: false,
            features: BTreeSet::new(),
            default_features: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// 互斥的 Git 可变选择器。 / Mutually exclusive mutable Git selector.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct GitReference {
    /// Requested branch. / 请求的分支。
    pub branch: Option<String>,
    /// Requested tag. / 请求的标签。
    pub tag: Option<String>,
    /// Requested commit revision. / 请求的 commit revision。
    pub rev: Option<String>,
}
