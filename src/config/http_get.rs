//! The one plain HTTPS request rafn makes outside gRPC: fetching the GitHub
//! Actions OIDC token.
//!
//! Deliberately not a general client. It sends one authenticated GET, never
//! follows redirects (so the bearer header cannot be replayed to another
//! host), caps the response size, and leaves the overall deadline to the
//! caller. TLS is the same stack the gRPC channel uses: rustls with ring and
//! the OS trust store (docs/adr/0001-rustls-ring-native-roots.md).

use std::error::Error as StdError;
use std::io;
use std::net::IpAddr;
use std::sync::Arc;

use http_body_util::{BodyExt, Empty, LengthLimitError, Limited};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::header::{ACCEPT, AUTHORIZATION, CONNECTION, HOST, HeaderValue, USER_AGENT};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use url::{Host, Position, Url};

/// Upper bound on the response body. A token response is a few kilobytes; the
/// cap only stops a misbehaving endpoint from filling memory.
pub(super) const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(super) struct Response {
    pub status: StatusCode,
    pub body: Bytes,
}

#[derive(Debug, Error)]
pub enum GetError {
    #[error("the URL has no host or port to connect to")]
    NoAuthority,

    #[error("unsupported URL scheme \"{0}\"")]
    UnsupportedScheme(String),

    #[error("could not connect: {0}")]
    Connect(#[source] io::Error),

    #[error(
        "no usable CA certificates were found in the system trust store{0}; \
         install your platform's CA bundle (for example the ca-certificates package)"
    )]
    NoTrustAnchors(String),

    #[error("could not set up TLS: {0}")]
    TlsConfig(#[source] rustls::Error),

    #[error("the host name is not valid for TLS")]
    InvalidServerName,

    #[error("TLS handshake failed: {0}")]
    Tls(#[source] io::Error),

    #[error("could not build the HTTP request: {0}")]
    Request(#[source] hyper::http::Error),

    #[error("HTTP error: {0}")]
    Http(#[source] hyper::Error),

    #[error("the response body is larger than {MAX_BODY_BYTES} bytes")]
    BodyTooLarge,

    #[error("could not read the response body: {0}")]
    Body(#[source] Box<dyn StdError + Send + Sync>),
}

/// `GET url` with the given `Authorization` header value.
///
/// Any HTTP status is returned as a [`Response`], not an error: whether a 4xx
/// is fatal is for the caller to say.
pub(super) async fn get(url: &Url, authorization: &HeaderValue) -> Result<Response, GetError> {
    let host = url.host().ok_or(GetError::NoAuthority)?;
    let port = url.port_or_known_default().ok_or(GetError::NoAuthority)?;

    // `Host`'s Display brackets IPv6 literals, which `connect` does not take.
    let bare_host = match &host {
        Host::Domain(name) => (*name).to_string(),
        Host::Ipv4(ip) => ip.to_string(),
        Host::Ipv6(ip) => ip.to_string(),
    };
    let tcp = TcpStream::connect((bare_host, port))
        .await
        .map_err(GetError::Connect)?;

    match url.scheme() {
        "https" => {
            let name = server_name(&host)?;
            let tls = tls_connector()?
                .connect(name, tcp)
                .await
                .map_err(GetError::Tls)?;
            exchange(url, authorization, TokioIo::new(tls)).await
        }
        "http" => exchange(url, authorization, TokioIo::new(tcp)).await,
        other => Err(GetError::UnsupportedScheme(other.to_string())),
    }
}

fn server_name(host: &Host<&str>) -> Result<ServerName<'static>, GetError> {
    match host {
        Host::Domain(name) => {
            ServerName::try_from((*name).to_string()).map_err(|_| GetError::InvalidServerName)
        }
        Host::Ipv4(ip) => Ok(ServerName::IpAddress(IpAddr::V4(*ip).into())),
        Host::Ipv6(ip) => Ok(ServerName::IpAddress(IpAddr::V6(*ip).into())),
    }
}

fn tls_connector() -> Result<TlsConnector, GetError> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let (added, _unparsable) = roots.add_parsable_certificates(native.certs);
    if added == 0 {
        let detail = native
            .errors
            .first()
            .map(|error| format!(" ({error})"))
            .unwrap_or_default();
        return Err(GetError::NoTrustAnchors(detail));
    }

    // Named explicitly rather than relying on a process-wide default provider,
    // which nothing in this crate installs: tonic is handed its own.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(GetError::TlsConfig)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Stops the task driving an HTTP/1 connection when the request is done or
/// abandoned (for example by the caller's timeout).
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn exchange<I>(url: &Url, authorization: &HeaderValue, io: I) -> Result<Response, GetError>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, connection) = http1::handshake::<_, Empty<Bytes>>(io)
        .await
        .map_err(GetError::Http)?;
    // hyper moves bytes only while the connection future is polled. Its result
    // is ignored because a failure on the wire also surfaces from
    // `send_request` or the body read below, with the better error.
    let _driver = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));

    let request = Request::builder()
        .method(Method::GET)
        .uri(&url[Position::BeforePath..Position::AfterQuery])
        .header(HOST, &url[Position::BeforeHost..Position::AfterPort])
        .header(AUTHORIZATION, authorization.clone())
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, concat!("rafn/", env!("CARGO_PKG_VERSION")))
        // One request per connection; also lets the driver task finish.
        .header(CONNECTION, "close")
        .body(Empty::new())
        .map_err(GetError::Request)?;

    let response = sender.send_request(request).await.map_err(GetError::Http)?;
    let (head, body) = response.into_parts();
    let body = Limited::new(body, MAX_BODY_BYTES)
        .collect()
        .await
        .map_err(body_error)?
        .to_bytes();

    Ok(Response {
        status: head.status,
        body,
    })
}

