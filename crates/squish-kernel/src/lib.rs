//! prompt-squish 的领域无关微内核。 / Domain-neutral microkernel for prompt-squish.
//!
//! 能力通过一个显式静态切片注册。内核不发现动态插件，也不持有全局服务定位器；
//! 每次调用所需的取消和事件通道都由 [`InvocationContext`] 明确传入。
//! Capabilities are registered through one explicit static slice. The kernel
//! neither discovers dynamic plugins nor owns a global service locator; each
//! invocation receives cancellation and event channels through [`InvocationContext`].

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use squish_protocol::{
    CapabilityId, Diagnostic, DiagnosticId, Event, EventPayload, ExitStatus, InvocationId, Phase,
    Severity,
};

/// 传给能力的领域无关命令。 / Domain-neutral command passed to a capability.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    name: String,
    arguments: BTreeMap<String, serde_json::Value>,
}

impl Command {
    /// 构造具有稳定名称及结构化参数的命令。 / Creates a command with a stable name and structured arguments.
    pub fn new(
        name: impl Into<String>,
        arguments: BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, InvalidCommand> {
        let name = name.into();
        if name.is_empty() {
            return Err(InvalidCommand);
        }
        Ok(Self { name, arguments })
    }

    /// 返回稳定命令名称。 / Returns the stable command name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 返回只读结构化参数。 / Returns the read-only structured arguments.
    pub fn arguments(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.arguments
    }
}

/// 命令名称为空。 / A command name was empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidCommand;

impl fmt::Display for InvalidCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("command name must not be empty")
    }
}

impl std::error::Error for InvalidCommand {}

/// 静态能力的可展示元数据。 / Display metadata for a static capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDescriptor {
    /// 跨版本稳定的能力 ID。 / Capability ID stable across versions.
    pub id: &'static str,
    /// 此能力处理的唯一命令名。 / Unique command name handled by this capability.
    pub command: &'static str,
    /// 面向用户的一行说明。 / One-line user-facing summary.
    pub summary: &'static str,
}

/// 能力执行失败；内核会把它转换成诊断和失败状态。 / Capability failure converted by the kernel into a diagnostic and failed status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityFailure {
    /// 稳定诊断代码。 / Stable diagnostic code.
    pub code: String,
    /// 面向用户的错误说明。 / User-facing error message.
    pub message: String,
}

impl CapabilityFailure {
    /// 创建可被统一调度器报告的失败。 / Creates a failure reportable by the unified dispatcher.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// 编译期链接、启动时显式注册的领域能力。 / Domain capability linked at compile time and explicitly registered at startup.
pub trait Capability: Sync {
    /// 返回静态能力描述。 / Returns the static capability descriptor.
    fn descriptor(&self) -> &'static CapabilityDescriptor;

    /// 执行命令；长任务应周期性检查 `context.is_cancelled()`。 / Executes a command; long tasks should periodically check `context.is_cancelled()`.
    fn execute(
        &self,
        command: &Command,
        context: &InvocationContext,
    ) -> Result<ExitStatus, CapabilityFailure>;
}

/// 线程安全的事件目的地。 / Thread-safe destination for invocation events.
pub trait EventSink: Send + Sync {
    /// 按收到顺序保存或展示事件。 / Stores or presents an event in receive order.
    fn emit(&self, event: Event) -> Result<(), SinkError>;
}

/// 事件目的地拒绝了事件。 / An event destination rejected an event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SinkError {
    message: String,
}

impl SinkError {
    /// 创建保留底层原因的事件错误。 / Creates an event error preserving its underlying reason.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for SinkError {}

/// 可克隆的协作式取消句柄。 / Cloneable cooperative cancellation handle.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// 请求取消；此操作是幂等的。 / Requests cancellation; this operation is idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// 返回调用者是否已请求取消。 / Returns whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// 单次命令调用的所有横切依赖。 / All cross-cutting dependencies for one command invocation.
#[derive(Clone)]
pub struct InvocationContext {
    id: InvocationId,
    cancellation: CancellationToken,
    events: Arc<dyn EventSink>,
    sequence: Arc<Mutex<u64>>,
}

impl InvocationContext {
    /// 创建不依赖全局状态的调用上下文。 / Creates an invocation context independent of global state.
    pub fn new(
        id: InvocationId,
        cancellation: CancellationToken,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            id,
            cancellation,
            events,
            sequence: Arc::new(Mutex::new(0)),
        }
    }

