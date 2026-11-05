//! The GitHub Actions OIDC token as a rafn credential.
//!
//! A job with `permissions: id-token: write` gets two environment variables
//! from the runner: a URL and a short-lived request token. A GET on the URL
//! with that request token and `&audience=<aud>` returns `{"value": "<jwt>"}`;
//! the JWT proves which repository and workflow run is asking, so a workflow
//! needs no stored secret. The rafn backend accepts it as a bearer credential
//! for the audience [`AUDIENCE`].

use std::fmt;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};
use reqwest::redirect;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

use super::loopback::is_loopback;

/// Set to `true` by the runner in every GitHub Actions job.
pub(super) const ACTIONS_ENV: &str = "GITHUB_ACTIONS";
/// Where to request the token. Present only when the job has `id-token: write`.
pub(super) const REQUEST_URL_ENV: &str = "ACTIONS_ID_TOKEN_REQUEST_URL";
/// Bearer value authorizing that request.
pub(super) const REQUEST_TOKEN_ENV: &str = "ACTIONS_ID_TOKEN_REQUEST_TOKEN";

/// The `aud` claim the backend expects.
const AUDIENCE: &str = "rafn";

/// The CLI is short-lived and waits on this before doing its work, so a
/// stalled endpoint must fail fast and say what it was waiting for.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on the response body. A token response is a few kilobytes; the
/// cap only stops a misbehaving endpoint from filling memory.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Why the token could not be obtained. Messages never include the request
/// token, the JWT, or any part of the response body: a mis-shaped response is
/// as likely to contain the secret as a well-formed one.
#[derive(Debug, Error)]
pub enum ActionsTokenError {
    #[error(
        "ACTIONS_ID_TOKEN_REQUEST_URL is not a usable URL ({reason}). GitHub Actions \
         provides an https:// URL there when the job has `permissions: id-token: write`."
    )]
    InvalidRequestUrl { reason: String },

    #[error(
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN cannot be sent as an HTTP header value \
         (it must be a single line of visible ASCII)."
    )]
    InvalidRequestToken,

    #[error("could not set up the HTTP client for the GitHub Actions OIDC token request")]
    Client {
        #[source]
        source: reqwest::Error,
    },

    /// The cause is kept in the error chain without the request URL, which
    /// reqwest would otherwise print: it carries job identifiers that have no
    /// business in a build log. The message names only the host.
    #[error("could not request a GitHub Actions OIDC token from {host}")]
    Request {
        host: String,
        #[source]
        source: reqwest::Error,
    },

    #[error(
        "timed out after {after:?} waiting for {host} to issue a GitHub Actions OIDC \
         token. Re-run the job, or set RAFN_API_KEY to authenticate with an API key."
    )]
    TimedOut { host: String, after: Duration },

    #[error(
        "{host} answered the GitHub Actions OIDC token request with HTTP status \
         {status}. Check that the job has `permissions: id-token: write`, or set \
         RAFN_API_KEY to authenticate with an API key instead."
    )]
    Rejected { host: String, status: u16 },

    #[error(
        "the GitHub Actions OIDC token response from {host} is not a JSON object with \
         a string \"value\" field ({reason})"
    )]
    BadResponse { host: String, reason: &'static str },

    #[error("the GitHub Actions OIDC token response from {host} is larger than 64 KiB")]
    BodyTooLarge { host: String },

    #[error(
        "the GitHub Actions OIDC token is not a well-formed JWT: expected three \
         non-empty dot-separated base64url segments"
    )]
    Malformed,
}

/// A GitHub Actions OIDC JWT, checked to be shaped like one.
///
/// As with [`ApiKey`](super::ApiKey), validating once here means the code
/// attaching it to an RPC never meets a value that cannot be a header, and
/// `Debug` is redacted with no `Display`, so the token cannot leak through
/// `{:?}` of an enclosing struct or an error chain.
#[derive(Clone)]
pub struct ActionsToken(String);

impl ActionsToken {
    /// Only the shape is checked: three non-empty base64url segments. The
    /// signature and claims are the backend's to verify, and rafn has no key
    /// to verify them with.
    pub fn parse(raw: &str) -> Result<Self, ActionsTokenError> {
        let token = raw.trim();
        let mut segments = token.split('.');
        let shape = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        );
        match shape {
            (Some(a), Some(b), Some(c), None)
                if [a, b, c].into_iter().all(is_base64url_segment) =>
            {
                Ok(Self(token.to_string()))
            }
            _ => Err(ActionsTokenError::Malformed),
        }
    }

    /// The raw token. Named loudly so each call site is easy to audit.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

fn is_base64url_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl fmt::Debug for ActionsToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActionsToken(<redacted>)")
    }
}

