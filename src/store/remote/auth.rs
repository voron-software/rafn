//! Attaching credentials to outgoing RPCs, and turning the server's
//! credential/entitlement refusals into errors a user can act on.

use thiserror::Error;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Code, Request, Status};

use super::channel::ServiceUrl;
use crate::config::{Credential, CredentialError, CredentialKind};

/// Header carrying the CLI's own version, so the server can tell old CLIs
/// apart when the wire contract evolves.
const CLIENT_VERSION_HEADER: &str = "x-rafn-client-version";

/// Stamps every outgoing RPC with the credential and client version.
///
/// Installed as the channel's interceptor, so no individual RPC can forget it.
#[derive(Clone, Debug)]
pub struct AuthInterceptor {
    bearer: MetadataValue<Ascii>,
    client_version: MetadataValue<Ascii>,
}

impl AuthInterceptor {
    /// Either kind of credential is sent the same way, as a bearer token.
    ///
    /// Fails only if tonic rejects a value that the credential's own parser
    /// accepted, which would mean the two validators disagree; reported rather
    /// than unwrapped so that disagreement is a clear error instead of a panic.
    pub fn new(credential: &Credential) -> Result<Self, CredentialError> {
        let mut bearer = MetadataValue::try_from(format!("Bearer {}", credential.expose_secret()))
            .map_err(|_| CredentialError::NotAHeaderValue)?;
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
        // ride along with, or shadow, the configured credential.
        metadata.insert("authorization", self.bearer.clone());
        metadata.insert(CLIENT_VERSION_HEADER, self.client_version.clone());
        Ok(request)
    }
}

/// Start of the message tonic gives a status it made up from the HTTP status
/// code of a response that carried no `grpc-status` (tonic `infer_grpc_status`).
/// A real rafn answer always has one, so this marks a refusal by something in
/// front of rafn (a proxy, a WAF, an IAM-gated Cloud Run front end) that never
/// looked at the credential. `bare_http_refusals_are_not_blamed_on_the_key_or_the_subscription`
/// fails if a tonic upgrade rewords it.
const TONIC_INFERRED_STATUS_PREFIX: &str = "grpc-status header missing";

