use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let code = xmlsquish_cli::run(std::env::args_os(), &mut io::stdout(), &mut io::stderr());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
