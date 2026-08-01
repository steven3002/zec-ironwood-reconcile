//! `capture`, collect a bounded interval from a node.
//!
//! The only command that opens a socket. It turns command-line arguments into a transport
//! and a request, runs the capture, and renders the outcome; every decision about whether
//! the capture is sound is made in [`crate::capture`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::capture::plan::CaptureRequest;
use crate::capture::run::{self, Archive, CaptureOptions, CaptureSummary};
use crate::capture::writer::OutputMode;
use crate::cli::args::CaptureArgs;
use crate::error::ReconcileError;
use crate::evidence::archive;
use crate::rpc::auth::Authentication;
use crate::rpc::client::HttpTransport;
use crate::rpc::method::NodeClient;

/// Blocks between progress lines.
const PROGRESS_INTERVAL: u32 = 100;

/// Runs a capture described by command-line arguments.
pub fn capture(args: &CaptureArgs, quiet: bool) -> Result<CaptureSummary, ReconcileError> {
    let request = CaptureRequest::new(
        args.network,
        args.from_height,
        args.to_height,
        args.tip_distance,
        args.expected_activation_height,
    )?;

    let authentication = Authentication::resolve(
        args.rpc_user.as_deref(),
        args.rpc_password.as_deref(),
        args.rpc_cookie_file.as_deref(),
    )?;

    let transport = HttpTransport::new(
        &args.rpc_url,
        authentication,
        Duration::from_secs(args.timeout_seconds),
        args.requests_per_second,
    )?;

    if transport.sends_credentials_off_host() {
        eprintln!(
            "warning: the endpoint is not on this machine, so HTTP Basic credentials will \
             travel unencrypted; prefer a local node or a tunnel"
        );
    }

    let mut emit = |line: &str| {
        if !quiet {
            eprintln!("{line}");
        }
    };

    let options = CaptureOptions {
        request,
        output_mode: output_mode(args),
        progress_interval: PROGRESS_INTERVAL,
    };

    // Checked before the node is read rather than after. An archive that cannot be written
    // is worth discovering in the first second of a capture, not once every block has been
    // fetched.
    if let Some(archive_path) = args.archive.as_deref() {
        refuse_existing_archive(archive_path, args.overwrite)?;
    }

    let client = NodeClient::new(&transport);
    let mut summary = run::run(&client, &options, &output_root(args), &mut emit)?;

    if let Some(archive_path) = args.archive.as_deref() {
        emit("packing the bundle into an archive");
        let digest = archive::pack_with_digest(&summary.bundle_root, archive_path)?;
        summary.archive = Some(Archive {
            path: archive_path.to_path_buf(),
            sha256: digest,
        });
    }

    Ok(summary)
}

/// Refuses to replace an archive that already exists unless asked to.
///
/// A bundle directory already refuses to be written over without `--overwrite`. The archive
/// is the artifact actually published alongside its digest, so silently replacing one, and
/// the `.sha256` beside it, is the more damaging of the two omissions: a third party
/// holding the old digest is left unable to verify anything, with nothing to say why.
fn refuse_existing_archive(path: &Path, overwrite: bool) -> Result<(), ReconcileError> {
    if overwrite || !path.exists() {
        return Ok(());
    }

    Err(ReconcileError::InvalidInput {
        reason: format!(
            "{} already exists; pass --overwrite to replace it and its digest",
            path.display()
        ),
    })
}

fn output_mode(args: &CaptureArgs) -> OutputMode {
    match (args.resume, args.overwrite) {
        (true, _) => OutputMode::Resume,
        (false, true) => OutputMode::Overwrite,
        (false, false) => OutputMode::Create,
    }
}

fn output_root(args: &CaptureArgs) -> PathBuf {
    args.output.clone()
}