    /// 返回本次调用的稳定 ID。 / Returns the stable ID of this invocation.
    pub fn id(&self) -> &InvocationId {
        &self.id
    }

    /// 返回调用者是否请求取消。 / Returns whether the caller requested cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// 返回用于向子任务传播取消的句柄。 / Returns a handle for propagating cancellation to child tasks.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// 发布负载并自动分配调用内序号。 / Publishes a payload and assigns its invocation-local sequence.
    pub fn emit(&self, payload: EventPayload) -> Result<(), SinkError> {
        let mut sequence = self
            .sequence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = sequence
            .checked_add(1)
            .ok_or_else(|| SinkError::new("invocation event sequence exhausted"))?;
        self.events
            .emit(Event::new(self.id.clone(), *sequence, payload))?;
        *sequence = next;
        Ok(())
    }
}

/// 静态注册表构造或调度错误。 / Static registry construction or dispatch error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelError {
    /// 两项能力声明了相同能力 ID。 / Two capabilities declared the same capability ID.
    DuplicateCapability(String),
    /// 两项能力声明了相同命令。 / Two capabilities declared the same command.
    DuplicateCommand(String),
    /// 能力描述包含空 ID 或命令。 / A capability descriptor contains an empty ID or command.
    InvalidDescriptor,
    /// 没有能力处理给定命令。 / No capability handles the requested command.
    UnknownCommand(String),
    /// 事件无法送达前端。 / An event could not be delivered to the frontend.
    EventSink(SinkError),
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCapability(id) => write!(formatter, "duplicate capability `{id}`"),
            Self::DuplicateCommand(command) => write!(formatter, "duplicate command `{command}`"),
            Self::InvalidDescriptor => {
                formatter.write_str("capability id and command must not be empty")
            }
            Self::UnknownCommand(command) => write!(formatter, "unknown command `{command}`"),
            Self::EventSink(error) => write!(formatter, "event sink failed: {error}"),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<SinkError> for KernelError {
    fn from(value: SinkError) -> Self {
        Self::EventSink(value)
    }
}

/// 已完成调度的结构化结果。 / Structured result of a completed dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    /// 处理命令的能力。 / Capability that handled the command.
    pub capability: CapabilityId,
    /// 终止状态。 / Terminal status.
    pub status: ExitStatus,
}

/// 对编译期能力切片的已验证视图。 / Validated view over a compile-time capability slice.
///
/// # 示例 / Example
///
/// ```
/// use std::{collections::BTreeMap, sync::Arc};
/// use squish_kernel::{
///     CancellationToken, Capability, CapabilityDescriptor, CapabilityFailure, Command,
///     EventSink, InvocationContext, Kernel, SinkError,
/// };
/// use squish_protocol::{Event, ExitStatus, InvocationId};
///
/// struct Build;
/// static BUILD: Build = Build;
/// static DESCRIPTION: CapabilityDescriptor = CapabilityDescriptor {
///     id: "build",
///     command: "build",
///     summary: "build a project",
/// };
///
/// impl Capability for Build {
///     fn descriptor(&self) -> &'static CapabilityDescriptor { &DESCRIPTION }
///     fn execute(
///         &self,
///         _command: &Command,
///         _context: &InvocationContext,
///     ) -> Result<ExitStatus, CapabilityFailure> {
///         Ok(ExitStatus::Success)
///     }
/// }
///
/// struct IgnoreEvents;
/// impl EventSink for IgnoreEvents {
///     fn emit(&self, _event: Event) -> Result<(), SinkError> { Ok(()) }
/// }
///
/// let capabilities: &[&dyn Capability] = &[&BUILD];
/// let kernel = Kernel::new(capabilities)?;
/// let context = InvocationContext::new(
///     InvocationId::new("example")?,
///     CancellationToken::default(),
///     Arc::new(IgnoreEvents),
/// );
/// let outcome = kernel.dispatch(&Command::new("build", BTreeMap::new())?, &context)?;
/// assert_eq!(outcome.status, ExitStatus::Success);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Kernel<'a> {
    capabilities: &'a [&'a dyn Capability],
}

