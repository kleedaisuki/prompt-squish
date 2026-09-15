//! prompt-squish 的终端与机器可读呈现领域。 / Terminal and machine-readable presentation domain for prompt-squish.
//!
//! 这里消费 [`squish_protocol::Event`]，但不执行任务，也不依赖任何构建领域。
//! 人类终端和 NDJSON 是同一事件流的两种投影；调用者负责事件排序和调度。
//! This crate consumes [`squish_protocol::Event`] but neither executes jobs nor
//! depends on a build domain. Human terminals and NDJSON are projections of the
//! same event stream; event ordering and scheduling belong to the caller.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    env,
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

use squish_protocol::{
    ActionId, ActionKind, ArtifactKind, CacheKind, Digest, DigestAlgorithm, Event, EventPayload,
    JobId, Phase, Severity,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// 动态进度出现前的默认延迟。 / Default delay before dynamic progress appears.
pub const DEFAULT_PROGRESS_DELAY: Duration = Duration::from_millis(500);

/// 动态进度两次重绘之间的默认间隔。 / Default interval between dynamic progress redraws.
pub const DEFAULT_PROGRESS_REFRESH: Duration = Duration::from_millis(100);

const CLEAR_LINE: &str = "\r\x1b[2K";

/// 事件呈现器的最小协议。 / Minimal protocol implemented by event renderers.
pub trait Renderer {
    /// 消费一个已经排序的协议事件。 / Consumes one already ordered protocol event.
    fn render(&mut self, event: &Event) -> io::Result<()>;

    /// 刷新延迟呈现状态；事件暂时静默时由宿主周期调用。
    /// Advances delayed presentation state; the host calls this periodically while events are quiet.
    fn tick(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// 清理临时 UI 并刷新输出。 / Clears transient UI and flushes output.
    fn finish(&mut self) -> io::Result<()>;
}

/// `--color` 的用户策略。 / User policy for `--color`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ColorMode {
    /// 根据终端与环境自动判断。 / Detect from terminal and environment.
    #[default]
    Auto,
    /// 始终输出 ANSI 色彩。 / Always emit ANSI colors.
    Always,
    /// 从不输出 ANSI 色彩。 / Never emit ANSI colors.
    Never,
}

/// `--progress` 的用户策略。 / User policy for `--progress`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProgressMode {
    /// 根据终端与环境自动判断。 / Detect from terminal and environment.
    #[default]
    Auto,
    /// 在能力允许时显示进度，并跳过首次延迟。 / Show progress when capable and skip the initial delay.
    Always,
    /// 完全隐藏可替换进度。 / Completely hide replaceable progress.
    Never,
}

/// 人类输出的详细程度。 / Verbosity of human-facing output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Verbosity {
    /// 只显示警告、错误和最终失败。 / Show only warnings, errors, and final failures.
    Quiet,
    /// 面向日常交互的默认输出。 / Default output for ordinary interaction.
    #[default]
    Normal,
    /// 包括 trace 诊断和生命周期细节。 / Include trace diagnostics and lifecycle detail.
    Verbose,
}

/// 输出流的终端能力快照。 / Snapshot of terminal capabilities for an output stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCapabilities {
    /// 输出是否连接交互终端。 / Whether output is attached to an interactive terminal.
    pub is_terminal: bool,
    /// 终端是否支持 ANSI 样式。 / Whether the terminal supports ANSI styling.
    pub supports_ansi: bool,
    /// 终端是否适合原地动态重绘。 / Whether in-place dynamic redraw is appropriate.
    pub supports_dynamic: bool,
}

impl TerminalCapabilities {
    /// 为任意标准写入流探测保守能力。 / Conservatively detects capabilities for a standard writer.
    pub fn detect<W: IsTerminal>(writer: &W) -> Self {
        let terminal = writer.is_terminal();
        Self {
            is_terminal: terminal,
            supports_ansi: terminal,
            supports_dynamic: terminal,
        }
    }

    /// 返回适用于文件、管道和测试快照的能力。 / Returns capabilities for files, pipes, and snapshots.
    pub const fn plain() -> Self {
        Self {
            is_terminal: false,
            supports_ansi: false,
            supports_dynamic: false,
        }
    }
}

/// 可在每次刷新时重新探测的终端端口。 / Terminal port that may be re-probed on every refresh.
pub trait TerminalProbe {
    /// 返回当前流能力。 / Returns current stream capabilities.
    fn capabilities(&self) -> TerminalCapabilities;

    /// 返回当前显示列数；未知时返回 `None`。 / Returns current display columns, or `None` when unknown.
    fn width(&self) -> Option<usize>;
}

/// 固定终端探测器，适合普通流和快照测试。 / Fixed terminal probe for ordinary streams and snapshot tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedTerminal {
    capabilities: TerminalCapabilities,
    width: Option<usize>,
}

impl FixedTerminal {
    /// 从能力创建；未知宽度会禁用动态重绘。 / Creates from capabilities; unknown width disables dynamic repainting.
    pub const fn new(capabilities: TerminalCapabilities) -> Self {
        Self {
            capabilities,
            width: None,
        }
    }

    /// 覆盖显示列数。 / Overrides display columns.
    pub const fn with_width(mut self, width: Option<usize>) -> Self {
        self.width = width;
        self
    }
}

impl TerminalProbe for FixedTerminal {
    fn capabilities(&self) -> TerminalCapabilities {
        self.capabilities
    }

