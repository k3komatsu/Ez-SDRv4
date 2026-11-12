//! `ezsdr-server [--runs-dir <path>]`: serves one Session over standard input and
//! output (EA-2, EA-8). `EZSDR_PROFILE`, when set, names the BindingProfile `connect`
//! without a profile uses (EA-9). `EZSDR_SERVER_FAILSTOP_SECS` (default 60) is how long the
//! process may outlive the end of its input before it exits with status 3, without cleanup
//! (EA-15). That is a cutoff, not a bound on cleanup: RS-8a bounds each cleanup step, not the
//! whole cleanup, so a cleanup still running when the delay ends is cut and writes no
//! Manifest. Diagnostics go to standard error.

use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ezsdr_server::{Config, Exit, serve_watched};

/// The status of the fail-stop backstop (EA-15).
const FAILSTOP_STATUS: i32 = 3;
const FAILSTOP_SECS: u64 = 60;

/// The input, which starts the backstop's timer when it ends or fails: a server still alive
/// `after` that exits at once, without cleanup, so a hung one never outlives its client.
struct Backstop<R> {
    input: R,
    after: Duration,
}

/// Ends the process at once with status 3 through `_exit`, not `exit`: no atexit handler or C++ static destructor (libuhd's) runs,
/// so none can hang the fail-stop while another thread is stuck in a device call (EA-15's "no cleanup").
fn fail_stop() -> ! {
    unsafe extern "C" {
        fn _exit(status: i32) -> !;
    }
    // SAFETY: `_exit` accepts any status, never returns and touches no Rust state.
    unsafe { _exit(FAILSTOP_STATUS) }
}

impl<R: Read> Read for Backstop<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(buffer);
        if matches!(read, Ok(0)) || read.as_ref().is_err_and(|error| error.kind() != io::ErrorKind::Interrupted) {
            let after = self.after;
            // A timer that cannot be created is the same fail-stop at once: this thread would otherwise panic
            // before it fires the watch, and the server would outlive its input with no timer.
            let timer = std::thread::Builder::new().name("ezsdr-failstop".to_owned()).spawn(move || {
                std::thread::sleep(after);
                fail_stop();
            });
            if timer.is_err() {
                fail_stop();
            }
        }
        read
    }
}

fn main() -> ExitCode {
    let mut runs_dir = PathBuf::from("ezsdr-runs");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs-dir" => match args.next() {
                Some(path) => runs_dir = PathBuf::from(path),
                None => return usage("--runs-dir needs a path"),
            },
            "--help" | "-h" => {
                println!("ezsdr-server [--runs-dir <path>]: serves one Session over stdin/stdout (ezsdr.protocol 2)");
                return ExitCode::SUCCESS;
            }
            other => return usage(&format!("unknown argument {other}")),
        }
    }
    let mut config = Config::new(runs_dir);
    // The lab server's own default profile (EA-9, VE-5); the library reads no environment.
    config.default_profile = std::env::var_os("EZSDR_PROFILE").filter(|path| !path.is_empty()).map(PathBuf::from);
    let after = match std::env::var("EZSDR_SERVER_FAILSTOP_SECS") {
        Ok(secs) => match secs.parse() {
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => return usage("EZSDR_SERVER_FAILSTOP_SECS is not a whole number of seconds"),
        },
        Err(_) => Duration::from_secs(FAILSTOP_SECS),
    };
    match serve_watched(Backstop { input: io::stdin(), after }, io::stdout().lock(), config) {
        Ok(Exit::Replied | Exit::EndOfInput) => ExitCode::SUCCESS,
        Ok(Exit::BadFrame) => ExitCode::from(2),
        // EA-15: a reply the client is no longer there to read is its departure.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ezsdr-server: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage(message: &str) -> ExitCode {
    eprintln!("ezsdr-server: {message}\nusage: ezsdr-server [--runs-dir <path>]");
    ExitCode::from(2)
}
