//! The one place a gRPC channel to the rafn cloud service is built, so TLS
//! and authentication are applied identically to every RPC.

use std::fmt;

use anyhow::{Context, Result};
use thiserror::Error;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use url::{Host, Url};

use super::auth::AuthInterceptor;
use crate::config::ApiKey;
use crate::proto::pb::benchmark_service_client::BenchmarkServiceClient;

/// A benchmark-service client whose every call carries the API key.
pub type Client = BenchmarkServiceClient<InterceptedService<Channel, AuthInterceptor>>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServiceUrlError {
    #[error(
        "unsupported scheme \"{scheme}\" in API URL \"{url}\": use https:// \
         (or http:// to a loopback host such as localhost for a local \
         development server)"
    )]
    UnsupportedScheme { scheme: String, url: String },

    #[error(
        "refusing to send the API key over unencrypted http:// to \"{host}\" \
         (API URL \"{url}\"): use https://. Plain http:// is accepted only for \
         loopback hosts (localhost, 127.0.0.1, [::1]) used for local development."
    )]
    InsecureHost { host: String, url: String },

    #[error(
        "invalid API URL \"{url}\" ({reason}): expected a full URL such as \
         https://api.rafn.dev"
    )]
    Malformed { url: String, reason: String },
}

/// How the connection to the service is secured, decided by the URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// `http://` to a loopback host: no encryption, for local development
    /// servers. [`ServiceUrl::parse`] is the only way to get one, and it
    /// refuses every other host.
    Plaintext,
    /// `https://`: TLS verified against the platform's trust store.
    Tls,
}

/// A service URL whose scheme and host have been checked, so a channel can be
/// built from it without re-validating anything. Any scheme other than `http`
/// or `https` is rejected up front rather than silently downgraded to
/// plaintext, and `http` is accepted only for loopback hosts because the
/// bearer API key rides on every request: HTTP/2 header compression is an
/// encoding, not encryption, so anyone on the path could read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUrl {
    /// What the user wrote, for messages that quote it back.
    raw: String,
    /// The `url` crate's normalized spelling of `raw`, which is what the
    /// channel connects to. The loopback check ran on that crate's reading of
    /// the string, and the connector's own parser and the system resolver may
    /// read the raw text differently (`127.1` and `0x7f.1` are loopback to
    /// `url` but only hostnames to a resolver that does not expand shorthand
    /// IPv4). Connecting to the normalized form makes the host that was checked
    /// the host that is dialed.
    normalized: String,
    transport: Transport,
}

impl ServiceUrl {
    pub fn parse(raw: &str) -> Result<Self, ServiceUrlError> {
        let raw = raw.trim();
        let mut parsed = Url::parse(raw).map_err(|e| ServiceUrlError::Malformed {
            url: raw.to_string(),
            reason: e.to_string(),
        })?;

        let transport = match parsed.scheme() {
            "http" if parsed.host().is_some_and(|host| is_loopback(&host)) => Transport::Plaintext,
            "http" => {
                return Err(ServiceUrlError::InsecureHost {
                    host: parsed.host_str().unwrap_or_default().to_string(),
                    url: raw.to_string(),
                });
            }
            "https" => Transport::Tls,
            other => {
                return Err(ServiceUrlError::UnsupportedScheme {
                    scheme: other.to_string(),
                    url: raw.to_string(),
                });
            }
        };

        // Neither a fragment nor a query means anything on a base URL, and
        // dropping them keeps text such as `#@evil.example` out of the string
        // handed to the connector.
        parsed.set_fragment(None);
        parsed.set_query(None);

        Ok(Self {
            raw: raw.to_string(),
            normalized: parsed.into(),
            transport,
        })
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }
}

/// Whether traffic to `host` stays on this machine. Judged on the parsed host
/// rather than the raw string, so `localhost.evil.example` and
/// `http://localhost@evil.example` (whose host is `evil.example`) do not pass.
fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        // `url` lowercases domains, so this also matches `LOCALHOST`.
        Host::Domain(name) => *name == "localhost",
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

