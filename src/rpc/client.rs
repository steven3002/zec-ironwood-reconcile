//! JSON-RPC transport.
//!
//! # Why there is a trait here
//!
//! [`RpcTransport`] exists so that capture can be exercised without a node. That is not a
//! generic abstraction over node implementations, the architecture deliberately has none —
//! but the seam that makes every guard, every failure path, and the credential-redaction
//! guarantee testable while a synced node is unavailable. There is exactly one production
//! implementation.
//!
//! # Plain HTTP only
//!
//! Zebra's RPC endpoint serves unencrypted HTTP, so this transport speaks only `http://`.
//! An `https://` endpoint is refused with a message saying why rather than failing inside
//! the transport, and a credentialed request to a non-loopback host is called out, because
//! HTTP Basic over a network sends the password in the clear.
//!
//! # Retrying is safe
//!
//! Every method this tool calls is a read. No request creates, spends, broadcasts, or
//! mutates anything, so a retried request cannot have a second effect.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::error::ReconcileError;
use crate::rpc::auth::Authentication;
use crate::rpc::dto;

/// Attempts per request, including the first.
const MAX_ATTEMPTS: u32 = 3;

/// Delay before the first retry. Later retries wait a multiple of it.
const RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// Upper bound on a single response body.
///
/// A consensus block is bounded at 2 MiB and hex encoding doubles it, so this leaves ample
/// headroom while refusing to buffer an unbounded response from a hostile or broken peer.
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

/// Appended only to failures an authentication mistake could actually produce.
const AUTHENTICATION_HINT: &str = "if the node requires authentication, check --rpc-cookie-file";

/// Performs one JSON-RPC call and yields the `result` member.
pub trait RpcTransport {
    fn call(
        &self,
        method: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, ReconcileError>;
}

/// Paces requests so that a capture cannot overwhelm the node it is reading from.
///
/// Intervals are integral milliseconds. The crate performs no floating-point arithmetic,
/// so a rate is expressed as whole requests per second rather than a fractional rate.
#[derive(Debug)]
pub struct RateLimiter {
    minimum_interval: Duration,
    last_request: Cell<Option<Instant>>,
}

impl RateLimiter {
    pub fn per_second(requests_per_second: u32) -> Result<Self, ReconcileError> {
        let interval_millis = 1_000_u32.checked_div(requests_per_second).ok_or_else(|| {
            ReconcileError::InvalidInput {
                reason: "--requests-per-second must be at least 1".to_owned(),
            }
        })?;

        Ok(Self {
            minimum_interval: Duration::from_millis(u64::from(interval_millis)),
            last_request: Cell::new(None),
        })
    }

    /// Blocks until the configured interval since the previous request has elapsed.
    pub fn wait(&self) {
        if let Some(previous) = self.last_request.get() {
            let remaining = self.minimum_interval.saturating_sub(previous.elapsed());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
        self.last_request.set(Some(Instant::now()));
    }

    pub const fn minimum_interval(&self) -> Duration {
        self.minimum_interval
    }
}

/// Blocking HTTP JSON-RPC transport.
pub struct HttpTransport {
    agent: ureq::Agent,
    url: String,
    authentication: Authentication,
    limiter: RateLimiter,
}

impl HttpTransport {
    pub fn new(
        url: &str,
        authentication: Authentication,
        timeout: Duration,
        requests_per_second: u32,
    ) -> Result<Self, ReconcileError> {
        validate_endpoint(url)?;

        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            // JSON-RPC reports application errors with an HTTP 200 and an `error` member,
            // and reports transport-level problems with a status this code inspects itself.
            // Either way the body carries the useful diagnosis, so it is always read.
            .http_status_as_error(false)
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build();

        Ok(Self {
            agent: ureq::Agent::new_with_config(config),
            url: url.to_owned(),
            authentication,
            limiter: RateLimiter::per_second(requests_per_second)?,
        })
    }

    /// Whether credentials would travel unencrypted to a host that is not this machine.
    pub fn sends_credentials_off_host(&self) -> bool {
        self.authentication.is_authenticated() && !endpoint_is_loopback(&self.url)
    }

