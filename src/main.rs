//! XML prompt compiler executable. / XML 提示词编译器可执行入口。
mod cli;
mod compiler;
mod squish;

use std::process::ExitCode;

/// Translate CLI status to the native process exit code. / 将 CLI 状态转换为进程退出码。
fn main() -> ExitCode {
    let code = cli::run_stdio(std::env::args_os());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
