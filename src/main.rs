//! Command-line entry point.
//!
//! Responsible only for argument parsing, dispatch, and translating a typed error into the
//! documented process exit code. All behaviour lives in the library.

use std::process::ExitCode as ProcessExitCode;

use clap::Parser;

use zec_ironwood_reconcile::cli::args::{CaptureArgs, Cli, Command, InspectArgs, VerifyArgs};
use zec_ironwood_reconcile::cli::exit::ExitCode;
use zec_ironwood_reconcile::commands::{capture, inspect, verify};
use zec_ironwood_reconcile::error::ReconcileError;
use zec_ironwood_reconcile::evidence::archive::ExtractionLimits;

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
        Command::Capture(args) => run_capture(args, cli.quiet),
        Command::Reconcile(_) => Err(unimplemented_command("reconcile")),
        Command::Verify(args) => run_verify(args),
        Command::Inspect(args) => run_inspect(args),
    }
}

/// Captures an interval and reports what was written.
///
/// Advisories are printed but do not change the exit code: they describe the evidence, not
/// a failure to collect it. A condition that makes a capture unusable is an error, and an
/// error never reaches this point.
fn run_capture(args: &CaptureArgs, quiet: bool) -> Result<ExitCode, ReconcileError> {
    let summary = capture::capture(args, quiet)?;
    print!("{}", capture::render(&summary));
    Ok(ExitCode::Success)
}

fn run_inspect(args: &InspectArgs) -> Result<ExitCode, ReconcileError> {
    let summary = inspect::inspect(&args.bundle)?;
    print!("{}", inspect::render(&summary));
    Ok(ExitCode::Success)
}

/// Verifies an archive's evidence against its manifest.
///
/// Reconciliation is not yet wired in, so no report hash is produced. When the caller
/// supplied an expected hash, that expectation cannot be met and the run must not report
/// success: an unmet expectation is a failure, not an absence of one.
fn run_verify(args: &VerifyArgs) -> Result<ExitCode, ReconcileError> {
    let workspace = tempfile::tempdir().map_err(|source| ReconcileError::Filesystem {
        path: "temporary extraction directory".to_owned(),
        source,
    })?;

    let (_, outcome) = verify::verify_archive(
        &args.archive,
        workspace.path(),
        args.expected_report_hash.as_deref(),
        &ExtractionLimits::default(),
    )?;

    print!("{}", verify::render(&outcome));

    match outcome.hash_matches() {
        Some(true) => Ok(ExitCode::Success),
        Some(false) => Ok(ExitCode::ChecksFailed),
        None if args.expected_report_hash.is_some() => {
            eprintln!(
                "error: a report hash was expected but reconciliation is not yet available in \
                 this build, so no comparison could be made"
            );
            Ok(ExitCode::ChecksFailed)
        }
        None => Ok(ExitCode::Success),
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
