//! `xmlsquish` 的纯启动参数解析器。 / Filesystem-free bootstrap argument parser for `xmlsquish`.
//!
//! 此 crate 只把 `argv` 转换成协议请求与显示设置；它不定位项目、不读取清单，也不
//! 探测终端。 / This crate only converts `argv` into a protocol request and presentation
//! settings; it does not locate projects, read manifests, or probe terminal capabilities.
//!
//! # 示例 / Example
//!
//! ```
//! use squish_cli::{BootstrapOutcome, MessageFormat, parse_from};
//! use squish_protocol::OperationRequest;
//!
//! let BootstrapOutcome::Invocation(parsed) =
//!     parse_from([
//!         "xmlsquish",
//!         "--config",
//!         "build.jobs=4",
//!         "build",
//!         "-t",
//!         "chat",
//!         "--message-format=json",
//!     ])?
//! else {
//!     unreachable!("the argv contains a command")
//! };
//! assert!(matches!(parsed.request, OperationRequest::Build(_)));
//! assert_eq!(parsed.presentation.message_format, Some(MessageFormat::Json));
//! assert_eq!(parsed.config_overrides[0].key(), "build.jobs");
//! # Ok::<(), squish_cli::ParseFailure>(())
//! ```

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::{collections::BTreeMap, ffi::OsString, fmt, path::PathBuf};

use clap::{
    ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum,
    error::ErrorKind,
};
use squish_protocol::{
    AddRequest, ArgumentName, ArtifactId, BuildRequest, DependencyKind, DependencyName,
    DependencySource, EmitKind, FeatureName, FormatRequest, FormatSelection, GitBranch,
    GitReference, GitRevision, GitTag, InspectRequest, InspectView, LockMode, NewPackageName,
    NewRequest, OpaqueSourceId, OperationRequest, PackageName, ProfileName, ProjectDestination,
    ProjectPath, RegistryName, RemoveRequest, RepositoryUrl, StyleEdition, TargetName, VcsChoice,
    VersionRequirement, WorkspaceScope,
};

/// 操作消息的外形。 / Shape of operational messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum MessageFormat {
    /// 面向人的完整输出。 / Full human-oriented output.
    Human,
    /// 稳定、逐行的简短输出。 / Stable, line-oriented short output.
    Short,
    /// 逐行 JSON 事件。 / Newline-delimited JSON events.
    Json,
}

/// 终端能力策略。 / Terminal capability policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum TerminalPolicy {
    /// 根据能力决定。 / Derive from terminal capabilities.
    Auto,
    /// 始终启用。 / Always enable.
    Always,
    /// 始终禁用。 / Always disable.
    Never,
}

/// 纯查询结果格式。 / Pure-query result format.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum QueryFormat {
    /// 面向人的描述。 / Human-oriented description.
    Human,
    /// 单个版本化 JSON 文档。 / One versioned JSON document.
    Json,
    /// 仅对 artifact 查询写出摘要验证后的原始字节。 / Write digest-verified raw bytes for artifact queries only.
    Raw,
}

/// 与领域请求正交的显示设置。 / Presentation settings orthogonal to the domain request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationSettings {
    /// 显式消息格式；`None` 让配置解析器决定默认值。 / Explicit message format; `None` defers to configuration.
    pub message_format: Option<MessageFormat>,
    /// 显式颜色策略。 / Explicit color policy.
    pub color: Option<TerminalPolicy>,
    /// 显式进度策略。 / Explicit progress policy.
    pub progress: Option<TerminalPolicy>,
    /// 是否启用无装饰、线性输出包。 / Whether the undecorated, linear output bundle is enabled.
    pub plain: bool,
    /// 是否抑制普通状态与成功摘要。 / Whether normal status and success summaries are suppressed.
    pub quiet: bool,
    /// 详细程度，范围为 `0..=2`。 / Verbosity level in `0..=2`.
    pub verbosity: u8,
    /// 查询文档格式；仅 `inspect` 有值。 / Query document format; set only for `inspect`.
    pub query_format: Option<QueryFormat>,
}

/// 协议暂未携带、但启动阶段必须保真的执行选项。 / Execution options not yet carried by the protocol but preserved by bootstrap parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionSettings {
    /// 从工作区选择中排除的包。 / Packages excluded from workspace selection.
    pub excluded_packages: Vec<PackageName>,
    /// 显式并发上限；`0` 表示管理器选择。 / Explicit concurrency bound; `0` asks the manager to choose.
    pub jobs: Option<u32>,
    /// 首次失败后是否继续接纳独立作业。 / Whether independent jobs remain admissible after the first failure.
    pub keep_going: bool,
    /// 带实参的查询主体。 / Inspection subject retaining its typed argument.
    pub inspect_subject: Option<InspectSubject>,
}

/// 一个保持原始 TOML 值文本的命令行配置覆盖。 / A CLI configuration override retaining its raw TOML value text.
///
/// 解析器仅验证 `KEY=VALUE` 的结构；TOML 值及配置模式由配置加载器验证，以便错误仍能
/// 指向正确的覆盖层与序号。 / The parser validates only the `KEY=VALUE` shape; the
/// configuration loader validates the TOML value and schema so errors retain the correct layer
/// and ordinal.
///
/// # 示例 / Example
///
/// ```
/// use squish_cli::parse_from;
///
/// let parsed = parse_from([
///     "xmlsquish",
///     "build",
///     "--config",
///     "term.message-format=\"json\"",
/// ])?
/// .into_invocation()
/// .expect("the argv contains a command");
/// let value = &parsed.config_overrides[0];
/// assert_eq!(value.key(), "term.message-format");
/// assert_eq!(value.value(), "\"json\"");
/// # Ok::<(), squish_cli::ParseFailure>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigOverride {
    key: String,
    value: String,
}