    fn width(&self) -> Option<usize> {
        self.width
    }
}

/// 与呈现有关的环境变量快照。 / Snapshot of presentation-related environment variables.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Environment {
    /// `NO_COLOR` 是否设置为非空值。 / Whether `NO_COLOR` has a non-empty value.
    pub no_color: bool,
    /// `TERM` 的值。 / Value of `TERM`.
    pub term: Option<String>,
    /// 是否处于持续集成环境。 / Whether execution is in continuous integration.
    pub ci: bool,
}

impl Environment {
    /// 从当前进程捕获一次稳定快照。 / Captures one stable snapshot from the current process.
    pub fn capture() -> Self {
        Self {
            no_color: env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
            term: env::var("TERM").ok(),
            ci: env::var_os("CI").is_some(),
        }
    }
}

/// 人类呈现器的完整策略。 / Complete policy for the human renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationOptions {
    /// 色彩策略。 / Color policy.
    pub color: ColorMode,
    /// 进度策略。 / Progress policy.
    pub progress: ProgressMode,
    /// 详细程度。 / Verbosity.
    pub verbosity: Verbosity,
    /// 进度首次出现前的延迟。 / Delay before progress first appears.
    pub progress_delay: Duration,
    /// 动态进度的最小刷新间隔。 / Minimum dynamic progress refresh interval.
    pub progress_refresh: Duration,
}

impl Default for PresentationOptions {
    fn default() -> Self {
        Self {
            color: ColorMode::Auto,
            progress: ProgressMode::Auto,
            verbosity: Verbosity::Normal,
            progress_delay: DEFAULT_PROGRESS_DELAY,
            progress_refresh: DEFAULT_PROGRESS_REFRESH,
        }
    }
}

/// 为测试和宿主提供可替换的单调时钟。 / Replaceable monotonic clock for tests and hosts.
pub trait Clock {
    /// 时钟上的时刻类型。 / Instant type on this clock.
    type Instant: Copy + Ord;

    /// 返回当前时刻。 / Returns the current instant.
    fn now(&self) -> Self::Instant;

    /// 返回从 `earlier` 到 `later` 的饱和时长。 / Returns saturating duration from `earlier` to `later`.
    fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration;
}

/// 使用 [`Instant`] 的生产单调时钟。 / Production monotonic clock backed by [`Instant`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    type Instant = Instant;

    fn now(&self) -> Self::Instant {
        Instant::now()
    }

    fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration {
        later.saturating_duration_since(earlier)
    }
}

#[derive(Clone, Debug)]
struct ProgressState<I> {
    started: I,
    last_draw: Option<I>,
    message: String,
    completed: u64,
    total: u64,
    visible: bool,
    rendered_width: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionState {
    Queued,
    Running,
    Complete,
}

#[derive(Clone, Debug)]
struct ActionView {
    kind: ActionKind,
    state: ActionState,
}

#[derive(Clone, Debug, Default)]
struct JobView {
    declared_actions: u64,
    actions: BTreeMap<ActionId, ActionView>,
}

/// 将协议事件稳定地编码成逐行 NDJSON。 / Encodes protocol events as stable line-delimited NDJSON.
///
/// # 示例 / Example
///
/// ```
/// use squish_presentation::{NdjsonRenderer, Renderer};
/// use squish_protocol::{Event, EventPayload, InvocationId, JobId};
///
/// let event = Event::new(
///     InvocationId::new("example").unwrap(),
///     0,
///     EventPayload::PlanReady { job: JobId::new("job").unwrap(), actions: 1 },
/// );
/// let mut renderer = NdjsonRenderer::new(Vec::new());
/// renderer.render(&event).unwrap();
/// renderer.finish().unwrap();
/// assert!(String::from_utf8(renderer.into_inner()).unwrap().ends_with('\n'));
/// ```
pub struct NdjsonRenderer<W> {
    writer: W,
}

impl<W: Write> NdjsonRenderer<W> {
    /// 创建不缓冲事件的 NDJSON 呈现器。 / Creates an NDJSON renderer that does not buffer events.
    pub const fn new(writer: W) -> Self {
        Self { writer }
    }

