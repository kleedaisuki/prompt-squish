//! 真实伪终端中的交互验收。 / Interactive acceptance in a real pseudoterminal.
//!
//! 这些测试故意启动 Cargo 构建出的真实 `xmlsquish` 进程，并通过 Unix PTY 或
//! Windows ConPTY 观察终端协议。它们不使用产品测试钩子，也不模拟 TTY 能力。
//! These tests intentionally launch Cargo's real `xmlsquish` binary and observe its
//! terminal protocol through a Unix PTY or Windows ConPTY. They use neither production
//! test hooks nor simulated TTY capabilities.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU16, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const INITIAL_SIZE: PtySize = PtySize {
    rows: 30,
    cols: 96,
    pixel_width: 0,
    pixel_height: 0,
};
const NARROW_SIZE: PtySize = PtySize {
    rows: 30,
    cols: 36,
    pixel_width: 0,
    pixel_height: 0,
};
#[cfg(not(windows))]
const EMERGENCY_RESET: &[u8] = b"\x1b[0m\x1b[?25h\r\x1b[2K";
#[cfg(windows)]
const EMERGENCY_RESET: &[u8] = b"\x1b[m\x1b[?25h\r\x1b[K";
const CANCELLATION_NOTICE: &str = "Cancelling; Ctrl-C again to force";
const TARGETS: usize = 1;
const WAIT: Duration = Duration::from_secs(30);

/// 位于仓库 `.temp` 下、离开作用域即删除的唯一验收目录。 / A unique acceptance
/// directory under the repository `.temp` tree that is removed on scope exit.
struct Fixture {
    root: PathBuf,
    project: PathBuf,
}

impl Fixture {
    /// 生成许多独立小源码，而非用一个超大文件伪造长任务。 / Generates many small,
    /// independent sources instead of manufacturing one memory-heavy input.
    fn create(label: &str) -> Self {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = workspace
            .join(".temp")
            .join(format!("pty-{label}-{}-{nonce}", std::process::id()));
        let project = root.join("project");
        fs::create_dir_all(&project).expect("create PTY fixture source directory");

        let mut manifest = String::from(
            "manifest-version = 1\n\n[package]\nname = \"pty-acceptance\"\nversion = \"1.0.0\"\nsource-root = \".\"\n",
        );
        for index in 0..TARGETS {
            manifest.push_str(&format!(
                "\n[target.target-{index:04}]\nentry = \"entry-{index:04}.xml\"\n"
            ));
            fs::write(
                project.join(format!("entry-{index:04}.xml")),
                format!(
                    "<xs:entry xmlns:xs=\"https://xmlsquish.moesegfault.dev/ns\"><Prompt><Item>{index}</Item></Prompt></xs:entry>\n"
                ),
            )
            .expect("write PTY fixture source");
        }
        fs::write(project.join("xmlsquish.toml"), manifest).expect("write PTY fixture manifest");
        Self { root, project }
    }