/// A validated, not yet performed, request for the token.
///
/// Everything that can be wrong locally (the URL's scheme and host, the
/// request token as a header) is checked in [`parse`](Self::parse), so a bad
/// environment fails before any socket is opened.
#[derive(Clone)]
pub struct ActionsTokenRequest {
    url: Url,
    authorization: HeaderValue,
}

impl ActionsTokenRequest {
    pub fn parse(request_url: &str, request_token: &str) -> Result<Self, ActionsTokenError> {
        let invalid = |reason: String| ActionsTokenError::InvalidRequestUrl { reason };

        let mut url = Url::parse(request_url.trim()).map_err(|e| invalid(e.to_string()))?;
        match (url.scheme(), url.host()) {
            (_, None) => return Err(invalid("it has no host".to_string())),
            ("https", _) => {}
            // Same rule as the API URL: the request token is a bearer
            // credential, so it may cross only an encrypted link or stay on
            // this machine.
            ("http", Some(host)) if is_loopback(&host) => {}
            ("http", Some(host)) => {
                return Err(invalid(format!(
                    "plain http:// is accepted only for loopback hosts, not {host}"
                )));
            }
            (scheme, _) => return Err(invalid(format!("unsupported scheme \"{scheme}\""))),
        }

        // The runner's URL already carries `?api-version=...`; `query_pairs_mut`
        // appends with `&` there and starts a query string otherwise.
        url.query_pairs_mut().append_pair("audience", AUDIENCE);

        let mut authorization = HeaderValue::from_str(&format!("bearer {}", request_token.trim()))
            .map_err(|_| ActionsTokenError::InvalidRequestToken)?;
        // Keeps the value out of the HTTP client's debug output.
        authorization.set_sensitive(true);

        Ok(Self { url, authorization })
    }

    /// Perform the request, bounded by a 10 second deadline.
    pub async fn fetch(&self) -> Result<ActionsToken, ActionsTokenError> {
        self.fetch_within(FETCH_TIMEOUT).await
    }

    pub(super) async fn fetch_within(
        &self,
        limit: Duration,
    ) -> Result<ActionsToken, ActionsTokenError> {
        let host = self.url.host_str().unwrap_or_default().to_string();

        let body = self.get(&host, limit).await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            value: String,
        }
        // The serde error is dropped on purpose: it can quote the offending
        // value, and the value is the secret.
        let TokenResponse { value } =
            serde_json::from_slice(&body).map_err(|_| ActionsTokenError::BadResponse {
                host,
                reason: "invalid JSON or missing \"value\"",
            })?;

