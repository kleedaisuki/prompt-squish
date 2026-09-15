//! `xmlsquish` 项目管理器的组合根。 / Composition root for the `xmlsquish` project manager.
//!
//! 只负责参数解析、项目定位、显式服务组合、事件呈现与退出码。 / Owns only argv parsing,
//! project location, explicit service composition, event presentation, and exit status.

use std::{
    io::{self, Write},
    path::Path,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use squish_cli::{
    BootstrapOutcome, InspectSubject as CliInspectSubject, MessageFormat, ParsedInvocation,
    QueryFormat, TerminalPolicy, parse_from_with_version,
};
use squish_config::{Config, ConfigHome, ConfigLoader};
use squish_fetch::{
    FilesystemHost, Limits, NoCredentials, NoopObserver, RegistryConfig, ReqwestTransport,
    SystemGitRunner,
};
use squish_host::{GitExecution, HostConfig, ProductionHost, RegistryEndpoint};
use squish_kernel::{
    CancellationToken, EventSink, InvocationContext, Kernel, KernelError, SinkError,
};
use squish_manager::{
    ArtifactLocator, InspectSubject, InvocationSettings, ManagerCapability, StorageLayout,
};
use squish_presentation::{
    ColorMode, Environment, HumanRenderer, InspectHumanRenderer, NdjsonRenderer,
    PresentationOptions, ProgressMode, Renderer, TerminalCapabilities, Verbosity,
};
use squish_protocol::{
    ActionKeyId, Event, EventPayload, ExitStatus, InvocationId, OperationRequest, OperationResult,
    ProjectPath,
};
use squish_repository::{Discovery, ProjectRepository};

/// 将内核事件串行送入唯一呈现器。 / Serializes kernel events into the sole renderer.
struct RenderingSink {
    renderer: Mutex<Box<dyn Renderer + Send>>,
    omit_inspect_result: bool,
}

impl RenderingSink {
    /// 完成临时 UI 并刷新目标流。 / Finishes transient UI and flushes the target stream.
    fn finish(&self) -> io::Result<()> {
        self.renderer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finish()
    }
}

impl EventSink for RenderingSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        // inspect 的领域结果是 stdout 查询文档，而非 stderr 状态文本。
        // An inspect domain result is a stdout query document, not stderr status prose.
        if self.omit_inspect_result
            && matches!(
                event.payload,
                EventPayload::OperationCompleted {
                    result: OperationResult::Inspect(_),
                    ..
                }
            )
        {
            return Ok(());
        }
        self.renderer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .render(&event)
            .map_err(|error| SinkError::new(error.to_string()))
    }
}

/// 运行一次进程调用。 / Runs one process invocation.
fn run() -> u8 {
    let parsed = match parse_from_with_version(std::env::args_os(), env!("CARGO_PKG_VERSION")) {
        Ok(BootstrapOutcome::BareHelp(help)) => {
            print_stdout(help.as_str());
            return 0;
        }
        Ok(BootstrapOutcome::Invocation(invocation)) => *invocation,
        Err(error) => {
            let code = error.exit_code();
            if code == 0 {
                print_stdout(&error.to_string());
            } else {
                print_stderr(&error.to_string());
            }
            return u8::try_from(code).unwrap_or(2);
        }
    };

    match execute(parsed) {
        Ok(code) => code,
        Err(error) => {
            print_stderr(&format!("internal error: {error}\n"));
            101
        }
    }
}

