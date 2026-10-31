//! Attaching credentials to outgoing RPCs, and turning the server's
//! credential/entitlement refusals into errors a user can act on.

use thiserror::Error;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Code, Request, Status};

use super::channel::ServiceUrl;
use crate::config::{ApiKey, ApiKeyError};

/// Header carrying the CLI's own version, so the server can tell old CLIs
/// apart when the wire contract evolves.
const CLIENT_VERSION_HEADER: &str = "x-rafn-client-version";

/// Stamps every outgoing RPC with the credentials and client version.
///
/// Installed as the channel's interceptor, so no individual RPC can forget it.
#[derive(Clone, Debug)]
pub struct AuthInterceptor {
    bearer: MetadataValue<Ascii>,
    client_version: MetadataValue<Ascii>,
}

impl AuthInterceptor {
    /// Fails only if tonic rejects a key that [`ApiKey::parse`] accepted,
    /// which would mean the two validators disagree; reported rather than
    /// unwrapped so that disagreement is a clear error instead of a panic.
    pub fn new(key: &ApiKey) -> Result<Self, ApiKeyError> {
        let mut bearer = MetadataValue::try_from(format!("Bearer {}", key.expose_secret()))
            .map_err(|_| ApiKeyError::Malformed)?;
        // Keeps HTTP/2 from indexing the value in its header-compression
        // table and makes tonic's own Debug output print it as "Sensitive".
        bearer.set_sensitive(true);

        Ok(Self {
            bearer,
            // `from_static` only panics on bytes invalid in a header, which
            // cargo's semver validation of the package version rules out (and
            // every interceptor test constructs one, so CI would catch it).
            client_version: MetadataValue::from_static(env!("CARGO_PKG_VERSION")),
        })
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let metadata = request.metadata_mut();
        // `insert` rather than `append`: a caller-supplied header must never
        // ride along with, or shadow, the configured key.
        metadata.insert("authorization", self.bearer.clone());
        metadata.insert(CLIENT_VERSION_HEADER, self.client_version.clone());
        Ok(request)
    }
}

/// A failed RPC, with credential and entitlement refusals singled out because
/// the user can fix them and a generic "call failed" would hide that.
#[derive(Debug, Error)]
pub enum RpcError {
    /// `UNAUTHENTICATED`: the key was sent but not accepted.
    #[error(
        "the rafn API at {endpoint} rejected the API key in RAFN_API_KEY \
         (server said: \"{message}\"). Check that the key is correct, has \
         not been revoked, and was issued for this endpoint."
    )]
    Unauthenticated { endpoint: String, message: String },

    /// `PERMISSION_DENIED`: the key is valid but the organization behind it
    /// is not entitled to the operation (inactive subscription, exhausted
    /// quota, ...). The reason is the server's, since only it knows.
    #[error(
        "the rafn API at {endpoint} refused this request: {message}. Your API \
         key was accepted, but its organization is not entitled to this \
         operation (for example because its subscription is inactive or its \
         quota is used up)."
    )]
    PermissionDenied { endpoint: String, message: String },

    /// Anything else. The status stays reachable as the error source: it is
    /// shown by `{:#}` (which is how the CLI prints errors) but not by the
    /// default `Display`, so detail such as internal server messages stays out
    /// of the short form.
    #[error("gRPC {rpc} call to {endpoint} failed")]
    Failed {
        rpc: &'static str,
        endpoint: String,
        #[source]
        status: Status,
    },
}