/// Renders a capture summary for a terminal.
pub fn render(summary: &CaptureSummary) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(out, "{label:<24} {value}");
    };

    line("Bundle:", &summary.bundle_root.display().to_string());
    line("Bundle id:", &summary.manifest.bundle_id);
    line("Network:", summary.manifest.network.name());
    line(
        "Interval:",
        &format!(
            "{}..={} ({} blocks)",
            summary.manifest.interval.start_height,
            summary.manifest.interval.end_height,
            summary.block_count
        ),
    );
    line(
        "Anchor:",
        &format!(
            "{} {}",
            summary.manifest.interval.anchor_height, summary.manifest.anchor.block_hash
        ),
    );
    line(
        "Node:",
        &format!(
            "{} {}",
            summary.manifest.source.implementation, summary.manifest.source.version
        ),
    );
    line("Tip at capture:", &summary.tip_at_capture.to_string());
    line("Files written:", &summary.files_written.to_string());
    line("Files reused:", &summary.files_reused.to_string());
    line("Files listed:", &summary.manifest.files.len().to_string());

    if let Some(archive) = &summary.archive {
        line("Archive:", &archive.path.display().to_string());
        line("Archive sha256:", &archive.sha256);
    }

    if !summary.advisories.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Advisories:");
        for advisory in &summary.advisories {
            let _ = writeln!(out, "  [{}] {}", advisory.id, advisory.detail);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::network::Network;

    fn args() -> CaptureArgs {
        CaptureArgs {
            rpc_url: "http://127.0.0.1:8232".to_owned(),
            from_height: 3_428_143,
            to_height: 3_428_243,
            network: Network::Mainnet,
            output: PathBuf::from("/tmp/bundle"),
            rpc_user: None,
            rpc_password: None,
            rpc_cookie_file: None,
            timeout_seconds: 30,
            requests_per_second: 10,
            tip_distance: 100,
            resume: false,
            overwrite: false,
            archive: None,
            expected_activation_height: None,
        }
    }

    #[test]
    fn resume_takes_precedence_over_overwrite() {
        // Continuing a capture and destroying it are opposite intentions; the conservative
        // one wins rather than the destructive one.
        let mut args = args();
        args.resume = true;
        args.overwrite = true;
        assert_eq!(output_mode(&args), OutputMode::Resume);
    }

    #[test]
    fn the_default_mode_refuses_to_write_over_anything() {
        assert_eq!(output_mode(&args()), OutputMode::Create);
    }

    #[test]
    fn overwrite_is_honoured_when_resume_is_absent() {
        let mut args = args();
        args.overwrite = true;
        assert_eq!(output_mode(&args), OutputMode::Overwrite);
    }

    #[test]
    fn an_existing_archive_is_not_replaced_without_being_asked() {
        // The bundle directory already refuses this. The archive is the artifact actually
        // published alongside a digest, so replacing one silently is the worse omission.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.tar.zst");
        std::fs::write(&path, b"a previously published archive").unwrap();

        let error = refuse_existing_archive(&path, false).unwrap_err();
        assert!(
            matches!(&error, ReconcileError::InvalidInput { reason } if reason.contains("--overwrite")),
            "{error:?}"
        );

        // Refused before anything is read, so the existing bytes are still there.
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"a previously published archive"
        );
    }

    #[test]
    fn an_absent_archive_path_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        assert!(refuse_existing_archive(&dir.path().join("new.tar.zst"), false).is_ok());
    }

    #[test]
    fn overwrite_permits_replacing_an_existing_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.tar.zst");
        std::fs::write(&path, b"old").unwrap();
        assert!(refuse_existing_archive(&path, true).is_ok());
    }

    #[test]
    fn an_https_endpoint_is_refused_before_any_request() {
        let mut args = args();
        args.rpc_url = "https://node.example/".to_owned();
        assert!(matches!(
            capture(&args, true),
            Err(ReconcileError::InvalidInput { .. })
        ));
    }

    #[test]
    fn a_reversed_interval_is_refused_before_any_request() {
        let mut args = args();
        args.from_height = 200;
        args.to_height = 100;
        assert!(matches!(
            capture(&args, true),
            Err(ReconcileError::InvalidInterval { .. })
        ));
    }

    #[test]
    fn conflicting_credentials_are_refused_before_any_request() {
        let mut args = args();
        args.rpc_password = Some("password".to_owned());
        args.rpc_cookie_file = Some(PathBuf::from("/nonexistent/.cookie"));
        assert!(matches!(
            capture(&args, true),
            Err(ReconcileError::InvalidInput { .. })
        ));
    }
}
