//! `ezsdr-server [--runs-dir <path>]`: serves one Session over standard input and
//! output (EA-2, EA-8). Diagnostics go to standard error.

use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use ezsdr_server::{Config, Exit, serve};

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
                println!("ezsdr-server [--runs-dir <path>]: serves one Session over stdin/stdout (ezsdr.protocol 1)");
                return ExitCode::SUCCESS;
            }
            other => return usage(&format!("unknown argument {other}")),
        }
    }
    let config = Config { runs_dir, implementations: Vec::new() };
    match serve(BufReader::new(io::stdin().lock()), io::stdout().lock(), config) {
        Ok(Exit::Replied | Exit::EndOfInput) => ExitCode::SUCCESS,
        Ok(Exit::BadFrame) => ExitCode::from(2),
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