    /// 取回底层写入器。 / Returns the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> Renderer for NdjsonRenderer<W> {
    fn render(&mut self, event: &Event) -> io::Result<()> {
        serde_json::to_writer(&mut self.writer, event).map_err(io::Error::other)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// 面向人类的事件呈现器，不拥有或执行任何任务。 / Human event renderer that owns and executes no jobs.
///
/// 调用者应在事件暂时静默时调用 [`Renderer::tick`]，从而让延迟进度按时出现。
/// The caller should invoke [`Renderer::tick`] while events are quiet so delayed
/// progress can appear on time.
pub struct HumanRenderer<W, C = SystemClock, T = FixedTerminal>
where
    C: Clock,
    T: TerminalProbe,
{
    writer: W,
    clock: C,
    options: PresentationOptions,
    terminal: T,
    environment: Environment,
    color: bool,
    dynamic: bool,
    progress: Option<ProgressState<C::Instant>>,
    jobs: BTreeMap<JobId, JobView>,
}

impl<W: Write> HumanRenderer<W, SystemClock, FixedTerminal> {
    /// 用系统时钟创建呈现器。 / Creates a renderer using the system clock.
    pub fn new(
        writer: W,
        capabilities: TerminalCapabilities,
        environment: Environment,
        options: PresentationOptions,
    ) -> Self {
        Self::with_clock(writer, capabilities, environment, options, SystemClock)
    }
}

impl<W: Write, C: Clock> HumanRenderer<W, C, FixedTerminal> {
    /// 用可注入时钟创建呈现器。 / Creates a renderer with an injectable clock.
    pub fn with_clock(
        writer: W,
        capabilities: TerminalCapabilities,
        environment: Environment,
        options: PresentationOptions,
        clock: C,
    ) -> Self {
        Self::with_clock_and_terminal(
            writer,
            FixedTerminal::new(capabilities),
            environment,
            options,
            clock,
        )
    }
}

impl<W: Write, C: Clock, T: TerminalProbe> HumanRenderer<W, C, T> {
    /// 用可刷新终端端口和时钟创建呈现器。 / Creates a renderer with refreshable terminal and clock ports.
    pub fn with_clock_and_terminal(
        writer: W,
        terminal: T,
        environment: Environment,
        options: PresentationOptions,
        clock: C,
    ) -> Self {
        let capabilities = terminal.capabilities();
        let color = color_enabled(options.color, capabilities, &environment);
        let dynamic_capable = capabilities.is_terminal
            && capabilities.supports_ansi
            && capabilities.supports_dynamic
            && terminal.width().is_some();
        let dynamic = options.verbosity != Verbosity::Quiet
            && dynamic_capable
            && match options.progress {
                ProgressMode::Auto => {
                    !environment.ci && environment.term.as_deref() != Some("dumb")
                }
                ProgressMode::Always => true,
                ProgressMode::Never => false,
            };
        Self {
            writer,
            clock,
            options,
            terminal,
            environment,
            color,
            dynamic,
            progress: None,
            jobs: BTreeMap::new(),
        }
    }

    /// 返回最终采用的色彩决策。 / Returns the resolved color decision.
    pub const fn uses_color(&self) -> bool {
        self.color
    }

    /// 返回是否启用原地动态进度。 / Returns whether in-place dynamic progress is enabled.
    pub const fn uses_dynamic_progress(&self) -> bool {
        self.dynamic
    }

    /// 返回探测到的能力。 / Returns the detected capabilities.
    pub fn capabilities(&self) -> TerminalCapabilities {
        self.terminal.capabilities()
    }

    /// 返回环境快照。 / Returns the environment snapshot.
    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// 取回底层写入器。 / Returns the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }

    fn clear_progress(&mut self) -> io::Result<()> {
        if let Some(state) = self.progress.as_ref().filter(|state| state.visible) {
            let width = self
                .terminal
                .width()
                .unwrap_or(state.rendered_width.max(1))
                .max(1);
            let rows = state.rendered_width.max(1).div_ceil(width);
            for row in 0..rows {
                self.writer.write_all(CLEAR_LINE.as_bytes())?;
                if row + 1 < rows {
                    self.writer.write_all(b"\x1b[1A")?;
                }
            }
        }
        if let Some(state) = &mut self.progress {
            state.visible = false;
        }
        Ok(())
    }

    fn write_persistent(&mut self, line: &str) -> io::Result<()> {
        self.clear_progress()?;
        // `line` 只由固定装饰和逐个净化的协议字段组成；再次净化会破坏本 crate 生成的 ANSI。
        // `line` contains only fixed decorations and individually sanitized protocol fields;
        // another sanitization pass would neutralize ANSI generated by this crate.
        writeln!(self.writer, "{line}")?;
        self.writer.flush()
    }

    fn draw_progress(&mut self, now: C::Instant) -> io::Result<()> {
        let Some(terminal_width) = self.terminal.width() else {
            self.clear_progress()?;
            return Ok(());
        };
        let Some(state) = &self.progress else {
            return Ok(());
        };
        let delay = match self.options.progress {
            ProgressMode::Always => Duration::ZERO,
            ProgressMode::Auto | ProgressMode::Never => self.options.progress_delay,
        };
        if self.clock.elapsed(state.started, now) < delay {
            return Ok(());
        }
        if state
            .last_draw
            .is_some_and(|last| self.clock.elapsed(last, now) < self.options.progress_refresh)
        {
            return Ok(());
        }
        let percent = if state.total == 0 {
            100
        } else {
            ((u128::from(state.completed) * 100) / u128::from(state.total)) as u64
        };
        let text = format!(
            "{} {:>3}% ({}/{})",
            sanitize(&state.message),
            percent,
            state.completed,
            state.total
        );
        // 预留最后一列，避免恰好填满终端触发自动换行。
        // Reserve the final column so an exactly full row cannot auto-wrap.
        let columns = terminal_width.saturating_sub(1);
        let text = truncate_middle(&text, columns);
        let rendered_width = UnicodeWidthStr::width(text.as_str());
        if state.visible {
            self.clear_progress()?;
        }
        write!(self.writer, "{CLEAR_LINE}{text}")?;
        self.writer.flush()?;
        let state = self.progress.as_mut().expect("progress state still exists");
        state.visible = true;
        state.last_draw = Some(now);
        state.rendered_width = rendered_width;
        Ok(())
    }

    fn update_progress(&mut self) -> io::Result<()> {
        if self.options.progress == ProgressMode::Never || !self.dynamic {
            return Ok(());
        }
        let now = self.clock.now();
        let declared: u64 = self.jobs.values().map(|job| job.declared_actions).sum();
        let observed = self.jobs.values().map(|job| job.actions.len() as u64).sum();
        // 乱序或部分事件流也不应产生 `completed > total` 的荒谬 UI。
        // Even partial or out-of-order streams must not render nonsensical `completed > total` UI.
        let total = declared.max(observed);
        let completed = self
            .jobs
            .values()
            .flat_map(|job| job.actions.values())
            .filter(|action| action.state == ActionState::Complete)
            .count() as u64;
        let active = self.jobs.iter().find_map(|(job, state)| {
            state.actions.iter().find_map(|(action, view)| {
                (view.state == ActionState::Running)
                    .then(|| format!("{} {} ({})", action_kind_name(view.kind), action, job))
            })
        });
        let message = active.unwrap_or_else(|| "Scheduling".to_owned());
        let started = self.progress.as_ref().map_or(now, |state| state.started);
        self.progress = Some(ProgressState {
            started,
            last_draw: self.progress.as_ref().and_then(|state| state.last_draw),
            message,
            completed,
            total,
            visible: self.progress.as_ref().is_some_and(|state| state.visible),
            rendered_width: self
                .progress
                .as_ref()
                .map_or(0, |state| state.rendered_width),
        });
        self.draw_progress(now)
    }

    fn queue_action(&mut self, job: &JobId, action: &ActionId, kind: ActionKind) {
        self.jobs.entry(job.clone()).or_default().actions.insert(
            action.clone(),
            ActionView {
                kind,
                state: ActionState::Queued,
            },
        );
    }

    fn set_action_state(&mut self, job: &JobId, action: &ActionId, state: ActionState) {
        let job = self.jobs.entry(job.clone()).or_default();
        if let Some(view) = job.actions.get_mut(action) {
            view.state = state;
        } else {
            job.actions.insert(
                action.clone(),
                ActionView {
                    kind: ActionKind::Compile,
                    state,
                },
            );
        }
    }

    fn action_kind(&self, job: &JobId, action: &ActionId) -> ActionKind {
        self.jobs
            .get(job)
            .and_then(|job| job.actions.get(action))
            .map_or(ActionKind::Compile, |view| view.kind)
    }

    fn render_diagnostic(&mut self, diagnostic: &squish_protocol::Diagnostic) -> io::Result<()> {
        let visible = match self.options.verbosity {
            Verbosity::Quiet => matches!(diagnostic.severity, Severity::Warning | Severity::Error),
            Verbosity::Normal => diagnostic.severity != Severity::Trace,
            Verbosity::Verbose => true,
        };
        if !visible {
            return Ok(());
        }
        let severity = severity_name(diagnostic.severity);
        let label = styled(self.color, diagnostic.severity, severity);
        let mut line = format!(
            "{label}[{}] {} ({})",
            sanitize(&diagnostic.code),
            sanitize(&diagnostic.message),
            phase_name(diagnostic.phase)
        );
        if let Some(span) = &diagnostic.primary {
            let bytes = span.bytes();
            line.push_str(&format!(
                " at {}:{}..{}",
                sanitize(span.source().as_str()),
                bytes.start,
                bytes.end
            ));
        }
        if let Some(help) = &diagnostic.help {
            line.push_str("; help: ");
            line.push_str(&sanitize(help));
        }
        self.write_persistent(&line)?;
        for related in &diagnostic.related {
            let bytes = related.span.bytes();
            self.write_persistent(&format!(
                "  related: {} at {}:{}..{}",
                sanitize(&related.label),
                sanitize(related.span.source().as_str()),
                bytes.start,
                bytes.end
            ))?;
        }
        Ok(())
    }
}

impl<W: Write, C: Clock, T: TerminalProbe> Renderer for HumanRenderer<W, C, T> {
    fn render(&mut self, event: &Event) -> io::Result<()> {
        match &event.payload {
            EventPayload::PlanReady { job, actions } => {
                self.jobs.entry(job.clone()).or_default().declared_actions = *actions;
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} ({} actions)",
                        styled_info(self.color, "Planned"),
                        sanitize(job.as_str()),
                        actions
                    ))?;
                }
                self.update_progress()?;
            }
            EventPayload::ActionQueued {
                job,
                action,
                kind,
                dependencies,
            } => {
                self.queue_action(job, action, *kind);
                if self.options.verbosity == Verbosity::Verbose {
                    let dependencies = dependencies
                        .iter()
                        .map(|dependency| sanitize(dependency.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.write_persistent(&format!(
                        "Queued {} {} ({}) after [{}]",
                        action_kind_name(*kind),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        dependencies
                    ))?;
                }
                self.update_progress()?;
            }
            EventPayload::ActionStarted { job, action } => {
                self.set_action_state(job, action, ActionState::Running);
                if !self.dynamic && self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({})",
                        styled_info(self.color, "Running"),
                        action_kind_name(self.action_kind(job, action)),
                        sanitize(action.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
                self.update_progress()?;
            }
            EventPayload::CacheHit {
                job,
                action,
                cache,
                digest,
                action_key,
                outputs,
            } => {
                if self.options.verbosity == Verbosity::Verbose {
                    let key = action_key
                        .as_ref()
                        .map_or("legacy-v1.0".to_owned(), |key| key.to_string());
                    self.write_persistent(&format!(
                        "Cached {} ({}, {}, {}, key {}, {} outputs)",
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        cache_name(*cache),
                        format_digest(digest),
                        sanitize(&key),
                        outputs.len()
                    ))?;
                }
            }
            EventPayload::ActionSucceeded {
                job,
                action,
                timing,
                artifacts,
            } => {
                let kind = self.action_kind(job, action);
                self.set_action_state(job, action, ActionState::Complete);
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({}, {} ms)",
                        styled_success(self.color, "Finished"),
                        action_kind_name(kind),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        timing.elapsed_ms
                    ))?;
                    for artifact in artifacts {
                        self.write_persistent(&format!(
                            "{} {} {} ({} bytes, {})",
                            styled_info(self.color, "Produced"),
                            artifact_kind(&artifact.kind),
                            sanitize(&artifact.uri),
                            artifact.size,
                            format_digest(&artifact.digest)
                        ))?;
                    }
                }
                self.update_progress()?;
            }
            EventPayload::ActionFailed {
                job,
                action,
                timing: _,
                diagnostic,
            } => {
                self.set_action_state(job, action, ActionState::Complete);
                self.render_diagnostic(diagnostic)?;
                self.update_progress()?;
            }
            EventPayload::ActionBlocked {
                job,
                action,
                blocked_by,
            } => {
                self.set_action_state(job, action, ActionState::Complete);
                if self.options.verbosity != Verbosity::Quiet {
                    let causes = blocked_by
                        .iter()
                        .map(|cause| sanitize(cause.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.write_persistent(&format!(
                        "{} {} ({}) blocked by {}",
                        styled(self.color, Severity::Warning, "Blocked"),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        causes
                    ))?;
                }
                self.update_progress()?;
            }
            EventPayload::ActionCancelled {
                job,
                action,
                timing: _,
            } => {
                self.set_action_state(job, action, ActionState::Complete);
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "Cancelled {} ({})",
                        sanitize(action.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
                self.update_progress()?;
            }
            EventPayload::Diagnostic(diagnostic) => self.render_diagnostic(diagnostic)?,
            EventPayload::OperationCompleted { job, result } => {
                if self.options.verbosity != Verbosity::Quiet {
                    let value = serde_json::to_string(result).map_err(io::Error::other)?;
                    self.write_persistent(&format!(
                        "Result {}: {}",
                        sanitize(job.as_str()),
                        sanitize(&value)
                    ))?;
                }
            }
            EventPayload::JobFinished(summary) => {
                self.clear_progress()?;
                self.jobs.remove(&summary.job);
                if self.options.verbosity != Verbosity::Quiet || summary.status.code() != 0 {
                    let label = if summary.status.code() == 0 {
                        styled_success(self.color, "Completed")
                    } else {
                        styled_error(self.color, "Failed")
                    };
                    self.write_persistent(&format!(
                        "{} {}: {} succeeded, {} failed, {} blocked, {} cancelled, {} cached ({} ms)",
                        label,
                        sanitize(summary.job.as_str()),
                        summary.totals.succeeded,
                        summary.totals.failed,
                        summary.totals.blocked,
                        summary.totals.cancelled,
                        summary.cache_hits,
                        summary.timing.elapsed_ms
                    ))?;
                }
                if self.jobs.is_empty() {
                    self.progress = None;
                } else {
                    self.update_progress()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> io::Result<()> {
        if self.dynamic {
            self.draw_progress(self.clock.now())?;
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.clear_progress()?;
        self.progress = None;
        self.writer.flush()
    }
}

fn color_enabled(
    mode: ColorMode,
    capabilities: TerminalCapabilities,
    environment: &Environment,
) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            capabilities.is_terminal
                && capabilities.supports_ansi
                && !environment.no_color
                && environment.term.as_deref() != Some("dumb")
        }
    }
}

fn sanitize(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\u{1b}' => result.push_str("\\u{1b}"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(result, "\\u{{{:x}}}", character as u32);
            }
            character => result.push(character),
        }
    }
    result
}

fn truncate_middle(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_owned();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }
    let available = max_width - 3;
    let prefix_budget = available / 3;
    let suffix_budget = available - prefix_budget;
    let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(value, true).collect();
    let prefix = take_graphemes(graphemes.iter().copied(), prefix_budget);
    let suffix_reversed = take_graphemes(graphemes.iter().rev().copied(), suffix_budget);
    let suffix: String = UnicodeSegmentation::graphemes(suffix_reversed.as_str(), true)
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn take_graphemes<'a>(graphemes: impl Iterator<Item = &'a str>, width_budget: usize) -> String {
    let mut width = 0;
    graphemes
        .take_while(|grapheme| {
            let next = UnicodeWidthStr::width(*grapheme);
            let fits = width + next <= width_budget;
            if fits {
                width += next;
            }
            fits
        })
        .collect()
}