impl fmt::Display for ServiceUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Connect to `url` and return a client that sends `key` on every call.
///
/// Credentials are required by the signature, so a missing key is caught by
/// whoever resolves it, before any socket is opened.
pub async fn connect(url: &ServiceUrl, key: &ApiKey) -> Result<Client> {
    let interceptor = AuthInterceptor::new(key)?;

    let mut endpoint = Endpoint::from_shared(url.normalized.clone())
        .with_context(|| format!("Invalid API URL {url}"))?;

    if url.transport() == Transport::Tls {
        // Native roots rather than the bundled webpki set: a CLI runs on
        // developer laptops and CI runners, where corporate TLS-inspecting
        // proxies install their CA in the OS trust store and a bundled list
        // would reject them. Trade-off: a host with no CA bundle (a bare
        // container) fails with tonic's "no native certs" error and needs
        // `ca-certificates` installed. `with_enabled_roots` picks up exactly
        // the root features compiled in via Cargo.toml.
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_enabled_roots())
            .with_context(|| format!("Failed to configure TLS for {url}"))?;
    }

    let channel = endpoint
        .connect()
        .await
        .with_context(|| format!("Failed to connect to gRPC service at {url}"))?;

    Ok(BenchmarkServiceClient::with_interceptor(
        channel,
        interceptor,
    ))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn https_selects_tls() -> Result<()> {
        let url = ServiceUrl::parse("https://api.rafn.dev")?;
        assert_eq!(url.transport(), Transport::Tls);
        assert_eq!(url.to_string(), "https://api.rafn.dev");
        Ok(())
    }

    #[test]
    fn http_to_loopback_selects_plaintext() -> Result<()> {
        for raw in [
            "http://localhost:50051",
            "http://LOCALHOST:50051",
            "http://localhost",
            "http://127.0.0.1:50051",
            "http://127.0.0.1",
            // The whole 127.0.0.0/8 block is loopback, not just .1.
            "http://127.1.2.3:50051",
            "http://[::1]:50051",
            "HTTP://127.0.0.1:50051",
        ] {
            let url = ServiceUrl::parse(raw)?;
            assert_eq!(url.transport(), Transport::Plaintext, "{raw}");
        }
        Ok(())
    }

    #[test]
    fn http_to_a_remote_host_is_rejected_because_the_key_would_travel_in_clear() {
        for raw in [
            "http://api.rafn.dev",
            "http://rafn.example.com:50051",
            "http://my-service-abc123-ew.a.run.app",
            "http://192.168.1.214:47822",
            "http://10.0.0.5",
            "http://[2001:db8::1]:50051",
            // Unspecified, not loopback: binding to it says nothing about
            // where the peer is.
            "http://0.0.0.0:50051",
            // Hosts that merely look like loopback.
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example",
            "http://localhost@evil.example",
            "http://127.0.0.1@evil.example:50051",
            "http://evil.example#@localhost",
        ] {
            let err = ServiceUrl::parse(raw).err();
            assert!(
                matches!(err, Some(ServiceUrlError::InsecureHost { .. })),
                "{raw} -> {err:?}"
            );
        }
    }

    #[test]
    fn the_insecure_host_error_names_the_host_and_points_to_https() {
        let message = ServiceUrl::parse("http://192.168.1.214:47822")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(message.contains("192.168.1.214"), "{message}");
        assert!(message.contains("https://"), "{message}");
        assert!(message.contains("localhost"), "{message}");
    }

    #[test]
    fn the_checked_host_is_the_host_the_connector_dials() -> Result<()> {
        // Spellings `url` reads as loopback but a resolver might not, and
        // text after the host that the connector must not mistake for one.
        for (raw, dialed_host) in [
            ("http://127.1:50051", "127.0.0.1"),
            ("http://0x7f.1:50051", "127.0.0.1"),
            ("http://2130706433:50051", "127.0.0.1"),
            ("http://LOCALHOST:50051", "localhost"),
            ("http://[::1]:50051", "[::1]"),
            ("http://127.0.0.1:50051#@evil.example", "127.0.0.1"),
            ("http://127.0.0.1:50051/?@evil.example", "127.0.0.1"),
        ] {
            let url = ServiceUrl::parse(raw)?;
            assert_eq!(url.transport(), Transport::Plaintext, "{raw}");

            let endpoint = Endpoint::from_shared(url.normalized.clone())?;
            assert_eq!(endpoint.uri().host(), Some(dialed_host), "{raw}");
            assert!(!url.normalized.contains("evil"), "{}", url.normalized);
        }
        Ok(())
    }

    #[test]
    fn messages_quote_the_url_as_the_user_wrote_it() -> Result<()> {
        let url = ServiceUrl::parse("http://127.1:50051")?;
        assert_eq!(url.to_string(), "http://127.1:50051");
        Ok(())
    }

    #[test]
    fn https_is_accepted_for_any_host() -> Result<()> {
        for raw in [
            "https://api.rafn.dev",
            "https://192.168.1.214:47822",
            "https://localhost:50051",
        ] {
            assert_eq!(ServiceUrl::parse(raw)?.transport(), Transport::Tls, "{raw}");
        }
        Ok(())
    }

    #[test]
    fn scheme_is_case_insensitive_and_whitespace_is_trimmed() -> Result<()> {
        let url = ServiceUrl::parse("  HTTPS://api.rafn.dev:443\n")?;
        assert_eq!(url.transport(), Transport::Tls);
        assert_eq!(url.to_string(), "HTTPS://api.rafn.dev:443");
        Ok(())
    }

    #[test]
    fn other_schemes_are_rejected_with_guidance() {
        for raw in [
            "grpc://host:50051",
            "ftp://host",
            "ws://host",
            "file:///tmp/x",
        ] {
            let err = ServiceUrl::parse(raw).err();
            assert!(
                matches!(err, Some(ServiceUrlError::UnsupportedScheme { .. })),
                "{raw} -> {err:?}"
            );
        }

        let message = ServiceUrl::parse("grpc://host:50051")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(message.contains("grpc"), "{message}");
        assert!(message.contains("https://"), "{message}");
    }

    #[test]
    fn schemeless_and_garbage_urls_are_rejected() {
        // A bare `host:port` parses as scheme `host`, so it surfaces as an
        // unsupported scheme; the point is that none of these is accepted.
        for raw in [
            "",
            "api.rafn.dev",
            "/just/a/path",
            "http://",
            "localhost:50051",
        ] {
            let err = ServiceUrl::parse(raw).err();
            assert!(
                matches!(
                    err,
                    Some(ServiceUrlError::Malformed { .. })
                        | Some(ServiceUrlError::UnsupportedScheme { .. })
                ),
                "{raw:?} -> {err:?}"
            );
        }
    }
}
