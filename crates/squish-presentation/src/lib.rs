//! prompt-squish 的终端与机器可读呈现领域。 / Terminal and machine-readable presentation domain for prompt-squish.
//!
//! 这里消费 [`squish_protocol::Event`]，但不执行任务，也不依赖任何构建领域。
//! 人类终端和 NDJSON 是同一事件流的两种投影；调用者负责事件排序和调度。
//! This crate consumes [`squish_protocol::Event`] but neither executes jobs nor
//! depends on a build domain. Human terminals and NDJSON are projections of the
//! same event stream; event ordering and scheduling belong to the caller.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

use squish_protocol::{
    ActionId, ActionKind, ArtifactKind, CacheKind, Digest, DigestAlgorithm, Event, EventPayload,
    ExitStatus, FinalizationId, FinalizationKind, JobId, OperationResult, Phase, PlanCloseReason,
    PlanId, PlanMode, PlanningStepKind, Severity,
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
    completed: Option<u64>,
    total: Option<u64>,
    visible: bool,
    rendered_width: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionState {
    Declared,
    Running,
    Terminal,
}

#[derive(Clone, Debug)]
struct ActionView {
    kind: ActionKind,
    state: ActionState,
}

#[derive(Clone, Debug, Default)]
struct PlanView {
    declared_actions: u64,
    mode: Option<PlanMode>,
    actions: BTreeMap<ActionId, ActionView>,
    closed: bool,
}

#[derive(Clone, Debug, Default)]
struct JobView {
    plans: BTreeMap<PlanId, PlanView>,
    finalizations: BTreeMap<FinalizationId, FinalizationKind>,
}

/// 将协议事件按规范编码成逐行 NDJSON。 / Encodes protocol events as canonical line-delimited NDJSON.
///
/// 每个事件立即写入并刷新，且使用 [`Event::encode_json`] 验证 v2 局部不变式；人类
/// 呈现策略绝不会污染机器流。 / Each event is written and flushed immediately through
/// [`Event::encode_json`], which validates the local v2 invariants; human presentation policy
/// can never contaminate the machine stream.
///
/// # 示例 / Example
///
/// ```
/// use squish_presentation::{NdjsonRenderer, Renderer};
/// use squish_protocol::{Event, EventPayload, InvocationId, JobId, PlanningAttemptId};
///
/// let event = Event::new(
///     InvocationId::new("example").unwrap(),
///     0,
///     EventPayload::PlanningStarted {
///         job: JobId::new("job").unwrap(),
///         attempt: PlanningAttemptId::new("attempt-1").unwrap(),
///     },
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
        let bytes = event.encode_json().map_err(io::Error::other)?;
        self.writer.write_all(&bytes)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// 面向人类的 v2 生命周期呈现器，不拥有或执行任何任务。 / Human v2 lifecycle renderer that owns and executes no jobs.
///
/// 非 TTY 输出永远追加写入。TTY 上的临时进度会延迟出现、最多每 100 ms 重绘一次，并
/// 在任何持久行或终态之前清除。宿主应在事件静默时调用 [`Renderer::tick`]。 /
/// Non-TTY output is always append-only. On a TTY, transient progress is delayed, repainted at
/// most every 100 ms, and cleared before every durable line or terminal event. The host should
/// invoke [`Renderer::tick`] while events are quiet.
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
    rendered_diagnostics: BTreeSet<String>,
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
            rendered_diagnostics: BTreeSet::new(),
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

    /// 返回当前探测能力。 / Returns the currently detected capabilities.
    pub fn capabilities(&self) -> TerminalCapabilities {
        self.terminal.capabilities()
    }

    /// 返回稳定环境快照。 / Returns the stable environment snapshot.
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

    fn stop_progress(&mut self) -> io::Result<()> {
        self.clear_progress()?;
        self.progress = None;
        Ok(())
    }

    fn write_persistent(&mut self, line: &str) -> io::Result<()> {
        self.clear_progress()?;
        writeln!(self.writer, "{line}")?;
        self.writer.flush()
    }

    fn begin_progress(
        &mut self,
        message: String,
        completed: Option<u64>,
        total: Option<u64>,
    ) -> io::Result<()> {
        if !self.dynamic || self.options.progress == ProgressMode::Never {
            return Ok(());
        }
        let now = self.clock.now();
        let existing = self.progress.take();
        let same_kind = existing
            .as_ref()
            .is_some_and(|state| state.total.is_some() == total.is_some());
        self.progress = Some(ProgressState {
            started: if same_kind {
                existing.as_ref().map_or(now, |state| state.started)
            } else {
                now
            },
            last_draw: existing.as_ref().and_then(|state| state.last_draw),
            message,
            completed,
            total,
            visible: existing.as_ref().is_some_and(|state| state.visible),
            rendered_width: existing.as_ref().map_or(0, |state| state.rendered_width),
        });
        self.draw_progress(now)
    }

    fn draw_progress(&mut self, now: C::Instant) -> io::Result<()> {
        let Some(width) = self.terminal.width() else {
            return self.stop_progress();
        };
        let Some(state) = &self.progress else {
            return Ok(());
        };
        let delay = if self.options.progress == ProgressMode::Always {
            Duration::ZERO
        } else {
            self.options.progress_delay
        };
        if self.clock.elapsed(state.started, now) < delay
            || state
                .last_draw
                .is_some_and(|last| self.clock.elapsed(last, now) < self.options.progress_refresh)
        {
            return Ok(());
        }
        let text = match (state.completed, state.total) {
            (Some(completed), Some(total)) => {
                let percent = if total == 0 {
                    100
                } else {
                    ((u128::from(completed) * 100) / u128::from(total)) as u64
                };
                format!(
                    "{} {:>3}% ({completed}/{total})",
                    sanitize(&state.message),
                    percent
                )
            }
            _ => format!(
                "{} ({:.1}s)",
                sanitize(&state.message),
                self.clock.elapsed(state.started, now).as_secs_f64()
            ),
        };
        let text = truncate_middle(&text, width.saturating_sub(1));
        let rendered_width = UnicodeWidthStr::width(text.as_str());
        if state.visible {
            self.clear_progress()?;
        }
        write!(self.writer, "{CLEAR_LINE}{text}")?;
        self.writer.flush()?;
        let state = self.progress.as_mut().expect("progress state exists");
        state.visible = true;
        state.last_draw = Some(now);
        state.rendered_width = rendered_width;
        Ok(())
    }

    fn plan_mut(&mut self, job: &JobId, plan: &PlanId) -> &mut PlanView {
        self.jobs
            .entry(job.clone())
            .or_default()
            .plans
            .entry(plan.clone())
            .or_default()
    }

    fn action_kind(&self, job: &JobId, plan: &PlanId, action: &ActionId) -> ActionKind {
        self.jobs
            .get(job)
            .and_then(|job| job.plans.get(plan))
            .and_then(|plan| plan.actions.get(action))
            .map_or(ActionKind::Compile, |view| view.kind)
    }

    fn set_action_state(
        &mut self,
        job: &JobId,
        plan: &PlanId,
        action: &ActionId,
        state: ActionState,
    ) {
        let view = self.plan_mut(job, plan);
        if let Some(action) = view.actions.get_mut(action) {
            action.state = state;
        }
    }

    fn update_action_progress(&mut self, job: &JobId, plan: &PlanId) -> io::Result<()> {
        let Some(view) = self.jobs.get(job).and_then(|job| job.plans.get(plan)) else {
            return Ok(());
        };
        if view.closed || view.mode != Some(PlanMode::Execute) {
            return self.stop_progress();
        }
        let total = view.declared_actions.max(view.actions.len() as u64);
        let completed = view
            .actions
            .values()
            .filter(|action| action.state == ActionState::Terminal)
            .count() as u64;
        let running: Vec<_> = view
            .actions
            .iter()
            .filter(|(_, action)| action.state == ActionState::Running)
            .collect();
        if total == 0 || completed == total {
            return self.stop_progress();
        }
        let message = match running.as_slice() {
            [] => "Waiting for runnable actions".to_owned(),
            [(action, view)] => format!("{} {}", action_kind_name(view.kind), action),
            [(action, _), ..] => {
                format!("Running {} actions (including {})", running.len(), action)
            }
        };
        self.begin_progress(message, Some(completed), Some(total))
    }

    fn render_diagnostic(&mut self, diagnostic: &squish_protocol::Diagnostic) -> io::Result<()> {
        if !self
            .rendered_diagnostics
            .insert(diagnostic.id.as_str().to_owned())
        {
            return Ok(());
        }
        let visible = match self.options.verbosity {
            Verbosity::Quiet => matches!(diagnostic.severity, Severity::Warning | Severity::Error),
            Verbosity::Normal => diagnostic.severity != Severity::Trace,
            Verbosity::Verbose => true,
        };
        if !visible {
            return Ok(());
        }
        let label = styled(
            self.color,
            diagnostic.severity,
            severity_name(diagnostic.severity),
        );
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

    fn render_operation_result(&mut self, job: &JobId, result: &OperationResult) -> io::Result<()> {
        if self.options.verbosity == Verbosity::Quiet {
            return Ok(());
        }
        let detail = match result {
            OperationResult::Unavailable { kind } => {
                format!("{} result unavailable", operation_kind_name(*kind))
            }
            OperationResult::Build(result) => {
                let artifacts: usize = result
                    .published
                    .iter()
                    .map(|target| target.artifacts.len())
                    .sum();
                format!(
                    "build: {} targets published, {artifacts} artifacts",
                    result.published.len()
                )
            }
            OperationResult::Format(result) => format!(
                "format: {} selected, {} changed{}",
                result.selected.len(),
                result.changed.len(),
                if result.check { " (check only)" } else { "" }
            ),
            OperationResult::Add(result) => format!(
                "{} dependency {}",
                if result.dry_run {
                    "would update"
                } else {
                    "updated"
                },
                sanitize(result.dependency.as_str())
            ),
            OperationResult::Remove(result) => format!(
                "{} dependency {}",
                if result.dry_run {
                    "would remove"
                } else {
                    "removed"
                },
                sanitize(result.dependency.as_str())
            ),
            OperationResult::Inspect(result) => {
                format!("inspection: {}", inspect_result_name(result))
            }
        };
        self.write_persistent(&format!(
            "{} {}: {detail}",
            styled_info(self.color, "Result"),
            sanitize(job.as_str())
        ))
    }
}

impl<W: Write, C: Clock, T: TerminalProbe> Renderer for HumanRenderer<W, C, T> {
    fn render(&mut self, event: &Event) -> io::Result<()> {
        match &event.payload {
            EventPayload::PlanningStarted { job, attempt } => {
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} ({})",
                        styled_info(self.color, "Planning"),
                        sanitize(job.as_str()),
                        sanitize(attempt.as_str())
                    ))?;
                }
                self.begin_progress(format!("Planning {}", job), None, None)?;
            }
            EventPayload::PlanningStepStarted {
                job, step, kind, ..
            } => {
                if !self.dynamic && self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({})",
                        styled_info(self.color, "Planning"),
                        planning_step_name(*kind),
                        sanitize(step.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
                self.begin_progress(format!("{} {}", planning_step_name(*kind), job), None, None)?;
            }
            EventPayload::PlanningStepSucceeded {
                job, step, timing, ..
            } => {
                self.stop_progress()?;
                if self.options.verbosity == Verbosity::Verbose {
                    self.write_persistent(&format!(
                        "Planned step {} ({}, {} ms)",
                        sanitize(step.as_str()),
                        sanitize(job.as_str()),
                        timing.elapsed_ms
                    ))?;
                }
            }
            EventPayload::PlanningStepFailed { diagnostic, .. } => {
                self.stop_progress()?;
                self.render_diagnostic(diagnostic)?;
            }
            EventPayload::PlanningStepCancelled { job, step, .. } => {
                self.stop_progress()?;
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "Cancelled planning step {} ({})",
                        sanitize(step.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
            }
            EventPayload::PlanningIssue { diagnostic, .. } => self.render_diagnostic(diagnostic)?,
            EventPayload::PlanningFailed { diagnostic, .. } => {
                self.stop_progress()?;
                self.render_diagnostic(diagnostic)?;
            }
            EventPayload::PlanningCancelled { job, .. } => {
                self.stop_progress()?;
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "Cancelled planning {}",
                        sanitize(job.as_str())
                    ))?;
                }
            }
            EventPayload::PlanReady {
                job,
                plan,
                mode,
                actions,
                issues,
                ..
            } => {
                self.stop_progress()?;
                let view = self.plan_mut(job, plan);
                view.declared_actions = *actions;
                view.mode = Some(*mode);
                if self.options.verbosity != Verbosity::Quiet {
                    let mode_text = match mode {
                        PlanMode::Execute => "execute",
                        PlanMode::ReportOnly => "report only",
                        PlanMode::Legacy => "legacy projection",
                    };
                    self.write_persistent(&format!(
                        "{} {} for {}: {} actions, {} issues ({mode_text})",
                        styled_info(self.color, "Planned"),
                        sanitize(plan.as_str()),
                        sanitize(job.as_str()),
                        actions,
                        issues
                    ))?;
                }
            }
            EventPayload::ActionDeclared {
                job,
                plan,
                action,
                kind,
                dependencies,
            } => {
                self.plan_mut(job, plan).actions.insert(
                    action.clone(),
                    ActionView {
                        kind: *kind,
                        state: ActionState::Declared,
                    },
                );
                if self.options.verbosity == Verbosity::Verbose {
                    let after = dependencies
                        .iter()
                        .map(|id| sanitize(id.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.write_persistent(&format!(
                        "Declared {} {} ({}) after [{}]",
                        action_kind_name(*kind),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        after
                    ))?;
                }
            }
            EventPayload::ActionStarted { job, plan, action } => {
                self.set_action_state(job, plan, action, ActionState::Running);
                if !self.dynamic && self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({})",
                        styled_info(self.color, "Running"),
                        action_kind_name(self.action_kind(job, plan, action)),
                        sanitize(action.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
                self.update_action_progress(job, plan)?;
            }
            EventPayload::CacheHit {
                job,
                plan,
                action,
                cache,
                digest,
                outputs,
                ..
            } => {
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} ({}, {}, {}, {} outputs)",
                        styled_success(self.color, "Cached"),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        cache_name(*cache),
                        format_digest(digest),
                        outputs.len()
                    ))?;
                }
                // 缓存命中是执行器观察到的事实，不是动作终态；内核稍后仍会发送
                // `ActionSucceeded` 或 `ActionFailed`。 / A cache hit is an executor fact,
                // not an action terminal state; the kernel still emits `ActionSucceeded` or
                // `ActionFailed` afterwards.
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionSucceeded {
                job,
                plan,
                action,
                timing,
                artifacts,
            } => {
                let kind = self.action_kind(job, plan, action);
                self.set_action_state(job, plan, action, ActionState::Terminal);
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
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionFailed {
                job,
                plan,
                action,
                timing,
                diagnostic,
            } => {
                let kind = self.action_kind(job, plan, action);
                self.set_action_state(job, plan, action, ActionState::Terminal);
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({}, {} ms)",
                        styled_error(self.color, "Failed"),
                        action_kind_name(kind),
                        sanitize(action.as_str()),
                        sanitize(job.as_str()),
                        timing.elapsed_ms
                    ))?;
                }
                self.render_diagnostic(diagnostic)?;
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionBlocked {
                job,
                plan,
                action,
                blocked_by,
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                if self.options.verbosity != Verbosity::Quiet {
                    let causes = blocked_by
                        .iter()
                        .map(|id| sanitize(id.as_str()))
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
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionCancelled {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "Cancelled {} ({})",
                        sanitize(action.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionSuperseded {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.stop_progress()?;
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} ({}) because project state changed",
                        styled(self.color, Severity::Warning, "Superseded"),
                        sanitize(action.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
            }
            EventPayload::PlanClosed { job, plan, reason } => {
                self.stop_progress()?;
                self.plan_mut(job, plan).closed = true;
                if self.options.verbosity != Verbosity::Quiet {
                    match reason {
                        PlanCloseReason::Executed => {}
                        PlanCloseReason::Reported => self.write_persistent(&format!(
                            "{} {} ({}) without execution",
                            styled_success(self.color, "Reported"),
                            sanitize(plan.as_str()),
                            sanitize(job.as_str())
                        ))?,
                        PlanCloseReason::Superseded => self.write_persistent(&format!(
                            "{} plan {} ({}); replanning",
                            styled(self.color, Severity::Warning, "Superseded"),
                            sanitize(plan.as_str()),
                            sanitize(job.as_str())
                        ))?,
                    }
                }
            }
            EventPayload::FinalizationStarted { job, id, kind } => {
                self.stop_progress()?;
                self.jobs
                    .entry(job.clone())
                    .or_default()
                    .finalizations
                    .insert(id.clone(), *kind);
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({})",
                        styled_info(self.color, "Finalizing"),
                        finalization_kind_name(*kind),
                        sanitize(id.as_str()),
                        sanitize(job.as_str())
                    ))?;
                }
            }
            EventPayload::FinalizationSucceeded { job, id, timing } => {
                self.stop_progress()?;
                let kind = self
                    .jobs
                    .get(job)
                    .and_then(|view| view.finalizations.get(id))
                    .copied();
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({}, {} ms)",
                        styled_success(self.color, "Finalized"),
                        finalization_name(kind),
                        sanitize(id.as_str()),
                        sanitize(job.as_str()),
                        timing.elapsed_ms
                    ))?;
                }
            }
            EventPayload::FinalizationFailed {
                job,
                id,
                timing,
                diagnostic,
            } => {
                self.stop_progress()?;
                let kind = self
                    .jobs
                    .get(job)
                    .and_then(|view| view.finalizations.get(id))
                    .copied();
                if self.options.verbosity != Verbosity::Quiet {
                    self.write_persistent(&format!(
                        "{} {} {} ({}, {} ms)",
                        styled_error(self.color, "Failed"),
                        finalization_name(kind),
                        sanitize(id.as_str()),
                        sanitize(job.as_str()),
                        timing.elapsed_ms
                    ))?;
                }
                self.render_diagnostic(diagnostic)?;
            }
            EventPayload::Diagnostic(diagnostic) => self.render_diagnostic(diagnostic)?,
            EventPayload::OperationCompleted { job, result } => {
                self.stop_progress()?;
                self.render_operation_result(job, result)?;
            }
            EventPayload::JobFinished(summary) => {
                self.stop_progress()?;
                self.jobs.remove(&summary.job);
                if self.options.verbosity != Verbosity::Quiet
                    || summary.status != ExitStatus::Success
                {
                    let label = match summary.status {
                        ExitStatus::Success => styled_success(self.color, "Completed"),
                        ExitStatus::Failed => styled_error(self.color, "Failed"),
                        ExitStatus::Cancelled => styled(self.color, Severity::Warning, "Cancelled"),
                    };
                    self.write_persistent(&format!("{} {}: {} succeeded, {} failed, {} blocked, {} cancelled, {} cached ({} ms)", label, sanitize(summary.job.as_str()), summary.totals.succeeded, summary.totals.failed, summary.totals.blocked, summary.totals.cancelled, summary.cache_hits, summary.timing.elapsed_ms))?;
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
        self.stop_progress()?;
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
fn planning_step_name(kind: PlanningStepKind) -> &'static str {
    match kind {
        PlanningStepKind::Recover => "Recovering",
        PlanningStepKind::Locate => "Locating",
        PlanningStepKind::Resolve => "Resolving",
        PlanningStepKind::Fetch => "Fetching",
        PlanningStepKind::ReconcileLock => "Reconciling lock",
        PlanningStepKind::Snapshot => "Snapshotting",
        PlanningStepKind::Scan => "Scanning",
        PlanningStepKind::ValidatePlan => "Validating plan",
        PlanningStepKind::PrepareCandidate => "Preparing candidate",
        _ => "Planning",
    }
}
fn finalization_kind_name(kind: FinalizationKind) -> &'static str {
    match kind {
        FinalizationKind::PersistBuildCatalog => "build catalog",
        _ => "operation record",
    }
}
fn finalization_name(kind: Option<FinalizationKind>) -> &'static str {
    kind.map_or("operation record", finalization_kind_name)
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
        ActionKind::Inspect => "inspect",
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
fn operation_kind_name(kind: squish_protocol::OperationKind) -> &'static str {
    match kind {
        squish_protocol::OperationKind::Build => "build",
        squish_protocol::OperationKind::Format => "format",
        squish_protocol::OperationKind::Add => "add",
        squish_protocol::OperationKind::Remove => "remove",
        squish_protocol::OperationKind::Inspect => "inspect",
    }
}
fn inspect_result_name(result: &squish_protocol::InspectResult) -> &'static str {
    match result {
        squish_protocol::InspectResult::Project(_) => "project",
        squish_protocol::InspectResult::Plan(_) => "plan",
        squish_protocol::InspectResult::Cache(_) => "cache",
        squish_protocol::InspectResult::Ir(_) => "IR",
        squish_protocol::InspectResult::Link(_) => "link",
        squish_protocol::InspectResult::Source(_) => "source",
        squish_protocol::InspectResult::Provenance(_) => "provenance",
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
        ActionKeyId, ActionTotals, Diagnostic, DiagnosticId, Digest, DigestAlgorithm, Event,
        EventPayload, ExitStatus, InvocationId, JobId, JobSummary, OperationKind, OperationResult,
        PlanDigest, PlanId, PlanMode, PlanningAttemptId, PlanningStepId, PlanningStepKind, Timing,
    };

    use super::*;

    #[derive(Clone, Default)]
    struct TestClock(Rc<Cell<u64>>);

    impl TestClock {
        fn advance(&self, milliseconds: u64) {
            self.0.set(self.0.get() + milliseconds);
        }
    }

    impl Clock for TestClock {
        type Instant = u64;

        fn now(&self) -> Self::Instant {
            self.0.get()
        }

        fn elapsed(&self, earlier: Self::Instant, later: Self::Instant) -> Duration {
            Duration::from_millis(later.saturating_sub(earlier))
        }
    }

    fn id<T>(value: &str) -> T
    where
        T: TryFrom<String>,
        <T as TryFrom<String>>::Error: std::fmt::Debug,
    {
        T::try_from(value.to_owned()).unwrap()
    }

    fn event(sequence: u64, payload: EventPayload) -> Event {
        Event::new(id::<InvocationId>("invocation"), sequence, payload)
    }

    fn diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
            id: id::<DiagnosticId>("diagnostic"),
            code: "PLAN001".to_owned(),
            severity: Severity::Error,
            phase: Phase::Orchestrate,
            message: message.to_owned(),
            primary: None,
            related: Vec::new(),
            help: Some("fix the manifest".to_owned()),
        }
    }

    fn plan_digest(byte: u8) -> PlanDigest {
        PlanDigest::new(Digest::new(DigestAlgorithm::Sha256, vec![byte; 32]).unwrap())
    }

    fn render_plain(events: &[Event]) -> String {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions::default(),
        );
        for event in events {
            renderer.render(event).unwrap();
        }
        renderer.finish().unwrap();
        String::from_utf8(renderer.into_inner()).unwrap()
    }

    #[test]
    fn planning_failure_with_zero_actions_has_truthful_snapshot() {
        let job = id::<JobId>("build");
        let attempt = id::<PlanningAttemptId>("attempt-1");
        let events = vec![
            event(
                0,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: attempt.clone(),
                },
            ),
            event(
                1,
                EventPayload::PlanningFailed {
                    job: job.clone(),
                    attempt,
                    diagnostic: diagnostic("cannot select a target"),
                },
            ),
            event(
                2,
                EventPayload::OperationCompleted {
                    job: job.clone(),
                    result: OperationResult::Unavailable {
                        kind: OperationKind::Build,
                    },
                },
            ),
            event(
                3,
                EventPayload::JobFinished(JobSummary {
                    job,
                    totals: ActionTotals::default(),
                    root_failures: 1,
                    cache_hits: 0,
                    timing: Timing { elapsed_ms: 12 },
                    status: ExitStatus::Failed,
                }),
            ),
        ];
        assert_eq!(
            render_plain(&events),
            "Planning build (attempt-1)\nerror[PLAN001] cannot select a target (orchestrate); help: fix the manifest\nResult build: build result unavailable\nFailed build: 0 succeeded, 0 failed, 0 blocked, 0 cancelled, 0 cached (12 ms)\n"
        );
    }

    #[test]
    fn superseded_plan_and_replan_are_explicit_snapshot() {
        let job = id::<JobId>("add");
        let attempt1 = id::<PlanningAttemptId>("attempt-1");
        let attempt2 = id::<PlanningAttemptId>("attempt-2");
        let plan1 = id::<PlanId>("plan-1");
        let plan2 = id::<PlanId>("plan-2");
        let commit = id::<ActionId>("commit");
        let compile = id::<ActionId>("compile");
        let digest = Digest::new(DigestAlgorithm::Blake3, vec![9; 32]).unwrap();
        let events = vec![
            event(
                0,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: attempt1.clone(),
                },
            ),
            event(
                1,
                EventPayload::PlanReady {
                    job: job.clone(),
                    attempt: attempt1,
                    plan: plan1.clone(),
                    digest: plan_digest(1),
                    mode: PlanMode::Execute,
                    actions: 1,
                    issues: 0,
                },
            ),
            event(
                2,
                EventPayload::ActionDeclared {
                    job: job.clone(),
                    plan: plan1.clone(),
                    action: commit.clone(),
                    kind: ActionKind::CommitTransaction,
                    dependencies: vec![],
                },
            ),
            event(
                3,
                EventPayload::ActionStarted {
                    job: job.clone(),
                    plan: plan1.clone(),
                    action: commit.clone(),
                },
            ),
            event(
                4,
                EventPayload::ActionSuperseded {
                    job: job.clone(),
                    plan: plan1.clone(),
                    action: commit,
                    timing: Timing { elapsed_ms: 2 },
                    reason: squish_protocol::SupersedeReason::AuthoritativeRevisionChanged,
                },
            ),
            event(
                5,
                EventPayload::PlanClosed {
                    job: job.clone(),
                    plan: plan1,
                    reason: PlanCloseReason::Superseded,
                },
            ),
            event(
                6,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: attempt2.clone(),
                },
            ),
            event(
                7,
                EventPayload::PlanReady {
                    job: job.clone(),
                    attempt: attempt2,
                    plan: plan2.clone(),
                    digest: plan_digest(2),
                    mode: PlanMode::Execute,
                    actions: 1,
                    issues: 0,
                },
            ),
            event(
                8,
                EventPayload::ActionDeclared {
                    job: job.clone(),
                    plan: plan2.clone(),
                    action: compile.clone(),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                },
            ),
            event(
                9,
                EventPayload::CacheHit {
                    job: job.clone(),
                    plan: plan2.clone(),
                    action: compile,
                    cache: CacheKind::Local,
                    digest,
                    action_key: Some(id::<ActionKeyId>("key")),
                    outputs: vec![],
                },
            ),
            event(
                10,
                EventPayload::PlanClosed {
                    job,
                    plan: plan2,
                    reason: PlanCloseReason::Executed,
                },
            ),
        ];
        assert_eq!(
            render_plain(&events),
            "Planning add (attempt-1)\nPlanned plan-1 for add: 1 actions, 0 issues (execute)\nRunning commit-transaction commit (add)\nSuperseded commit (add) because project state changed\nSuperseded plan plan-1 (add); replanning\nPlanning add (attempt-2)\nPlanned plan-2 for add: 1 actions, 0 issues (execute)\nCached compile (add, local, blake3:0909090909090909090909090909090909090909090909090909090909090909, 0 outputs)\n"
        );
    }

    #[test]
    fn report_only_plan_never_claims_execution_snapshot() {
        let job = id::<JobId>("inspect-plan");
        let attempt = id::<PlanningAttemptId>("attempt");
        let plan = id::<PlanId>("plan");
        let events = vec![
            event(
                0,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: attempt.clone(),
                },
            ),
            event(
                1,
                EventPayload::PlanReady {
                    job: job.clone(),
                    attempt,
                    plan: plan.clone(),
                    digest: plan_digest(3),
                    mode: PlanMode::ReportOnly,
                    actions: 1,
                    issues: 0,
                },
            ),
            event(
                2,
                EventPayload::ActionDeclared {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: id::<ActionId>("compile"),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                },
            ),
            event(
                3,
                EventPayload::PlanClosed {
                    job,
                    plan,
                    reason: PlanCloseReason::Reported,
                },
            ),
        ];
        assert_eq!(
            render_plain(&events),
            "Planning inspect-plan (attempt)\nPlanned plan for inspect-plan: 1 actions, 0 issues (report only)\nReported plan (inspect-plan) without execution\n"
        );
    }

    #[test]
    fn build_catalog_finalization_success_is_post_plan_and_append_only() {
        let job = id::<JobId>("build");
        let finalization = id::<FinalizationId>("persist-build-catalog");
        let events = vec![
            event(
                0,
                EventPayload::FinalizationStarted {
                    job: job.clone(),
                    id: finalization.clone(),
                    kind: FinalizationKind::PersistBuildCatalog,
                },
            ),
            event(
                1,
                EventPayload::FinalizationSucceeded {
                    job,
                    id: finalization,
                    timing: Timing { elapsed_ms: 4 },
                },
            ),
        ];
        let output = render_plain(&events);
        assert_eq!(
            output,
            "Finalizing build catalog persist-build-catalog (build)\nFinalized build catalog persist-build-catalog (build, 4 ms)\n"
        );
        assert!(!output.contains('\x1b'));
        assert!(!output.contains('\r'));
        assert!(!output.contains("actions"));
    }

    #[test]
    fn build_catalog_finalization_failure_is_colored_but_not_color_dependent() {
        let job = id::<JobId>("build");
        let finalization = id::<FinalizationId>("catalog");
        let events = [
            event(
                0,
                EventPayload::FinalizationStarted {
                    job: job.clone(),
                    id: finalization.clone(),
                    kind: FinalizationKind::PersistBuildCatalog,
                },
            ),
            event(
                1,
                EventPayload::FinalizationFailed {
                    job,
                    id: finalization,
                    timing: Timing { elapsed_ms: 7 },
                    diagnostic: diagnostic("could not persist terminal build facts"),
                },
            ),
        ];
        let render = |color| {
            let mut renderer = HumanRenderer::new(
                Vec::new(),
                TerminalCapabilities::plain(),
                Environment::default(),
                PresentationOptions {
                    color,
                    ..PresentationOptions::default()
                },
            );
            for event in &events {
                renderer.render(event).unwrap();
            }
            renderer.finish().unwrap();
            String::from_utf8(renderer.into_inner()).unwrap()
        };
        let plain = render(ColorMode::Never);
        assert_eq!(
            plain,
            "Finalizing build catalog catalog (build)\nFailed build catalog catalog (build, 7 ms)\nerror[PLAN001] could not persist terminal build facts (orchestrate); help: fix the manifest\n"
        );
        let colored = render(ColorMode::Always);
        assert!(colored.contains("\x1b[36mFinalizing\x1b[0m build catalog"));
        assert!(colored.contains("\x1b[31;1mFailed\x1b[0m build catalog"));
        assert!(colored.contains("\x1b[31;1merror\x1b[0m[PLAN001]"));
    }

    #[test]
    fn color_modes_no_color_and_non_tty_are_independent() {
        let event = event(0, EventPayload::Diagnostic(diagnostic("bad input")));
        let tty = TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: false,
        };
        let render = |color, environment, capabilities| {
            let mut renderer = HumanRenderer::new(
                Vec::new(),
                capabilities,
                environment,
                PresentationOptions {
                    color,
                    ..PresentationOptions::default()
                },
            );
            renderer.render(&event).unwrap();
            renderer.finish().unwrap();
            String::from_utf8(renderer.into_inner()).unwrap()
        };
        assert_eq!(
            render(ColorMode::Never, Environment::default(), tty),
            "error[PLAN001] bad input (orchestrate); help: fix the manifest\n"
        );
        assert!(
            render(
                ColorMode::Always,
                Environment::default(),
                TerminalCapabilities::plain()
            )
            .contains("\x1b[31;1merror\x1b[0m")
        );
        assert!(
            !render(
                ColorMode::Auto,
                Environment {
                    no_color: true,
                    ..Environment::default()
                },
                tty
            )
            .contains('\x1b')
        );
        let piped = render(
            ColorMode::Auto,
            Environment::default(),
            TerminalCapabilities::plain(),
        );
        assert!(!piped.contains('\x1b'));
        assert!(!piped.contains('\r'));
    }

    #[test]
    fn delayed_progress_is_unicode_width_bounded_and_stops_at_terminal_event() {
        let clock = TestClock::default();
        let terminal = FixedTerminal::new(TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        })
        .with_width(Some(24));
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            terminal,
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress_delay: Duration::from_millis(500),
                ..PresentationOptions::default()
            },
            clock.clone(),
        );
        let job = id::<JobId>("构建-job");
        let attempt = id::<PlanningAttemptId>("attempt");
        let step = id::<PlanningStepId>("resolve");
        renderer
            .render(&event(
                0,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: attempt.clone(),
                },
            ))
            .unwrap();
        renderer
            .render(&event(
                1,
                EventPayload::PlanningStepStarted {
                    job: job.clone(),
                    attempt: attempt.clone(),
                    step: step.clone(),
                    kind: PlanningStepKind::Resolve,
                },
            ))
            .unwrap();
        assert!(!String::from_utf8_lossy(renderer.writer.as_slice()).contains("0.5s"));
        clock.advance(500);
        renderer.tick().unwrap();
        let before_terminal = String::from_utf8_lossy(renderer.writer.as_slice()).into_owned();
        assert!(before_terminal.contains("0.5s"));
        renderer
            .render(&event(
                2,
                EventPayload::PlanningStepSucceeded {
                    job,
                    attempt,
                    step,
                    timing: Timing { elapsed_ms: 500 },
                },
            ))
            .unwrap();
        let after_terminal = String::from_utf8_lossy(renderer.writer.as_slice()).into_owned();
        clock.advance(500);
        renderer.tick().unwrap();
        assert_eq!(
            String::from_utf8_lossy(renderer.writer.as_slice()),
            after_terminal
        );
        assert!(after_terminal.contains("\r\x1b[2K"));
    }

    #[test]
    fn dynamic_progress_exposes_parallel_actions_without_a_spinner() {
        let clock = TestClock::default();
        let terminal = FixedTerminal::new(TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        })
        .with_width(Some(80));
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            terminal,
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            clock,
        );
        let job = id::<JobId>("build");
        let plan = id::<PlanId>("plan");
        let attempt = id::<PlanningAttemptId>("attempt");
        let first = id::<ActionId>("alpha");
        let second = id::<ActionId>("beta");
        for value in [
            EventPayload::PlanReady {
                job: job.clone(),
                attempt,
                plan: plan.clone(),
                digest: plan_digest(4),
                mode: PlanMode::Execute,
                actions: 2,
                issues: 0,
            },
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan.clone(),
                action: first.clone(),
                kind: ActionKind::Compile,
                dependencies: vec![],
            },
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan.clone(),
                action: second.clone(),
                kind: ActionKind::Link,
                dependencies: vec![],
            },
            EventPayload::ActionStarted {
                job: job.clone(),
                plan: plan.clone(),
                action: second,
            },
            EventPayload::ActionStarted {
                job,
                plan,
                action: first,
            },
        ] {
            renderer.render(&event(0, value)).unwrap();
        }
        renderer.clock.advance(100);
        renderer.tick().unwrap();
        let output = String::from_utf8_lossy(renderer.writer.as_slice());
        assert!(output.contains("Running 2 actions (including alpha)"));
        assert!(!output.contains("|") && !output.contains("/ build"));
    }

    #[test]
    fn cache_hit_does_not_finish_a_started_action() {
        let clock = TestClock::default();
        let terminal = FixedTerminal::new(TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        })
        .with_width(Some(80));
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
        let job = id::<JobId>("build");
        let plan = id::<PlanId>("plan");
        let alpha = id::<ActionId>("alpha");
        let beta = id::<ActionId>("beta");
        for payload in [
            EventPayload::PlanReady {
                job: job.clone(),
                attempt: id::<PlanningAttemptId>("attempt"),
                plan: plan.clone(),
                digest: plan_digest(5),
                mode: PlanMode::Execute,
                actions: 2,
                issues: 0,
            },
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan.clone(),
                action: alpha.clone(),
                kind: ActionKind::Compile,
                dependencies: vec![],
            },
            EventPayload::ActionDeclared {
                job: job.clone(),
                plan: plan.clone(),
                action: beta.clone(),
                kind: ActionKind::Link,
                dependencies: vec![],
            },
            EventPayload::ActionStarted {
                job: job.clone(),
                plan: plan.clone(),
                action: alpha.clone(),
            },
            EventPayload::ActionStarted {
                job: job.clone(),
                plan: plan.clone(),
                action: beta.clone(),
            },
        ] {
            renderer.render(&event(0, payload)).unwrap();
        }
        clock.advance(100);
        renderer.tick().unwrap();
        renderer
            .render(&event(
                5,
                EventPayload::CacheHit {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: alpha.clone(),
                    cache: CacheKind::Local,
                    digest: Digest::new(DigestAlgorithm::Blake3, vec![6; 32]).unwrap(),
                    action_key: Some(id::<ActionKeyId>("alpha-key")),
                    outputs: vec![],
                },
            ))
            .unwrap();
        clock.advance(100);
        renderer.tick().unwrap();

        let plan_view = &renderer.jobs[&job].plans[&plan];
        assert_eq!(plan_view.actions[&alpha].state, ActionState::Running);
        assert_eq!(plan_view.actions[&beta].state, ActionState::Running);
        let before_terminal = String::from_utf8_lossy(renderer.writer.as_slice()).into_owned();
        assert!(before_terminal.contains("Running 2 actions (including alpha)   0% (0/2)"));
        assert!(!before_terminal.contains(" 50% (1/2)"));

        renderer
            .render(&event(
                6,
                EventPayload::ActionSucceeded {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: alpha.clone(),
                    timing: Timing { elapsed_ms: 3 },
                    artifacts: vec![],
                },
            ))
            .unwrap();
        clock.advance(100);
        renderer.tick().unwrap();

        let plan_view = &renderer.jobs[&job].plans[&plan];
        assert_eq!(plan_view.actions[&alpha].state, ActionState::Terminal);
        assert_eq!(plan_view.actions[&beta].state, ActionState::Running);
        let after_terminal = String::from_utf8_lossy(renderer.writer.as_slice());
        assert!(after_terminal.contains("link beta  50% (1/2)"));
    }

    #[test]
    fn ndjson_is_one_canonical_object_per_line() {
        let value = event(
            0,
            EventPayload::PlanningStarted {
                job: id::<JobId>("job"),
                attempt: id::<PlanningAttemptId>("attempt"),
            },
        );
        let mut renderer = NdjsonRenderer::new(Vec::new());
        renderer.render(&value).unwrap();
        renderer.finish().unwrap();
        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(output.trim()).unwrap(),
            serde_json::to_value(value).unwrap()
        );
    }

    #[test]
    fn sanitizer_and_middle_truncation_preserve_graphemes() {
        assert_eq!(sanitize("x\n\u{1b}[31m"), "x\\n\\u{1b}[31m");
        let truncated = truncate_middle("解析-👩‍💻-a-very-long-target", 14);
        assert!(UnicodeWidthStr::width(truncated.as_str()) <= 14);
        assert!(!truncated.contains('�'));
    }
}
