//! 双阶段交互中断协调。 / Two-stage interactive interrupt coordination.
//!
//! 信号回调只修改原子状态、设置协作式取消令牌，并在第二次中断时调用两个
//! 无锁端口。它不会获取呈现器或标准错误锁。 / The signal callback only changes
//! atomic state, latches the cooperative cancellation token, and invokes two
//! lock-free ports on the second interrupt. It never acquires renderer or stderr locks.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use squish_kernel::CancellationToken;

/// 可交给 OS 中断安装器的无锁回调。 / Lock-free callback accepted by an OS interrupt installer.
pub(crate) type InterruptHandler = Box<dyn Fn() + Send + Sync + 'static>;

const ACTIVE: u8 = 0;
const CANCELLING: u8 = 1;
const FORCE_EXITING: u8 = 2;
const INTERRUPTED_EXIT_CODE: u8 = 130;
const EMERGENCY_RESET: &[u8] = b"\x1b[0m\x1b[?25h\r\x1b[2K";

/// 中断协调器的可观察状态。 / Observable state of the interrupt coordinator.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterruptState {
    /// 尚未收到中断。 / No interrupt has been received.
    Active,
    /// 已请求协作式取消。 / Cooperative cancellation has been requested.
    Cancelling,
    /// 已开始不可等待的强制退出。 / Non-waiting forced exit has begun.
    ForceExiting,
}

/// 第二次中断时使用的无锁终端恢复端口。 / Lock-free terminal restoration port used on the second interrupt.
pub(crate) trait EmergencyRestore: Send + Sync {
    /// 尽力恢复终端；实现必须有界、幂等且不得获取全局标准错误锁。
    /// Best-effort terminal restoration; implementations must be bounded,
    /// idempotent, and must not acquire the global stderr lock.
    fn restore(&self);
}

/// 第二次中断时使用的进程终止端口。 / Process termination port used on the second interrupt.
pub(crate) trait ProcessTerminator: Send + Sync {
    /// 立即以给定代码终止；测试替身允许返回。 / Immediately terminates with the code; test fakes may return.
    fn exit(&self, code: u8);
}

/// 拥有唯一三态中断策略及其副作用。 / Owns the sole three-state interrupt policy and its effects.
pub(crate) struct InterruptCoordinator {
    state: AtomicU8,
    cancellation: CancellationToken,
    terminal: Arc<dyn EmergencyRestore>,
    terminator: Arc<dyn ProcessTerminator>,
}

impl InterruptCoordinator {
    /// 组合协调器；调用方应将同一实例放入唯一的 OS 信号处理器。
    /// Composes the coordinator; the caller should place the same instance in
    /// the process's sole OS signal handler.
    pub(crate) fn new(
        cancellation: CancellationToken,
        terminal: Arc<dyn EmergencyRestore>,
        terminator: Arc<dyn ProcessTerminator>,
    ) -> Self {
        Self {
            state: AtomicU8::new(ACTIVE),
            cancellation,
            terminal,
            terminator,
        }
    }

    /// 返回传给内核的同一协作式取消令牌。 / Returns the same cooperative token passed to the kernel.
    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// 通过可注入安装端口注册此协调器。 / Registers this coordinator through an injectable installation port.
    ///
    /// 该边界让组合根能够把安装失败映射为稳定启动诊断，而测试无需篡改真实进程的
    /// 唯一信号处理器。 / This boundary lets the composition root map installation failure to
    /// a stable bootstrap diagnostic without tests mutating the process-global signal handler.
    pub(crate) fn install_with<E>(
        self: &Arc<Self>,
        install: impl FnOnce(InterruptHandler) -> Result<(), E>,
    ) -> Result<(), E> {
        let coordinator = Arc::clone(self);
        install(Box::new(move || coordinator.on_interrupt()))
    }