fn styled(color: bool, severity: Severity, text: &str) -> String {
    let code = match severity {
        Severity::Trace => "2",
        Severity::Info => "36",
        Severity::Warning => "33;1",
        Severity::Error => "31;1",
    };
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn styled_info(color: bool, text: &str) -> String {
    styled(color, Severity::Info, text)
}

fn styled_success(color: bool, text: &str) -> String {
    if color {
        format!("\x1b[32;1m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

fn styled_error(color: bool, text: &str) -> String {
    styled(color, Severity::Error, text)
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Trace => "trace",
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Discover => "discover",
        Phase::Resolve => "resolve",
        Phase::Snapshot => "snapshot",
        Phase::Scan => "scan",
        Phase::Parse => "parse",
        Phase::Analyze => "analyze",
        Phase::Instantiate => "instantiate",
        Phase::Cache => "cache",
        Phase::Link => "link",
        Phase::Emit => "emit",
        Phase::Format => "format",
        Phase::Manage => "manage",
        Phase::Publish => "publish",
        Phase::Orchestrate => "orchestrate",
    }
}

fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Resolve => "resolve",
        ActionKind::Snapshot => "snapshot",
        ActionKind::Scan => "scan",
        ActionKind::Compile => "compile",
        ActionKind::Link => "link",
        ActionKind::Instantiate => "instantiate",
        ActionKind::Backend => "backend",
        ActionKind::Publish => "publish",
        ActionKind::Format => "format",
        ActionKind::ResolveCandidate => "resolve-candidate",
        ActionKind::CommitTransaction => "commit-transaction",
    }
}

fn cache_name(cache: CacheKind) -> &'static str {
    match cache {
        CacheKind::Local => "local",
        CacheKind::Remote => "remote",
    }
}