impl ConfigOverride {
    /// 返回非空的点分配置键。 / Returns the non-empty dotted configuration key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// 返回未经 TOML 解析的值文本。 / Returns the value text without TOML parsing.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// 重新组成配置加载器接受的无损 `KEY=VALUE` 文本。 / Reassembles lossless `KEY=VALUE` text accepted by the configuration loader.
    pub fn into_assignment(self) -> String {
        format!("{}={}", self.key, self.value)
    }
}

/// 带实参、无路径猜测的查询主体。 / Typed inspection subject with no path guessing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectSubject {
    /// IR 产物标识。 / IR artifact identifier.
    Ir(ArtifactId),
    /// 链接目标标识。 / Link target identifier.
    Link(TargetName),
    /// 源码标识。 / Source identifier.
    Source(OpaqueSourceId),
    /// 来源证明产物标识。 / Provenance artifact identifier.
    Provenance(ArtifactId),
    /// 缓存动作键。 / Cache action key.
    Cache(String),
}

/// 启动解析的完整成功值。 / Complete successful bootstrap parse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedInvocation {
    /// 可直接交给管理器内核的类型化请求。 / Typed request ready for the manager kernel.
    pub request: OperationRequest,
    /// 不参与语义缓存键的显示设置。 / Presentation settings excluded from semantic cache keys.
    pub presentation: PresentationSettings,
    /// 调度或协议扩展所消费的保真执行设置。 / Lossless execution settings consumed by scheduling or a protocol extension.
    pub execution: ExecutionSettings,
    /// 按用户给定顺序排列的命令行配置覆盖。 / CLI configuration overrides in exact user-supplied order.
    pub config_overrides: Vec<ConfigOverride>,
}

/// 启动解析的成功结果。 / Successful bootstrap parse outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootstrapOutcome {
    /// 可执行的类型化操作。 / Executable typed operation.
    Invocation(Box<ParsedInvocation>),
    /// 裸调用请求的顶层帮助文本。 / Top-level help requested by a bare invocation.
    BareHelp(BareHelp),
}

impl BootstrapOutcome {
    /// 若结果为操作则返回它，否则保留帮助结果。 / Returns the operation when present, preserving a help result otherwise.
    pub fn into_invocation(self) -> Result<ParsedInvocation, BareHelp> {
        match self {
            Self::Invocation(invocation) => Ok(*invocation),
            Self::BareHelp(help) => Err(help),
        }
    }
}

/// 可由根二进制直接写到 stdout 的裸调用帮助。 / Bare-invocation help ready for the root binary to write to stdout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BareHelp {
    text: String,
}

impl BareHelp {
    /// 返回完整帮助文本。 / Returns the complete help text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// 取得完整帮助文本。 / Consumes the value and returns the complete help text.
    pub fn into_string(self) -> String {
        self.text
    }
}

/// 启动解析失败及其机器输出提示。 / Bootstrap parse failure and its machine-output hint.
#[derive(Debug)]
pub struct ParseFailure {
    error: clap::Error,
    json_requested: bool,
}

impl ParseFailure {
    /// 返回 Clap 的结构化错误类别。 / Returns Clap's structured error kind.
    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    /// 返回适合进程采用的退出码；帮助与版本为 `0`，用法错误为 `2`。 / Returns the process exit code; help/version use `0`, usage failures use `2`.
    pub fn exit_code(&self) -> i32 {
        self.error.exit_code()
    }

    /// 精确的 JSON 消息承诺是否已在启动扫描中出现。 / Whether bootstrap scanning observed an exact JSON-message promise.
    pub const fn json_requested(&self) -> bool {
        self.json_requested
    }

    /// 借用底层 Clap 错误供根二进制渲染。 / Borrows the underlying Clap error for root-binary rendering.
    pub const fn clap_error(&self) -> &clap::Error {
        &self.error
    }

    /// 取得底层 Clap 错误。 / Consumes this value and returns the underlying Clap error.
    pub fn into_clap_error(self) -> clap::Error {
        self.error
    }
}

impl fmt::Display for ParseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ParseFailure {}

/// 仅解析参数，不访问文件系统、环境变量或终端。 / Parses arguments without accessing the filesystem, environment, or terminal.
pub fn parse_from<I, T>(arguments: I) -> Result<BootstrapOutcome, ParseFailure>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    parse_from_with_version(arguments, env!("CARGO_PKG_VERSION"))
}

