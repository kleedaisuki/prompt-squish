//! `xmlsquish` 项目管理器的组合根。 / Composition root for the `xmlsquish` project manager.
//!
//! 只负责参数解析、项目定位、显式服务组合、事件呈现与退出码。 / Owns only argv parsing,
//! project location, explicit service composition, event presentation, and exit status.

mod interrupt;

use std::{
    ffi::OsString,
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
    FilesystemHost, Limits, NoopObserver, RegistryConfig, ReqwestTransport, SystemGitRunner,
};
use squish_host::{
    CredentialRoute, EnvironmentCredentials, GitExecution, HostConfig, ProductionHost,
    RegistryEndpoint,
};
use squish_kernel::{
    CancellationToken, EventSink, InvocationContext, Kernel, KernelError, SinkError,
};
use squish_manager::{
    ArtifactLocator, InspectSubject, InvocationSettings, ManagerCapability, StorageLayout,
};
use squish_presentation::{
    ColorMode, Environment, HumanRenderer, InspectHumanRenderer, NdjsonRenderer,
    PresentationOptions, ProgressMode, Renderer, SystemClock, SystemTerminal, TerminalProbe,
    Verbosity,
};
use squish_protocol::{
    ActionKeyId, Event, EventPayload, ExitStatus, InvocationId, OperationRequest, OperationResult,
    ProjectPath,
};
use squish_repository::{Discovery, ProjectRepository};
use squish_store::{BlobDigest, Cas};

use crate::interrupt::{InterruptCoordinator, StdProcessTerminator, StderrEmergencyRestore};

/// 内核启动前的稳定机器记录。 / Stable machine record for failures before kernel dispatch.
#[derive(serde::Serialize)]
struct BootstrapRecord<'a> {
    /// 根启动协议版本。 / Root bootstrap schema version.
    schema: &'static str,
    /// 记录类别：诊断或终态。 / Record kind: diagnostic or terminal.
    kind: &'static str,
    /// 失败所在的启动阶段。 / Bootstrap phase containing the failure.
    phase: &'a str,
    /// 稳定诊断代码。 / Stable diagnostic code.
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    /// 人类可读说明。 / Human-readable explanation.
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    /// 终态，仅终态记录携带。 / Terminal status, present only on terminal records.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    /// 对应进程退出码。 / Corresponding process exit code.
    exit_code: u8,
}

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
    // 只在进程边界捕获一次；后续组件不得再次读取全局环境。
    // Capture once at the process boundary; downstream components never reread globals.
    let environment = std::env::vars_os().collect::<Vec<_>>();
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
            } else if error.json_requested() {
                return bootstrap_failure(true, "parse", "CLI_USAGE", &error.to_string(), 2, true);
            } else {
                print_stderr(&error.to_string());
            }
            return u8::try_from(code).unwrap_or(2);
        }
    };

    match execute(parsed, environment) {
        Ok(code) => code,
        Err(error) => {
            print_stderr(&format!("internal error: {error}\n"));
            101
        }
    }
}