    /// 返回当前中断状态。 / Returns the current interrupt state.
    #[cfg(test)]
    pub(crate) fn state(&self) -> InterruptState {
        match self.state.load(Ordering::Acquire) {
            ACTIVE => InterruptState::Active,
            CANCELLING => InterruptState::Cancelling,
            FORCE_EXITING => InterruptState::ForceExiting,
            _ => unreachable!("interrupt state is written only by this module"),
        }
    }

    /// 处理一次中断投递。 / Handles one interrupt delivery.
    ///
    /// 第一次仅锁存取消；第二次依次恢复终端并退出；后续调用无副作用。
    /// The first delivery only latches cancellation; the second restores then
    /// exits; later deliveries have no effect.
    pub(crate) fn on_interrupt(&self) {
        loop {
            match self.state.load(Ordering::Acquire) {
                ACTIVE => {
                    // Cancel before publishing `Cancelling`: a concurrent second delivery
                    // can therefore never force-exit before cancellation is latched.
                    // 在发布 `Cancelling` 前取消，保证并发第二次投递不会抢先强退。
                    self.cancellation.cancel();
                    if self
                        .state
                        .compare_exchange(ACTIVE, CANCELLING, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return;
                    }
                }
                CANCELLING => {
                    if self
                        .state
                        .compare_exchange(
                            CANCELLING,
                            FORCE_EXITING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        self.terminal.restore();
                        self.terminator.exit(INTERRUPTED_EXIT_CODE);
                        return;
                    }
                }
                FORCE_EXITING => return,
                _ => unreachable!("interrupt state is written only by this module"),
            }
        }
    }
}

/// 对继承的标准错误设备执行固定恢复序列。 / Writes a fixed restoration sequence to inherited stderr.
pub(crate) struct StderrEmergencyRestore {
    enabled: bool,
}

impl StderrEmergencyRestore {
    /// 从标准错误终端能力快照建立恢复租约。 / Captures the restoration lease from stderr capabilities.
    pub(crate) const fn capture(ansi_dynamic_terminal: bool) -> Self {
        Self {
            enabled: ansi_dynamic_terminal,
        }
    }
}

impl EmergencyRestore for StderrEmergencyRestore {
    fn restore(&self) {
        if self.enabled {
            platform::write_stderr_once(EMERGENCY_RESET);
        }
    }
}

/// 使用标准库立即终止进程。 / Immediately terminates the process through the standard library.
pub(crate) struct StdProcessTerminator;