/// A failed RPC, with credential and entitlement refusals singled out because
/// the user can fix them and a generic "call failed" would hide that.
#[derive(Debug, Error)]
pub enum RpcError {
    /// `UNAUTHENTICATED`: the credential was sent but not accepted.
    #[error(
        "the rafn API at {endpoint} rejected {} (server said: \"{message}\"). {}",
        .credential.rejected_description(),
        .credential.rejection_advice()
    )]
    Unauthenticated {
        endpoint: String,
        message: String,
        credential: CredentialKind,
    },

    /// `PERMISSION_DENIED`: the credential is valid but the organization
    /// behind it is not entitled to the operation (inactive subscription,
    /// exhausted quota, ...). The reason is the server's, since only it knows.
    #[error(
        "the rafn API at {endpoint} refused this request: {message}. {} was \
         accepted, but its organization is not entitled to this operation (for \
         example because its subscription is inactive or its quota is used up).",
        .credential.accepted_subject()
    )]
    PermissionDenied {
        endpoint: String,
        message: String,
        credential: CredentialKind,
    },

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
    pub fn from_status(
        status: Status,
        rpc: &'static str,
        endpoint: &ServiceUrl,
        credential: CredentialKind,
    ) -> Self {
        let endpoint = endpoint.to_string();
        let message = match status.message() {
            "" => "no reason given".to_string(),
            message => message.to_string(),
        };

        // Only a status the rafn backend itself sent says anything about the
        // credential or the organization. An inferred one stays `Failed`, whose
        // chain shows the HTTP code, rather than send the user to rotate a key
        // or check billing over a refusal that never reached rafn.
        let from_service = !status.message().starts_with(TONIC_INFERRED_STATUS_PREFIX);

        match status.code() {
            Code::Unauthenticated if from_service => Self::Unauthenticated {
                endpoint,
                message,
                credential,
            },
            Code::PermissionDenied if from_service => Self::PermissionDenied {
                endpoint,
                message,
                credential,
            },
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
    use crate::config::{ActionsToken, ApiKey};

    const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZXBvOmFjbWUvd2lkZ2V0cyJ9.c2lnbmF0dXJl";

    fn text(value: &MetadataValue<Ascii>) -> Result<&str> {
        value.to_str().context("metadata value is not ASCII")
    }

    fn interceptor(key: &str) -> Result<AuthInterceptor> {
        Ok(AuthInterceptor::new(&Credential::ApiKey(ApiKey::parse(
            key,
        )?))?)
    }

    fn oidc_interceptor(jwt: &str) -> Result<AuthInterceptor> {
        Ok(AuthInterceptor::new(&Credential::ActionsToken(
            ActionsToken::parse(jwt)?,
        ))?)
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
    fn an_actions_token_is_sent_as_the_bearer_too() -> Result<()> {
        let request = oidc_interceptor(JWT)?.call(Request::new(()))?;
        let metadata = request.metadata();

        let header = metadata.get("authorization").context("no authorization")?;
        assert_eq!(text(header)?, format!("Bearer {JWT}"));
        assert!(header.is_sensitive());
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
    fn an_actions_token_overrides_caller_supplied_authorization() -> Result<()> {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Bearer other"));
        let request = oidc_interceptor(JWT)?.call(request)?;

        let values = request
            .metadata()
            .get_all("authorization")
            .iter()
            .map(text)
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(values, [format!("Bearer {JWT}")]);
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
    fn debug_never_contains_the_actions_token() -> Result<()> {
        let interceptor = oidc_interceptor(JWT)?;
        for rendered in [format!("{interceptor:?}"), format!("{interceptor:#?}")] {
            assert!(!rendered.contains("eyJ"), "leaked: {rendered}");
            assert!(!rendered.contains("c2lnbmF0dXJl"), "leaked: {rendered}");
        }
        Ok(())
    }

    #[test]
    fn unauthenticated_becomes_an_actionable_error() -> Result<()> {
        let error = RpcError::from_status(
            Status::unauthenticated("bad key"),
            "push_results",
            &url()?,
            CredentialKind::ApiKey,
        );

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
    fn a_rejected_actions_token_does_not_blame_the_api_key() -> Result<()> {
        let error = RpcError::from_status(
            Status::unauthenticated("unknown repository"),
            "push_results",
            &url()?,
            CredentialKind::ActionsOidc,
        );

        assert!(
            matches!(error, RpcError::Unauthenticated { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("GitHub Actions OIDC token"), "{message}");
        assert!(message.contains("unknown repository"), "{message}");
        assert!(message.contains("https://api.rafn.dev"), "{message}");
        // The user may never have had a key; do not send them looking for one
        // except as the alternative.
        assert!(!message.contains("rejected the API key"), "{message}");
        assert!(!message.contains("revoked"), "{message}");
        Ok(())
    }

    #[test]
    fn a_denied_actions_token_is_called_a_token_not_a_key() -> Result<()> {
        let error = RpcError::from_status(
            Status::permission_denied("subscription inactive"),
            "push_results",
            &url()?,
            CredentialKind::ActionsOidc,
        );

        let message = error.to_string();
        assert!(message.contains("subscription inactive"), "{message}");
        assert!(message.contains("not entitled"), "{message}");
        assert!(message.contains("OIDC token was accepted"), "{message}");
        assert!(!message.contains("API key"), "{message}");
        Ok(())
    }

    #[test]
    fn permission_denied_carries_the_servers_reason() -> Result<()> {
        let error = RpcError::from_status(
            Status::permission_denied("subscription inactive"),
            "push_results",
            &url()?,
            CredentialKind::ApiKey,
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
    fn statuses_tonic_inferred_from_http_codes_stay_generic() -> Result<()> {
        for (code, kind) in [Code::Unauthenticated, Code::PermissionDenied]
            .into_iter()
            .flat_map(|code| {
                [CredentialKind::ApiKey, CredentialKind::ActionsOidc].map(|kind| (code, kind))
            })
        {
            let status = Status::new(
                code,
                "grpc-status header missing, mapped from HTTP status code 403",
            );
            let error = RpcError::from_status(status, "push_results", &url()?, kind);
            assert!(
                matches!(error, RpcError::Failed { .. }),
                "{code:?} {kind:?}: {error:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn empty_server_messages_still_read_as_sentences() -> Result<()> {
        let error = RpcError::from_status(
            Status::permission_denied(""),
            "push_results",
            &url()?,
            CredentialKind::ApiKey,
        );
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
            let error =
                RpcError::from_status(status, "push_results", &url()?, CredentialKind::ApiKey);
            assert!(matches!(error, RpcError::Failed { .. }), "{error:?}");
        }
        Ok(())
    }

    #[test]
    fn failed_hides_status_detail_by_default_and_shows_it_in_the_chain() -> Result<()> {
        let status = Status::internal("ClickHouse: table default.benchmarks does not exist");
        let error = anyhow::Error::new(RpcError::from_status(
            status,
            "push_results",
            &url()?,
            CredentialKind::ApiKey,
        ));

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