/// 使用组合二进制的版本解析参数，不访问文件系统、环境变量或终端。 / Parses arguments with the composing binary's version, without accessing the filesystem, environment, or terminal.
///
/// 库自身不知道最终可执行文件的发布版本；根二进制应传入自己的静态版本字符串，使
/// `--version` 与实际发行包一致。 / The library cannot know the final executable's release
/// version; the root binary should pass its own static version string so `--version` identifies
/// the actual distribution.
///
/// # 示例 / Example
///
/// ```
/// use clap::error::ErrorKind;
/// use squish_cli::parse_from_with_version;
///
/// let version = parse_from_with_version(["xmlsquish", "--version"], "3.2.1").unwrap_err();
/// assert_eq!(version.kind(), ErrorKind::DisplayVersion);
/// assert!(version.to_string().contains("3.2.1"));
/// ```
pub fn parse_from_with_version<I, T>(
    arguments: I,
    version: &'static str,
) -> Result<BootstrapOutcome, ParseFailure>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let arguments: Vec<OsString> = arguments.into_iter().map(Into::into).collect();
    if arguments.len() == 1 {
        return Ok(BootstrapOutcome::BareHelp(BareHelp {
            text: Cli::command().version(version).render_help().to_string(),
        }));
    }
    let json_requested = exact_json_requested(&arguments);
    let mut matches = Cli::command()
        .version(version)
        .try_get_matches_from(arguments.iter().cloned())
        .map_err(|error| ParseFailure {
            error,
            json_requested,
        })?;
    let mut cli = Cli::from_arg_matches_mut(&mut matches).map_err(|error| ParseFailure {
        error,
        json_requested,
    })?;
    // Clap propagates a global argument from the deepest subcommand and can therefore discard
    // earlier root occurrences. / Clap 会从最深子命令传播全局参数，因此可能丢弃更早的根层
    // 出现项。Clap 已完成类型与用法验证；这里只从原 argv 恢复跨边界的稳定顺序。
    cli.config_overrides = ordered_config_overrides(&arguments);
    cli.into_invocation()
        .map(Box::new)
        .map(BootstrapOutcome::Invocation)
        .map_err(|error| ParseFailure {
            error,
            json_requested,
        })
}

#[derive(Debug, Parser)]
#[command(
    name = "xmlsquish",
    version,
    about = "Build prompts and manage xmlsquish projects",
    subcommand_required = true,
    after_help = "Examples:\n  xmlsquish new my-prompt\n  xmlsquish build -t chat\n  xmlsquish fmt --check --plain"
)]
struct Cli {
    #[command(flatten)]
    presentation: PresentationArgs,
    /// 用 TOML 值覆盖一个配置键；重复项按顺序应用。 / Override one configuration key with a TOML value; repeat to apply in order.
    #[arg(
        long = "config",
        global = true,
        value_name = "KEY=VALUE",
        value_parser = parse_config_override,
        action = ArgAction::Append
    )]
    config_overrides: Vec<ConfigOverride>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Args)]
struct PresentationArgs {
    /// Suppress status, progress, and successful summaries.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    /// Increase detail; repeat at most twice.
    #[arg(short, long, global = true, action = ArgAction::Count, conflicts_with = "quiet")]
    verbose: u8,
    /// Choose human, short, or newline-delimited JSON operation messages.
    #[arg(long, global = true)]
    message_format: Option<MessageFormat>,
    /// Control ANSI color.
    #[arg(long, global = true)]
    color: Option<TerminalPolicy>,
    /// Control terminal progress repainting.
    #[arg(long, global = true)]
    progress: Option<TerminalPolicy>,
    /// Use accessible, undecorated, append-only output.
    #[arg(long, global = true)]
    plain: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new, immediately buildable project.
    #[command(
        after_help = "Examples:\n  xmlsquish new my-prompt\n  xmlsquish new prompts/support --name support --vcs=none"
    )]
    New(NewArgs),
    /// Build selected prompts.
    #[command(
        after_help = "Examples:\n  xmlsquish build -t chat\n  xmlsquish build --workspace --frozen --message-format=json"
    )]
    Build(BuildArgs),
    /// Format project-owned XML sources.
    #[command(
        after_help = "Examples:\n  xmlsquish fmt\n  xmlsquish fmt --check --path prompts/chat.xml"
    )]
    Fmt(FormatArgs),
    /// Add or update a dependency.
    #[command(
        after_help = "Examples:\n  xmlsquish add prompt-common@^2 --rename common\n  xmlsquish add common --path ../common --dry-run"
    )]
    Add(AddArgs),
    /// Remove a direct dependency alias.
    #[command(
        after_help = "Examples:\n  xmlsquish remove common\n  xmlsquish remove common -p support --dry-run"
    )]
    Remove(RemoveArgs),
    /// Inspect a typed manager object without changing it.
    #[command(
        after_help = "Examples:\n  xmlsquish inspect ir ir:sha256:abcd\n  xmlsquish inspect artifact target/prompts/chat.prompt --format=json"
    )]
    Inspect(InspectArgs),
}