/// 完成项目定位、组合与内核调度。 / Locates, composes, and dispatches through the kernel.
fn execute(
    mut invocation: ParsedInvocation,
    environment: Vec<(OsString, OsString)>,
) -> Result<u8, Box<dyn std::error::Error>> {
    // 在任何文件系统工作前安装处理器；定位期到达的信号会在进入内核时继续生效。
    // Install before filesystem work; a signal received during location remains set on kernel entry.
    let bootstrap_json = bootstrap_json_requested(&invocation);
    let terminal = SystemTerminal::stderr();
    let terminal_capabilities = terminal.capabilities();
    let cancellation = CancellationToken::default();
    let coordinator = Arc::new(InterruptCoordinator::new(
        cancellation,
        Arc::new(StderrEmergencyRestore::capture(
            terminal_capabilities.is_terminal
                && terminal_capabilities.supports_ansi
                && terminal_capabilities.supports_dynamic,
        )),
        Arc::new(StdProcessTerminator),
    ));
    if let Err(error) = coordinator.install_with(ctrlc::set_handler) {
        let message = format!("could not install interrupt handler: {error}");
        return Ok(bootstrap_failure(
            bootstrap_json,
            "signal",
            "SIGNAL001",
            &message,
            1,
            true,
        ));
    }
    let explicit = requested_project(&invocation.request);
    let discovery = if explicit == Path::new(".") {
        Discovery::Implicit(std::env::current_dir()?)
    } else {
        Discovery::Explicit(explicit.to_path_buf())
    };
    let repository = match ProjectRepository::discover(discovery) {
        Ok(repository) => repository,
        Err(error) => {
            let message = format!("could not discover project: {error}");
            return Ok(bootstrap_failure(
                bootstrap_json,
                "discover",
                "PROJECT001",
                &message,
                1,
                true,
            ));
        }
    };
    let root = repository.root().to_path_buf();
    set_project(&mut invocation.request, &root)?;
    let config = match load_config(&root, &invocation) {
        Ok(config) => config,
        Err(error) => {
            let message = format!("could not load configuration: {error}");
            return Ok(bootstrap_failure(
                bootstrap_json,
                "config",
                "CONFIG001",
                &message,
                1,
                true,
            ));
        }
    };
    let operation_json = !matches!(invocation.request, OperationRequest::Inspect(_))
        && invocation.presentation.message_format.map_or(
            config.term.message_format == squish_config::MessageFormat::Json,
            |value| value == MessageFormat::Json,
        );
    let (host, storage) = match compose_host(root, &config, environment) {
        Ok(composed) => composed,
        Err(error) => {
            let message = format!("could not initialize project services: {error}");
            return Ok(bootstrap_failure(
                operation_json,
                "host",
                "HOST001",
                &message,
                1,
                true,
            ));
        }
    };
    let cas = match Cas::open(storage.cas_root()) {
        Ok(cas) => cas,
        Err(error) => {
            let message = format!("could not open output store: {error}");
            return Ok(bootstrap_failure(
                operation_json,
                "host",
                "HOST001",
                &message,
                1,
                true,
            ));
        }
    };

    let query = matches!(invocation.request, OperationRequest::Inspect(_));
    let sink = Arc::new(rendering_sink(&invocation, &config, query, terminal));
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

    let context = InvocationContext::new(
        invocation_id(),
        coordinator.cancellation_token(),
        sink.clone(),
    );
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
    if query
        && outcome.summary.status == ExitStatus::Success
        && let Err(error) =
            render_query_result(&outcome.result, invocation.presentation.query_format)
    {
        return Ok(output_failure(
            Box::new(error),
            outcome.summary.status.code(),
        ));
    }
    if !query
        && !operation_json
        && let Err(error) = render_format_diffs(&outcome.result, &cas)
    {
        return Ok(output_failure(
            Box::new(error),
            outcome.summary.status.code(),
        ));
    }
    Ok(outcome.summary.status.code())
}