impl<'a> Kernel<'a> {
    /// 验证并创建显式静态注册表。 / Validates and creates an explicit static registry.
    pub fn new(capabilities: &'a [&'a dyn Capability]) -> Result<Self, KernelError> {
        validate(capabilities)?;
        Ok(Self { capabilities })
    }

    /// 返回静态注册的能力，供帮助和补全界面使用。 / Returns statically registered capabilities for help and completion UIs.
    pub fn capabilities(
        &self,
    ) -> impl ExactSizeIterator<Item = &'static CapabilityDescriptor> + '_ {
        self.capabilities.iter().map(|item| item.descriptor())
    }

    /// 查找能力并通过统一生命周期执行命令。 / Finds a capability and executes a command through the unified lifecycle.
    pub fn dispatch(
        &self,
        command: &Command,
        context: &InvocationContext,
    ) -> Result<DispatchOutcome, KernelError> {
        let capability = self.find(command.name())?;
        let id = CapabilityId::new(capability.descriptor().id)
            .expect("validated capability ID must remain non-empty");
        context.emit(EventPayload::CommandStarted {
            capability: id.clone(),
            command: command.name().to_owned(),
        })?;
        let status = self.execute(capability, command, context)?;
        context.emit(EventPayload::CommandFinished { status })?;
        Ok(DispatchOutcome {
            capability: id,
            status,
        })
    }

    fn find(&self, command: &str) -> Result<&'a dyn Capability, KernelError> {
        self.capabilities
            .iter()
            .copied()
            .find(|item| item.descriptor().command == command)
            .ok_or_else(|| KernelError::UnknownCommand(command.to_owned()))
    }

    fn execute(
        &self,
        capability: &dyn Capability,
        command: &Command,
        context: &InvocationContext,
    ) -> Result<ExitStatus, KernelError> {
        if context.is_cancelled() {
            return Ok(ExitStatus::Cancelled);
        }
        match capability.execute(command, context) {
            Ok(status) => Ok(status),
            Err(failure) => {
                emit_failure(context, failure)?;
                Ok(ExitStatus::Failed)
            }
        }
    }
}

fn validate(capabilities: &[&dyn Capability]) -> Result<(), KernelError> {
    let mut ids = HashSet::with_capacity(capabilities.len());
    let mut commands = HashSet::with_capacity(capabilities.len());
    for capability in capabilities {
        let descriptor = capability.descriptor();
        if descriptor.id.is_empty() || descriptor.command.is_empty() {
            return Err(KernelError::InvalidDescriptor);
        }
        if !ids.insert(descriptor.id) {
            return Err(KernelError::DuplicateCapability(descriptor.id.to_owned()));
        }
        if !commands.insert(descriptor.command) {
            return Err(KernelError::DuplicateCommand(descriptor.command.to_owned()));
        }
    }
    Ok(())
}