#[derive(Debug, Args)]
struct NewArgs {
    /// Destination that must not already exist.
    #[arg(value_name = "PATH")]
    destination: PathBuf,
    /// Explicit package identity; otherwise infer it from PATH.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Select Git management or no VCS files.
    #[arg(long, value_name = "git|none")]
    vcs: Option<VcsArg>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum VcsArg {
    Git,
    None,
}

#[derive(Debug, Args)]
struct SelectionArgs {
    /// Use an explicit project manifest (no file is opened during parsing).
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<String>,
    /// Select a package; repeat to select several packages.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE", action = ArgAction::Append, conflicts_with = "workspace")]
    packages: Vec<String>,
    /// Select every eligible workspace package.
    #[arg(long, conflicts_with = "packages")]
    workspace: bool,
    /// Exclude a package from --workspace.
    #[arg(long, value_name = "PACKAGE", action = ArgAction::Append, requires = "workspace")]
    exclude: Vec<String>,
}

#[derive(Debug, Args)]
struct ResolutionArgs {
    /// Forbid lockfile mutation.
    #[arg(long)]
    locked: bool,
    /// Forbid network access.
    #[arg(long)]
    offline: bool,
    /// Combine --locked and --offline.
    #[arg(long)]
    frozen: bool,
}

#[derive(Debug, Args)]
struct BuildArgs {
    #[command(flatten)]
    selection: SelectionArgs,
    /// Select a declared target; repeat to select several targets.
    #[arg(short = 't', long = "target", value_name = "TARGET", action = ArgAction::Append)]
    targets: Vec<String>,
    /// Select a named build profile.
    #[arg(long, default_value = "dev")]
    profile: String,
    /// Bound active jobs; zero lets the manager choose.
    #[arg(short = 'j', long)]
    jobs: Option<u32>,
    /// Continue independent work after a failure (the default).
    #[arg(long, default_value_t = true, action = ArgAction::SetTrue, conflicts_with = "no_keep_going")]
    keep_going: bool,
    /// Stop admitting new work after the first failure.
    #[arg(long, action = ArgAction::SetTrue)]
    no_keep_going: bool,
    /// Select one output kind; repeat instead of using commas.
    #[arg(long, value_name = "prompt|ir|debug", action = ArgAction::Append)]
    emit: Vec<EmitArg>,
    /// Bind TARGET.NAME=VALUE; repeat for several arguments.
    #[arg(long = "arg", value_name = "TARGET.NAME=VALUE", value_parser = parse_argument, action = ArgAction::Append)]
    arguments: Vec<(String, String)>,
    #[command(flatten)]
    resolution: ResolutionArgs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum EmitArg {
    Prompt,
    Ir,
    Debug,
}

#[derive(Debug, Args)]
struct FormatArgs {
    #[command(flatten)]
    selection: SelectionArgs,
    /// Select an explicit source path; repeatable.
    #[arg(long, value_name = "PATH", action = ArgAction::Append)]
    path: Vec<String>,
    /// Report changes without writing.
    #[arg(long)]
    check: bool,
    /// Emit differences and imply --check.
    #[arg(long)]
    diff: bool,
    /// Override the manifest's formatting style edition.
    #[arg(long, value_name = "EDITION")]
    style_edition: Option<String>,
}

#[derive(Debug, Args)]
struct PackageSelection {
    /// Use an explicit project manifest (no file is opened during parsing).
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<String>,
    /// Select the workspace package to edit.
    #[arg(short = 'p', long = "package", value_name = "PACKAGE")]
    package: Option<String>,
}

#[derive(Debug, Args)]
struct DependencyKindArgs {
    /// Add or remove a development dependency.
    #[arg(long, conflicts_with = "build")]
    dev: bool,
    /// Add or remove a build-tool dependency.
    #[arg(long, conflicts_with = "dev")]
    build: bool,
}

#[derive(Debug, Args)]
struct AddArgs {
    /// Dependency specification, such as common or common@^2.
    spec: String,
    #[command(flatten)]
    selection: PackageSelection,
    /// Store the dependency under this local alias.
    #[arg(long, value_name = "ALIAS")]
    rename: Option<String>,
    /// Use a local project path.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["git", "registry"])]
    path: Option<String>,
    /// Use a Git repository.
    #[arg(long, value_name = "URL", conflicts_with_all = ["path", "registry"])]
    git: Option<String>,
    /// Select a Git revision.
    #[arg(long, value_name = "REV", requires = "git", conflicts_with_all = ["tag", "branch"])]
    rev: Option<String>,
    /// Select a Git tag.
    #[arg(long, value_name = "TAG", requires = "git", conflicts_with_all = ["rev", "branch"])]
    tag: Option<String>,
    /// Select a Git branch.
    #[arg(long, value_name = "BRANCH", requires = "git", conflicts_with_all = ["rev", "tag"])]
    branch: Option<String>,
    /// Use a named package registry.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["path", "git"])]
    registry: Option<String>,
    /// Enable a comma-separated feature list.
    #[arg(long, value_name = "LIST", value_delimiter = ',', value_parser = parse_feature, action = ArgAction::Append)]
    features: Vec<String>,
    /// Disable default features.
    #[arg(long)]
    no_default_features: bool,
    /// Mark the dependency optional.
    #[arg(long)]
    optional: bool,
    #[command(flatten)]
    kind: DependencyKindArgs,
    #[command(flatten)]
    resolution: ResolutionArgs,
    /// Plan and validate without committing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// Direct dependency alias.
    alias: String,
    #[command(flatten)]
    selection: PackageSelection,
    #[command(flatten)]
    kind: DependencyKindArgs,
    #[command(flatten)]
    resolution: ResolutionArgs,
    /// Plan and validate without committing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct InspectArgs {
    /// Use an explicit project manifest (no file is opened during parsing).
    #[arg(long, value_name = "PATH", global = true)]
    manifest_path: Option<String>,
    /// Choose human, JSON, or raw artifact output.
    #[arg(long, default_value = "human", global = true)]
    format: QueryFormat,
    #[command(subcommand)]
    subject: InspectCommand,
}

#[derive(Debug, Subcommand)]
enum InspectCommand {
    /// Inspect reusable intermediate representation.
    Ir { identifier: String },
    /// Inspect resolved bindings for a target.
    Link { identifier: String },
    /// Inspect source provenance.
    Source { identifier: String },
    /// Inspect artifact provenance by immutable artifact ID.
    Provenance { identifier: String },
    /// Explain an action cache key.
    Cache { action_key: String },
    /// Validate an explicitly named artifact path.
    Artifact { path: String },
}

impl Cli {
    fn into_invocation(self) -> Result<ParsedInvocation, clap::Error> {
        validate_presentation(&self.presentation)?;
        let color = if self.presentation.plain {
            Some(TerminalPolicy::Never)
        } else {
            self.presentation.color
        };
        let progress = if self.presentation.plain {
            Some(TerminalPolicy::Never)
        } else {
            self.presentation.progress
        };
        let presentation = PresentationSettings {
            message_format: self.presentation.message_format,
            color,
            progress,
            plain: self.presentation.plain,
            quiet: self.presentation.quiet,
            verbosity: self.presentation.verbose,
            query_format: None,
        };

        let mut invocation = match self.command {
            Command::New(args) => new_invocation(args, presentation),
            Command::Build(args) => build_invocation(args, presentation),
            Command::Fmt(args) => format_invocation(args, presentation),
            Command::Add(args) => add_invocation(args, presentation),
            Command::Remove(args) => remove_invocation(args, presentation),
            Command::Inspect(args) => inspect_invocation(args, presentation),
        }?;
        invocation.config_overrides = self.config_overrides;
        Ok(invocation)
    }
}

fn new_invocation(
    args: NewArgs,
    presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    let name = args
        .name
        .map(|value| new_package_name(value, "package name"))
        .transpose()?;
    let vcs = args.vcs.map(|vcs| match vcs {
        VcsArg::Git => VcsChoice::Git,
        VcsArg::None => VcsChoice::None,
    });
    Ok(simple_invocation(
        OperationRequest::New(NewRequest {
            destination: ProjectDestination::new(args.destination),
            name,
            vcs,
        }),
        presentation,
    ))
}

fn build_invocation(
    args: BuildArgs,
    presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    let emit = emit_kinds(args.emit)?;
    let arguments = collect_arguments(args.arguments)?;
    let (project, scope, excluded_packages) = selection(args.selection)?;
    let request = BuildRequest {
        project,
        scope,
        targets: ids(args.targets, TargetName::new, "target")?,
        profile: id(args.profile, ProfileName::new, "profile")?,
        arguments,
        emit,
        lock: lock_mode(args.resolution),
    };
    Ok(ParsedInvocation {
        request: OperationRequest::Build(request),
        presentation,
        execution: ExecutionSettings {
            excluded_packages,
            jobs: args.jobs,
            keep_going: !args.no_keep_going,
            inspect_subject: None,
        },
        config_overrides: Vec::new(),
    })
}

fn format_invocation(
    args: FormatArgs,
    presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    let (project, scope, excluded_packages) = selection(args.selection)?;
    let selection = if args.path.is_empty() {
        FormatSelection::All
    } else {
        FormatSelection::Sources(ids(args.path, OpaqueSourceId::new, "path")?)
    };
    let style = args
        .style_edition
        .map(|value| id(value, StyleEdition::new, "style edition"))
        .transpose()?;
    Ok(ParsedInvocation {
        request: OperationRequest::Format(FormatRequest {
            project,
            scope,
            selection,
            style,
            check: args.check || args.diff,
            diff: args.diff,
        }),
        presentation,
        execution: ExecutionSettings {
            excluded_packages,
            jobs: None,
            keep_going: true,
            inspect_subject: None,
        },
        config_overrides: Vec::new(),
    })
}

fn add_invocation(
    args: AddArgs,
    presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    let (package_name, version) = dependency_spec(&args.spec)?;
    let source = dependency_source(&args, version)?;
    let (dependency, rename) = match args.rename {
        Some(alias) => (
            id(alias, DependencyName::new, "dependency alias")?,
            Some(id(package_name.clone(), PackageName::new, "package")?),
        ),
        None => (
            id(package_name.clone(), DependencyName::new, "dependency")?,
            None,
        ),
    };
    let features = ids(args.features, FeatureName::new, "feature")?;
    let request = AddRequest {
        project: project(args.selection.manifest_path)?,
        package: args
            .selection
            .package
            .map(|value| id(value, PackageName::new, "package"))
            .transpose()?,
        dependency,
        rename,
        source,
        kind: dependency_kind(args.kind),
        features,
        no_default_features: args.no_default_features,
        optional: args.optional,
        lock: lock_mode(args.resolution),
        dry_run: args.dry_run,
    };
    Ok(simple_invocation(
        OperationRequest::Add(request),
        presentation,
    ))
}

fn remove_invocation(
    args: RemoveArgs,
    presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    let request = RemoveRequest {
        project: project(args.selection.manifest_path)?,
        package: args
            .selection
            .package
            .map(|value| id(value, PackageName::new, "package"))
            .transpose()?,
        dependency: id(args.alias, DependencyName::new, "dependency alias")?,
        kind: dependency_kind(args.kind),
        lock: lock_mode(args.resolution),
        dry_run: args.dry_run,
    };
    Ok(simple_invocation(
        OperationRequest::Remove(request),
        presentation,
    ))
}

fn inspect_invocation(
    args: InspectArgs,
    mut presentation: PresentationSettings,
) -> Result<ParsedInvocation, clap::Error> {
    if presentation.message_format.is_some() {
        return Err(usage(
            "inspect uses --format=human|json|raw and rejects --message-format",
        ));
    }
    presentation.query_format = Some(args.format);
    let (view, subject) = match args.subject {
        InspectCommand::Ir { identifier } => {
            let value = id(identifier, ArtifactId::new, "IR identifier")?;
            (
                InspectView::Ir(value.clone()),
                Some(InspectSubject::Ir(value)),
            )
        }
        InspectCommand::Link { identifier } => {
            let value = id(identifier, TargetName::new, "link identifier")?;
            (
                InspectView::Link(value.clone()),
                Some(InspectSubject::Link(value)),
            )
        }
        InspectCommand::Source { identifier } => {
            let value = id(identifier, OpaqueSourceId::new, "source identifier")?;
            (
                InspectView::Source(value.clone()),
                Some(InspectSubject::Source(value)),
            )
        }
        InspectCommand::Provenance { identifier } => {
            let value = id(identifier, ArtifactId::new, "artifact identifier")?;
            (
                InspectView::Provenance(value.clone()),
                Some(InspectSubject::Provenance(value)),
            )
        }
        InspectCommand::Cache { action_key } => {
            require_nonempty(&action_key, "action key")?;
            (InspectView::Cache, Some(InspectSubject::Cache(action_key)))
        }
        InspectCommand::Artifact { path } => {
            let path = id(path, ProjectPath::new, "artifact path")?;
            (InspectView::Artifact(path), None)
        }
    };
    if args.format == QueryFormat::Raw && !matches!(view, InspectView::Artifact(_)) {
        return Err(usage("inspect --format=raw is supported only for artifact"));
    }
    Ok(ParsedInvocation {
        request: OperationRequest::Inspect(InspectRequest {
            project: project(args.manifest_path)?,
            view,
        }),
        presentation,
        execution: ExecutionSettings {
            excluded_packages: Vec::new(),
            jobs: None,
            keep_going: true,
            inspect_subject: subject,
        },
        config_overrides: Vec::new(),
    })
}

fn simple_invocation(
    request: OperationRequest,
    presentation: PresentationSettings,
) -> ParsedInvocation {
    ParsedInvocation {
        request,
        presentation,
        execution: ExecutionSettings {
            excluded_packages: Vec::new(),
            jobs: None,
            keep_going: true,
            inspect_subject: None,
        },
        config_overrides: Vec::new(),
    }
}

fn validate_presentation(args: &PresentationArgs) -> Result<(), clap::Error> {
    if args.quiet && args.verbose > 0 {
        return Err(cli_error(
            ErrorKind::ArgumentConflict,
            "--quiet conflicts with --verbose",
        ));
    }
    if args.verbose > 2 {
        return Err(usage("--verbose may be repeated at most twice"));
    }
    if args.plain && !matches!(args.color, None | Some(TerminalPolicy::Never)) {
        return Err(usage(
            "--plain conflicts with --color=auto and --color=always",
        ));
    }
    if args.plain && !matches!(args.progress, None | Some(TerminalPolicy::Never)) {
        return Err(usage(
            "--plain conflicts with --progress=auto and --progress=always",
        ));
    }
    Ok(())
}

fn selection(
    args: SelectionArgs,
) -> Result<(ProjectPath, WorkspaceScope, Vec<PackageName>), clap::Error> {
    let scope = if args.workspace {
        WorkspaceScope::Workspace
    } else if args.packages.is_empty() {
        WorkspaceScope::Current
    } else {
        WorkspaceScope::Packages(ids(args.packages, PackageName::new, "package")?)
    };
    Ok((
        project(args.manifest_path)?,
        scope,
        ids(args.exclude, PackageName::new, "excluded package")?,
    ))
}

fn project(manifest_path: Option<String>) -> Result<ProjectPath, clap::Error> {
    id(
        manifest_path.unwrap_or_else(|| ".".to_owned()),
        ProjectPath::new,
        "manifest path",
    )
}

fn lock_mode(args: ResolutionArgs) -> LockMode {
    if args.frozen || (args.locked && args.offline) {
        LockMode::Frozen
    } else if args.locked {
        LockMode::Locked
    } else if args.offline {
        LockMode::Offline
    } else {
        LockMode::Update
    }
}

fn dependency_kind(args: DependencyKindArgs) -> DependencyKind {
    if args.dev {
        DependencyKind::Development
    } else if args.build {
        DependencyKind::Build
    } else {
        DependencyKind::Normal
    }
}

fn dependency_source(
    args: &AddArgs,
    version: Option<String>,
) -> Result<DependencySource, clap::Error> {
    if let Some(path) = &args.path {
        reject_version_for_non_registry(version.as_deref())?;
        return Ok(DependencySource::Path {
            path: id(path.clone(), ProjectPath::new, "dependency path")?,
        });
    }
    if let Some(repository) = &args.git {
        reject_version_for_non_registry(version.as_deref())?;
        let reference = if let Some(revision) = &args.rev {
            GitReference::Revision(id(revision.clone(), GitRevision::new, "Git revision")?)
        } else if let Some(tag) = &args.tag {
            GitReference::Tag(id(tag.clone(), GitTag::new, "Git tag")?)
        } else {
            GitReference::Branch(id(
                args.branch.clone().unwrap_or_else(|| "HEAD".to_owned()),
                GitBranch::new,
                "Git branch",
            )?)
        };
        return Ok(DependencySource::Git {
            repository: id(repository.clone(), RepositoryUrl::new, "Git URL")?,
            reference,
        });
    }
    Ok(DependencySource::Registry {
        registry: args
            .registry
            .clone()
            .map(|value| id(value, RegistryName::new, "registry"))
            .transpose()?,
        version: id(
            version.unwrap_or_else(|| "*".to_owned()),
            VersionRequirement::new,
            "version requirement",
        )?,
    })
}

fn reject_version_for_non_registry(version: Option<&str>) -> Result<(), clap::Error> {
    if version.is_some() {
        Err(usage(
            "a @VERSION suffix is only valid for a registry dependency",
        ))
    } else {
        Ok(())
    }
}

fn dependency_spec(spec: &str) -> Result<(String, Option<String>), clap::Error> {
    require_nonempty(spec, "dependency specification")?;
    if let Some((name, version)) = spec.rsplit_once('@')
        && !name.is_empty()
    {
        require_nonempty(version, "version requirement")?;
        return Ok((name.to_owned(), Some(version.to_owned())));
    }
    Ok((spec.to_owned(), None))
}

fn emit_kinds(values: Vec<EmitArg>) -> Result<Vec<EmitKind>, clap::Error> {
    let mut result = Vec::new();
    for value in values {
        let kind = match value {
            EmitArg::Prompt => EmitKind::Prompt,
            EmitArg::Ir => EmitKind::BinaryIr,
            EmitArg::Debug => EmitKind::DebugInfo,
        };
        if !result.contains(&kind) {
            result.push(kind);
        }
    }
    if result.is_empty() {
        result.push(EmitKind::Prompt);
    }
    if result == [EmitKind::DebugInfo] {
        return Err(usage("--emit=debug requires --emit=prompt or --emit=ir"));
    }
    Ok(result)
}

fn collect_arguments(
    values: Vec<(String, String)>,
) -> Result<BTreeMap<ArgumentName, String>, clap::Error> {
    let mut result = BTreeMap::new();
    for (name, value) in values {
        let name = id(name, ArgumentName::new, "argument name")?;
        if result.insert(name, value).is_some() {
            return Err(usage("the same --arg name may not be supplied twice"));
        }
    }
    Ok(result)
}

fn parse_argument(value: &str) -> Result<(String, String), String> {
    let (name, value) = value
        .split_once('=')
        .ok_or_else(|| "expected TARGET.NAME=VALUE".to_owned())?;
    let (target, argument) = name
        .split_once('.')
        .ok_or_else(|| "expected TARGET.NAME before '='".to_owned())?;
    if target.is_empty() || argument.is_empty() {
        return Err("TARGET and NAME must both be non-empty".to_owned());
    }
    Ok((name.to_owned(), value.to_owned()))
}

fn parse_config_override(value: &str) -> Result<ConfigOverride, String> {
    let (key, value) = value
        .split_once('=')
        .ok_or_else(|| "expected KEY=VALUE".to_owned())?;
    let key = key.trim();
    if key.is_empty() || key.split('.').any(|part| part.trim().is_empty()) {
        return Err("configuration key and each dotted component must be non-empty".to_owned());
    }
    Ok(ConfigOverride {
        key: key.to_owned(),
        value: value.to_owned(),
    })
}

fn ordered_config_overrides(arguments: &[OsString]) -> Vec<ConfigOverride> {
    let mut overrides = Vec::new();
    let mut index = 1;
    while index < arguments.len() {
        let Some(argument) = arguments[index].to_str() else {
            index += 1;
            continue;
        };
        if argument == "--" {
            break;
        }
        let value = if argument == "--config" {
            index += 1;
            arguments.get(index).and_then(|value| value.to_str())
        } else {
            argument.strip_prefix("--config=")
        };
        if let Some(value) = value {
            // `Cli::try_parse_from` succeeded, so every recognized occurrence already passed this
            // parser. / Clap 已成功完成同一解析器的校验，因此此处不会失败。
            overrides.push(
                parse_config_override(value)
                    .expect("a successfully parsed --config must remain structurally valid"),
            );
        }
        index += 1;
    }
    overrides
}

fn parse_feature(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err("feature names must not be empty".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

fn ids<T, F, E>(values: Vec<String>, constructor: F, label: &str) -> Result<Vec<T>, clap::Error>
where
    F: Fn(String) -> Result<T, E> + Copy,
{
    values
        .into_iter()
        .map(|value| id(value, constructor, label))
        .collect()
}

fn id<T, F, E>(value: String, constructor: F, label: &str) -> Result<T, clap::Error>
where
    F: FnOnce(String) -> Result<T, E>,
{
    constructor(value).map_err(|_| usage(format!("{label} must not be empty")))
}

fn new_package_name(value: String, label: &str) -> Result<NewPackageName, clap::Error> {
    NewPackageName::new(value).map_err(|error| usage(format!("invalid {label}: {error}")))
}

fn require_nonempty(value: &str, label: &str) -> Result<(), clap::Error> {
    if value.is_empty() {
        Err(usage(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn usage(message: impl Into<String>) -> clap::Error {
    cli_error(ErrorKind::InvalidValue, message)
}

fn cli_error(kind: ErrorKind, message: impl Into<String>) -> clap::Error {
    Cli::command().error(kind, message.into())
}

fn exact_json_requested(arguments: &[OsString]) -> bool {
    arguments
        .iter()
        .any(|argument| argument == "--message-format=json")
        || arguments
            .windows(2)
            .any(|pair| pair[0] == "--message-format" && pair[1] == "json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation<const N: usize>(arguments: [&str; N]) -> ParsedInvocation {
        parse_from(arguments)
            .unwrap()
            .into_invocation()
            .expect("argv contains an operation command")
    }

    #[test]
    fn unknown_command_is_usage_and_never_becomes_a_path() {
        let error = parse_from(["xmlsquish", "nearby.xml"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn bare_invocation_is_a_successful_help_outcome() {
        let outcome = parse_from(["xmlsquish"]).unwrap();
        let BootstrapOutcome::BareHelp(help) = outcome else {
            panic!("bare argv must request help")
        };
        assert!(help.as_str().contains("Usage:"));
        assert!(help.as_str().contains("build"));
    }

    #[test]
    fn build_maps_selection_emit_arguments_and_resolution() {
        let parsed = invocation([
            "xmlsquish",
            "--plain",
            "build",
            "--workspace",
            "--exclude",
            "ops",
            "-t",
            "chat",
            "--emit=prompt",
            "--emit=prompt",
            "--emit=debug",
            "--arg",
            "chat.name=Klee",
            "--locked",
            "--offline",
        ]);
        let OperationRequest::Build(request) = parsed.request else {
            panic!("expected build")
        };
        assert_eq!(request.scope, WorkspaceScope::Workspace);
        assert_eq!(request.emit, vec![EmitKind::Prompt, EmitKind::DebugInfo]);
        assert_eq!(request.lock, LockMode::Frozen);
        assert_eq!(
            request.arguments[&ArgumentName::new("chat.name").unwrap()],
            "Klee"
        );
        assert_eq!(
            parsed.execution.excluded_packages,
            vec![PackageName::new("ops").unwrap()]
        );
        assert!(parsed.presentation.plain);
        assert_eq!(parsed.presentation.color, Some(TerminalPolicy::Never));
        assert_eq!(parsed.presentation.progress, Some(TerminalPolicy::Never));
    }

    #[test]
    fn bare_build_defaults_to_prompt_output() {
        let parsed = invocation(["xmlsquish", "build"]);
        let OperationRequest::Build(request) = parsed.request else {
            panic!("expected build")
        };
        assert_eq!(request.emit, vec![EmitKind::Prompt]);
    }

    #[test]
    fn debug_without_primary_output_is_rejected() {
        let error = parse_from(["xmlsquish", "build", "--emit=debug"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn format_diff_implies_check_and_paths_are_not_read() {
        let parsed = invocation(["xmlsquish", "fmt", "--diff", "--path", "does-not-exist.xml"]);
        let OperationRequest::Format(request) = parsed.request else {
            panic!("expected fmt")
        };
        assert!(request.check && request.diff);
        assert!(
            matches!(request.selection, FormatSelection::Sources(ref paths) if paths[0].as_str() == "does-not-exist.xml")
        );
    }

    #[test]
    fn add_sources_conflict_before_any_domain_work() {
        let error = parse_from([
            "xmlsquish",
            "add",
            "common",
            "--path",
            "../common",
            "--git",
            "https://invalid.example/x",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn add_rename_and_registry_spec_map_to_protocol() {
        let parsed = invocation([
            "xmlsquish",
            "add",
            "prompt-common@^2",
            "--rename",
            "common",
            "--registry",
            "corp",
            "--dry-run",
        ]);
        let OperationRequest::Add(request) = parsed.request else {
            panic!("expected add")
        };
        assert_eq!(request.dependency.as_str(), "common");
        assert_eq!(request.rename.unwrap().as_str(), "prompt-common");
        assert!(request.dry_run);
        assert!(
            matches!(request.source, DependencySource::Registry { registry: Some(ref name), ref version } if name.as_str() == "corp" && version.as_str() == "^2")
        );
    }

    #[test]
    fn comma_delimited_features_parse_into_individual_protocol_ids() {
        let parsed = invocation(["xmlsquish", "add", "common", "--features", "alpha,beta"]);
        let OperationRequest::Add(request) = parsed.request else {
            panic!("expected add")
        };
        assert_eq!(
            request
                .features
                .iter()
                .map(FeatureName::as_str)
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn target_argument_requires_both_identity_components() {
        for malformed in [".name=value", "target.=value", ".=value"] {
            let error = parse_from(["xmlsquish", "build", "--arg", malformed]).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{malformed}");
        }
        let parsed = invocation(["xmlsquish", "build", "--arg", "target.name="]);
        let OperationRequest::Build(request) = parsed.request else {
            panic!("expected build")
        };
        assert_eq!(
            request.arguments[&ArgumentName::new("target.name").unwrap()],
            ""
        );
    }

    #[test]
    fn inspect_has_typed_subject_and_query_format() {
        let parsed = invocation([
            "xmlsquish",
            "inspect",
            "artifact",
            "missing.prompt",
            "--format=json",
        ]);
        assert_eq!(parsed.presentation.query_format, Some(QueryFormat::Json));
        assert!(parsed.execution.inspect_subject.is_none());
        assert!(matches!(
            parsed.request,
            OperationRequest::Inspect(InspectRequest {
                view: InspectView::Artifact(ref path),
                ..
            }) if path.as_str() == "missing.prompt"
        ));
    }

    #[test]
    fn raw_query_format_is_restricted_to_artifact_content() {
        let parsed = invocation([
            "xmlsquish",
            "inspect",
            "artifact",
            "target/chat.prompt",
            "--format=raw",
        ]);
        assert_eq!(parsed.presentation.query_format, Some(QueryFormat::Raw));
        let error =
            parse_from(["xmlsquish", "inspect", "ir", "module", "--format=raw"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn inspect_rejects_operation_format_but_preserves_json_promise() {
        let error =
            parse_from(["xmlsquish", "inspect", "ir", "x", "--message-format=json"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.json_requested());
    }

    #[test]
    fn plain_rejects_contradictory_policy() {
        let error = parse_from(["xmlsquish", "build", "--plain", "--color=always"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn quiet_conflicts_with_verbosity_and_verbosity_is_bounded() {
        assert_eq!(
            parse_from(["xmlsquish", "-q", "build", "-v"])
                .unwrap_err()
                .kind(),
            ErrorKind::ArgumentConflict
        );
        assert_eq!(
            parse_from(["xmlsquish", "build", "-vvv"])
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn unknown_emit_in_json_mode_is_machine_renderable_usage() {
        let error =
            parse_from(["xmlsquish", "build", "--emit=wat", "--message-format=json"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.json_requested());
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }
}