        ActionsToken::parse(&value)
    }

    /// One GET, returning the body of a success response.
    ///
    /// The client is built per call and nothing is pooled: the request is made
    /// once per process. Redirects are refused so the bearer header cannot be
    /// replayed to another host, and `timeout` covers connecting, TLS and
    /// reading the whole body.
    async fn get(&self, host: &str, limit: Duration) -> Result<Vec<u8>, ActionsTokenError> {
        install_crypto_provider();
        let client = reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .timeout(limit)
            .user_agent(concat!("rafn/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|source| ActionsTokenError::Client {
                source: source.without_url(),
            })?;
        let failed = |error: reqwest::Error| request_failed(error, host, limit);

        let mut response = client
            .get(self.url.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(failed)?;

        // Checked before the body is read: an error page is not worth
        // downloading, and its text is never shown.
        if !response.status().is_success() {
            return Err(ActionsTokenError::Rejected {
                host: host.to_string(),
                status: response.status().as_u16(),
            });
        }

        let too_large = || ActionsTokenError::BodyTooLarge {
            host: host.to_string(),
        };
        if response
            .content_length()
            .is_some_and(|declared| declared > MAX_BODY_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(failed)? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// reqwest has no built-in crypto provider here: `rustls-no-provider` keeps
/// aws-lc-rs (and the C toolchain it needs on some targets) out of the build,
/// so one must be installed process-wide before a client is built. This is
/// the same ring provider the gRPC channel is configured with.
fn install_crypto_provider() {
    // `Err` only means a provider is already installed, by an earlier call or
    // by another crate; that one serves just as well.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn request_failed(error: reqwest::Error, host: &str, limit: Duration) -> ActionsTokenError {
    if error.is_timeout() {
        ActionsTokenError::TimedOut {
            host: host.to_string(),
            after: limit,
        }
    } else {
        ActionsTokenError::Request {
            host: host.to_string(),
            source: error.without_url(),
        }
    }
}

impl fmt::Debug for ActionsTokenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActionsTokenRequest(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZXBvOmFjbWUvd2lkZ2V0cyJ9.c2ln-_bmF0dXJl";

    #[test]
    fn parse_keeps_a_three_segment_base64url_token() -> Result<()> {
        assert_eq!(ActionsToken::parse(JWT)?.expose_secret(), JWT);
        assert_eq!(
            ActionsToken::parse(&format!("  {JWT}\n"))?.expose_secret(),
            JWT
        );
        Ok(())
    }

    #[test]
    fn parse_rejects_anything_that_is_not_three_base64url_segments() {
        for bad in [
            "", "   ", "token", "a.b", "a.b.c.d", "a..c", ".b.c", "a.b.",
            // Standard base64 and padding are not base64url.
            "a+b.c.d", "a/b.c.d", "a.b.c=", "a b.c.d", "a.b\n.c", "é.b.c", "\0.b.c",
        ] {
            assert!(
                matches!(ActionsToken::parse(bad), Err(ActionsTokenError::Malformed)),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn debug_is_redacted_even_when_pretty_printed() -> Result<()> {
        let token = ActionsToken::parse(JWT)?;
        for rendered in [format!("{token:?}"), format!("{token:#?}")] {
            assert_eq!(
                rendered.replace(char::is_whitespace, ""),
                "ActionsToken(<redacted>)"
            );
        }
        Ok(())
    }

    #[test]
    fn the_request_url_gets_the_audience_after_its_existing_query() -> Result<()> {
        let request =
            ActionsTokenRequest::parse("https://tokens.example/oidc?api-version=2.0", "t")?;
        assert_eq!(
            request.url.as_str(),
            "https://tokens.example/oidc?api-version=2.0&audience=rafn"
        );
        let request = ActionsTokenRequest::parse("https://tokens.example/oidc", "t")?;
        assert_eq!(
            request.url.as_str(),
            "https://tokens.example/oidc?audience=rafn"
        );
        Ok(())
    }

    #[test]
    fn the_authorization_header_is_a_sensitive_bearer() -> Result<()> {
        let request = ActionsTokenRequest::parse("https://tokens.example/oidc", " t0k ")?;
        assert_eq!(request.authorization.to_str()?, "bearer t0k");
        assert!(request.authorization.is_sensitive());
        Ok(())
    }

    #[test]
    fn plain_http_is_accepted_only_for_loopback() {
        for ok in [
            "http://localhost:8080/oidc",
            "http://127.0.0.1/oidc",
            "http://[::1]:9/oidc",
        ] {
            assert!(ActionsTokenRequest::parse(ok, "t").is_ok(), "{ok}");
        }
        for bad in [
            "http://tokens.example/oidc",
            "http://localhost.evil.example/oidc",
            "http://localhost@evil.example/oidc",
            "ftp://tokens.example/oidc",
            "file:///etc/passwd",
            "tokens.example/oidc",
            "",
        ] {
            assert!(
                matches!(
                    ActionsTokenRequest::parse(bad, "t"),
                    Err(ActionsTokenError::InvalidRequestUrl { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn debug_of_a_request_hides_the_url_and_the_token() -> Result<()> {
        let request = ActionsTokenRequest::parse("https://tokens.example/oidc", "SECRET")?;
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("SECRET"), "{rendered}");
        assert!(!rendered.contains("tokens.example"), "{rendered}");
        Ok(())
    }

    #[test]
    fn error_messages_name_the_variables_a_user_would_look_for() {
        let url = ActionsTokenError::InvalidRequestUrl {
            reason: "x".to_string(),
        }
        .to_string();
        assert!(url.contains(REQUEST_URL_ENV), "{url}");
        let token = ActionsTokenError::InvalidRequestToken.to_string();
        assert!(token.contains(REQUEST_TOKEN_ENV), "{token}");
    }

    // The tests below run the real client against a local endpoint
    // (`fake_endpoint`), pinning the behaviour that distinguishes a credential
    // fetch from an ordinary GET: it never leaves the host it was pointed at,
    // reads a bounded amount, and says nothing about what the server returned.

    use std::io::Read;
    use std::net::{TcpListener, TcpStream};

    use anyhow::Context;

    use crate::config::fake_endpoint::{FakeEndpoint, Reply};

    const LONG_TIMEOUT: Duration = Duration::from_secs(10);

    fn request_to(url: &str) -> Result<ActionsTokenRequest> {
        Ok(ActionsTokenRequest::parse(url, "request-token")?)
    }

    fn token_json(pad: usize) -> String {
        format!(r#"{{"value":"{JWT}","pad":"{}"}}"#, "a".repeat(pad))
    }

    #[tokio::test]
    async fn sends_one_get_with_the_expected_headers() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(token_json(0)))?;
        let request = request_to(&endpoint.url("/a/b?x=1"))?;

        let token = request.fetch_within(LONG_TIMEOUT).await?;

        assert_eq!(token.expose_secret(), JWT);
        let seen = endpoint.requests()?;
        assert_eq!(seen.len(), 1);
        let head = seen.first().context("no request")?;
        assert_eq!(head.request_line, "GET /a/b?x=1&audience=rafn HTTP/1.1");
        assert_eq!(head.header("authorization"), Some("bearer request-token"));
        assert_eq!(head.header("accept"), Some("application/json"));
        assert_eq!(
            head.header("user-agent"),
            Some(concat!("rafn/", env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(head.header("host"), Some(request.url.authority()));
        Ok(())
    }

    #[tokio::test]
    async fn a_refusal_never_shows_the_body_the_server_sent() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::status(418, "SECRET-in-an-error-page"))?;

        let error = request_to(&endpoint.url("/"))?
            .fetch_within(LONG_TIMEOUT)
            .await
            .err()
            .context("418 should be refused")?;

        assert!(
            matches!(error, ActionsTokenError::Rejected { status: 418, .. }),
            "{error:?}"
        );
        assert!(!format!("{error:#} {error:?}").contains("SECRET"));
        Ok(())
    }

    #[tokio::test]
    async fn redirects_are_not_followed() -> Result<()> {
        let target = FakeEndpoint::start(Reply::json(token_json(0)))?;
        let endpoint = FakeEndpoint::start(Reply::redirect(&target.url("/stolen")))?;

        let error = request_to(&endpoint.url("/"))?
            .fetch_within(LONG_TIMEOUT)
            .await
            .err()
            .context("a redirect should be refused")?;

        assert!(
            matches!(error, ActionsTokenError::Rejected { status: 302, .. }),
            "{error:?}"
        );
        assert!(
            target.requests()?.is_empty(),
            "the bearer header must not follow a redirect"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_declared_oversized_body_is_refused() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(token_json(MAX_BODY_BYTES)))?;

        let error = request_to(&endpoint.url("/"))?
            .fetch_within(LONG_TIMEOUT)
            .await
            .err()
            .context("a body over the cap should fail")?;

        assert!(
            matches!(error, ActionsTokenError::BodyTooLarge { .. }),
            "{error:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_undeclared_oversized_body_is_refused_while_streaming() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::chunked_json(token_json(MAX_BODY_BYTES)))?;

        let error = request_to(&endpoint.url("/"))?
            .fetch_within(LONG_TIMEOUT)
            .await
            .err()
            .context("a streamed body over the cap should fail")?;

        assert!(
            matches!(error, ActionsTokenError::BodyTooLarge { .. }),
            "{error:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_body_exactly_at_the_cap_is_accepted() -> Result<()> {
        let overhead = token_json(0).len();
        let body = token_json(MAX_BODY_BYTES - overhead);
        assert_eq!(body.len(), MAX_BODY_BYTES);

        for reply in [Reply::json(body.clone()), Reply::chunked_json(body)] {
            let endpoint = FakeEndpoint::start(reply)?;
            let token = request_to(&endpoint.url("/"))?
                .fetch_within(LONG_TIMEOUT)
                .await?;
            assert_eq!(token.expose_secret(), JWT);
        }
        Ok(())
    }

    #[tokio::test]
    async fn the_request_url_is_kept_out_of_errors() -> Result<()> {
        let url = format!("{}jobs/JOB-ID-123/oidctoken", FakeEndpoint::closed_url()?);

        let error = request_to(&url)?
            .fetch_within(LONG_TIMEOUT)
            .await
            .err()
            .context("nothing is listening")?;

        assert!(
            matches!(error, ActionsTokenError::Request { .. }),
            "{error:?}"
        );
        let rendered = format!("{error} {error:#} {error:?}");
        assert!(!rendered.contains("JOB-ID-123"), "{rendered}");
        assert!(rendered.contains("127.0.0.1"), "{rendered}");
        Ok(())
    }

    #[tokio::test]
    async fn https_opens_with_a_tls_handshake() -> Result<()> {
        // The listener is a raw socket, not a TLS server: all that matters is
        // the first bytes the client sends. A TLS record starts with content
        // type 0x16 (handshake) and legacy version 0x03xx. Without this, a
        // client that sent plain HTTP to an `https://` URL would pass every
        // other test here, which only talk to loopback `http://`.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let request = request_to(&format!("https://{addr}/oidc"))?;

        let capture = std::thread::spawn(move || -> Result<Vec<u8>> {
            let (mut socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut head = [0u8; 2];
            let len = socket.read(&mut head)?;
            Ok(head[..len].to_vec())
        });

        let result = request.fetch_within(LONG_TIMEOUT).await;

        // If the client never connected, unblock `accept` so the failure below
        // is an assertion rather than a hang.
        let _ = TcpStream::connect(addr);
        let head = capture
            .join()
            .map_err(|_| anyhow::anyhow!("capture thread panicked"))??;
        assert_eq!(
            head.get(..2),
            Some(&[0x16, 0x03][..]),
            "first bytes: {head:?}"
        );
        // The server hung up mid-handshake, so the request itself fails.
        assert!(result.is_err());
        Ok(())
    }
}