    /// 为一次独立调用返回隔离的 manager home。 / Returns an isolated manager home
    /// for one invocation.
    fn home(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 持有项目构建根的真实维护锁，使内核停在已呈现的恢复步骤。 /
    /// Holds the project build root's real maintenance lock so the kernel remains in a
    /// rendered recovery step.
    fn hold_maintenance_lock(&self) -> File {
        // The default build root is `<project>/target/xmlsquish`; coordination survives root
        // replacement in a sibling named from the root leaf. 默认构建根是
        // `<project>/target/xmlsquish`；协调锁使用根叶名命名的兄弟文件，以便在整体替换根后仍然存活。
        let lock_path = self.project.join("target/.xmlsquish.xmlsquish.lock");
        fs::create_dir_all(lock_path.parent().unwrap()).expect("create maintenance lock directory");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .expect("open project maintenance lock");
        lock.lock_exclusive()
            .expect("hold project maintenance lock");
        lock
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 带通知的 PTY 字节快照。 / A PTY byte snapshot with arrival notification.
#[derive(Default)]
struct CapturedOutput {
    bytes: Mutex<Vec<u8>>,
    changed: Condvar,
}

/// 一个真实 PTY 子进程及其并发输出捕获。 / A real PTY child with concurrent output capture.
struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    output: Arc<CapturedOutput>,
    columns: Arc<AtomicU16>,
    reader: Option<thread::JoinHandle<()>>,
}

impl PtySession {
    /// 在原生 PTY（Windows 为 ConPTY）中启动真实二进制。 / Spawns the real binary in
    /// the native PTY (ConPTY on Windows).
    fn spawn(project: &Path, home: &Path) -> Self {
        let pair = native_pty_system()
            .openpty(INITIAL_SIZE)
            .expect("open native PTY");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_xmlsquish"));
        command.args([
            "build",
            "--jobs=1",
            "--color=always",
            "--progress=always",
            "-vv",
        ]);
        command.cwd(project);
        command.env("XMLSQUISH_HOME", home);
        command.env("TERM", "xterm-256color");
        command.env_remove("NO_COLOR");

        let child = pair
            .slave
            .spawn_command(command)
            .expect("spawn xmlsquish in PTY");
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let writer = Arc::new(Mutex::new(
            pair.master.take_writer().expect("take PTY writer"),
        ));
        let output = Arc::new(CapturedOutput::default());
        let captured = Arc::clone(&output);
        // The reader must not own the input pipe. On pre-24H2 ConPTY,
        // ClosePseudoConsole may wait for that pipe to close while the reader waits for output
        // EOF, forming an unbounded shutdown cycle. A weak reference still answers live cursor
        // queries but lets `finish_output` close stdin before closing the pseudoconsole.
        // 读取线程不能拥有输入管道。在 24H2 之前的 ConPTY 上，ClosePseudoConsole 可能
        // 等待输入管道关闭，而读取线程又等待输出 EOF，从而形成无界关闭环。弱引用仍能
        // 在会话存活时回答光标查询，同时允许 `finish_output` 先关闭 stdin。
        let terminal_input = Arc::downgrade(&writer);
        let columns = Arc::new(AtomicU16::new(INITIAL_SIZE.cols));
        let reported_columns = Arc::clone(&columns);
        let reader = thread::spawn(move || {
            let mut buffer = [0_u8; 8 * 1024];
            let mut pending = Vec::new();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let bytes = &buffer[..read];
                        captured
                            .bytes
                            .lock()
                            .expect("PTY output lock")
                            .extend_from_slice(bytes);
                        captured.changed.notify_all();
                        pending.extend_from_slice(bytes);
                        while let Some(position) = find_bytes(&pending, b"\x1b[6n") {
                            // ConPTY translates GetConsoleScreenBufferInfo synchronization into
                            // a device-status query. A PTY is only transport, so this tiny terminal
                            // response is required to let the real Windows API call complete.
                            // ConPTY 会把控制台信息同步转换成设备状态查询；PTY 只是传输层，
                            // 因此这里必须像真实终端一样回答，不能把产品进程误判为卡死。
                            let column = reported_columns.load(Ordering::Acquire).max(1);
                            let Some(terminal_input) = terminal_input.upgrade() else {
                                return;
                            };
                            let mut terminal_input = terminal_input.lock().expect("PTY input lock");
                            write!(terminal_input, "\x1b[1;{column}R")
                                .expect("answer terminal status query");
                            terminal_input
                                .flush()
                                .expect("flush terminal status response");
                            pending.drain(..position + 4);
                        }
                        if pending.len() > 3 {
                            let keep = pending.len() - 3;
                            pending.drain(..keep);
                        }
                    }
                }
            }
        });