fn emit_failure(
    context: &InvocationContext,
    failure: CapabilityFailure,
) -> Result<(), KernelError> {
    let diagnostic = Diagnostic {
        id: DiagnosticId::new(format!("{}:{}", context.id(), failure.code))
            .expect("invocation and diagnostic code produce a non-empty ID"),
        code: failure.code,
        severity: Severity::Error,
        phase: Phase::Orchestrate,
        message: failure.message,
        primary: None,
        related: Vec::new(),
        help: None,
    };
    context.emit(EventPayload::Diagnostic(diagnostic))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static ECHO_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
        id: "test.echo",
        command: "echo",
        summary: "echo test command",
    };

    struct Echo;

    impl Capability for Echo {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &ECHO_DESCRIPTOR
        }

        fn execute(
            &self,
            _command: &Command,
            _context: &InvocationContext,
        ) -> Result<ExitStatus, CapabilityFailure> {
            Ok(ExitStatus::Success)
        }
    }

    static FAIL_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
        id: "test.fail",
        command: "fail",
        summary: "fail test command",
    };

    struct Fail;

    impl Capability for Fail {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &FAIL_DESCRIPTOR
        }

        fn execute(
            &self,
            _command: &Command,
            _context: &InvocationContext,
        ) -> Result<ExitStatus, CapabilityFailure> {
            Err(CapabilityFailure::new("test_failure", "expected failure"))
        }
    }

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<Event>>);

    impl EventSink for RecordingSink {
        fn emit(&self, event: Event) -> Result<(), SinkError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn context(sink: Arc<RecordingSink>, cancellation: CancellationToken) -> InvocationContext {
        InvocationContext::new(InvocationId::new("test-run").unwrap(), cancellation, sink)
    }

    #[test]
    fn dispatch_emits_ordered_lifecycle() {
        static ECHO: Echo = Echo;
        let sink = Arc::new(RecordingSink::default());
        let capabilities: &[&dyn Capability] = &[&ECHO];
        let kernel = Kernel::new(capabilities).unwrap();
        let outcome = kernel
            .dispatch(
                &Command::new("echo", BTreeMap::new()).unwrap(),
                &context(sink.clone(), CancellationToken::default()),
            )
            .unwrap();

        assert_eq!(outcome.status, ExitStatus::Success);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
    }

    #[test]
    fn cancellation_skips_capability_but_finishes_lifecycle() {
        static ECHO: Echo = Echo;
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let sink = Arc::new(RecordingSink::default());
        let capabilities: &[&dyn Capability] = &[&ECHO];
        let outcome = Kernel::new(capabilities)
            .unwrap()
            .dispatch(
                &Command::new("echo", BTreeMap::new()).unwrap(),
                &context(sink.clone(), cancellation),
            )
            .unwrap();

        assert_eq!(outcome.status, ExitStatus::Cancelled);
        assert!(matches!(
            sink.0.lock().unwrap()[1].payload,
            EventPayload::CommandFinished {
                status: ExitStatus::Cancelled
            }
        ));
    }

    #[test]
    fn registry_rejects_duplicate_commands() {
        static ECHO: Echo = Echo;
        let capabilities: &[&dyn Capability] = &[&ECHO, &ECHO];
        let error = Kernel::new(capabilities).err().unwrap();
        assert!(matches!(error, KernelError::DuplicateCapability(_)));
    }

    #[test]
    fn capability_failure_becomes_diagnostic_and_failed_finish() {
        static FAIL: Fail = Fail;
        let sink = Arc::new(RecordingSink::default());
        let capabilities: &[&dyn Capability] = &[&FAIL];
        let outcome = Kernel::new(capabilities)
            .unwrap()
            .dispatch(
                &Command::new("fail", BTreeMap::new()).unwrap(),
                &context(sink.clone(), CancellationToken::default()),
            )
            .unwrap();

        assert_eq!(outcome.status, ExitStatus::Failed);
        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[1].payload, EventPayload::Diagnostic(_)));
        assert!(matches!(
            events[2].payload,
            EventPayload::CommandFinished {
                status: ExitStatus::Failed
            }
        ));
    }

    #[test]
    fn unknown_command_does_not_emit_partial_lifecycle() {
        static ECHO: Echo = Echo;
        let sink = Arc::new(RecordingSink::default());
        let capabilities: &[&dyn Capability] = &[&ECHO];
        let error = Kernel::new(capabilities)
            .unwrap()
            .dispatch(
                &Command::new("missing", BTreeMap::new()).unwrap(),
                &context(sink.clone(), CancellationToken::default()),
            )
            .unwrap_err();

        assert!(matches!(error, KernelError::UnknownCommand(_)));
        assert!(sink.0.lock().unwrap().is_empty());
    }
}