fn format_digest(digest: &Digest) -> String {
    let algorithm = match digest.algorithm() {
        DigestAlgorithm::Sha256 => "sha256".to_owned(),
        DigestAlgorithm::Blake3 => "blake3".to_owned(),
        DigestAlgorithm::Other(name) => sanitize(name),
    };
    format!("{algorithm}:{}", digest.hex())
}

fn artifact_kind(kind: &ArtifactKind) -> String {
    match kind {
        ArtifactKind::BinaryIr => "binary-ir".to_owned(),
        ArtifactKind::Prompt => "prompt".to_owned(),
        ArtifactKind::DebugInfo => "debug-info".to_owned(),
        ArtifactKind::Metadata => "metadata".to_owned(),
        ArtifactKind::Other(name) => sanitize(name),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use squish_protocol::{
        ActionId, ActionTotals, Artifact, ArtifactId, BuildResult, Diagnostic, DiagnosticId,
        Digest, DigestAlgorithm, Event, ExitStatus, InvocationId, JobId, JobSummary,
        OperationResult, Timing,
    };

    use super::*;

    #[derive(Clone, Default)]
    struct FakeClock(Rc<Cell<Duration>>);

    impl FakeClock {
        fn advance(&self, duration: Duration) {
            self.0.set(self.0.get() + duration);
        }
    }

    impl Clock for FakeClock {
        type Instant = Duration;

        fn now(&self) -> Self::Instant {
            self.0.get()
        }

        fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration {
            later.saturating_sub(earlier)
        }
    }

    #[derive(Clone)]
    struct ResizableTerminal(Rc<Cell<usize>>);

    impl TerminalProbe for ResizableTerminal {
        fn capabilities(&self) -> TerminalCapabilities {
            terminal()
        }

        fn width(&self) -> Option<usize> {
            Some(self.0.get())
        }
    }

    fn id(value: &str) -> JobId {
        JobId::new(value).unwrap()
    }

    fn action(value: &str) -> ActionId {
        ActionId::new(value).unwrap()
    }

    fn event(sequence: u64, payload: EventPayload) -> Event {
        Event::new(InvocationId::new("run").unwrap(), sequence, payload)
    }

    fn diagnostic(severity: Severity, message: &str) -> Event {
        event(
            20,
            EventPayload::Diagnostic(Diagnostic {
                id: DiagnosticId::new("diag").unwrap(),
                code: "P001".to_owned(),
                severity,
                phase: Phase::Parse,
                message: message.to_owned(),
                primary: None,
                related: Vec::new(),
                help: None,
            }),
        )
    }

    fn plan(sequence: u64, actions: u64) -> Event {
        event(
            sequence,
            EventPayload::PlanReady {
                job: id("build:main"),
                actions,
            },
        )
    }

    fn queued(sequence: u64) -> Event {
        event(
            sequence,
            EventPayload::ActionQueued {
                job: id("build:main"),
                action: action("compile:main"),
                kind: ActionKind::Compile,
                dependencies: Vec::new(),
            },
        )
    }

    fn started(sequence: u64) -> Event {
        event(
            sequence,
            EventPayload::ActionStarted {
                job: id("build:main"),
                action: action("compile:main"),
            },
        )
    }

    fn succeeded(sequence: u64) -> Event {
        event(
            sequence,
            EventPayload::ActionSucceeded {
                job: id("build:main"),
                action: action("compile:main"),
                timing: Timing { elapsed_ms: 12 },
                artifacts: vec![Artifact {
                    id: ArtifactId::new("prompt").unwrap(),
                    kind: ArtifactKind::Prompt,
                    uri: "target/main.prompt".to_owned(),
                    size: 42,
                    digest: Digest::new(DigestAlgorithm::Sha256, vec![0xab; 32]).unwrap(),
                }],
            },
        )
    }

    fn terminal() -> TerminalCapabilities {
        TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        }
    }

    fn sized_terminal() -> FixedTerminal {
        FixedTerminal::new(terminal()).with_width(Some(80))
    }

    #[test]
    fn ndjson_emits_exactly_one_unmodified_event_per_line() {
        let first = plan(0, 1);
        let second = queued(1);
        let third = event(
            2,
            EventPayload::OperationCompleted {
                job: id("build:main"),
                result: OperationResult::Build(BuildResult {
                    published: vec![],
                    build_record: None,
                }),
            },
        );
        let mut renderer = NdjsonRenderer::new(Vec::new());
        renderer.render(&first).unwrap();
        renderer.render(&second).unwrap();
        renderer.render(&third).unwrap();
        renderer.finish().unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(serde_json::from_str::<Event>(lines[0]).unwrap(), first);
        assert_eq!(serde_json::from_str::<Event>(lines[1]).unwrap(), second);
        assert_eq!(serde_json::from_str::<Event>(lines[2]).unwrap(), third);
        assert!(!output.contains('\r'));
        assert!(!output.contains('\u{1b}'));
    }

    #[test]
    fn auto_color_obeys_terminal_no_color_and_dumb_but_not_ci() {
        let unknown_width = HumanRenderer::new(
            Vec::new(),
            terminal(),
            Environment::default(),
            PresentationOptions::default(),
        );
        assert!(unknown_width.uses_color());
        assert!(!unknown_width.uses_dynamic_progress());
        for environment in [
            Environment {
                no_color: true,
                ..Environment::default()
            },
            Environment {
                term: Some("dumb".to_owned()),
                ..Environment::default()
            },
        ] {
            assert!(
                !HumanRenderer::new(
                    Vec::new(),
                    terminal(),
                    environment,
                    PresentationOptions::default(),
                )
                .uses_color()
            );
        }
        let ci = HumanRenderer::new(
            Vec::new(),
            terminal(),
            Environment {
                ci: true,
                ..Environment::default()
            },
            PresentationOptions::default(),
        );
        assert!(ci.uses_color());
        assert!(!ci.uses_dynamic_progress());
    }

    #[test]
    fn explicit_color_modes_override_environment_hint() {
        let hostile = Environment {
            no_color: true,
            term: Some("dumb".to_owned()),
            ci: true,
        };
        let always = PresentationOptions {
            color: ColorMode::Always,
            ..PresentationOptions::default()
        };
        assert!(
            HumanRenderer::new(Vec::new(), TerminalCapabilities::plain(), hostile, always)
                .uses_color()
        );
        let never = PresentationOptions {
            color: ColorMode::Never,
            ..PresentationOptions::default()
        };
        assert!(
            !HumanRenderer::new(Vec::new(), terminal(), Environment::default(), never).uses_color()
        );
    }

    #[test]
    fn non_terminal_lifecycle_is_stable_append_only_output() {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                ..PresentationOptions::default()
            },
        );
        for item in [plan(0, 1), queued(1), started(2), succeeded(3)] {
            renderer.render(&item).unwrap();
        }
        renderer.finish().unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(
            output,
            concat!(
                "Planned build:main (1 actions)\n",
                "Running compile compile:main (build:main)\n",
                "Finished compile compile:main (build:main, 12 ms)\n",
                "Produced prompt target/main.prompt (42 bytes, sha256:abababababababababababababababababababababababababababababababab)\n",
            )
        );
        assert!(!output.contains('\r'));
        assert!(!output.contains('\u{1b}'));
    }

    #[test]
    fn dynamic_progress_waits_500ms_then_throttles_to_100ms() {
        let clock = FakeClock::default();
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            sized_terminal(),
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                ..PresentationOptions::default()
            },
            clock.clone(),
        );
        for item in [plan(0, 1), queued(1), started(2)] {
            renderer.render(&item).unwrap();
        }
        let baseline = renderer.writer.len();
        clock.advance(Duration::from_millis(499));
        renderer.tick().unwrap();
        assert_eq!(renderer.writer.len(), baseline);
        clock.advance(Duration::from_millis(1));
        renderer.tick().unwrap();
        let once = renderer.writer.len();
        assert!(once > baseline);
        clock.advance(Duration::from_millis(99));
        renderer.tick().unwrap();
        assert_eq!(renderer.writer.len(), once);
        clock.advance(Duration::from_millis(1));
        renderer.tick().unwrap();
        assert!(renderer.writer.len() > once);
    }

    #[test]
    fn explicit_progress_always_skips_delay_and_never_suppresses_repaint() {
        let clock = FakeClock::default();
        let mut always = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            sized_terminal(),
            Environment {
                ci: true,
                term: Some("dumb".to_owned()),
                ..Environment::default()
            },
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            clock.clone(),
        );
        always.render(&plan(0, 1)).unwrap();
        assert!(always.uses_dynamic_progress());
        assert!(String::from_utf8_lossy(&always.writer).contains("Scheduling"));

        let mut never = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            sized_terminal(),
            Environment::default(),
            PresentationOptions {
                verbosity: Verbosity::Quiet,
                progress: ProgressMode::Never,
                ..PresentationOptions::default()
            },
            clock,
        );
        for item in [plan(0, 1), queued(1), started(2)] {
            never.render(&item).unwrap();
        }
        assert!(!never.uses_dynamic_progress());
        assert!(never.writer.is_empty());
    }

    #[test]
    fn durable_diagnostic_clears_a_visible_progress_line_first() {
        let clock = FakeClock::default();
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            sized_terminal(),
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            clock,
        );
        for item in [plan(0, 1), queued(1), started(2)] {
            renderer.render(&item).unwrap();
        }
        renderer
            .render(&diagnostic(Severity::Warning, "oops"))
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert!(output.contains("\r\x1b[2Kwarning[P001] oops (parse)\n"));
    }

    #[test]
    fn finished_job_cannot_resurrect_progress_on_later_tick() {
        let clock = FakeClock::default();
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            sized_terminal(),
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            clock.clone(),
        );
        renderer.render(&plan(0, 1)).unwrap();
        renderer
            .render(&event(
                1,
                EventPayload::JobFinished(JobSummary {
                    job: id("build:main"),
                    totals: ActionTotals {
                        succeeded: 1,
                        ..ActionTotals::default()
                    },
                    root_failures: 0,
                    cache_hits: 0,
                    timing: Timing { elapsed_ms: 2 },
                    status: ExitStatus::Success,
                }),
            ))
            .unwrap();
        let length = renderer.writer.len();
        clock.advance(Duration::from_secs(1));
        renderer.tick().unwrap();
        assert_eq!(renderer.writer.len(), length);
    }

    #[test]
    fn progress_tracks_resized_width_without_splitting_graphemes() {
        let width = Rc::new(Cell::new(24));
        let terminal = ResizableTerminal(width.clone());
        let clock = FakeClock::default();
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            terminal,
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            clock.clone(),
        );
        renderer
            .render(&event(
                0,
                EventPayload::PlanReady {
                    job: id("构建:very-long-target-name"),
                    actions: 1,
                },
            ))
            .unwrap();
        renderer
            .render(&event(
                1,
                EventPayload::ActionQueued {
                    job: id("构建:very-long-target-name"),
                    action: action("compile:👩‍💻-very-long-action"),
                    kind: ActionKind::Compile,
                    dependencies: Vec::new(),
                },
            ))
            .unwrap();
        renderer
            .render(&event(
                2,
                EventPayload::ActionStarted {
                    job: id("构建:very-long-target-name"),
                    action: action("compile:👩‍💻-very-long-action"),
                },
            ))
            .unwrap();
        let first = String::from_utf8_lossy(&renderer.writer)
            .rsplit(CLEAR_LINE)
            .next()
            .unwrap()
            .to_owned();
        assert!(UnicodeWidthStr::width(first.as_str()) <= 23);

        width.set(12);
        clock.advance(DEFAULT_PROGRESS_REFRESH);
        let before_resize = renderer.writer.len();
        renderer.tick().unwrap();
        let repaint = &renderer.writer[before_resize..];
        assert_eq!(
            String::from_utf8_lossy(repaint).matches(CLEAR_LINE).count(),
            3,
            "two reflowed rows are cleared before the new frame is drawn"
        );
        let second = String::from_utf8_lossy(&renderer.writer)
            .rsplit(CLEAR_LINE)
            .next()
            .unwrap()
            .to_owned();
        assert!(UnicodeWidthStr::width(second.as_str()) <= 11);
        assert!(!second.contains('\u{fffd}'));
    }

    #[test]
    fn control_characters_cannot_inject_lines_or_ansi() {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions::default(),
        );
        renderer
            .render(&diagnostic(
                Severity::Error,
                "bad\nforged\r\x1b[31mred\tend",
            ))
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert!(!output.contains('\u{1b}'));
        assert!(output.contains(r"bad\nforged\r\u{1b}[31mred\tend"));
    }

    #[test]
    fn color_is_applied_only_to_renderer_owned_labels() {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            terminal(),
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Always,
                ..PresentationOptions::default()
            },
        );
        renderer
            .render(&diagnostic(Severity::Error, "user\x1b[32mtext"))
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert!(output.starts_with("\x1b[31;1merror\x1b[0m"));
        assert!(output.contains(r"user\u{1b}[32mtext"));
        assert_eq!(output.matches('\u{1b}').count(), 2);
    }

    #[test]
    fn quiet_hides_status_but_preserves_errors_and_failed_summary() {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions {
                verbosity: Verbosity::Quiet,
                ..PresentationOptions::default()
            },
        );
        renderer.render(&plan(0, 1)).unwrap();
        renderer
            .render(&diagnostic(Severity::Info, "hidden"))
            .unwrap();
        renderer
            .render(&diagnostic(Severity::Error, "shown"))
            .unwrap();
        renderer
            .render(&event(
                30,
                EventPayload::JobFinished(JobSummary {
                    job: id("build:main"),
                    totals: ActionTotals {
                        failed: 1,
                        ..ActionTotals::default()
                    },
                    root_failures: 1,
                    cache_hits: 0,
                    timing: Timing { elapsed_ms: 9 },
                    status: ExitStatus::Failed,
                }),
            ))
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert!(!output.contains("Planned"));
        assert!(!output.contains("hidden"));
        assert!(output.contains("error[P001] shown"));
        assert!(output.contains("Failed build:main"));
    }

    #[test]
    fn verbose_includes_queue_parent_and_cache_identity() {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions {
                verbosity: Verbosity::Verbose,
                ..PresentationOptions::default()
            },
        );
        renderer
            .render(&event(
                1,
                EventPayload::ActionQueued {
                    job: id("build:main"),
                    action: action("link:main"),
                    kind: ActionKind::Link,
                    dependencies: vec![action("compile:main")],
                },
            ))
            .unwrap();
        renderer
            .render(&event(
                2,
                EventPayload::CacheHit {
                    job: id("build:main"),
                    action: action("link:main"),
                    cache: CacheKind::Local,
                    digest: Digest::new(DigestAlgorithm::Blake3, vec![1; 32]).unwrap(),
                    action_key: Some(squish_protocol::ActionKeyId::new("key:link").unwrap()),
                    outputs: vec![],
                },
            ))
            .unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert!(output.contains("Queued link link:main (build:main) after [compile:main]"));
        assert!(output.contains("Cached link:main (build:main, local, blake3:0101010101010101010101010101010101010101010101010101010101010101, key key:link, 0 outputs)"));
    }
}
