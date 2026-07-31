//! Node authentication and the redaction of its secrets.
//!
//! Zebra enables cookie authentication by default and writes a file containing
//! `__cookie__:<password>` into its cookie directory at startup, so the cookie file is the
//! primary path here and an explicit user and password is the fallback.
//!
//! # Secrets must not escape this module
//!
//! A captured bundle is intended to be published. The RPC password, the endpoint that may
//! embed it, and the cookie contents must therefore appear in no artifact, no error, and no
//! log line. Two mechanisms enforce this:
//!
//! - [`Secret`] has no `Display`, and its `Debug` prints a placeholder, so a secret cannot
//!   be formatted into a string by accident;
//! - [`Authentication::scrub`] is applied to every message that originates outside this
//!   crate, because a transport error can quote a URL that carries credentials.
//!
//! The one place the secret is rendered is [`Authentication::header_value`], whose result
//! goes directly into a request header and is never retained.

use std::fmt;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::error::ReconcileError;

/// Username Zebra and zcashd write into a cookie file.
pub const COOKIE_USER: &str = "__cookie__";

/// File name of the cookie inside a node's cookie directory.
pub const COOKIE_FILE_NAME: &str = ".cookie";

/// Text substituted for a secret wherever one would otherwise be rendered.
pub const REDACTION: &str = "[redacted]";

/// A credential that must never be printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Exposes the secret. Every call site is a place a secret could leak, so there are
    /// deliberately few of them.
    fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTION)
    }
}

/// How the tool authenticates to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authentication {
    /// No credentials. Valid only against a node with authentication disabled.
    Anonymous,
    Basic {
        user: String,
        secret: Secret,
    },
}

impl Authentication {
    /// Chooses an authentication method from the supplied options.
    ///
    /// Supplying both a cookie file and an explicit password is refused rather than
    /// resolved by precedence: the two describe different intentions, and silently
    /// honouring one would leave the operator believing the other was in use.
    pub fn resolve(
        user: Option<&str>,
        password: Option<&str>,
        cookie_file: Option<&Path>,
    ) -> Result<Self, ReconcileError> {
        let explicit_password = password.is_some();

        if cookie_file.is_some() && explicit_password {
            return Err(ReconcileError::InvalidInput {
                reason: "supply either --rpc-cookie-file or --rpc-password, not both".to_owned(),
            });
        }

        if let Some(path) = cookie_file {
            return Self::from_cookie_file(path);
        }

        match (user, password) {
            (Some(user), Some(password)) => Ok(Self::Basic {
                user: user.to_owned(),
                secret: Secret::new(password),
            }),
            (Some(_), None) => Err(ReconcileError::InvalidInput {
                reason: "--rpc-user was given without --rpc-password".to_owned(),
            }),
            (None, Some(_)) => Err(ReconcileError::InvalidInput {
                reason: "--rpc-password was given without --rpc-user".to_owned(),
            }),
            (None, None) => match default_cookie_path() {
                Some(path) if path.is_file() => Self::from_cookie_file(&path),
                _ => Ok(Self::Anonymous),
            },
        }
    }

    /// Reads a node cookie file.
    ///
    /// The file holds a single `user:password` line. Its contents are trimmed because a
    /// cookie written by one implementation may end with a newline and one written by
    /// another may not.
    pub fn from_cookie_file(path: &Path) -> Result<Self, ReconcileError> {
        let contents =
            std::fs::read_to_string(path).map_err(|source| ReconcileError::Filesystem {
                path: path.display().to_string(),
                source,
            })?;

        Self::from_cookie_contents(&contents).map_err(|reason| ReconcileError::InvalidInput {
            reason: format!("cookie file {}: {reason}", path.display()),
        })
    }

    /// Parses cookie contents, without reference to where they came from.
    ///
    /// The error type is a plain string so that a caller adds the path; the contents
    /// themselves are never quoted back, since they are the secret.
    fn from_cookie_contents(contents: &str) -> Result<Self, String> {
        let line = contents.trim();
        if line.is_empty() {
            return Err("is empty".to_owned());
        }

        let (user, password) = line
            .split_once(':')
            .ok_or_else(|| "does not contain a `user:password` pair".to_owned())?;

        if user.is_empty() {
            return Err("has an empty user".to_owned());
        }
        if password.is_empty() {
            return Err("has an empty password".to_owned());
        }

        Ok(Self::Basic {
            user: user.to_owned(),
            secret: Secret::new(password),
        })
    }