fn body_error(error: Box<dyn StdError + Send + Sync>) -> GetError {
    if error.is::<LengthLimitError>() {
        GetError::BodyTooLarge
    } else {
        GetError::Body(error)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::net::TcpListener;

    use anyhow::{Context, Result};

    use super::*;
    use crate::config::fake_endpoint::{FakeEndpoint, Reply};

    fn bearer() -> Result<HeaderValue> {
        Ok(HeaderValue::from_static("bearer test"))
    }

    #[tokio::test]
    async fn sends_one_get_with_the_expected_headers() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(r#"{"ok":true}"#))?;
        let url = Url::parse(&endpoint.url("/a/b?x=1"))?;

        let response = get(&url, &bearer()?).await?;

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(&response.body[..], br#"{"ok":true}"#);
        let seen = endpoint.requests()?;
        let request = seen.first().context("no request")?;
        assert_eq!(request.request_line, "GET /a/b?x=1 HTTP/1.1");
        assert_eq!(request.header("authorization"), Some("bearer test"));
        assert_eq!(request.header("accept"), Some("application/json"));
        assert_eq!(
            request.header("user-agent"),
            Some(concat!("rafn/", env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(request.header("host"), Some(url.authority()));
        Ok(())
    }

    #[tokio::test]
    async fn error_statuses_are_returned_not_raised() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::status(418, "teapot"))?;
        let url = Url::parse(&endpoint.url("/"))?;

        let response = get(&url, &bearer()?).await?;

        assert_eq!(response.status.as_u16(), 418);
        Ok(())
    }

    #[tokio::test]
    async fn redirects_are_not_followed() -> Result<()> {
        let target = FakeEndpoint::start(Reply::json("{}"))?;
        let endpoint = FakeEndpoint::start(Reply::redirect(&target.url("/stolen")))?;
        let url = Url::parse(&endpoint.url("/"))?;

        let response = get(&url, &bearer()?).await?;

        assert_eq!(response.status, StatusCode::FOUND);
        assert!(
            target.requests()?.is_empty(),
            "the bearer header must not follow a redirect"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_oversized_body_is_refused() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(vec![b'a'; MAX_BODY_BYTES + 1]))?;
        let url = Url::parse(&endpoint.url("/"))?;

        let error = get(&url, &bearer()?)
            .await
            .err()
            .context("a body over the cap should fail")?;

        assert!(matches!(error, GetError::BodyTooLarge), "{error:?}");
        Ok(())
    }

    #[tokio::test]
    async fn a_body_at_the_cap_is_accepted() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(vec![b'a'; MAX_BODY_BYTES]))?;
        let url = Url::parse(&endpoint.url("/"))?;

        let response = get(&url, &bearer()?).await?;

        assert_eq!(response.body.len(), MAX_BODY_BYTES);
        Ok(())
    }

    #[tokio::test]
    async fn refused_connections_report_the_io_error() -> Result<()> {
        let url = Url::parse(&FakeEndpoint::closed_url()?)?;

        let error = get(&url, &bearer()?)
            .await
            .err()
            .context("nothing is listening")?;

        assert!(matches!(error, GetError::Connect(_)), "{error:?}");
        Ok(())
    }

    #[tokio::test]
    async fn https_opens_with_a_tls_handshake() -> Result<()> {
        // The listener is a raw socket, not a TLS server: all that matters is
        // the first bytes the client sends. A TLS record starts with content
        // type 0x16 (handshake) and legacy version 0x03xx. Without this, a
        // branch that sent plain HTTP to an `https://` URL would pass every
        // other test here, which only talk to loopback `http://`.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = Url::parse(&format!("https://{}/", listener.local_addr()?))?;

        let capture = std::thread::spawn(move || -> Result<Vec<u8>> {
            let (mut socket, _) = listener.accept()?;
            let mut head = [0u8; 2];
            let len = socket.read(&mut head)?;
            Ok(head[..len].to_vec())
        });

        let result = get(&url, &bearer()?).await;

        let head = capture
            .join()
            .map_err(|_| anyhow::anyhow!("capture thread panicked"))??;
        assert_eq!(head.first(), Some(&0x16), "first bytes: {head:?}");
        assert_eq!(head.get(1), Some(&0x03), "first bytes: {head:?}");
        // The server hung up mid-handshake, so the request itself fails.
        assert!(result.is_err());
        Ok(())
    }
}
