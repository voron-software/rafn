//! The one place a gRPC channel to the rafn cloud service is built, so TLS
//! and authentication are applied identically to every RPC.

use std::fmt;

use anyhow::{Context, Result};
use thiserror::Error;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use url::Url;

use super::auth::AuthInterceptor;
use crate::config::ApiKey;
use crate::proto::pb::benchmark_service_client::BenchmarkServiceClient;

/// A benchmark-service client whose every call carries the API key.
pub type Client = BenchmarkServiceClient<InterceptedService<Channel, AuthInterceptor>>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServiceUrlError {
    #[error(
        "unsupported scheme \"{scheme}\" in API URL \"{url}\": use https:// \
         (or http:// for a local development server)"
    )]
    UnsupportedScheme { scheme: String, url: String },

    #[error(
        "invalid API URL \"{url}\" ({reason}): expected a full URL such as \
         https://api.rafn.dev"
    )]
    Malformed { url: String, reason: String },
}

/// How the connection to the service is secured, decided by the URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// `http://`: no encryption, for local development servers.
    Plaintext,
    /// `https://`: TLS verified against the platform's trust store.
    Tls,
}

/// A service URL whose scheme has been checked, so a channel can be built
/// from it without re-validating anything. Any scheme other than `http` or
/// `https` is rejected up front rather than silently downgraded to plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceUrl {
    raw: String,
    transport: Transport,
}

impl ServiceUrl {
    pub fn parse(raw: &str) -> Result<Self, ServiceUrlError> {
        let raw = raw.trim();
        let parsed = Url::parse(raw).map_err(|e| ServiceUrlError::Malformed {
            url: raw.to_string(),
            reason: e.to_string(),
        })?;

        let transport = match parsed.scheme() {
            "http" => Transport::Plaintext,
            "https" => Transport::Tls,
            other => {
                return Err(ServiceUrlError::UnsupportedScheme {
                    scheme: other.to_string(),
                    url: raw.to_string(),
                });
            }
        };

        Ok(Self {
            raw: raw.to_string(),
            transport,
        })
    }

    pub fn transport(&self) -> Transport {
        self.transport
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

    let mut endpoint =
        Endpoint::from_shared(url.raw.clone()).with_context(|| format!("Invalid API URL {url}"))?;

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
    fn http_selects_plaintext() -> Result<()> {
        let url = ServiceUrl::parse("http://localhost:50051")?;
        assert_eq!(url.transport(), Transport::Plaintext);
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