        Self {
            master: Some(pair.master),
            writer: Some(writer),
            child: Some(child),
            output,
            columns,
            reader: Some(reader),
        }
    }

    /// 返回当前输出快照。 / Returns the current output snapshot.
    fn bytes(&self) -> Vec<u8> {
        self.output.bytes.lock().expect("PTY output lock").clone()
    }

    /// 向终端输入一个控制字符并立即刷新。 / Writes and flushes one terminal control character.
    fn control_c(&self) {
        let mut writer = self
            .writer
            .as_ref()
            .expect("PTY writer is open")
            .lock()
            .expect("PTY input lock");
        writer.write_all(&[0x03]).expect("write Ctrl-C to PTY");
        writer.flush().expect("flush Ctrl-C to PTY");
    }

    /// 调整真实 PTY，同时更新终端状态查询的应答。 / Resizes the real PTY and updates
    /// the terminal's device-status response.
    fn resize(&self, size: PtySize) {
        self.master
            .as_ref()
            .expect("PTY master is open")
            .resize(size)
            .expect("resize native PTY");
        self.columns.store(size.cols, Ordering::Release);
    }

    /// 有界等待子进程并返回退出码。 / Waits for the child with a bound and returns its exit code.
    fn wait(&mut self, timeout: Duration) -> u32 {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("PTY child is available")
                .try_wait()
                .expect("poll PTY child")
            {
                return status.exit_code();
            }
            if Instant::now() >= deadline {
                let _ = self.child.as_mut().expect("PTY child is available").kill();
                let bytes = self.bytes();
                panic!(
                    "PTY child did not exit; tail: {:?}",
                    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(2_000)..])
                );
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// 关闭 PTY 端点并汇合读取线程，保证终态字节已全部进入快照。 / Closes PTY
    /// endpoints and joins the reader so all terminal bytes precede the returned snapshot.
    fn finish_output(&mut self) -> Vec<u8> {
        self.writer.take();
        self.master.take();
        if let Some(reader) = self.reader.take() {
            reader.join().expect("join PTY reader");
        }
        self.bytes()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child
            && child.try_wait().ok().flatten().is_none()
        {
            let _ = child.kill();
        }
        self.writer.take();
        self.master.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// 真实交互会动态重绘、响应 resize，并在恢复锁释放后协作取消。
/// Real interaction dynamically repaints, responds to resize, and cooperatively cancels after
/// the recovery lock is released.
#[test]
fn interactive_progress_resize_and_cooperative_interrupt_are_real() {
    let fixture = Fixture::create("cooperative");
    let home = fixture.home("home");
    let recovery_lock = fixture.hold_maintenance_lock();
    let mut session = PtySession::spawn(&fixture.project, &home);

    let wide = wait_for_blocked_recovery(&session, WAIT);
    assert!(
        strip_ansi(&wide).starts_with("Recovering (build-cli-"),
        "wide terminal omitted the live recovery phase"
    );
    let before_resize = session.bytes().len();
    session.resize(NARROW_SIZE);
    let frame = wait_for_recovery_frame_after(&session, before_resize, WAIT);
    let narrow = strip_ansi(&frame);
    assert!(
        narrow.starts_with("Recovering")
            && (narrow.contains("...") || narrow.contains("(build-cli-")),
        "resize lost the live recovery phase: {:?}",
        String::from_utf8_lossy(&frame)
    );

    let before_interrupt = session.bytes().len();
    session.control_c();
    wait_for_cooperative_cancel_after(&session, before_interrupt, WAIT);
    drop(recovery_lock);
    assert_eq!(session.wait(WAIT), 130);
    let raw = session.finish_output();
    assert!(
        raw.windows(2).any(|pair| pair == b"\x1b["),
        "ANSI was absent"
    );
    let plain = strip_ansi(&raw);
    assert!(
        plain.contains("Cancelled build-cli-"),
        "cooperative cancellation omitted its terminal summary: {plain}"
    );
    #[cfg(not(windows))]
    assert!(
        find_bytes(&raw[before_interrupt.min(raw.len())..], EMERGENCY_RESET).is_none(),
        "one interrupt unexpectedly used the emergency restoration path"
    );
}

/// 第二次中断会走紧急退出路径，尽力恢复终端并立即以 130 退出。 /
/// A second interrupt takes the emergency-exit path, best-effort restores the terminal, and
/// immediately exits with 130. Exact reset bytes are independently asserted by
/// `interrupt::tests::emergency_restore_writes_the_exact_fixed_sequence` with a fake writer.
/// Unix PTYs deliver the first Ctrl-C as SIGINT and the now-cancellable recovery wait may finish
/// before another byte can be observed; the deterministic handler unit test owns that boundary.
/// Unix PTY 会把第一次 Ctrl-C 直接作为 SIGINT 交付，而现在可取消的恢复等待可能在第二个
/// 字节可观察前就结束；该边界由确定性的 handler 单元测试负责。
#[cfg(windows)]
#[test]
fn second_interrupt_takes_emergency_exit_and_restores_terminal() {
    let fixture = Fixture::create("emergency");
    let home = fixture.home("home");
    let _recovery_lock = fixture.hold_maintenance_lock();
    let mut session = PtySession::spawn(&fixture.project, &home);

    wait_for_blocked_recovery(&session, WAIT);
    let before_first = session.bytes().len();
    session.control_c();
    let before_interrupts = send_second_interrupt_after_notice(&session, before_first, WAIT);
    assert_eq!(session.wait(WAIT), 130);
    let raw = session.finish_output();
    assert_emergency_exit_observed(&raw, before_interrupts);
}

/// ConPTY 是终端仿真器而不是透明管道：它会消费冗余的 SGR reset 与 cursor-show，
/// 并可能把清行恢复折叠为控制台输入模式复位。因此 Windows 上接受清行或 VT 输入模式
/// 退出序列；没有协作终态且退出 130 仍区分紧急路径与第一次中断路径。
/// ConPTY is a terminal emulator rather than a transparent pipe: it consumes redundant SGR
/// reset and cursor-show commands and canonicalizes `CSI 2 K` to `CSI K`. The repeatably
/// observable Windows reset byte is therefore the clear-line sequence after the second input;
/// absence of a cooperative terminal summary plus exit 130 distinguishes the emergency path.
#[cfg(windows)]
fn assert_emergency_exit_observed(raw: &[u8], before_interrupts: usize) {
    let suffix = &raw[before_interrupts.min(raw.len())..];
    let plain = strip_ansi(suffix);
    assert!(
        find_bytes(suffix, EMERGENCY_RESET).is_some()
            || find_bytes(suffix, b"\x1b[K").is_some()
            || find_bytes(suffix, b"\x1b[?9001l").is_some(),
        "ConPTY exposed neither terminal clearing nor input-mode restoration; tail: {:?}",
        String::from_utf8_lossy(&raw[raw.len().saturating_sub(2_000)..])
    );
    assert!(
        !plain.contains(" succeeded,"),
        "second interrupt unexpectedly followed the cooperative completion path: {plain}"
    );
    assert!(
        !plain.contains("publish build:publish:"),
        "a complete progress frame appeared after the emergency interrupt: {plain}"
    );
}

/// 在输出快照锁内确认首次提示并发送第二次中断。 /
/// Confirms the first notice and sends the second interrupt while holding the output-snapshot
/// lock, so the reader cannot append a cooperative terminal summary between those observations.
#[cfg(windows)]
fn send_second_interrupt_after_notice(
    session: &PtySession,
    offset: usize,
    timeout: Duration,
) -> usize {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let plain = strip_ansi(&bytes[offset.min(bytes.len())..]);
        if plain
            .split_inclusive(['\r', '\n'])
            .any(|line| line.trim() == CANCELLATION_NOTICE)
        {
            let before_second_interrupt = bytes.len();
            session.control_c();
            return before_second_interrupt;
        }
        assert!(
            Instant::now() < deadline,
            "no complete cancellation notice after first interrupt; tail: {:?}",
            &plain[plain.len().saturating_sub(2_000)..]
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        bytes = session
            .output
            .changed
            .wait_timeout(bytes, remaining)
            .expect("PTY output wait")
            .0;
    }
}

/// 等待第一次中断产生提示或已完成的协作取消。 /
/// Waits for either the first-interrupt notice or a completed cooperative cancellation.
fn wait_for_cooperative_cancel_after(session: &PtySession, offset: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let plain = strip_ansi(&bytes[offset.min(bytes.len())..]);
        if plain.contains(CANCELLATION_NOTICE) || plain.contains("Cancelled build-cli-") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no cooperative cancellation after first interrupt; tail: {:?}",
            &plain[plain.len().saturating_sub(2_000)..]
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        bytes = session
            .output
            .changed
            .wait_timeout(bytes, remaining)
            .expect("PTY output wait")
            .0;
    }
}