    /// Renders the HTTP Basic `Authorization` header value, if credentials are configured.
    pub fn header_value(&self) -> Option<String> {
        match self {
            Self::Anonymous => None,
            Self::Basic { user, secret } => {
                let encoded = BASE64.encode(format!("{user}:{}", secret.expose()));
                Some(format!("Basic {encoded}"))
            }
        }
    }

    pub const fn is_authenticated(&self) -> bool {
        matches!(self, Self::Basic { .. })
    }

    /// Removes the configured secret from text produced outside this crate.
    ///
    /// Transport errors can quote the request URL, and a URL may carry credentials in its
    /// userinfo component. Scrubbing is applied to the message rather than trusted not to
    /// be needed.
    pub fn scrub(&self, text: &str) -> String {
        let scrubbed = scrub_url_userinfo(text);
        match self {
            // Replacing an empty needle would insert the placeholder between every
            // character, so an empty secret is left alone; it cannot leak anything.
            Self::Basic { secret, .. } if !secret.is_empty() => {
                scrubbed.replace(secret.expose(), REDACTION)
            }
            _ => scrubbed,
        }
    }
}

/// Default path of a Zebra cookie file, following the XDG cache convention.
///
/// Zebra writes the cookie into its cache directory unless configured otherwise. A node
/// with a custom `cookie_dir` requires `--rpc-cookie-file`, which is why discovery failing
/// is not an error: it falls back to an unauthenticated attempt whose failure names the
/// cause.
pub fn default_cookie_path() -> Option<PathBuf> {
    let cache = match std::env::var_os("XDG_CACHE_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(cache.join("zebra").join(COOKIE_FILE_NAME))
}

/// Replaces the userinfo component of any URL appearing in text.
///
/// Operates on `scheme://userinfo@host` occurrences. This is not a general URL parser; it
/// is a targeted removal of the one component that can carry a credential.
fn scrub_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(marker) = rest.find("://") {
        let Some((before, after)) = rest.split_at_checked(marker.saturating_add(3)) else {
            break;
        };
        out.push_str(before);

        // Userinfo ends at the first `@`, and cannot extend past the end of the authority.
        let authority_end = after
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(after.len());
        let Some(authority) = after.get(..authority_end) else {
            break;
        };

        match authority.rfind('@') {
            Some(at) => {
                out.push_str(REDACTION);
                out.push('@');
                if let Some(host) = authority.get(at.saturating_add(1)..) {
                    out.push_str(host);
                }
            }
            None => out.push_str(authority),
        }

        rest = after.get(authority_end..).unwrap_or("");
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn cookie_file(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(COOKIE_FILE_NAME);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.sync_all().unwrap();
        (dir, path)
    }

    #[test]
    fn a_zebra_cookie_file_is_parsed() {
        // Byte-for-byte the shape Zebra 6.2.3 writes: no trailing newline.
        let (_dir, path) = cookie_file("__cookie__:/cxwI6JxcDBbMcYyFD1UlLUA7h7NNoQcUch2Y");
        let authentication = Authentication::from_cookie_file(&path).unwrap();

        match &authentication {
            Authentication::Basic { user, secret } => {
                assert_eq!(user, COOKIE_USER);
                assert!(!secret.is_empty());
            }
            other => panic!("expected basic credentials, got {other:?}"),
        }
    }

    #[test]
    fn a_cookie_file_with_a_trailing_newline_is_parsed() {
        let (_dir, path) = cookie_file("__cookie__:secret\n");
        let authentication = Authentication::from_cookie_file(&path).unwrap();
        assert_eq!(
            authentication.header_value(),
            Authentication::Basic {
                user: COOKIE_USER.to_owned(),
                secret: Secret::new("secret"),
            }
            .header_value()
        );
    }

    #[test]
    fn a_password_containing_a_colon_survives_parsing() {
        // Only the first colon separates the pair; the rest belongs to the password.
        let (_dir, path) = cookie_file("__cookie__:a:b:c");
        let authentication = Authentication::from_cookie_file(&path).unwrap();
        match authentication {
            Authentication::Basic { secret, .. } => assert_eq!(secret, Secret::new("a:b:c")),
            other => panic!("expected basic credentials, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_cookie_file_is_rejected() {
        for contents in ["", "   ", "nocolon", ":password", "__cookie__:"] {
            let (_dir, path) = cookie_file(contents);
            assert!(
                Authentication::from_cookie_file(&path).is_err(),
                "accepted malformed cookie contents {contents:?}"
            );
        }
    }

    #[test]
    fn a_rejected_cookie_file_never_quotes_its_contents() {
        let (_dir, path) = cookie_file("nocolon-but-secret-looking");
        let message = Authentication::from_cookie_file(&path)
            .unwrap_err()
            .to_string();
        assert!(
            !message.contains("nocolon-but-secret-looking"),
            "the error quoted the cookie contents: {message}"
        );
    }

    #[test]
    fn basic_credentials_encode_as_http_basic() {
        let authentication = Authentication::Basic {
            user: "user".to_owned(),
            secret: Secret::new("password"),
        };
        // "user:password" base64 encoded.
        assert_eq!(
            authentication.header_value().as_deref(),
            Some("Basic dXNlcjpwYXNzd29yZA==")
        );
    }

    #[test]
    fn anonymous_authentication_sends_no_header() {
        assert_eq!(Authentication::Anonymous.header_value(), None);
        assert!(!Authentication::Anonymous.is_authenticated());
    }

    #[test]
    fn a_secret_is_not_printable() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), REDACTION);

        let authentication = Authentication::Basic {
            user: "user".to_owned(),
            secret,
        };
        let rendered = format!("{authentication:?}");
        assert!(
            !rendered.contains("hunter2"),
            "the secret was printed: {rendered}"
        );
    }

    #[test]
    fn scrubbing_removes_the_configured_secret() {
        let authentication = Authentication::Basic {
            user: "user".to_owned(),
            secret: Secret::new("hunter2"),
        };
        let scrubbed = authentication.scrub("connection refused (password hunter2)");
        assert!(!scrubbed.contains("hunter2"));
        assert!(scrubbed.contains(REDACTION));
    }

    #[test]
    fn scrubbing_removes_credentials_embedded_in_a_url() {
        let scrubbed = Authentication::Anonymous
            .scrub("failed to reach http://user:hunter2@127.0.0.1:8232/ after 3 attempts");
        assert!(!scrubbed.contains("hunter2"));
        assert!(scrubbed.contains("http://[redacted]@127.0.0.1:8232/"));
        assert!(scrubbed.contains("after 3 attempts"));
    }

    #[test]
    fn scrubbing_leaves_a_url_without_credentials_intact() {
        let text = "failed to reach http://127.0.0.1:8232/ after 3 attempts";
        assert_eq!(Authentication::Anonymous.scrub(text), text);
    }

    #[test]
    fn scrubbing_handles_several_urls_in_one_message() {
        let scrubbed = Authentication::Anonymous
            .scrub("http://a:b@host1/x then http://host2/y then http://c:d@host3");
        assert!(!scrubbed.contains("a:b@"));
        assert!(!scrubbed.contains("c:d@"));
        assert!(scrubbed.contains("host2/y"));
    }

    #[test]
    fn an_empty_secret_does_not_corrupt_a_message() {
        let authentication = Authentication::Basic {
            user: "user".to_owned(),
            secret: Secret::new(""),
        };
        assert_eq!(authentication.scrub("plain message"), "plain message");
    }

    #[test]
    fn supplying_both_a_cookie_file_and_a_password_is_refused() {
        let (_dir, path) = cookie_file("__cookie__:secret");
        let result = Authentication::resolve(Some("user"), Some("password"), Some(&path));
        assert!(matches!(result, Err(ReconcileError::InvalidInput { .. })));
    }

    #[test]
    fn a_half_supplied_credential_pair_is_refused() {
        assert!(Authentication::resolve(Some("user"), None, None).is_err());
        assert!(Authentication::resolve(None, Some("password"), None).is_err());
    }

    #[test]
    fn an_explicit_cookie_file_is_preferred_over_discovery() {
        let (_dir, path) = cookie_file("__cookie__:from-the-explicit-file");
        let authentication = Authentication::resolve(None, None, Some(&path)).unwrap();
        assert!(authentication.is_authenticated());
    }

    #[test]
    fn the_default_cookie_path_follows_the_cache_convention() {
        let path = default_cookie_path().expect("HOME or XDG_CACHE_HOME should be set in tests");
        assert!(path.ends_with("zebra/.cookie"), "{}", path.display());
    }
}
