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
    ExitStatus, FinalizationId, FinalizationKind, JobId, NewResult, OperationResult, Phase,
    PlanCloseReason, PlanId, PlanMode, PlanningStepKind, Severity, VcsChoice, VcsDisposition,
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

    /// 呈现已发布的协作取消，并停止后续瞬态进度。 /
    /// Presents published cooperative cancellation and suppresses later transient progress.
    fn cancellation_requested(&mut self) -> io::Result<()> {
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
    /// 面向脚本与紧凑日志的稳定单行生命周期。 / Stable one-line lifecycle for scripts and compact logs.
    Short,
    /// 面向日常交互的默认输出。 / Default output for ordinary interaction.
    #[default]
    Normal,
    /// 包括 trace 诊断和生命周期细节。 / Include trace diagnostics and lifecycle detail.
    Verbose,
    /// 包括完整协议身份和内容摘要，供深度诊断。 / Include full protocol identities and content
    /// digests for deep diagnostics.
    Trace,
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

/// 生产环境的标准错误终端探测器。 / Production terminal probe for standard error.
///
/// 交互能力在构造时从标准错误流捕获一次，避免重定向策略在一次调用中漂移；显示宽度
/// 则在每次 [`TerminalProbe::width`] 调用时重新读取，因此窗口缩放会被后续 `tick`
/// 观察到。无法可靠读取宽度时返回 `None`，呈现器将保守地禁用或停止动态重绘。
/// / Interactive capabilities are snapshotted from standard error at construction so redirection
/// policy cannot drift during an invocation. Display width is queried again on every
/// [`TerminalProbe::width`] call, allowing subsequent ticks to observe terminal resizes. When the
/// width cannot be read reliably, `None` conservatively disables or stops dynamic repainting.
#[derive(Clone, Copy, Debug)]
pub struct SystemTerminal {
    capabilities: TerminalCapabilities,
    width: fn() -> Option<usize>,
}

impl SystemTerminal {
    /// 探测当前进程的标准错误流。 / Probes the current process standard-error stream.
    ///
    /// # 示例 / Example
    ///
    /// ```
    /// use squish_presentation::{SystemTerminal, TerminalProbe};
    ///
    /// let terminal = SystemTerminal::stderr();
    /// // Width is deliberately queried at use time rather than cached.
    /// let _current_columns = terminal.width();
    /// ```
    pub fn stderr() -> Self {
        let stderr = io::stderr();
        Self {
            capabilities: TerminalCapabilities::detect(&stderr),
            width: stderr_width,
        }
    }

    #[cfg(test)]
    const fn with_width_probe(
        capabilities: TerminalCapabilities,
        width: fn() -> Option<usize>,
    ) -> Self {
        Self {
            capabilities,
            width,
        }
    }
}

impl TerminalProbe for SystemTerminal {
    fn capabilities(&self) -> TerminalCapabilities {
        self.capabilities
    }

    fn width(&self) -> Option<usize> {
        (self.width)()
    }
}

fn stderr_width() -> Option<usize> {
    let stderr = io::stderr();
    terminal_size::terminal_size_of(&stderr)
        .map(|(terminal_size::Width(width), _)| usize::from(width))
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

/// 统一选择产品概念、短显示指纹或完整诊断细节。 / Uniformly selects product concepts,
/// short display fingerprints, or complete diagnostic detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HumanDetail {
    Product,
    Fingerprint,
    Trace,
}

impl HumanDetail {
    /// 从用户选择的详细程度建立统一投影策略。 / Builds one projection policy from the
    /// user-selected verbosity.
    const fn from_verbosity(verbosity: Verbosity) -> Self {
        match verbosity {
            Verbosity::Verbose => Self::Fingerprint,
            Verbosity::Trace => Self::Trace,
            _ => Self::Product,
        }
    }

    /// 仅在 trace 投影中返回内部标识。 / Returns an internal identity only in the trace
    /// projection.
    fn identity(self, value: &str) -> String {
        match self {
            Self::Product | Self::Fingerprint => String::new(),
            Self::Trace => format!(" {}", sanitize(value)),
        }
    }

    /// 仅在 trace 投影中追加作业上下文。 / Appends job context only in the trace projection.
    fn context(self, job: &JobId) -> String {
        match self {
            Self::Product | Self::Fingerprint => String::new(),
            Self::Trace => format!(" ({})", sanitize(job.as_str())),
        }
    }

    /// 按投影策略描述缓存命中，默认保留来源和产物数但隐藏内容摘要。 / Describes a cache
    /// hit according to the projection policy, retaining source and output count while hiding the
    /// content digest by default.
    fn cache(self, cache: CacheKind, digest: &Digest, outputs: usize) -> String {
        match self {
            Self::Product => format!("{}, {outputs} outputs", cache_name(cache)),
            Self::Fingerprint => format!(
                "{}, {}, {outputs} outputs",
                cache_name(cache),
                format_fingerprint(digest)
            ),
            Self::Trace => format!(
                "{}, {}, {outputs} outputs",
                cache_name(cache),
                format_digest(digest)
            ),
        }
    }

    /// 按投影策略描述产物；默认 URI 不暴露内容寻址缓存内部键。 / Describes an artifact
    /// according to the projection policy; the default URI does not expose content-addressed cache
    /// keys.
    fn artifact(self, artifact: &squish_protocol::Artifact) -> String {
        let uri = product_artifact_uri(&artifact.uri);
        match self {
            Self::Product => format!("{uri} ({} bytes)", artifact.size),
            Self::Fingerprint => format!(
                "{uri} ({} bytes, {})",
                artifact.size,
                format_fingerprint(&artifact.digest)
            ),
            Self::Trace => format!(
                "{} ({} bytes, {})",
                sanitize(&artifact.uri),
                artifact.size,
                format_digest(&artifact.digest)
            ),
        }
    }
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
    cancellation_requested: bool,
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
    /// 返回本次人类输出唯一采用的细节策略。 / Returns the single detail policy used by this
    /// human projection.
    const fn detail(&self) -> HumanDetail {
        HumanDetail::from_verbosity(self.options.verbosity)
    }

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
        let dynamic = matches!(
            options.verbosity,
            Verbosity::Normal | Verbosity::Verbose | Verbosity::Trace
        ) && dynamic_capable
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
            cancellation_requested: false,
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
        if self.cancellation_requested
            || !self.dynamic
            || self.options.progress == ProgressMode::Never
        {
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
            [(_, view)] => action_kind_name(view.kind).to_owned(),
            [(_, _), ..] => format!("Running {} actions", running.len()),
        };
        self.begin_progress(message, Some(completed), Some(total))
    }

    fn render_short(&mut self, event: &Event) -> io::Result<()> {
        match &event.payload {
            EventPayload::PlanningStarted { .. } => self.write_persistent("plan"),
            EventPayload::PlanningStepStarted { kind, .. } => {
                self.write_persistent(&format!("plan:{}", planning_step_short_name(*kind)))
            }
            EventPayload::PlanningStepSucceeded { .. } => Ok(()),
            EventPayload::PlanningStepFailed { diagnostic, .. }
            | EventPayload::PlanningIssue { diagnostic, .. }
            | EventPayload::PlanningFailed { diagnostic, .. } => self.render_diagnostic(diagnostic),
            EventPayload::PlanningStepCancelled { .. } | EventPayload::PlanningCancelled { .. } => {
                self.write_persistent("cancel:plan")
            }
            EventPayload::PlanReady {
                job,
                plan,
                mode,
                actions,
                issues,
                ..
            } => {
                let view = self.plan_mut(job, plan);
                view.declared_actions = *actions;
                view.mode = Some(*mode);
                self.write_persistent(&format!(
                    "plan:ready mode={} actions={} issues={}",
                    plan_mode_name(*mode),
                    actions,
                    issues
                ))
            }
            EventPayload::ActionDeclared {
                job,
                plan,
                action,
                kind,
                ..
            } => {
                self.plan_mut(job, plan).actions.insert(
                    action.clone(),
                    ActionView {
                        kind: *kind,
                        state: ActionState::Declared,
                    },
                );
                Ok(())
            }
            EventPayload::ActionStarted { job, plan, action } => {
                self.set_action_state(job, plan, action, ActionState::Running);
                self.write_persistent(&format!(
                    "run:{}",
                    action_kind_name(self.action_kind(job, plan, action))
                ))
            }
            EventPayload::CacheHit {
                job,
                plan,
                action,
                cache,
                outputs,
                ..
            } => self.write_persistent(&format!(
                "cache:{} {} outputs={}",
                cache_name(*cache),
                action_kind_name(self.action_kind(job, plan, action)),
                outputs.len()
            )),
            EventPayload::ActionSucceeded {
                job,
                plan,
                action,
                timing,
                ..
            } => {
                let kind = self.action_kind(job, plan, action);
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.write_persistent(&format!(
                    "ok:{} {}ms",
                    action_kind_name(kind),
                    timing.elapsed_ms
                ))
            }
            EventPayload::ActionFailed {
                job,
                plan,
                action,
                diagnostic,
                ..
            } => {
                let kind = self.action_kind(job, plan, action);
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.write_persistent(&format!("fail:{}", action_kind_name(kind)))?;
                self.render_diagnostic(diagnostic)
            }
            EventPayload::ActionBlocked {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.write_persistent(&format!(
                    "blocked:{}",
                    action_kind_name(self.action_kind(job, plan, action))
                ))
            }
            EventPayload::ActionCancelled {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.write_persistent(&format!(
                    "cancel:{}",
                    action_kind_name(self.action_kind(job, plan, action))
                ))
            }
            EventPayload::ActionSuperseded {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                self.write_persistent(&format!(
                    "superseded:{}",
                    action_kind_name(self.action_kind(job, plan, action))
                ))
            }
            EventPayload::PlanClosed { job, plan, reason } => {
                self.plan_mut(job, plan).closed = true;
                if *reason == PlanCloseReason::Executed {
                    Ok(())
                } else {
                    self.write_persistent(&format!("plan:{}", plan_close_name(*reason)))
                }
            }
            EventPayload::FinalizationStarted { job, id, kind } => {
                self.jobs
                    .entry(job.clone())
                    .or_default()
                    .finalizations
                    .insert(id.clone(), *kind);
                self.write_persistent(&format!("finalize:{}", finalization_kind_short_name(*kind)))
            }
            EventPayload::FinalizationSucceeded { job, id, timing } => {
                let kind = self
                    .jobs
                    .get(job)
                    .and_then(|view| view.finalizations.get(id))
                    .copied();
                self.write_persistent(&format!(
                    "finalized:{} {}ms",
                    finalization_short_name(kind),
                    timing.elapsed_ms
                ))
            }
            EventPayload::FinalizationFailed {
                job,
                id,
                diagnostic,
                ..
            } => {
                let kind = self
                    .jobs
                    .get(job)
                    .and_then(|view| view.finalizations.get(id))
                    .copied();
                self.write_persistent(&format!("fail:finalize:{}", finalization_short_name(kind)))?;
                self.render_diagnostic(diagnostic)
            }
            EventPayload::Diagnostic(diagnostic) => self.render_diagnostic(diagnostic),
            EventPayload::OperationCompleted { result, .. } => {
                self.write_persistent(&format!("result {}", short_operation_result(result)))
            }
            EventPayload::JobFinished(summary) => {
                self.jobs.remove(&summary.job);
                self.write_persistent(&format!(
                    "done status={} ok={} failed={} blocked={} cancelled={} cached={} {}ms",
                    exit_status_name(summary.status),
                    summary.totals.succeeded,
                    summary.totals.failed,
                    summary.totals.blocked,
                    summary.totals.cancelled,
                    summary.cache_hits,
                    summary.timing.elapsed_ms
                ))
            }
            _ => Ok(()),
        }
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
            Verbosity::Short | Verbosity::Normal => diagnostic.severity != Severity::Trace,
            Verbosity::Verbose | Verbosity::Trace => true,
        };
        if !visible {
            return Ok(());
        }
        let label = styled(
            self.color,
            diagnostic.severity,
            severity_name(diagnostic.severity),
        );
        if self.options.verbosity == Verbosity::Short {
            let mut line = format!(
                "{label}[{}] {}",
                sanitize(&diagnostic.code),
                sanitize(&diagnostic.message)
            );
            if let Some(span) = &diagnostic.primary {
                let bytes = span.bytes();
                line.push_str(&format!(
                    " @ {}:{}..{}",
                    sanitize(span.source().as_str()),
                    bytes.start,
                    bytes.end
                ));
            }
            return self.write_persistent(&line);
        }
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
            OperationResult::New(result) => human_new_result(result),
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
        let detail_policy = self.detail();
        self.write_persistent(&format!(
            "{}{}: {detail}",
            styled_info(self.color, "Result"),
            detail_policy.identity(job.as_str())
        ))
    }
}