    fn attempt(&self, body: &[u8]) -> Result<Vec<u8>, Attempt> {
        self.limiter.wait();

        let mut request = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json");

        if let Some(value) = self.authentication.header_value() {
            request = request.header("Authorization", &value);
        }

        let mut response = request.send(body).map_err(|source| {
            Attempt::retryable(if could_be_authentication(&source) {
                format!("{source}; {AUTHENTICATION_HINT}")
            } else {
                source.to_string()
            })
        })?;

        let status = response.status();
        let payload = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(|source| {
                Attempt::retryable(format!("could not read response body: {source}"))
            })?;

        if !status.is_success() {
            let excerpt = String::from_utf8_lossy(&payload)
                .chars()
                .take(200)
                .collect::<String>();
            let mut message = format!("node returned HTTP {status}: {excerpt}");
            // A node that answers properly rather than hanging up says so with a status.
            if matches!(status.as_u16(), 401 | 403) {
                message = format!("{message}; {AUTHENTICATION_HINT}");
            }
            return Err(if status.is_server_error() {
                Attempt::retryable(message)
            } else {
                Attempt::final_failure(message)
            });
        }

        Ok(payload)
    }
}

impl RpcTransport for HttpTransport {
    fn call(
        &self,
        method: &str,
        params: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, ReconcileError> {
        let body = serde_json::to_vec(&dto::Request::new(method, params)).map_err(|source| {
            ReconcileError::Internal {
                reason: format!("could not encode a {method} request: {source}"),
            }
        })?;

        let mut attempt = 1_u32;
        let payload = loop {
            match self.attempt(&body) {
                Ok(payload) => break payload,
                Err(failure) if failure.retryable && attempt < MAX_ATTEMPTS => {
                    std::thread::sleep(RETRY_BACKOFF.saturating_mul(attempt));
                    attempt = attempt.saturating_add(1);
                }
                Err(failure) => {
                    return Err(ReconcileError::Rpc(self.authentication.scrub(&format!(
                        "{method} failed after {attempt} attempt(s): {}",
                        failure.message
                    ))));
                }
            }
        };

        decode_result(method, &payload).map_err(|error| match error {
            ReconcileError::Rpc(message) => {
                ReconcileError::Rpc(self.authentication.scrub(&message))
            }
            other => other,
        })
    }
}

/// One failed attempt, and whether repeating it could succeed.
struct Attempt {
    message: String,
    retryable: bool,
}

impl Attempt {
    fn retryable(message: String) -> Self {
        Self {
            message,
            retryable: true,
        }
    }

