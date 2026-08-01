//! Command-line entry point.
//!
//! Responsible only for argument parsing, dispatch, and translating a typed error into the
//! documented process exit code. All behaviour lives in the library.

use std::process::ExitCode as ProcessExitCode;

use clap::Parser;

use zec_ironwood_reconcile::cli::args::{
    CaptureArgs, Cli, Command, InspectArgs, ReconcileArgs, VerifyArgs,
};
use zec_ironwood_reconcile::cli::exit::ExitCode;
use zec_ironwood_reconcile::commands::{capture, inspect, reconcile, verify};
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
        Command::Reconcile(args) => run_reconcile(args),
        Command::Verify(args) => run_verify(args),
        Command::Inspect(args) => run_inspect(args),
    }
}

/// Reconciles a bundle and writes the report artifacts.
///
/// The exit code is the reconciliation's own verdict: a completed run whose accounting
/// comparison failed exits 1, never 0.
fn run_reconcile(args: &ReconcileArgs) -> Result<ExitCode, ReconcileError> {
    let reconciliation = reconcile::reconcile(&args.bundle)?;
    reconcile::write_reports(&reconciliation, &args.output)?;

    print!("{}", reconcile::render(&reconciliation));
    Ok(ExitCode::from_check_status(
        reconciliation.report.overall_status,
    ))
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

/// Verifies an archive offline and reproduces its report hash.
///
/// Extraction happens into a temporary directory that is removed on exit, so verifying an
/// archive leaves nothing behind and never touches the archive itself.
///
/// A supplied expectation that cannot be met is a failure, not an absence of one: a
/// mismatch exits 1, and so does a run whose own checks failed even when the hash matched.
fn run_verify(args: &VerifyArgs) -> Result<ExitCode, ReconcileError> {
    let workspace = tempfile::tempdir().map_err(|source| ReconcileError::Filesystem {
        path: "temporary extraction directory".to_owned(),
        source,
    })?;

    let (outcome, reconciliation) = verify::verify_and_reconcile(
        &args.archive,
        workspace.path(),
        args.expected_report_hash.as_deref(),
        &ExtractionLimits::default(),
    )?;

    print!("{}", verify::render(&outcome));

    if outcome.hash_matches() == Some(false) {
        return Ok(ExitCode::ChecksFailed);
    }

    Ok(ExitCode::from_check_status(
        reconciliation.report.overall_status,
    ))
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "exit codes are a closed set of small non-negative integers"
)]
fn to_process_code(code: ExitCode) -> ProcessExitCode {
    ProcessExitCode::from(code.code() as u8)
}