/// 完成项目定位、组合与内核调度。 / Locates, composes, and dispatches through the kernel.
fn execute(mut invocation: ParsedInvocation) -> Result<u8, Box<dyn std::error::Error>> {
    // 在任何文件系统工作前安装处理器；定位期到达的信号会在进入内核时继续生效。
    // Install before filesystem work; a signal received during location remains set on kernel entry.
    let cancellation = CancellationToken::default();
    let signal_token = cancellation.clone();
    ctrlc::set_handler(move || signal_token.cancel())?;
    let explicit = requested_project(&invocation.request);
    let discovery = if explicit == Path::new(".") {
        Discovery::Implicit(std::env::current_dir()?)
    } else {
        Discovery::Explicit(explicit.to_path_buf())
    };
    let repository = match ProjectRepository::discover(discovery) {
        Ok(repository) => repository,
        Err(error) => {
            print_stderr(&format!(
                "error[PROJECT001] could not discover project: {error}\n"
            ));
            return Ok(1);
        }
    };
    let root = repository.root().to_path_buf();
    set_project(&mut invocation.request, &root)?;
    let config = match load_config(&root, &invocation) {
        Ok(config) => config,
        Err(error) => {
            print_stderr(&format!(
                "error[CONFIG001] could not load configuration: {error}\n"
            ));
            return Ok(1);
        }
    };
    let host = match compose_host(root, &config) {
        Ok(host) => host,
        Err(error) => {
            print_stderr(&format!(
                "error[HOST001] could not initialize project services: {error}\n"
            ));
            return Ok(1);
        }
    };

    let query = matches!(invocation.request, OperationRequest::Inspect(_));
    let sink = Arc::new(rendering_sink(&invocation, &config, query));
    let settings = InvocationSettings {
        excluded_packages: invocation.execution.excluded_packages,
        jobs: invocation
            .execution
            .jobs
            .map_or(config.build.jobs, |value| value as usize),
        keep_going: if keep_going_was_explicit() {
            invocation.execution.keep_going
        } else {
            config.build.keep_going
        },
        inspect_subject: manager_inspect_subject(invocation.execution.inspect_subject)?,
    };
    let manager = ManagerCapability::new(host, settings);
    let capabilities: [&dyn squish_kernel::Capability; 1] = [&manager];
    let kernel = Kernel::new(&capabilities)?;

    let context = InvocationContext::new(invocation_id(), cancellation, sink.clone());
    let outcome = match kernel.dispatch(&invocation.request, &context) {
        Ok(outcome) => outcome,
        Err(KernelError::Emit(error)) => {
            print_stderr(&format!(
                "error[OUTPUT001] could not write output: {error}\n"
            ));
            return Ok(1);
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = sink.finish() {
        print_stderr(&format!(
            "error[OUTPUT001] could not flush output: {error}\n"
        ));
        return Ok(1);
    }
    if query && outcome.summary.status == ExitStatus::Success {
        render_query_result(&outcome.result, invocation.presentation.query_format)?;
    }
    Ok(outcome.summary.status.code())
}

/// 用项目内互不重叠的持久责任组合生产服务。 / Composes production services with non-overlapping project-local responsibilities.
fn compose_host(
    root: std::path::PathBuf,
    config: &Config,
) -> Result<ProductionHost, Box<dyn std::error::Error>> {
    let state = &config.manager.storage_root;
    let storage = StorageLayout::new(
        state.join("cas"),
        state.join("actions.sqlite3"),
        root.join("target/xmlsquish"),
        state.join("catalog"),
    )?;
    let filesystem = Arc::new(FilesystemHost::new(&root)?);
    Ok(ProductionHost::open(HostConfig {
        project_root: root,
        source_cache_root: config.source.cache_root.clone(),
        storage,
        registries: config
            .registries
            .iter()
            .map(|(name, registry)| RegistryEndpoint {
                name: name.clone(),
                config: RegistryConfig {
                    id: registry.id.as_str().to_owned(),
                    index: registry.index.as_str().to_owned(),
                },
            })
            .collect(),
        credentials: Arc::new(NoCredentials),
        http: Arc::new(ReqwestTransport::new()?),
        git: GitExecution::Runner(Arc::new(SystemGitRunner::default())),
        limits: Limits::default(),
        observer: Arc::new(NoopObserver),
        filesystem,
    })?)
}

/// 构造符合 stdout/stderr 边界的呈现目的地。 / Builds a renderer honoring the stdout/stderr boundary.
fn rendering_sink(invocation: &ParsedInvocation, config: &Config, query: bool) -> RenderingSink {
    let message_format = if query {
        MessageFormat::Human
    } else {
        invocation.presentation.message_format.unwrap_or({
            match config.term.message_format {
                squish_config::MessageFormat::Human => MessageFormat::Human,
                squish_config::MessageFormat::Short => MessageFormat::Short,
                squish_config::MessageFormat::Json => MessageFormat::Json,
            }
        })
    };
    let renderer: Box<dyn Renderer + Send> = if message_format == MessageFormat::Json {
        Box::new(NdjsonRenderer::new(io::stdout()))
    } else {
        let stderr = io::stderr();
        let options = PresentationOptions {
            color: terminal_mode(
                invocation.presentation.color,
                config.term.color,
                invocation.presentation.plain,
            ),
            progress: progress_mode(
                invocation.presentation.progress,
                config.term.progress,
                invocation.presentation.plain || invocation.presentation.quiet,
            ),
            verbosity: if invocation.presentation.quiet || query {
                Verbosity::Quiet
            } else if message_format == MessageFormat::Short {
                Verbosity::Short
            } else if invocation.presentation.verbosity > 0 {
                Verbosity::Verbose
            } else {
                match config.term.verbosity {
                    squish_config::Verbosity::Quiet => Verbosity::Quiet,
                    squish_config::Verbosity::Normal => Verbosity::Normal,
                    squish_config::Verbosity::Verbose | squish_config::Verbosity::Trace => {
                        Verbosity::Verbose
                    }
                }
            },
            ..PresentationOptions::default()
        };
        Box::new(HumanRenderer::new(
            stderr,
            TerminalCapabilities::detect(&io::stderr()),
            Environment::capture(),
            options,
        ))
    };
    RenderingSink {
        renderer: Mutex::new(renderer),
        omit_inspect_result: query,
    }
}

/// 将 CLI 查询对象转换为管理器已类型化的补充设置。 / Converts the CLI query subject to the manager's typed supplement.
fn manager_inspect_subject(
    subject: Option<CliInspectSubject>,
) -> Result<Option<InspectSubject>, Box<dyn std::error::Error>> {
    Ok(match subject {
        Some(CliInspectSubject::Cache(value)) => {
            Some(InspectSubject::CacheKey(ActionKeyId::new(value)?))
        }
        Some(CliInspectSubject::Artifact(path)) => Some(InspectSubject::ArtifactPath(
            ArtifactLocator::new(path.as_str())?,
        )),
        Some(
            CliInspectSubject::Ir(_) | CliInspectSubject::Link(_) | CliInspectSubject::Source(_),
        )
        | None => None,
    })
}

/// 保留类型化请求，仅将项目选择归一为已发现根。 / Preserves typed intent while normalizing only project selection.
fn set_project(
    request: &mut OperationRequest,
    root: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let project = ProjectPath::new(root.to_string_lossy().into_owned())?;
    match request {
        OperationRequest::Build(value) => value.project = project,
        OperationRequest::Format(value) => value.project = project,
        OperationRequest::Add(value) => value.project = project,
        OperationRequest::Remove(value) => value.project = project,
        OperationRequest::Inspect(value) => value.project = project,
    }
    Ok(())
}

/// 返回解析器保留的项目选择。 / Returns the parser-preserved project selection.
fn requested_project(request: &OperationRequest) -> &Path {
    Path::new(match request {
        OperationRequest::Build(value) => value.project.as_str(),
        OperationRequest::Format(value) => value.project.as_str(),
        OperationRequest::Add(value) => value.project.as_str(),
        OperationRequest::Remove(value) => value.project.as_str(),
        OperationRequest::Inspect(value) => value.project.as_str(),
    })
}

/// 将纯查询负载写成单个 stdout 文档。 / Writes a pure-query payload as one stdout document.
fn render_query_result(
    result: &OperationResult,
    format: Option<QueryFormat>,
) -> Result<(), Box<dyn std::error::Error>> {
    let OperationResult::Inspect(result) = result else {
        return Ok(());
    };
    let mut stdout = io::stdout().lock();
    match format.unwrap_or(QueryFormat::Human) {
        QueryFormat::Json => serde_json::to_writer(&mut stdout, result)?,
        QueryFormat::Human => {
            let mut renderer = InspectHumanRenderer::new(stdout);
            renderer.render(result)?;
            renderer.finish()?;
            return Ok(());
        }
    }
    stdout.write_all(b"\n")?;
    Ok(())
}

fn terminal_mode(
    policy: Option<TerminalPolicy>,
    configured: squish_config::ColorPolicy,
    plain: bool,
) -> ColorMode {
    if plain {
        return ColorMode::Never;
    }
    match policy.unwrap_or(match configured {
        squish_config::ColorPolicy::Auto => TerminalPolicy::Auto,
        squish_config::ColorPolicy::Always => TerminalPolicy::Always,
        squish_config::ColorPolicy::Never => TerminalPolicy::Never,
    }) {
        TerminalPolicy::Auto => ColorMode::Auto,
        TerminalPolicy::Always => ColorMode::Always,
        TerminalPolicy::Never => ColorMode::Never,
    }
}

fn progress_mode(
    policy: Option<TerminalPolicy>,
    configured: squish_config::ProgressPolicy,
    disabled: bool,
) -> ProgressMode {
    if disabled {
        return ProgressMode::Never;
    }
    match policy.unwrap_or(match configured {
        squish_config::ProgressPolicy::Auto => TerminalPolicy::Auto,
        squish_config::ProgressPolicy::Always => TerminalPolicy::Always,
        squish_config::ProgressPolicy::Never => TerminalPolicy::Never,
    }) {
        TerminalPolicy::Auto => ProgressMode::Auto,
        TerminalPolicy::Always => ProgressMode::Always,
        TerminalPolicy::Never => ProgressMode::Never,
    }
}

/// 按 defaults < user < workspace < environment < CLI 加载确定性配置。 /
/// Loads deterministic configuration in defaults < user < workspace < environment < CLI order.
fn load_config(
    root: &Path,
    invocation: &ParsedInvocation,
) -> Result<Config, Box<dyn std::error::Error>> {
    let mut overrides = environment_overrides();
    overrides.extend(
        invocation
            .config_overrides
            .iter()
            .cloned()
            .map(squish_cli::ConfigOverride::into_assignment),
    );
    if let Some(value) = invocation.presentation.color {
        overrides.push(format!("term.color = \"{:?}\"", value).to_ascii_lowercase());
    }
    if let Some(value) = invocation.presentation.progress {
        overrides.push(format!("term.progress = \"{:?}\"", value).to_ascii_lowercase());
    }
    if let Some(value) = invocation.presentation.message_format {
        overrides.push(format!("term.message-format = \"{:?}\"", value).to_ascii_lowercase());
    }
    if invocation.presentation.plain {
        overrides.extend([
            "term.color = \"never\"".into(),
            "term.progress = \"never\"".into(),
        ]);
    }
    if invocation.presentation.quiet {
        overrides.push("term.verbosity = \"quiet\"".into());
    } else if invocation.presentation.verbosity == 1 {
        overrides.push("term.verbosity = \"verbose\"".into());
    } else if invocation.presentation.verbosity >= 2 {
        overrides.push("term.verbosity = \"trace\"".into());
    }
    if let Some(jobs) = invocation.execution.jobs {
        overrides.push(format!("build.jobs = {jobs}"));
    }
    if keep_going_was_explicit() {
        overrides.push(format!(
            "build.keep-going = {}",
            invocation.execution.keep_going
        ));
    }
    Ok(ConfigLoader::new(ConfigHome::new(config_home()?))
        .workspace_root(root)
        .cli_base(std::env::current_dir()?)
        .cli_overrides(overrides)
        .load()?
        .config)
}

/// 将支持的进程环境值转为显式的强类型覆盖。 / Converts supported process environment values into explicit typed overrides.
fn environment_overrides() -> Vec<String> {
    const VALUES: &[(&str, &str, bool)] = &[
        ("XMLSQUISH_SOURCE_CACHE_ROOT", "source.cache-root", true),
        ("XMLSQUISH_STORAGE_ROOT", "manager.storage-root", true),
        ("XMLSQUISH_JOBS", "build.jobs", false),
        ("XMLSQUISH_KEEP_GOING", "build.keep-going", false),
        ("XMLSQUISH_COLOR", "term.color", true),
        ("XMLSQUISH_PROGRESS", "term.progress", true),
        ("XMLSQUISH_MESSAGE_FORMAT", "term.message-format", true),
        ("XMLSQUISH_VERBOSITY", "term.verbosity", true),
    ];
    VALUES
        .iter()
        .filter_map(|(variable, key, quoted)| {
            let value = std::env::var(variable).ok()?;
            let value = if *quoted {
                serde_json::to_string(&value).expect("a process string is JSON encodable")
            } else {
                value
            };
            Some(format!("{key} = {value}"))
        })
        .collect()
}

/// 显式解析跨平台用户配置根，不在配置领域内隐式读环境。 /
/// Explicitly resolves the cross-platform user config root, keeping environment reads outside the config domain.
fn config_home() -> io::Result<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("XMLSQUISH_HOME") {
        return absolute(path.into());
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("APPDATA") {
        return absolute(std::path::PathBuf::from(path).join("xmlsquish"));
    }
    #[cfg(not(windows))]
    {
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            return absolute(std::path::PathBuf::from(path).join("xmlsquish"));
        }
        if let Some(path) = std::env::var_os("HOME") {
            return absolute(std::path::PathBuf::from(path).join(".config/xmlsquish"));
        }
    }
    absolute(std::path::PathBuf::from(".xmlsquish"))
}

fn absolute(path: std::path::PathBuf) -> io::Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn keep_going_was_explicit() -> bool {
    std::env::args_os().any(|argument| argument == "--keep-going" || argument == "--no-keep-going")
}

fn invocation_id() -> InvocationId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    InvocationId::new(format!("cli-{}-{nanos}", std::process::id()))
        .expect("generated invocation IDs are non-empty")
}

fn print_stdout(message: &str) {
    let _ = io::stdout().write_all(message.as_bytes());
}

fn print_stderr(message: &str) {
    let _ = io::stderr().write_all(message.as_bytes());
}

/// 将稳定退出码交给操作系统。 / Returns the stable exit code to the operating system.
fn main() -> ExitCode {
    ExitCode::from(run())
}
