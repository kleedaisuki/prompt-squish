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

    /// 持有真实发布器恢复锁，使内核停在已呈现的恢复步骤。 /
    /// Holds the real publisher recovery lock so the kernel remains in a rendered recovery step.
    fn hold_recovery_lock(&self) -> File {
        let state = self.project.join("target/xmlsquish/.squish-publish");
        fs::create_dir_all(&state).expect("create publisher state directory");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(state.join("lock"))
            .expect("open publisher recovery lock");
        lock.lock_exclusive().expect("hold publisher recovery lock");
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
    fn control_c(&mut self) {
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

/// 真实交互会动态重绘、响应 resize，并把第一次中断收敛为唯一取消终态。
/// Real interaction dynamically repaints, responds to resize, and converges the first
/// interrupt into one cancellation terminal state.
#[test]
fn interactive_progress_resize_and_cooperative_interrupt_are_real() {
    let fixture = Fixture::create("cooperative");
    let recovery_lock = fixture.hold_recovery_lock();
    let mut session = PtySession::spawn(&fixture.project, &fixture.home("home"));

    let wide = wait_for_blocked_recovery(&session, WAIT);
    assert!(
        visible_frame_width(&wide) > usize::from(NARROW_SIZE.cols - 1),
        "wide frame did not require truncation after resize"
    );
    assert!(
        !strip_ansi(&wide).contains("..."),
        "wide frame was truncated"
    );
    let before_resize = session.bytes().len();
    session.resize(NARROW_SIZE);
    let frame = wait_for_recovery_frame_after(&session, before_resize, true, WAIT);
    assert!(
        visible_frame_width(&frame) <= usize::from(NARROW_SIZE.cols - 1),
        "dynamic frame exceeded resized width: {:?}",
        String::from_utf8_lossy(&frame)
    );
    assert!(
        strip_ansi(&frame).contains("..."),
        "narrow frame did not exercise renderer truncation: {:?}",
        String::from_utf8_lossy(&frame)
    );

    let before_interrupt = session.bytes().len();
    session.control_c();
    wait_for_notice_after(&session, before_interrupt, WAIT);
    drop(recovery_lock);
    assert_eq!(session.wait(WAIT), 130);
    let raw = session.finish_output();
    assert!(
        raw.windows(2).any(|pair| pair == b"\x1b["),
        "ANSI was absent"
    );
    let plain = strip_ansi(&raw);
    let terminal_summaries = plain.matches("Cancelled build-cli-").count();
    assert_eq!(
        terminal_summaries, 1,
        "cooperative cancellation must emit exactly one terminal summary: {plain}"
    );
    assert!(
        plain.contains(" succeeded,"),
        "cooperative terminal summary omitted totals: {plain}"
    );
    #[cfg(not(windows))]
    assert!(
        find_bytes(&raw[before_interrupt.min(raw.len())..], EMERGENCY_RESET).is_none(),
        "one interrupt unexpectedly used the emergency restoration path"
    );
}

/// 第二次中断会在真实终端上写恢复序列并立即以 130 退出。 / A second interrupt writes
/// the emergency restoration sequence to the real terminal and immediately exits with 130.
#[test]
fn second_interrupt_emits_emergency_reset_before_exit() {
    let fixture = Fixture::create("emergency");
    let _recovery_lock = fixture.hold_recovery_lock();
    let mut session = PtySession::spawn(&fixture.project, &fixture.home("home"));

    wait_for_blocked_recovery(&session, WAIT);
    let before_first = session.bytes().len();
    session.control_c();
    wait_for_notice_after(&session, before_first, WAIT);
    let before_interrupts = session.bytes().len();
    session.control_c();
    assert_eq!(session.wait(WAIT), 130);
    let raw = session.finish_output();
    assert_emergency_restore(&raw, before_interrupts);
}

/// 验证平台可观察的紧急恢复序列。 / Verifies the platform-observable emergency reset.
#[cfg(not(windows))]
fn assert_emergency_restore(raw: &[u8], _before_interrupts: usize) {
    assert!(
        find_bytes(raw, EMERGENCY_RESET).is_some(),
        "second interrupt did not write the exact emergency reset; tail: {:?}",
        String::from_utf8_lossy(&raw[raw.len().saturating_sub(2_000)..])
    );
    let plain = strip_ansi(raw);
    assert!(
        !plain.contains(" succeeded,"),
        "emergency exit unexpectedly emitted a cooperative terminal summary: {plain}"
    );
}

/// ConPTY 是终端仿真器而不是透明管道：它会消费冗余的 SGR reset 与 cursor-show，
/// 再把 `CSI 2 K` 规范化为 `CSI K`。因此 Windows 上可重复观察的恢复字节是第二次中断
/// 之后的清行序列；没有协作终态且退出 130 则区分了紧急路径与第一次中断路径。
/// ConPTY is a terminal emulator rather than a transparent pipe: it consumes redundant SGR
/// reset and cursor-show commands and canonicalizes `CSI 2 K` to `CSI K`. The repeatably
/// observable Windows reset byte is therefore the clear-line sequence after the second input;
/// absence of a cooperative terminal summary plus exit 130 distinguishes the emergency path.
#[cfg(windows)]
fn assert_emergency_restore(raw: &[u8], before_interrupts: usize) {
    let suffix = &raw[before_interrupts.min(raw.len())..];
    let plain = strip_ansi(suffix);
    assert!(
        find_bytes(suffix, EMERGENCY_RESET).is_some() || find_bytes(suffix, b"\x1b[K").is_some(),
        "ConPTY did not expose the emergency clear sequence; tail: {:?}",
        String::from_utf8_lossy(&raw[raw.len().saturating_sub(2_000)..])
    );
    assert!(
        !plain.contains(" succeeded,"),
        "second interrupt unexpectedly followed the cooperative completion path: {plain}"
    );
    assert!(
        !plain.contains("Recovering"),
        "a complete progress frame appeared after the emergency interrupt: {plain}"
    );
}

/// 在发送偏移后等待呈现器拥有的完整取消提示行。 /
/// Waits after the send offset for the renderer-owned complete cancellation notice line.
fn wait_for_notice_after(session: &PtySession, offset: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let plain = strip_ansi(&bytes[offset.min(bytes.len())..]);
        if plain
            .split_inclusive(['\r', '\n'])
            .any(|line| line.trim() == CANCELLATION_NOTICE)
        {
            return;
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

/// 等待已完成的 repository recovery 证据及其后的同作业 runtime 恢复帧。 /
/// Waits for completed repository-recovery evidence and the same job's later runtime recovery frame.
fn wait_for_blocked_recovery(session: &PtySession, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let plain = strip_ansi(&bytes);
        if let Some(frame) = ordered_recovery_frame(&plain) {
            return frame.into_bytes();
        }
        assert!(
            Instant::now() < deadline,
            "no ordered recovery sequence; tail: {:?}",
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

/// 从持久步骤终态和完整重绘语法中证明 open-build-runtime 恢复阶段。 /
/// Proves the open-build-runtime recovery phase from persistent step terminals and a full repaint.
fn ordered_recovery_frame(transcript: &str) -> Option<String> {
    let (recover_end, job) = completed_step_after(transcript, 0, "recover")?;
    let suffix = &transcript[recover_end..];
    let mut offset = 0;
    while let Some(relative) = suffix[offset..].find("Recovering ") {
        let start = offset + relative;
        let end = start + suffix[start..].find("s)")? + 2;
        let frame = &suffix[start..end];
        if progress_job(frame, "Recovering") == Some(job) {
            return Some(frame.to_owned());
        }
        offset = end;
    }
    None
}

/// 查找 CR/LF 定界的 trace 步骤终态并返回结束偏移与 job。 /
/// Finds a CR/LF-delimited trace step terminal and returns its end offset and job.
fn completed_step_after<'a>(
    transcript: &'a str,
    offset: usize,
    step: &str,
) -> Option<(usize, &'a str)> {
    let prefix = format!("Planned step {step} (");
    let relative = transcript[offset..].find(&prefix)?;
    let start = offset + relative;
    if start > 0 && !matches!(transcript.as_bytes()[start - 1], b'\r' | b'\n') {
        return None;
    }
    let line_end = transcript[start..]
        .find(['\r', '\n'])
        .map_or(transcript.len(), |end| start + end);
    let line = &transcript[start..line_end];
    let body = line.strip_prefix(&prefix)?;
    let (milliseconds, job) = body.split_once(" ms) (")?;
    let job = job.strip_suffix(')')?;
    milliseconds.parse::<u64>().ok()?;
    Some((line_end, job))
}

/// 解析完整的 `<phase> (<job>) (<seconds>s)` trace 帧并返回 job。 /
/// Parses a complete `<phase> (<job>) (<seconds>s)` trace frame and returns its job.
fn progress_job<'a>(frame: &'a str, phase: &str) -> Option<&'a str> {
    let rest = frame.strip_prefix(phase)?.strip_prefix(' ')?;
    let (job, elapsed) = rest.rsplit_once(") (")?;
    let job = job.strip_prefix('(')?;
    let seconds = elapsed.strip_suffix("s)")?;
    (job.starts_with("build-cli-") && seconds.parse::<f64>().is_ok()).then_some(job)
}

/// 等待一个由后继 CR 封闭、且语义完整的恢复帧；控制符片段绝不算帧。 /
/// Waits for a recovery frame closed by a later CR; control-only fragments never count.
fn wait_for_recovery_frame_after(
    session: &PtySession,
    offset: usize,
    truncated: bool,
    timeout: Duration,
) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = session.output.bytes.lock().expect("PTY output lock");
    loop {
        let suffix = &bytes[offset.min(bytes.len())..];
        let plain = strip_ansi(suffix);
        let mut searched = 0;
        while let Some(relative) = plain[searched..].find("Recovering") {
            let start = searched + relative;
            if let Some(relative_end) = plain[start..].find("s)") {
                let end = start + relative_end + 2;
                let frame = &plain[start..end];
                if complete_recovery_grammar(frame, truncated) {
                    return frame.as_bytes().to_vec();
                }
                searched = end;
            } else {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no complete semantic recovery frame followed offset; tail: {:?}",
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

/// 验证 `Recovering (<job>) (<seconds>s)` 的完整 trace 终端语法。 /
/// Validates the complete `Recovering (<job>) (<seconds>s)` trace terminal grammar.
fn complete_recovery_grammar(frame: &str, truncated: bool) -> bool {
    let Some((label, elapsed)) = frame.trim().rsplit_once(" (") else {
        return false;
    };
    let Some(seconds) = elapsed.strip_suffix("s)") else {
        return false;
    };
    let label_is_complete = if truncated {
        label.starts_with("Recovering") && label.contains("...")
    } else {
        label.starts_with("Recovering (build-cli-") && label.ends_with(')')
    };
    label_is_complete && seconds.parse::<f64>().is_ok()
}

/// 计算动态帧的可见 ASCII 宽度；产品动作 ID 与计数均为 ASCII。 / Measures the visible
/// ASCII width of a dynamic frame; product action IDs and counters are ASCII.
fn visible_frame_width(frame: &[u8]) -> usize {
    strip_ansi(frame).chars().count()
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