/// 用互不重叠的持久责任组合生产服务。 / Composes production services with non-overlapping persistence responsibilities.
///
/// CAS 与内容键动作索引在多项目间共享，但 build catalog 记录的是项目相对
/// locator，因而必须按规范项目身份分区。 / CAS and the content-keyed action index are
/// shared across projects, while the build catalog contains project-relative locators and is
/// therefore partitioned by canonical project identity.
fn compose_host(
    root: std::path::PathBuf,
    config: &Config,
    environment: Vec<(OsString, OsString)>,
) -> Result<(ProductionHost, StorageLayout), Box<dyn std::error::Error>> {
    let state = &config.manager.storage_root;
    let catalog = state
        .join("catalog")
        .join("projects")
        .join(project_namespace(&root));
    let storage = StorageLayout::new(
        state.join("cas"),
        state.join("actions.sqlite3"),
        root.join("target/xmlsquish"),
        catalog,
    )?;
    let filesystem = Arc::new(FilesystemHost::new(&root)?);
    let registries = config
        .registries
        .iter()
        .map(|(name, registry)| RegistryEndpoint {
            name: name.clone(),
            config: RegistryConfig {
                id: registry.id.as_str().to_owned(),
                index: registry.index.as_str().to_owned(),
                auth_scope: registry.auth_scope.as_str().to_owned(),
            },
        })
        .collect::<Vec<_>>();
    let credential_routes = registries
        .iter()
        .map(|endpoint| {
            Ok(CredentialRoute {
                registry_id: endpoint.config.id.clone(),
                auth_scope: endpoint.config.auth_scope.clone(),
                primary_origin: registry_primary_origin(&endpoint.config.index)?,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let credentials = EnvironmentCredentials::from_snapshot(credential_routes, environment)?;
    let host = ProductionHost::open(HostConfig {
        project_root: root,
        source_cache_root: config.source.cache_root.clone(),
        storage: storage.clone(),
        registries,
        credentials: Arc::new(credentials),
        http: Arc::new(ReqwestTransport::new()?),
        git: GitExecution::Runner(Arc::new(SystemGitRunner::default())),
        limits: Limits::default(),
        observer: Arc::new(NoopObserver),
        filesystem,
    })?;
    Ok((host, storage))
}

/// 从已验证的 sparse index URL 取得规范 ASCII origin。 / Extracts the canonical ASCII
/// origin from an already validated sparse-index URL.
fn registry_primary_origin(index: &str) -> Result<String, Box<dyn std::error::Error>> {
    let url = url::Url::parse(
        index
            .strip_prefix("sparse+")
            .ok_or("missing sparse+ prefix")?,
    )?;
    Ok(url.origin().ascii_serialization())
}

/// 从规范项目路径派生稳定且不泄露路径的 catalog 命名空间。 /
/// Derives a stable, path-opaque catalog namespace from the canonical project path.
///
/// 领域分隔防止未来将普通内容摘要误当作项目身份。路径已由项目发现层规范化；
/// Unix 原始字节与 Windows UTF-16LE 令持久身份不依赖有损的 Unicode 转换。 /
/// Domain separation prevents a regular content digest from being confused with a project
/// identity. Project discovery has already canonicalized the path; Unix raw bytes and Windows
/// UTF-16LE make the persistent identity independent of lossy Unicode conversion.
fn project_namespace(root: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"xmlsquish-project-catalog-v1\0");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(root.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in root.as_os_str().encode_wide() {
            hasher.update(&unit.to_le_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// 构造符合 stdout/stderr 边界的呈现目的地。 / Builds a renderer honoring the stdout/stderr boundary.
fn rendering_sink(
    invocation: &ParsedInvocation,
    config: &Config,
    query: bool,
    terminal: SystemTerminal,
) -> RenderingSink {
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
        Box::new(HumanRenderer::with_clock_and_terminal(
            stderr,
            terminal,
            Environment::capture(),
            options,
            SystemClock,
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
fn render_query_result(result: &OperationResult, format: Option<QueryFormat>) -> io::Result<()> {
    let OperationResult::Inspect(result) = result else {
        return Ok(());
    };
    let mut stdout = io::stdout().lock();
    match format.unwrap_or(QueryFormat::Human) {
        QueryFormat::Json => {
            serde_json::to_writer(&mut stdout, result).map_err(json_output_error)?
        }
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

fn json_output_error(error: serde_json::Error) -> io::Error {
    match error.io_error_kind() {
        Some(kind) => io::Error::new(kind, error),
        None => io::Error::other(error.to_string()),
    }
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

/// 判定命令行或环境是否已在访问项目前承诺 JSON。 /
/// Determines whether argv or environment promised JSON before project access.
fn bootstrap_json_requested(invocation: &ParsedInvocation) -> bool {
    invocation.presentation.message_format == Some(MessageFormat::Json)
        || std::env::var("XMLSQUISH_MESSAGE_FORMAT").is_ok_and(|value| value == "json")
        || invocation.config_overrides.iter().any(|value| {
            value.key() == "term.message-format"
                && matches!(value.value().trim(), "json" | "\"json\"" | "'json'")
        })
}

/// 输出一个启动诊断，并可选跟随一个无内核 ID 的终态记录。 /
/// Emits one bootstrap diagnostic and optionally a terminal record carrying no kernel IDs.
fn bootstrap_failure(
    json: bool,
    phase: &str,
    code: &str,
    message: &str,
    exit_code: u8,
    terminal: bool,
) -> u8 {
    if !json {
        print_stderr(&format!("error[{code}] {message}\n"));
        return exit_code;
    }
    let mut stdout = io::stdout().lock();
    let diagnostic = BootstrapRecord {
        schema: "xmlsquish-bootstrap-v1",
        kind: "diagnostic",
        phase,
        code: Some(code),
        message: Some(message),
        status: None,
        exit_code,
    };
    let write = serde_json::to_writer(&mut stdout, &diagnostic)
        .and_then(|()| stdout.write_all(b"\n").map_err(serde_json::Error::io));
    if write.is_ok() && terminal {
        let finished = BootstrapRecord {
            schema: "xmlsquish-bootstrap-v1",
            kind: "finished",
            phase,
            code: None,
            message: None,
            status: Some("failed"),
            exit_code,
        };
        let _ = serde_json::to_writer(&mut stdout, &finished)
            .and_then(|()| stdout.write_all(b"\n").map_err(serde_json::Error::io));
    }
    exit_code
}

/// 将人类 fmt 差异产物按结果顺序流式写入 stdout。 / Streams human fmt diff artifacts to stdout in result order.
fn render_format_diffs(result: &OperationResult, cas: &Cas) -> io::Result<()> {
    let OperationResult::Format(result) = result else {
        return Ok(());
    };
    if result.diffs.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout().lock();
    for artifact in &result.diffs {
        if *artifact.digest.algorithm() != squish_protocol::DigestAlgorithm::Blake3 {
            return Err(io::Error::other("format diff uses an unsupported digest"));
        }
        let digest: BlobDigest = artifact.digest.hex().parse().map_err(io::Error::other)?;
        let bytes = cas
            .get(digest)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("format diff is absent from the content store"))?;
        stdout.write_all(&bytes)?;
        if !bytes.ends_with(b"\n") {
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// 丢弃已关闭管道；其他输出错误保持可观测且返回 1。 / Ignores a closed pipe; other output failures remain observable and return 1.
fn output_failure(error: Box<dyn std::error::Error>, broken_pipe_status: u8) -> u8 {
    if error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
    {
        return broken_pipe_status;
    }
    print_stderr(&format!(
        "error[OUTPUT001] could not write output: {error}\n"
    ));
    1
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
