//! Command-line entry point.
//!
//! Responsible only for argument parsing, dispatch, and translating a typed error into the
//! documented process exit code. All behaviour lives in the library.

use std::process::ExitCode as ProcessExitCode;

use clap::Parser;

use zec_ironwood_reconcile::cli::args::{Cli, Command};
use zec_ironwood_reconcile::cli::exit::ExitCode;
use zec_ironwood_reconcile::error::ReconcileError;

fn main() -> ProcessExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(code) => to_process_code(code),
        Err(error) => {
            eprintln!("error[{}]: {error}", error.stable_id());
            to_process_code(ExitCode::from(&error))
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, ReconcileError> {
    match &cli.command {
        Command::Capture(_) => Err(unimplemented_command("capture")),
        Command::Reconcile(_) => Err(unimplemented_command("reconcile")),
        Command::Verify(_) => Err(unimplemented_command("verify")),
        Command::Inspect(_) => Err(unimplemented_command("inspect")),
    }
}

fn unimplemented_command(name: &str) -> ReconcileError {
    ReconcileError::Internal {
        reason: format!("command `{name}` is not yet implemented"),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "exit codes are a closed set of small non-negative integers"
)]
fn to_process_code(code: ExitCode) -> ProcessExitCode {
    ProcessExitCode::from(code.code() as u8)
}