    /// A failure that repeating cannot fix, such as a rejected request.
    fn final_failure(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

/// Whether a transport failure is consistent with an authentication mistake.
///
/// Zebra closes the connection without an HTTP response when credentials are missing or
/// wrong, so an authentication mistake reaches this code as a peer that hung up on a
/// connection it had already accepted. A refused connection, a name that does not resolve,
/// and a timeout cannot have that cause, and naming authentication for those sends the
/// reader after a credential problem that does not exist.
///
/// Measured against Zebra 6.2.3 on 2026-08-01: a wrong cookie yields
/// `io: Peer disconnected`, which `ureq` raises as [`std::io::ErrorKind::UnexpectedEof`];
/// a closed port yields `io: Connection refused`; an unresolvable host yields
/// `io: failed to lookup address information`. Peers that reset rather than close cleanly
/// are admitted too, since the symptom is the same hang-up.
fn could_be_authentication(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Io(source) => matches!(
            source.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

/// Extracts the `result` member of a JSON-RPC response.
pub fn decode_result(method: &str, payload: &[u8]) -> Result<serde_json::Value, ReconcileError> {
    let response: dto::Response = serde_json::from_slice(payload).map_err(|source| {
        ReconcileError::Rpc(format!(
            "{method} returned an unreadable response: {source}"
        ))
    })?;

    if let Some(error) = response.error {
        return Err(ReconcileError::Rpc(format!(
            "{method} was rejected by the node: {} (code {})",
            error.message, error.code
        )));
    }

    response.result.ok_or_else(|| {
        ReconcileError::Rpc(format!("{method} returned neither a result nor an error"))
    })
}

/// Rejects an endpoint this transport cannot serve honestly.
fn validate_endpoint(url: &str) -> Result<(), ReconcileError> {
    if url.starts_with("http://") {
        return Ok(());
    }

    let reason = if url.starts_with("https://") {
        "--rpc-url must be an http:// endpoint; Zebra's RPC port does not offer TLS, and \
         this tool does not link a TLS stack it could not use"
    } else {
        "--rpc-url must be an http:// endpoint"
    };

    Err(ReconcileError::InvalidInput {
        reason: reason.to_owned(),
    })
}

/// Whether an endpoint addresses this machine.
///
/// Used only to decide whether to warn about sending credentials in the clear, so a host
/// it cannot classify is treated as remote.
fn endpoint_is_loopback(url: &str) -> bool {
    let Some(after_scheme) = url.split("://").nth(1) else {
        return false;
    };

    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host_and_port = authority.rsplit('@').next().unwrap_or(authority);

    let host = match host_and_port.strip_prefix('[') {
        // An IPv6 literal is bracketed, and its own colons must not be read as a port.
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host_and_port.split(':').next().unwrap_or(host_and_port),
    };

    host == "localhost" || host == "::1" || host.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::auth::Secret;

    #[test]
    fn an_https_endpoint_is_refused_with_a_reason() {
        let error = validate_endpoint("https://node.example/").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("http://"), "{message}");
        assert!(message.contains("TLS"), "{message}");
    }

    #[test]
    fn a_non_http_endpoint_is_refused() {
        for url in ["node.example:8232", "ftp://node/", "", "//node"] {
            assert!(validate_endpoint(url).is_err(), "accepted {url:?}");
        }
    }

    #[test]
    fn an_http_endpoint_is_accepted() {
        assert!(validate_endpoint("http://127.0.0.1:8232/").is_ok());
    }

    #[test]
    fn loopback_endpoints_are_recognised() {
        for url in [
            "http://127.0.0.1:8232/",
            "http://127.0.0.1:8232",
            "http://localhost:18232/",
            "http://[::1]:8232/",
            "http://user:pass@127.0.0.1:8232/",
        ] {
            assert!(endpoint_is_loopback(url), "not recognised as local: {url}");
        }
    }

    #[test]
    fn remote_endpoints_are_recognised() {
        for url in [
            "http://192.168.1.5:8232/",
            "http://node.example:8232/",
            "http://127x.example/",
        ] {
            assert!(
                !endpoint_is_loopback(url),
                "wrongly treated as local: {url}"
            );
        }
    }

    #[test]
    fn a_hang_up_on_an_accepted_connection_could_be_authentication() {
        for kind in [
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
        ] {
            let error = ureq::Error::Io(std::io::Error::new(kind, "Peer disconnected"));
            assert!(could_be_authentication(&error), "{kind:?}");
        }
    }

    #[test]
    fn an_unreachable_node_is_never_blamed_on_authentication() {
        // Observed against Zebra 6.2.3: a closed port and an unresolvable host both reach
        // the transport as an `io` failure, and neither can be a credential problem.
        let unreachable = [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::HostUnreachable,
            std::io::ErrorKind::NetworkUnreachable,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::Other,
        ];
        for kind in unreachable {
            let error = ureq::Error::Io(std::io::Error::new(kind, "Connection refused"));
            assert!(!could_be_authentication(&error), "{kind:?}");
        }

        assert!(!could_be_authentication(&ureq::Error::HostNotFound));
        assert!(!could_be_authentication(&ureq::Error::ConnectionFailed));
    }

    #[test]
    fn a_result_member_is_extracted() {
        let value = decode_result(
            "getinfo",
            br#"{"jsonrpc":"2.0","id":1,"result":{"blocks":7}}"#,
        )
        .unwrap();
        assert_eq!(value, serde_json::json!({"blocks": 7}));
    }

    #[test]
    fn an_application_error_becomes_an_rpc_failure() {
        // Recorded verbatim from Zebra 6.2.3, which serves this with HTTP 200.
        let error = decode_result(
            "getblock",
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-8,"message":"block height not in best chain"}}"#,
        )
        .unwrap_err();

        assert!(matches!(error, ReconcileError::Rpc(_)));
        let message = error.to_string();
        assert!(
            message.contains("block height not in best chain"),
            "{message}"
        );
        assert!(message.contains("-8"), "{message}");
    }

    #[test]
    fn a_response_with_neither_member_is_a_failure() {
        assert!(decode_result("getinfo", br#"{"jsonrpc":"2.0","id":1}"#).is_err());
    }

    #[test]
    fn an_unreadable_response_is_a_failure() {
        assert!(decode_result("getinfo", b"<html>gateway error</html>").is_err());
    }

    #[test]
    fn the_rate_limiter_derives_an_integral_interval() {
        assert_eq!(
            RateLimiter::per_second(10).unwrap().minimum_interval(),
            Duration::from_millis(100)
        );
        assert_eq!(
            RateLimiter::per_second(1).unwrap().minimum_interval(),
            Duration::from_millis(1_000)
        );
        assert_eq!(
            RateLimiter::per_second(4).unwrap().minimum_interval(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn a_zero_rate_is_refused_rather_than_stalling_forever() {
        assert!(matches!(
            RateLimiter::per_second(0),
            Err(ReconcileError::InvalidInput { .. })
        ));
    }

    #[test]
    fn the_rate_limiter_paces_successive_requests() {
        let limiter = RateLimiter::per_second(50);
        let limiter = limiter.unwrap();
        let started = Instant::now();
        limiter.wait();
        limiter.wait();
        assert!(
            started.elapsed() >= Duration::from_millis(20),
            "the second request was not delayed"
        );
    }

    #[test]
    fn a_credentialed_remote_endpoint_is_detected() {
        let transport = HttpTransport::new(
            "http://192.168.1.5:8232/",
            Authentication::Basic {
                user: "user".to_owned(),
                secret: Secret::new("password"),
            },
            Duration::from_secs(5),
            10,
        )
        .unwrap();
        assert!(transport.sends_credentials_off_host());
    }

    #[test]
    fn a_credentialed_local_endpoint_is_not_flagged() {
        let transport = HttpTransport::new(
            "http://127.0.0.1:8232/",
            Authentication::Basic {
                user: "user".to_owned(),
                secret: Secret::new("password"),
            },
            Duration::from_secs(5),
            10,
        )
        .unwrap();
        assert!(!transport.sends_credentials_off_host());
    }

    #[test]
    fn an_anonymous_remote_endpoint_is_not_flagged() {
        let transport = HttpTransport::new(
            "http://192.168.1.5:8232/",
            Authentication::Anonymous,
            Duration::from_secs(5),
            10,
        )
        .unwrap();
        assert!(!transport.sends_credentials_off_host());
    }
}