impl ProcessTerminator for StdProcessTerminator {
    fn exit(&self, code: u8) {
        std::process::exit(i32::from(code));
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::{c_int, c_void};

    unsafe extern "C" {
        fn write(fd: c_int, buffer: *const c_void, count: usize) -> isize;
    }

    pub(super) fn write_stderr_once(bytes: &'static [u8]) {
        // SAFETY / 安全性：`bytes` 是在调用期间有效的静态不可变缓冲区；文件描述符 2
        // 只是借用的继承 stderr，本函数既不关闭也不保留它。单次 write 的短写或失败被刻意忽略。
        // `bytes` is a static immutable buffer valid for the call; fd 2 is only a
        // borrowed inherited stderr and is neither closed nor retained. A short or
        // failed single write is intentionally ignored.
        unsafe {
            let _ = write(2, bytes.as_ptr().cast::<c_void>(), bytes.len());
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    type Handle = *mut c_void;
    const STD_ERROR_HANDLE: u32 = u32::MAX - 11;
    const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;

    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> Handle;
        fn WriteFile(
            handle: Handle,
            buffer: *const c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

    pub(super) fn write_stderr_once(bytes: &'static [u8]) {
        // SAFETY / 安全性：标准错误句柄仅在两次平台调用期间借用；静态缓冲区在调用期间有效，
        // `written` 是有效的栈输出指针，且不使用异步 OVERLAPPED。句柄不会被关闭或保存。
        // The stderr handle is borrowed only across these calls; the static buffer
        // and stack output pointer remain valid, no asynchronous OVERLAPPED is used,
        // and the handle is neither closed nor retained.
        unsafe {
            let handle = GetStdHandle(STD_ERROR_HANDLE);
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                return;
            }
            let mut written = 0;
            let _ = WriteFile(
                handle,
                bytes.as_ptr().cast::<c_void>(),
                u32::try_from(bytes.len()).expect("fixed emergency sequence fits u32"),
                &mut written,
                std::ptr::null_mut(),
            );
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    pub(super) fn write_stderr_once(_bytes: &'static [u8]) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, Mutex};
    use std::thread;

    #[derive(Default)]
    struct CallLog(Mutex<Vec<String>>);

    impl CallLog {
        fn entries(&self) -> Vec<String> {
            self.0.lock().expect("call log lock").clone()
        }
    }

    struct FakeRestore(Arc<CallLog>);

    impl EmergencyRestore for FakeRestore {
        fn restore(&self) {
            self.0
                .0
                .lock()
                .expect("call log lock")
                .push("restore".into());
        }
    }

    struct FakeTerminator(Arc<CallLog>);

    impl ProcessTerminator for FakeTerminator {
        fn exit(&self, code: u8) {
            self.0
                .0
                .lock()
                .expect("call log lock")
                .push(format!("exit:{code}"));
        }
    }

    fn fixture() -> (Arc<InterruptCoordinator>, CancellationToken, Arc<CallLog>) {
        let cancellation = CancellationToken::default();
        let log = Arc::new(CallLog::default());
        let coordinator = Arc::new(InterruptCoordinator::new(
            cancellation.clone(),
            Arc::new(FakeRestore(Arc::clone(&log))),
            Arc::new(FakeTerminator(Arc::clone(&log))),
        ));
        (coordinator, cancellation, log)
    }

    #[test]
    fn deliveries_cancel_then_restore_and_exit_exactly_once() {
        let (coordinator, cancellation, log) = fixture();

        coordinator.on_interrupt();
        assert!(cancellation.is_cancelled());
        assert_eq!(coordinator.state(), InterruptState::Cancelling);
        assert!(log.entries().is_empty());

        coordinator.on_interrupt();
        assert_eq!(coordinator.state(), InterruptState::ForceExiting);
        assert_eq!(log.entries(), ["restore", "exit:130"]);

        coordinator.on_interrupt();
        assert_eq!(log.entries(), ["restore", "exit:130"]);
    }

    #[test]
    fn cancellation_token_is_the_same_latch() {
        let (coordinator, cancellation, _) = fixture();
        let exported = coordinator.cancellation_token();
        coordinator.on_interrupt();
        assert!(cancellation.is_cancelled());
        assert!(exported.is_cancelled());
    }

    #[test]
    fn non_terminal_restore_is_a_no_op() {
        let restore = StderrEmergencyRestore::capture(false);
        assert!(!restore.enabled);
        restore.restore();
        assert!(!restore.enabled);
    }

    #[test]
    fn concurrent_deliveries_have_one_force_effect() {
        const THREADS: usize = 16;
        let (coordinator, cancellation, log) = fixture();
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::new();

        for _ in 0..THREADS {
            let coordinator = Arc::clone(&coordinator);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                coordinator.on_interrupt();
            }));
        }
        for handle in handles {
            handle.join().expect("interrupt delivery thread");
        }

        assert!(cancellation.is_cancelled());
        assert_eq!(coordinator.state(), InterruptState::ForceExiting);
        assert_eq!(log.entries(), ["restore", "exit:130"]);
    }

    #[test]
    fn installer_failure_is_returned_without_changing_state() {
        let (coordinator, cancellation, log) = fixture();
        let result = coordinator.install_with(|_| Err::<(), _>("handler unavailable"));

        assert_eq!(result, Err("handler unavailable"));
        assert!(!cancellation.is_cancelled());
        assert_eq!(coordinator.state(), InterruptState::Active);
        assert!(log.entries().is_empty());
    }
}
