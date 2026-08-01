//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::domain::network::Network;

/// Default number of blocks the chain tip must exceed the requested end height before a
/// capture is permitted. Zcash offers no protocol finality, so a capture taken too close
/// to the tip can be invalidated by a reorganisation after the fact.
///
/// **The value is a conservative choice, not a derived one.** No specification states a
/// depth beyond which a Zcash reorganisation cannot occur, and no source for one was found.
/// At the 75-second target spacing, 100 blocks is roughly two hours of chain — far beyond
/// any reorganisation depth observed in practice, and cheap, since it only delays a capture.
/// Anyone who has a sourced figure should use it via `--tip-distance` rather than infer one
/// from this default.
pub const DEFAULT_TIP_DISTANCE: u32 = 100;

#[derive(Debug, Parser)]
#[command(
    name = "zec-ironwood-reconcile",
    version,
    about = "Reconstructs Orchard and Ironwood value-pool changes over a bounded Zcash block interval",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Suppress progress output.
    #[arg(long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Collect a bounded block interval and anchor values from a Zebra RPC endpoint.
    Capture(CaptureArgs),
    /// Reconstruct value-pool changes from an evidence bundle.
    Reconcile(ReconcileArgs),
    /// Verify an evidence archive offline against a published report hash.
    Verify(VerifyArgs),
    /// Display bundle metadata without running a reconciliation.
    Inspect(InspectArgs),
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    #[arg(long)]
    pub rpc_url: String,

    #[arg(long)]
    pub from_height: u32,

    #[arg(long)]
    pub to_height: u32,

    #[arg(long)]
    pub network: Network,

    #[arg(long)]
    pub output: PathBuf,

    #[arg(long)]
    pub rpc_user: Option<String>,

    /// Prefer `--rpc-cookie-file`. A password supplied here may be visible in the process
    /// list to other users on the same machine.
    #[arg(long)]
    pub rpc_password: Option<String>,

    /// Path to the node's authentication cookie. Zebra enables cookie authentication by
    /// default and writes this file into its cache directory at startup.
    #[arg(long)]
    pub rpc_cookie_file: Option<PathBuf>,

    #[arg(long, default_value_t = 30)]
    pub timeout_seconds: u64,

    /// Whole requests per second. Integral because this crate performs no floating-point
    /// arithmetic, so the pacing interval is derived in milliseconds.
    #[arg(long, default_value_t = 10)]
    pub requests_per_second: u32,

    /// Minimum number of blocks the chain tip must exceed `--to-height`.
    #[arg(long, default_value_t = DEFAULT_TIP_DISTANCE)]
    pub tip_distance: u32,

    /// Continue an interrupted capture rather than starting over.
    #[arg(long)]
    pub resume: bool,

    #[arg(long)]
    pub overwrite: bool,

    /// Also pack the finished bundle into this `.tar.zst` archive and write its digest
    /// alongside as `<archive>.sha256`. The archive and that digest are what a third party
    /// needs in order to reproduce the result offline.
    #[arg(long)]
    pub archive: Option<PathBuf>,

    /// Assert the expected activation height for the selected network.
    #[arg(long)]
    pub expected_activation_height: Option<u32>,
}

#[derive(Debug, Args)]
pub struct ReconcileArgs {
    /// Path to an evidence bundle directory.
    pub bundle: PathBuf,

    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Path to an evidence archive.
    pub archive: PathBuf,

    /// Canonical report hash the reconciliation is expected to reproduce.
    #[arg(long)]
    pub expected_report_hash: Option<String>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Path to an evidence bundle directory or archive.
    pub bundle: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn capture_parses_required_options() {
        let cli = Cli::try_parse_from([
            "zec-ironwood-reconcile",
            "capture",
            "--rpc-url",
            "http://127.0.0.1:8232",
            "--from-height",
            "3428143",
            "--to-height",
            "3429143",
            "--network",
            "mainnet",
            "--output",
            "./evidence/bundle",
        ])
        .unwrap();

        let Command::Capture(args) = cli.command else {
            panic!("expected capture command");
        };
        assert_eq!(args.from_height, 3_428_143);
        assert_eq!(args.to_height, 3_429_143);
        assert_eq!(args.network, Network::Mainnet);
        assert_eq!(args.tip_distance, DEFAULT_TIP_DISTANCE);
    }

    #[test]
    fn capture_rejects_a_missing_required_option() {
        let result = Cli::try_parse_from([
            "zec-ironwood-reconcile",
            "capture",
            "--rpc-url",
            "http://127.0.0.1:8232",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn capture_rejects_an_unknown_network() {
        let result = Cli::try_parse_from([
            "zec-ironwood-reconcile",
            "capture",
            "--rpc-url",
            "http://127.0.0.1:8232",
            "--from-height",
            "1",
            "--to-height",
            "2",
            "--network",
            "regtest",
            "--output",
            "./out",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn verify_accepts_an_expected_hash() {
        let cli = Cli::try_parse_from([
            "zec-ironwood-reconcile",
            "verify",
            "./bundle.tar.zst",
            "--expected-report-hash",
            "abc123",
        ])
        .unwrap();

        let Command::Verify(args) = cli.command else {
            panic!("expected verify command");
        };
        assert_eq!(args.expected_report_hash.as_deref(), Some("abc123"));
    }

    #[test]
    fn quiet_is_accepted_on_every_subcommand() {
        for argv in [
            vec!["zec-ironwood-reconcile", "inspect", "./b", "--quiet"],
            vec!["zec-ironwood-reconcile", "verify", "./b.tar.zst", "--quiet"],
        ] {
            assert!(Cli::try_parse_from(argv).unwrap().quiet);
        }
    }
}