impl<W: Write, C: Clock, T: TerminalProbe> Renderer for HumanRenderer<W, C, T> {
    fn render(&mut self, event: &Event) -> io::Result<()> {
        if self.options.verbosity == Verbosity::Short {
            return self.render_short(event);
        }
        match &event.payload {
            EventPayload::PlanningStarted { job, attempt } => {
                if self.options.verbosity != Verbosity::Quiet {
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{}{}{}",
                        styled_info(self.color, "Planning"),
                        detail.identity(attempt.as_str()),
                        detail.context(job)
                    ))?;
                }
                self.begin_progress(format!("Planning {job}"), None, None)?;
            }
            EventPayload::PlanningStepStarted {
                job, step, kind, ..
            } => {
                if !self.dynamic && self.options.verbosity != Verbosity::Quiet {
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{}{}",
                        styled_info(self.color, "Planning"),
                        planning_step_name(*kind),
                        detail.identity(step.as_str()),
                        detail.context(job)
                    ))?;
                }
                self.begin_progress(format!("{} {}", planning_step_name(*kind), job), None, None)?;
            }
            EventPayload::PlanningStepSucceeded {
                job, step, timing, ..
            } => {
                self.stop_progress()?;
                if matches!(
                    self.options.verbosity,
                    Verbosity::Verbose | Verbosity::Trace
                ) {
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "Planned step{} ({} ms){}",
                        detail.identity(step.as_str()),
                        timing.elapsed_ms,
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "Cancelled planning step{}{}",
                        detail.identity(step.as_str()),
                        detail.context(job)
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
                        "Cancelled planning{}",
                        self.detail().context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{}{}: {} actions, {} issues ({mode_text}){}",
                        styled_info(self.color, "Planned"),
                        detail.identity(plan.as_str()),
                        actions,
                        issues,
                        detail.context(job)
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
                if self.options.verbosity == Verbosity::Trace {
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{}{}",
                        styled_info(self.color, "Running"),
                        action_kind_name(self.action_kind(job, plan, action)),
                        detail.identity(action.as_str()),
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{} ({}){}",
                        styled_success(self.color, "Cached"),
                        action_kind_name(self.action_kind(job, plan, action)),
                        detail.identity(action.as_str()),
                        detail.cache(*cache, digest, outputs.len()),
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{} ({} ms){}",
                        styled_success(self.color, "Finished"),
                        action_kind_name(kind),
                        detail.identity(action.as_str()),
                        timing.elapsed_ms,
                        detail.context(job)
                    ))?;
                    for artifact in artifacts {
                        self.write_persistent(&format!(
                            "{} {} {}",
                            styled_info(self.color, "Produced"),
                            artifact_kind(&artifact.kind),
                            detail.artifact(artifact)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{} ({} ms){}",
                        styled_error(self.color, "Failed"),
                        action_kind_name(kind),
                        detail.identity(action.as_str()),
                        timing.elapsed_ms,
                        detail.context(job)
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
                    let detail = self.detail();
                    let causes = if detail == HumanDetail::Trace {
                        format!(
                            " by {}",
                            blocked_by
                                .iter()
                                .map(|id| sanitize(id.as_str()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    } else {
                        String::new()
                    };
                    self.write_persistent(&format!(
                        "{} {}{}{}{}",
                        styled(self.color, Severity::Warning, "Blocked"),
                        action_kind_name(self.action_kind(job, plan, action)),
                        detail.identity(action.as_str()),
                        causes,
                        detail.context(job)
                    ))?;
                }
                self.update_action_progress(job, plan)?;
            }
            EventPayload::ActionCancelled {
                job, plan, action, ..
            } => {
                self.set_action_state(job, plan, action, ActionState::Terminal);
                if self.options.verbosity != Verbosity::Quiet {
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "Cancelled {}{}{}",
                        action_kind_name(self.action_kind(job, plan, action)),
                        detail.identity(action.as_str()),
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{}{} because project state changed",
                        styled(self.color, Severity::Warning, "Superseded"),
                        action_kind_name(self.action_kind(job, plan, action)),
                        detail.identity(action.as_str()),
                        detail.context(job)
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
                            "{} plan{}{} without execution",
                            styled_success(self.color, "Reported"),
                            self.detail().identity(plan.as_str()),
                            self.detail().context(job)
                        ))?,
                        PlanCloseReason::Superseded => self.write_persistent(&format!(
                            "{} plan{}{}; replanning",
                            styled(self.color, Severity::Warning, "Superseded"),
                            self.detail().identity(plan.as_str()),
                            self.detail().context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{}{}",
                        styled_info(self.color, "Finalizing"),
                        finalization_kind_name(*kind),
                        detail.identity(id.as_str()),
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{} ({} ms){}",
                        styled_success(self.color, "Finalized"),
                        finalization_name(kind),
                        detail.identity(id.as_str()),
                        timing.elapsed_ms,
                        detail.context(job)
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
                    let detail = self.detail();
                    self.write_persistent(&format!(
                        "{} {}{} ({} ms){}",
                        styled_error(self.color, "Failed"),
                        finalization_name(kind),
                        detail.identity(id.as_str()),
                        timing.elapsed_ms,
                        detail.context(job)
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
                    self.write_persistent(&format!("{}{}: {} succeeded, {} failed, {} blocked, {} cancelled, {} cached ({} ms)", label, self.detail().identity(summary.job.as_str()), summary.totals.succeeded, summary.totals.failed, summary.totals.blocked, summary.totals.cancelled, summary.cache_hits, summary.timing.elapsed_ms))?;
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

    fn cancellation_requested(&mut self) -> io::Result<()> {
        if self.cancellation_requested {
            return Ok(());
        }
        self.cancellation_requested = true;
        self.stop_progress()?;
        writeln!(self.writer, "Cancelling; Ctrl-C again to force")?;
        self.writer.flush()
    }

    fn finish(&mut self) -> io::Result<()> {
        self.stop_progress()?;
        self.writer.flush()
    }
}

/// 类型化查询结果的人类可读投影。 / Human-readable projector for typed inspection results.
///
/// 该呈现器用于纯查询命令的 `stdout`，从不发出 ANSI 或原地重绘控制符。字段和值来自
/// [`squish_protocol::InspectResult`]，因此调用方不需要退回到 pretty-JSON。 /
/// This renderer is intended for pure-query `stdout`. It never emits ANSI or in-place repaint
/// controls. Its fields and values come directly from [`squish_protocol::InspectResult`], so a
/// caller never needs a pretty-JSON fallback.
///
/// # 示例 / Example
///
/// ```
/// use squish_presentation::InspectHumanRenderer;
/// use squish_protocol::{InspectResult, PackageName, ProjectInspection, TargetName};
///
/// let result = InspectResult::Project(ProjectInspection {
///     packages: vec![PackageName::new("workspace").unwrap()],
///     targets: vec![TargetName::new("chat").unwrap()],
/// });
/// let mut renderer = InspectHumanRenderer::new(Vec::new());
/// renderer.render(&result).unwrap();
/// renderer.finish().unwrap();
/// let output = String::from_utf8(renderer.into_inner()).unwrap();
/// assert!(output.contains("Packages (1)"));
/// ```
pub struct InspectHumanRenderer<W, T = FixedTerminal>
where
    T: TerminalProbe,
{
    writer: W,
    terminal: T,
}

impl<W: Write> InspectHumanRenderer<W, FixedTerminal> {
    /// 创建适合管道、文件和普通 `stdout` 捕获的追加式呈现器。 /
    /// Creates an append-only renderer suitable for pipes, files, and ordinary `stdout` capture.
    pub const fn new(writer: W) -> Self {
        Self {
            writer,
            terminal: FixedTerminal::new(TerminalCapabilities::plain()),
        }
    }
}

impl<W: Write, T: TerminalProbe> InspectHumanRenderer<W, T> {
    /// 使用可刷新终端宽度探测器创建呈现器；色彩能力会被有意忽略。 /
    /// Creates a renderer with a refreshable width probe; color capabilities are intentionally ignored.
    pub const fn with_terminal(writer: W, terminal: T) -> Self {
        Self { writer, terminal }
    }

    /// 呈现一个完整的类型化查询结果。 / Renders one complete typed inspection result.
    pub fn render(&mut self, result: &squish_protocol::InspectResult) -> io::Result<()> {
        match result {
            squish_protocol::InspectResult::Project(value) => {
                self.line("Project")?;
                self.collection("Packages", value.packages.iter().map(|item| item.as_str()))?;
                self.collection("Targets", value.targets.iter().map(|item| item.as_str()))
            }
            squish_protocol::InspectResult::Plan(value) => {
                self.line("Plan")?;
                self.key("Job", value.job.as_str())?;
                self.key("Identity", value.plan.as_str())?;
                self.key("Digest", &format_digest(value.digest.digest()))?;
                self.key("Mode", plan_mode_name(value.mode))?;
                self.line(&format!("  Actions ({})", value.actions.len()))?;
                if value.actions.is_empty() {
                    return self.line("    (none)");
                }
                for action in &value.actions {
                    self.line(&format!(
                        "    {} {}",
                        action_kind_name(action.kind),
                        sanitize(action.action.as_str())
                    ))?;
                    self.key_at(
                        6,
                        "Action key",
                        &action
                            .action_key
                            .as_ref()
                            .map_or_else(|| "(not materialized)".to_owned(), ToString::to_string),
                    )?;
                    self.collection_at(
                        6,
                        "Depends on",
                        action.dependencies.iter().map(|item| item.as_str()),
                    )?;
                }
                Ok(())
            }
            squish_protocol::InspectResult::Cache(value) => {
                self.line("Cache")?;
                self.line(&format!("  Actions ({})", value.actions.len()))?;
                if value.actions.is_empty() {
                    return self.line("    (none)");
                }
                for action in &value.actions {
                    self.line(&format!("    {}", action.action_key))?;
                    self.key_at(6, "Result", &format_digest(&action.result_digest))?;
                    self.artifacts(6, "Outputs", &action.outputs)?;
                }
                Ok(())
            }
            squish_protocol::InspectResult::Ir(value) => {
                self.line("IR")?;
                self.artifact(2, &value.artifact)
            }
            squish_protocol::InspectResult::Link(value) => {
                self.line("Link")?;
                self.key("Target", value.target.as_str())?;
                self.line("  Static link map")?;
                self.artifact(4, &value.link_map)
            }
            squish_protocol::InspectResult::Source(value) => {
                self.line("Source")?;
                self.key("Identity", value.source.as_str())?;
                self.key("Digest", &format_digest(&value.digest))?;
                self.key("Size", &format!("{} bytes", value.size))
            }
            squish_protocol::InspectResult::Provenance(value) => {
                self.line("Provenance")?;
                self.line("  Artifact")?;
                self.artifact(4, &value.artifact)?;
                self.artifacts(2, "Evidence", &value.evidence)
            }
        }
    }

    /// 刷新查询输出。 / Flushes query output.
    pub fn finish(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    /// 取回底层写入器。 / Returns the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }

    fn line(&mut self, value: &str) -> io::Result<()> {
        let value = sanitize(value);
        let value = self
            .terminal
            .width()
            .map_or(value.clone(), |width| truncate_middle(&value, width));
        writeln!(self.writer, "{value}")
    }

    fn key(&mut self, key: &str, value: &str) -> io::Result<()> {
        self.key_at(2, key, value)
    }

    fn key_at(&mut self, indent: usize, key: &str, value: &str) -> io::Result<()> {
        self.line(&format!("{}{key}: {}", " ".repeat(indent), sanitize(value)))
    }

    fn collection<'a>(
        &mut self,
        label: &str,
        values: impl Iterator<Item = &'a str>,
    ) -> io::Result<()> {
        self.collection_at(2, label, values)
    }

    fn collection_at<'a>(
        &mut self,
        indent: usize,
        label: &str,
        values: impl Iterator<Item = &'a str>,
    ) -> io::Result<()> {
        let values: Vec<_> = values.collect();
        self.line(&format!("{}{label} ({})", " ".repeat(indent), values.len()))?;
        if values.is_empty() {
            return self.line(&format!("{}(none)", " ".repeat(indent + 2)));
        }
        for value in values {
            self.line(&format!("{}- {}", " ".repeat(indent + 2), sanitize(value)))?;
        }
        Ok(())
    }

    fn artifacts(
        &mut self,
        indent: usize,
        label: &str,
        artifacts: &[squish_protocol::Artifact],
    ) -> io::Result<()> {
        self.line(&format!(
            "{}{label} ({})",
            " ".repeat(indent),
            artifacts.len()
        ))?;
        if artifacts.is_empty() {
            return self.line(&format!("{}(none)", " ".repeat(indent + 2)));
        }
        for artifact in artifacts {
            self.artifact(indent + 2, artifact)?;
        }
        Ok(())
    }

    fn artifact(&mut self, indent: usize, artifact: &squish_protocol::Artifact) -> io::Result<()> {
        self.line(&format!(
            "{}{} ({})",
            " ".repeat(indent),
            sanitize(artifact.id.as_str()),
            artifact_kind(&artifact.kind)
        ))?;
        self.key_at(indent + 2, "URI", &artifact.uri)?;
        self.key_at(indent + 2, "Size", &format!("{} bytes", artifact.size))?;
        self.key_at(indent + 2, "Digest", &format_digest(&artifact.digest))
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
fn planning_step_short_name(kind: PlanningStepKind) -> &'static str {
    match kind {
        PlanningStepKind::Recover => "recover",
        PlanningStepKind::Locate => "locate",
        PlanningStepKind::Resolve => "resolve",
        PlanningStepKind::Fetch => "fetch",
        PlanningStepKind::ReconcileLock => "lock",
        PlanningStepKind::Snapshot => "snapshot",
        PlanningStepKind::Scan => "scan",
        PlanningStepKind::ValidatePlan => "validate",
        PlanningStepKind::PrepareCandidate => "candidate",
        _ => "work",
    }
}
fn plan_mode_name(mode: PlanMode) -> &'static str {
    match mode {
        PlanMode::Execute => "execute",
        PlanMode::ReportOnly => "report",
        PlanMode::Legacy => "legacy",
    }
}
fn plan_close_name(reason: PlanCloseReason) -> &'static str {
    match reason {
        PlanCloseReason::Executed => "executed",
        PlanCloseReason::Reported => "reported",
        PlanCloseReason::Superseded => "superseded",
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
fn finalization_kind_short_name(kind: FinalizationKind) -> &'static str {
    match kind {
        FinalizationKind::PersistBuildCatalog => "build-catalog",
        _ => "operation-record",
    }
}
fn finalization_short_name(kind: Option<FinalizationKind>) -> &'static str {
    kind.map_or("operation-record", finalization_kind_short_name)
}
fn action_kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::CreateProject => "create-project",
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

/// 将内部内容寻址 URI 投影为用户概念，其余位置保持原样。 / Projects internal
/// content-addressed URIs to a user concept while preserving ordinary locations verbatim.
fn product_artifact_uri(uri: &str) -> String {
    if uri.starts_with("cas://") {
        "content-addressed cache".to_owned()
    } else {
        sanitize(uri)
    }
}
fn operation_kind_name(kind: squish_protocol::OperationKind) -> &'static str {
    match kind {
        squish_protocol::OperationKind::New => "new",
        squish_protocol::OperationKind::Build => "build",
        squish_protocol::OperationKind::Format => "format",
        squish_protocol::OperationKind::Add => "add",
        squish_protocol::OperationKind::Remove => "remove",
        squish_protocol::OperationKind::Inspect => "inspect",
    }
}
fn exit_status_name(status: ExitStatus) -> &'static str {
    match status {
        ExitStatus::Success => "success",
        ExitStatus::Failed => "failed",
        ExitStatus::Cancelled => "cancelled",
    }
}
fn short_operation_result(result: &OperationResult) -> String {
    match result {
        OperationResult::Unavailable { kind } => {
            format!("{} unavailable", operation_kind_name(*kind))
        }
        OperationResult::New(result) => short_new_result(result),
        OperationResult::Build(result) => format!("build published={}", result.published.len()),
        OperationResult::Format(result) => format!(
            "format selected={} changed={} check={}",
            result.selected.len(),
            result.changed.len(),
            result.check
        ),
        OperationResult::Add(result) => format!(
            "add dependency={} dry-run={}",
            sanitize(result.dependency.as_str()),
            result.dry_run
        ),
        OperationResult::Remove(result) => format!(
            "remove dependency={} dry-run={}",
            sanitize(result.dependency.as_str()),
            result.dry_run
        ),
        OperationResult::Inspect(result) => format!("inspect view={}", inspect_result_name(result)),
    }
}

/// 生成面向日常终端的项目创建摘要，不暴露摘要或事务内部状态。 /
/// Builds the ordinary terminal summary for project creation without exposing digests or transaction internals.
fn human_new_result(result: &NewResult) -> String {
    let files = result
        .created
        .iter()
        .map(|file| sanitize(file.path.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let placement = result.workspace.as_ref().map_or_else(
        || "standalone project".to_owned(),
        |workspace| format!("workspace member {}", sanitize(workspace.member.as_str())),
    );
    format!(
        "created package {} at {} (target {}; files: {}; {}; {})",
        sanitize(result.package.as_str()),
        sanitize(&result.path.as_path().to_string_lossy()),
        sanitize(result.target.as_str()),
        files,
        placement,
        vcs_description(result)
    )
}

/// 生成稳定、紧凑的项目创建生命周期行。 / Builds the stable compact project-creation lifecycle line.
fn short_new_result(result: &NewResult) -> String {
    let files = result
        .created
        .iter()
        .map(|file| sanitize(file.path.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let workspace = result.workspace.as_ref().map_or_else(
        || "standalone".to_owned(),
        |placement| sanitize(placement.member.as_str()),
    );
    format!(
        "new package={} path={} target={} files={} workspace={} vcs={}",
        sanitize(result.package.as_str()),
        sanitize(&result.path.as_path().to_string_lossy()),
        sanitize(result.target.as_str()),
        files,
        workspace,
        vcs_short_name(result)
    )
}

/// 把类型化 VCS 结果投影为简洁的人类描述。 / Projects the typed VCS result into a concise human description.
fn vcs_description(result: &NewResult) -> &'static str {
    match (result.vcs.kind, result.vcs.disposition) {
        (VcsChoice::Git, VcsDisposition::Created) => "Git repository created",
        (VcsChoice::Git, VcsDisposition::Reused) => "enclosing Git repository reused",
        (VcsChoice::None, VcsDisposition::Disabled) => "version control disabled",
        // 协议验证负责拒绝不一致组合；呈现器仍保持总函数以便展示诊断事件。 /
        // Protocol validation rejects inconsistent pairs; rendering stays total for diagnostic streams.
        (VcsChoice::Git, VcsDisposition::Disabled) => "Git disabled",
        (VcsChoice::None, VcsDisposition::Created) => "repository created",
        (VcsChoice::None, VcsDisposition::Reused) => "enclosing repository reused",
    }
}

/// 把类型化 VCS 结果投影为稳定短标签。 / Projects the typed VCS result into a stable short label.
fn vcs_short_name(result: &NewResult) -> &'static str {
    match (result.vcs.kind, result.vcs.disposition) {
        (VcsChoice::Git, VcsDisposition::Created) => "git-created",
        (VcsChoice::Git, VcsDisposition::Reused) => "git-reused",
        (VcsChoice::None, VcsDisposition::Disabled) => "none",
        (VcsChoice::Git, VcsDisposition::Disabled) => "git-disabled",
        (VcsChoice::None, VcsDisposition::Created) => "repository-created",
        (VcsChoice::None, VcsDisposition::Reused) => "repository-reused",
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

/// 生成适合终端关联问题的短显示指纹，不改变任何缓存身份。 / Produces a short display
/// fingerprint suitable for correlating terminal output without changing any cache identity.
fn format_fingerprint(digest: &Digest) -> String {
    let full = format_digest(digest);
    let Some((algorithm, hex)) = full.split_once(':') else {
        return full;
    };
    format!("{algorithm}:{}", &hex[..hex.len().min(12)])
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
        ActionKeyId, ActionTotals, Artifact, ArtifactId, CacheInspection, CachedAction,
        CreatedProjectFile, Diagnostic, DiagnosticId, Digest, DigestAlgorithm, Event, EventPayload,
        ExitStatus, InspectResult, InvocationId, IrInspection, JobId, JobSummary, LinkInspection,
        NewPackageName, NewResult, OpaqueSourceId, OperationKind, OperationResult, PackageName,
        PlanDigest, PlanId, PlanInspection, PlanMode, PlannedAction, PlanningAttemptId,
        PlanningStepId, PlanningStepKind, ProjectDestination, ProjectFilePath, ProjectInspection,
        ProvenanceInspection, SourceInspection, TargetName, Timing, VcsChoice, VcsDisposition,
        VcsResult, WorkspacePlacement,
    };

    use super::*;

    thread_local! {
        static TEST_TERMINAL_WIDTH: Cell<Option<usize>> = const { Cell::new(None) };
        static TEST_WIDTH_READS: Cell<usize> = const { Cell::new(0) };
    }

    fn injected_terminal_width() -> Option<usize> {
        TEST_WIDTH_READS.set(TEST_WIDTH_READS.get() + 1);
        TEST_TERMINAL_WIDTH.get()
    }

    fn set_injected_terminal_width(width: Option<usize>) {
        TEST_TERMINAL_WIDTH.set(width);
        TEST_WIDTH_READS.set(0);
    }

    fn interactive_capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        }
    }

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

    fn render_with_verbosity(events: &[Event], verbosity: Verbosity) -> String {
        let mut renderer = HumanRenderer::new(
            Vec::new(),
            TerminalCapabilities::plain(),
            Environment::default(),
            PresentationOptions {
                verbosity,
                ..PresentationOptions::default()
            },
        );
        for event in events {
            renderer.render(event).unwrap();
        }
        renderer.finish().unwrap();
        String::from_utf8(renderer.into_inner()).unwrap()
    }

    fn artifact(id_value: &str, uri: &str) -> Artifact {
        Artifact {
            id: id::<ArtifactId>(id_value),
            kind: ArtifactKind::BinaryIr,
            uri: uri.to_owned(),
            size: 42,
            digest: Digest::new(DigestAlgorithm::Sha256, vec![7; 32]).unwrap(),
        }
    }

    fn new_result() -> NewResult {
        NewResult {
            package: id::<NewPackageName>("demo"),
            path: ProjectDestination::new("/workspace/demo"),
            manifest: ProjectDestination::new("/workspace/demo/xmlsquish.toml"),
            target: id::<TargetName>("prompt"),
            created: [".gitignore", "src/prompt.xml", "xmlsquish.toml"]
                .into_iter()
                .map(|path| CreatedProjectFile {
                    path: ProjectFilePath::new(path).unwrap(),
                    digest: Digest::new(DigestAlgorithm::Blake3, vec![7; 32]).unwrap(),
                    size: 42,
                })
                .collect(),
            workspace: Some(WorkspacePlacement {
                manifest: ProjectDestination::new("/workspace/xmlsquish.toml"),
                member: ProjectFilePath::new("tools/demo").unwrap(),
            }),
            vcs: VcsResult {
                kind: VcsChoice::Git,
                disposition: VcsDisposition::Reused,
            },
        }
    }

    #[test]
    fn new_result_has_exact_human_project_summary() {
        let output = render_plain(&[event(
            0,
            EventPayload::OperationCompleted {
                job: id::<JobId>("new"),
                result: OperationResult::New(new_result()),
            },
        )]);

        assert_eq!(
            output,
            "Result: created package demo at /workspace/demo (target prompt; files: .gitignore, src/prompt.xml, xmlsquish.toml; workspace member tools/demo; enclosing Git repository reused)\n"
        );
    }

    #[test]
    fn new_result_has_stable_short_summary_and_labels() {
        let output = render_with_verbosity(
            &[event(
                0,
                EventPayload::OperationCompleted {
                    job: id::<JobId>("new"),
                    result: OperationResult::New(new_result()),
                },
            )],
            Verbosity::Short,
        );

        assert_eq!(
            output,
            "result new package=demo path=/workspace/demo target=prompt files=.gitignore,src/prompt.xml,xmlsquish.toml workspace=tools/demo vcs=git-reused\n"
        );
        assert_eq!(operation_kind_name(OperationKind::New), "new");
        assert_eq!(
            action_kind_name(ActionKind::CreateProject),
            "create-project"
        );
    }

    #[test]
    fn new_result_reports_standalone_and_disabled_vcs_without_internals() {
        let mut result = new_result();
        result.workspace = None;
        result.vcs = VcsResult {
            kind: VcsChoice::None,
            disposition: VcsDisposition::Disabled,
        };

        let output = render_plain(&[event(
            0,
            EventPayload::OperationCompleted {
                job: id::<JobId>("new"),
                result: OperationResult::New(result),
            },
        )]);

        assert!(output.contains("standalone project; version control disabled"));
        assert!(!output.contains("blake3"));
        assert!(!output.contains("transaction"));
    }

    fn render_inspect(result: &InspectResult, width: Option<usize>) -> String {
        let terminal = FixedTerminal::new(TerminalCapabilities::plain()).with_width(width);
        let mut renderer = InspectHumanRenderer::with_terminal(Vec::new(), terminal);
        renderer.render(result).unwrap();
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
            "Planning\nerror[PLAN001] cannot select a target (orchestrate); help: fix the manifest\nResult: build result unavailable\nFailed: 0 succeeded, 0 failed, 0 blocked, 0 cancelled, 0 cached (12 ms)\n"
        );
    }

    #[test]
    fn short_lifecycle_is_distinct_compact_and_preserves_failures() {
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
        let short = render_with_verbosity(&events, Verbosity::Short);
        assert_eq!(
            short,
            "plan\nerror[PLAN001] cannot select a target\nresult build unavailable\ndone status=failed ok=0 failed=0 blocked=0 cancelled=0 cached=0 12ms\n"
        );
        assert_ne!(short, render_with_verbosity(&events, Verbosity::Normal));
        assert!(!short.contains('\x1b'));
        assert!(!short.contains('\r'));
    }

    #[test]
    fn human_detail_tiers_hide_internals_but_keep_cache_and_artifact_value() {
        let job = id::<JobId>("job-0123456789abcdef0123456789abcdef");
        let attempt = id::<PlanningAttemptId>("attempt-0123456789abcdef");
        let plan = id::<PlanId>("plan-0123456789abcdef");
        let action = id::<ActionId>("action-0123456789abcdef");
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
                    digest: plan_digest(5),
                    mode: PlanMode::Execute,
                    actions: 1,
                    issues: 0,
                },
            ),
            event(
                2,
                EventPayload::ActionDeclared {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: action.clone(),
                    kind: ActionKind::Compile,
                    dependencies: vec![],
                },
            ),
            event(
                3,
                EventPayload::ActionStarted {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: action.clone(),
                },
            ),
            event(
                4,
                EventPayload::CacheHit {
                    job: job.clone(),
                    plan: plan.clone(),
                    action: action.clone(),
                    cache: CacheKind::Local,
                    digest: Digest::new(DigestAlgorithm::Blake3, vec![6; 32]).unwrap(),
                    action_key: Some(id::<ActionKeyId>("key-0123456789abcdef")),
                    outputs: vec![artifact("cached-output", "target/prompt.xsir")],
                },
            ),
            event(
                5,
                EventPayload::ActionSucceeded {
                    job: job.clone(),
                    plan,
                    action,
                    timing: Timing { elapsed_ms: 9 },
                    artifacts: vec![artifact("output-0123456789abcdef", "target/prompt.xsir")],
                },
            ),
            event(
                6,
                EventPayload::JobFinished(JobSummary {
                    job,
                    totals: ActionTotals {
                        succeeded: 1,
                        ..ActionTotals::default()
                    },
                    root_failures: 0,
                    cache_hits: 1,
                    timing: Timing { elapsed_ms: 11 },
                    status: ExitStatus::Success,
                }),
            ),
        ];

        let normal = render_with_verbosity(&events, Verbosity::Normal);
        assert_eq!(
            normal,
            "Planning\nPlanned: 1 actions, 0 issues (execute)\nRunning compile\nCached compile (local, 1 outputs)\nFinished compile (9 ms)\nProduced binary-ir target/prompt.xsir (42 bytes)\nCompleted: 1 succeeded, 0 failed, 0 blocked, 0 cancelled, 1 cached (11 ms)\n"
        );
        assert!(!normal.contains("sha256"));
        assert!(!normal.contains("blake3"));
        assert!(!normal.contains("0123456789abcdef"));

        let verbose = render_with_verbosity(&events, Verbosity::Verbose);
        assert_eq!(
            verbose,
            "Planning\nPlanned: 1 actions, 0 issues (execute)\nRunning compile\nCached compile (local, blake3:060606060606, 1 outputs)\nFinished compile (9 ms)\nProduced binary-ir target/prompt.xsir (42 bytes, sha256:070707070707)\nCompleted: 1 succeeded, 0 failed, 0 blocked, 0 cancelled, 1 cached (11 ms)\n"
        );
        assert!(!verbose.contains("0123456789abcdef"));
        assert!(!verbose.contains("0606060606060606"));

        let short = render_with_verbosity(&events, Verbosity::Short);
        assert!(short.contains("cache:local compile outputs=1"));
        assert!(!short.contains("sha256"));
        assert!(!short.contains("blake3"));
        assert!(!short.contains("0123456789abcdef"));

        let trace = render_with_verbosity(&events, Verbosity::Trace);
        assert!(trace.contains("job-0123456789abcdef0123456789abcdef"));
        assert!(trace.contains("action-0123456789abcdef"));
        assert!(
            trace.contains(
                "blake3:0606060606060606060606060606060606060606060606060606060606060606"
            )
        );
        assert!(
            trace.contains(
                "sha256:0707070707070707070707070707070707070707070707070707070707070707"
            )
        );
    }

    #[test]
    fn typed_inspect_project_plan_and_cache_snapshots_cover_empty_collections() {
        let empty_project = InspectResult::Project(ProjectInspection {
            packages: vec![],
            targets: vec![],
        });
        assert_eq!(
            render_inspect(&empty_project, None),
            "Project\n  Packages (0)\n    (none)\n  Targets (0)\n    (none)\n"
        );

        let plan = InspectResult::Plan(PlanInspection {
            job: id::<JobId>("build"),
            plan: id::<PlanId>("plan-1"),
            digest: plan_digest(8),
            mode: PlanMode::ReportOnly,
            actions: vec![PlannedAction {
                action: id::<ActionId>("compile-chat"),
                kind: ActionKind::Compile,
                action_key: None,
                dependencies: vec![],
            }],
        });
        let plan_output = render_inspect(&plan, None);
        assert_eq!(
            plan_output,
            "Plan\n  Job: build\n  Identity: plan-1\n  Digest: sha256:0808080808080808080808080808080808080808080808080808080808080808\n  Mode: report\n  Actions (1)\n    compile compile-chat\n      Action key: (not materialized)\n      Depends on (0)\n        (none)\n"
        );

        let empty_cache = InspectResult::Cache(CacheInspection { actions: vec![] });
        assert_eq!(
            render_inspect(&empty_cache, None),
            "Cache\n  Actions (0)\n    (none)\n"
        );
        let cache = InspectResult::Cache(CacheInspection {
            actions: vec![CachedAction {
                action_key: id::<ActionKeyId>("compile-key"),
                result_digest: Digest::new(DigestAlgorithm::Blake3, vec![9; 32]).unwrap(),
                outputs: vec![],
            }],
        });
        let cache_output = render_inspect(&cache, None);
        assert!(cache_output.contains("compile-key"));
        assert!(cache_output.contains("Outputs (0)\n        (none)"));
    }

    #[test]
    fn typed_inspect_artifact_views_are_readable_unicode_safe_and_ansi_free() {
        let ir_artifact = artifact("ir", "cas://模块/\u{1b}[31m非常长的规范身份");
        let cases = [
            (
                "IR",
                InspectResult::Ir(IrInspection {
                    artifact: ir_artifact.clone(),
                }),
            ),
            (
                "Link",
                InspectResult::Link(LinkInspection {
                    target: id::<TargetName>("聊天-target"),
                    link_map: artifact("link-map", "cas://link"),
                }),
            ),
            (
                "Source",
                InspectResult::Source(SourceInspection {
                    source: id::<OpaqueSourceId>("src/聊天.xml"),
                    digest: Digest::new(DigestAlgorithm::Sha256, vec![1; 32]).unwrap(),
                    size: 88,
                }),
            ),
            (
                "Provenance",
                InspectResult::Provenance(ProvenanceInspection {
                    artifact: artifact("prompt", "target/chat.prompt"),
                    evidence: vec![],
                }),
            ),
        ];
        for (heading, result) in cases {
            let output = render_inspect(&result, Some(24));
            assert!(output.starts_with(heading));
            assert!(!output.contains('\x1b'));
            assert!(!output.contains('\r'));
            for line in output.lines() {
                assert!(UnicodeWidthStr::width(line) <= 24, "{line:?}");
                assert!(!line.contains('�'));
            }
        }
        let provenance = render_inspect(
            &InspectResult::Provenance(ProvenanceInspection {
                artifact: artifact("prompt", "target/chat.prompt"),
                evidence: vec![],
            }),
            None,
        );
        assert!(provenance.contains("Evidence (0)\n    (none)"));
    }

    #[test]
    fn typed_inspect_nonempty_project_preserves_stable_input_order() {
        let result = InspectResult::Project(ProjectInspection {
            packages: vec![id::<PackageName>("core"), id::<PackageName>("app")],
            targets: vec![id::<TargetName>("alpha"), id::<TargetName>("beta")],
        });
        assert_eq!(
            render_inspect(&result, None),
            "Project\n  Packages (2)\n    - core\n    - app\n  Targets (2)\n    - alpha\n    - beta\n"
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
            "Planning\nPlanned: 1 actions, 0 issues (execute)\nRunning commit-transaction\nSuperseded commit-transaction because project state changed\nSuperseded plan; replanning\nPlanning\nPlanned: 1 actions, 0 issues (execute)\nCached compile (local, 0 outputs)\n"
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
            "Planning\nPlanned: 1 actions, 0 issues (report only)\nReported plan without execution\n"
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
            "Finalizing build catalog\nFinalized build catalog (4 ms)\n"
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
            "Finalizing build catalog\nFailed build catalog (7 ms)\nerror[PLAN001] could not persist terminal build facts (orchestrate); help: fix the manifest\n"
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
    fn cancellation_notice_is_idempotent_and_suppresses_later_progress() {
        let terminal = FixedTerminal::new(TerminalCapabilities {
            is_terminal: true,
            supports_ansi: true,
            supports_dynamic: true,
        })
        .with_width(Some(96));
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            terminal,
            Environment::default(),
            PresentationOptions {
                color: ColorMode::Never,
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            TestClock::default(),
        );
        let job = id::<JobId>("build-cli-1");
        renderer
            .render(&event(
                0,
                EventPayload::PlanningStarted {
                    job: job.clone(),
                    attempt: id::<PlanningAttemptId>("attempt-1"),
                },
            ))
            .unwrap();

        renderer.cancellation_requested().unwrap();
        renderer.cancellation_requested().unwrap();
        renderer
            .render(&event(
                1,
                EventPayload::PlanningStepStarted {
                    job,
                    attempt: id::<PlanningAttemptId>("attempt-1"),
                    step: id::<PlanningStepId>("scan"),
                    kind: PlanningStepKind::Scan,
                },
            ))
            .unwrap();
        renderer.tick().unwrap();
        renderer.finish().unwrap();

        let output = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(
            output
                .matches("Cancelling; Ctrl-C again to force\n")
                .count(),
            1
        );
        let after_notice = output
            .split_once("Cancelling; Ctrl-C again to force\n")
            .unwrap()
            .1;
        assert!(
            after_notice.is_empty(),
            "progress resumed: {after_notice:?}"
        );
    }

    #[test]
    fn system_terminal_refreshes_injected_width_without_changing_capability_snapshot() {
        set_injected_terminal_width(Some(96));
        let terminal =
            SystemTerminal::with_width_probe(interactive_capabilities(), injected_terminal_width);

        assert_eq!(terminal.width(), Some(96));
        TEST_TERMINAL_WIDTH.set(Some(41));
        assert_eq!(terminal.width(), Some(41));
        assert_eq!(terminal.capabilities(), interactive_capabilities());
        assert_eq!(TEST_WIDTH_READS.get(), 2);
    }

    #[test]
    fn unknown_initial_width_disables_dynamic_progress_even_if_width_later_appears() {
        set_injected_terminal_width(None);
        let terminal =
            SystemTerminal::with_width_probe(interactive_capabilities(), injected_terminal_width);
        let mut renderer = HumanRenderer::with_clock_and_terminal(
            Vec::new(),
            terminal,
            Environment::default(),
            PresentationOptions {
                progress: ProgressMode::Always,
                ..PresentationOptions::default()
            },
            TestClock::default(),
        );
        assert!(!renderer.uses_dynamic_progress());

        TEST_TERMINAL_WIDTH.set(Some(80));
        renderer.tick().unwrap();
        assert!(renderer.writer.is_empty());
    }

    #[test]
    fn resize_is_observed_on_tick_and_clear_uses_the_new_width() {
        set_injected_terminal_width(Some(40));
        let clock = TestClock::default();
        let terminal =
            SystemTerminal::with_width_probe(interactive_capabilities(), injected_terminal_width);
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
        let job = id::<JobId>("resize-job");
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
        assert!(renderer.uses_dynamic_progress());
        assert!(
            renderer
                .progress
                .as_ref()
                .is_some_and(|state| state.visible)
        );

        let before_resize = renderer.writer.len();
        TEST_TERMINAL_WIDTH.set(Some(8));
        clock.advance(100);
        renderer.tick().unwrap();
        assert!(
            TEST_WIDTH_READS.get() >= 2,
            "construction and tick must both probe width"
        );
        let repaint = String::from_utf8_lossy(&renderer.writer[before_resize..]);
        assert!(
            repaint.matches(CLEAR_LINE).count() >= 2,
            "a narrower terminal must clear every previously wrapped row: {repaint:?}"
        );

        let before_terminal = renderer.writer.len();
        renderer
            .render(&event(
                2,
                EventPayload::PlanningStepSucceeded {
                    job,
                    attempt,
                    step,
                    timing: Timing { elapsed_ms: 100 },
                },
            ))
            .unwrap();
        let cleared = String::from_utf8_lossy(&renderer.writer[before_terminal..]);
        assert!(cleared.starts_with(CLEAR_LINE));
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
        assert!(output.contains("Running 2 actions"));
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
        assert!(before_terminal.contains("Running 2 actions   0% (0/2)"));
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
        assert!(after_terminal.contains("link  50% (1/2)"));
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
    fn ndjson_preserves_native_new_result_event() {
        let value = event(
            0,
            EventPayload::OperationCompleted {
                job: id::<JobId>("new"),
                result: OperationResult::New(new_result()),
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