impl RpcError {
    pub fn from_status(status: Status, rpc: &'static str, endpoint: &ServiceUrl) -> Self {
        let endpoint = endpoint.to_string();
        let message = match status.message() {
            "" => "no reason given".to_string(),
            message => message.to_string(),
        };

        match status.code() {
            Code::Unauthenticated => Self::Unauthenticated { endpoint, message },
            Code::PermissionDenied => Self::PermissionDenied { endpoint, message },
            _ => Self::Failed {
                rpc,
                endpoint,
                status,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use super::*;

    fn text(value: &MetadataValue<Ascii>) -> Result<&str> {
        value.to_str().context("metadata value is not ASCII")
    }

    fn interceptor(key: &str) -> Result<AuthInterceptor> {
        Ok(AuthInterceptor::new(&ApiKey::parse(key)?)?)
    }

    fn url() -> Result<ServiceUrl> {
        Ok(ServiceUrl::parse("https://api.rafn.dev")?)
    }

    #[test]
    fn sets_authorization_and_client_version() -> Result<()> {
        let request = interceptor("rk_live_abc123")?.call(Request::new(()))?;
        let metadata = request.metadata();

        assert_eq!(
            text(metadata.get("authorization").context("no authorization")?)?,
            "Bearer rk_live_abc123"
        );
        assert_eq!(
            text(
                metadata
                    .get(CLIENT_VERSION_HEADER)
                    .context("no client version")?
            )?,
            env!("CARGO_PKG_VERSION")
        );
        Ok(())
    }

    #[test]
    fn overrides_caller_supplied_authorization() -> Result<()> {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Bearer other"));
        let request = interceptor("rk_live_abc123")?.call(request)?;

        let values = request
            .metadata()
            .get_all("authorization")
            .iter()
            .map(text)
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(values, ["Bearer rk_live_abc123"]);
        Ok(())
    }

    #[test]
    fn bearer_header_is_marked_sensitive() -> Result<()> {
        let request = interceptor("rk_live_abc123")?.call(Request::new(()))?;
        let header = request
            .metadata()
            .get("authorization")
            .context("no authorization")?;
        assert!(header.is_sensitive());
        Ok(())
    }

    #[test]
    fn every_key_the_parser_accepts_fits_in_a_header() -> Result<()> {
        // All visible ASCII at once: the parser and tonic must agree.
        let all_graphic: String = (b'!'..=b'~').map(char::from).collect();
        interceptor(&all_graphic)?;
        Ok(())
    }

    #[test]
    fn debug_never_contains_the_secret() -> Result<()> {
        let interceptor = interceptor("rk_live_supersecret")?;
        for rendered in [format!("{interceptor:?}"), format!("{interceptor:#?}")] {
            assert!(!rendered.contains("supersecret"), "leaked: {rendered}");
        }
        Ok(())
    }

    #[test]
    fn unauthenticated_becomes_an_actionable_error() -> Result<()> {
        let error =
            RpcError::from_status(Status::unauthenticated("bad key"), "push_results", &url()?);

        assert!(
            matches!(error, RpcError::Unauthenticated { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("RAFN_API_KEY"), "{message}");
        assert!(message.contains("https://api.rafn.dev"), "{message}");
        assert!(message.contains("bad key"), "{message}");
        Ok(())
    }

    #[test]
    fn permission_denied_carries_the_servers_reason() -> Result<()> {
        let error = RpcError::from_status(
            Status::permission_denied("subscription inactive"),
            "push_results",
            &url()?,
        );

        assert!(
            matches!(error, RpcError::PermissionDenied { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("subscription inactive"), "{message}");
        assert!(message.contains("not entitled"), "{message}");
        // Distinct from a credential problem: the key itself is fine.
        assert!(!message.contains("rejected the API key"), "{message}");
        Ok(())
    }

    #[test]
    fn empty_server_messages_still_read_as_sentences() -> Result<()> {
        let error = RpcError::from_status(Status::permission_denied(""), "push_results", &url()?);
        assert!(error.to_string().contains("no reason given"), "{error}");
        Ok(())
    }

    #[test]
    fn other_statuses_keep_the_generic_treatment() -> Result<()> {
        for status in [
            Status::internal("boom"),
            Status::unavailable("down"),
            Status::resource_exhausted("slow down"),
            Status::not_found("nope"),
        ] {
            let error = RpcError::from_status(status, "push_results", &url()?);
            assert!(matches!(error, RpcError::Failed { .. }), "{error:?}");
        }
        Ok(())
    }

    #[test]
    fn failed_hides_status_detail_by_default_and_shows_it_in_the_chain() -> Result<()> {
        let status = Status::internal("ClickHouse: table default.benchmarks does not exist");
        let error = anyhow::Error::new(RpcError::from_status(status, "push_results", &url()?));

        let short = format!("{error}");
        assert_eq!(
            short,
            "gRPC push_results call to https://api.rafn.dev failed"
        );
        assert!(
            !short.contains("ClickHouse"),
            "default Display should not leak the wrapped status: {short}"
        );

        let full = format!("{error:#}");
        assert!(full.contains("Internal"), "{full}");
        assert!(
            full.contains("ClickHouse: table default.benchmarks does not exist"),
            "{full}"
        );
        Ok(())
    }
}