/// 等待进入被项目锁阻塞的恢复阶段。 / Waits for recovery blocked on the project lock.
fn wait_for_blocked_recovery(session: &PtySession, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let plain = strip_ansi(&bytes);
        if let Some(frame) = recovery_frame(&plain) {
            return frame.into_bytes();
        }
        assert!(
            Instant::now() < deadline,
            "no live recovery frame; tail: {:?}",
            &plain[plain.len().saturating_sub(2_000)..]
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        bytes = session
            .output
            .changed
            .wait_timeout(bytes, remaining)
            .expect("PTY output wait")
            .0;
    }
}

fn recovery_frame(transcript: &str) -> Option<String> {
    let start = transcript.rfind("Recovering")?;
    let relative_end = transcript[start..].find(')')?;
    Some(transcript[start..=start + relative_end].to_owned())
}

/// 等待 offset 之后语义完整的恢复进度帧。 / Waits for a complete recovery progress frame after an offset.
fn wait_for_recovery_frame_after(
    session: &PtySession,
    offset: usize,
    timeout: Duration,
) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let suffix = &bytes[offset.min(bytes.len())..];
        let plain = strip_ansi(suffix);
        if let Some(frame) = recovery_frame(&plain) {
            return frame.into_bytes();
        }
        assert!(
            Instant::now() < deadline,
            "no complete recovery frame followed offset; tail: {:?}",
            String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(2_000)..])
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        bytes = session
            .output
            .changed
            .wait_timeout(bytes, remaining)
            .expect("PTY output wait")
            .0;
    }
}

/// 移除 CSI 序列以检查稳定的人类语义。 / Removes CSI sequences for stable human semantics.
fn strip_ansi(bytes: &[u8]) -> String {
    let mut plain = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            plain.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&plain).into_owned()
}

/// 在字节流中查找子串而不假设 UTF-8 分块边界。 / Finds a byte substring without
/// assuming UTF-8 chunk boundaries.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}
