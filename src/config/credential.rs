//! Which credential the CLI authenticates with, and how it is obtained.
//!
//! Resolution order, in one place:
//!
//! 1. `RAFN_API_KEY` set: an API key, even inside GitHub Actions. An explicit
//!    key always wins. A set-but-broken key is an error rather than a reason
//!    to fall through, since silently switching credential would hide that a
//!    secret is wired up wrong.
//! 2. Else, when GitHub Actions has granted the job `id-token: write`: an OIDC
//!    token fetched from the runner, so the workflow needs no secret.
//! 3. Else an error saying how to get either.
//!
//! The work is split in two so the part that can only fail locally runs
//! first and without I/O: [`CredentialSource::from_env`] decides and validates
//! (this is where a missing key fails), and [`CredentialSource::resolve`] does
//! the one network request if the source needs it. What the remote store holds
//! is a [`Credential`], which is always resolved, so it cannot carry an
//! unfetched token.
//!
//! Nothing here runs for `[backend] type = "local"` or `rafn push --dry-run`:
//! callers only reach for a credential when they are about to contact the
//! service.

use std::env::VarError;

use thiserror::Error;
use tracing::debug;

use super::actions_token::{
    ACTIONS_ENV, ActionsToken, ActionsTokenError, ActionsTokenRequest, REQUEST_TOKEN_ENV,
    REQUEST_URL_ENV,
};
use super::api_key::{API_KEY_ENV, ApiKey, ApiKeyError};

/// Which kind of credential is in use, for messages that must name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    ApiKey,
    ActionsOidc,
}

impl CredentialKind {
    /// Object of "the service rejected ...".
    pub fn rejected_description(self) -> &'static str {
        match self {
            Self::ApiKey => "the API key in RAFN_API_KEY",
            Self::ActionsOidc => "the GitHub Actions OIDC token",
        }
    }

    /// Subject of "... was accepted, but its organization is not entitled".
    pub fn accepted_subject(self) -> &'static str {
        match self {
            Self::ApiKey => "Your API key",
            Self::ActionsOidc => "The GitHub Actions OIDC token",
        }
    }

    /// What the user can do about a rejection.
    pub fn rejection_advice(self) -> &'static str {
        match self {
            Self::ApiKey => {
                "Check that the key is correct, has not been revoked, and was issued \
                 for this endpoint."
            }
            Self::ActionsOidc => {
                "GitHub issued this token for the workflow run with audience `rafn`. \
                 Check that the account that owns this repository is registered with \
                 rafn, or set RAFN_API_KEY to authenticate with an API key instead."
            }
        }
    }
}

/// Why no credential could be produced.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// `RAFN_API_KEY` is absent (and there is no Actions token to fall back
    /// on), empty, or malformed.
    #[error(transparent)]
    ApiKey(#[from] ApiKeyError),

    /// Running in GitHub Actions, but the runner offered no OIDC token: the
    /// job lacks `id-token: write`, or the run is a pull request from a fork,
    /// which GitHub never grants it.
    #[error(
        "RAFN_API_KEY is not set, and GitHub Actions did not provide an OIDC token \
         for this job. Add `permissions: id-token: write` to the job and no key is \
         needed; GitHub does not grant that permission to pull requests from forks. \
         Alternatively set RAFN_API_KEY from a secret (see the `api-key` input), \
         though secrets are not available to forks either. To use rafn without the \
         cloud, set `type = \"local\"` under [backend] in rafn.toml."
    )]
    IdTokenUnavailable,

    #[error(transparent)]
    ActionsToken(#[from] ActionsTokenError),

    /// A credential the validators accepted that tonic then refused as a
    /// header value, meaning the two disagree about what is valid. Reported
    /// rather than unwrapped so that is a clear error instead of a panic.
    #[error("the credential cannot be sent as an authorization header")]
    NotAHeaderValue,
}

/// A credential ready to attach to an RPC.
///
/// `Debug` is redacted by both variants, and there is deliberately no
/// `Display`.
#[derive(Clone, Debug)]
pub enum Credential {
    ApiKey(ApiKey),
    ActionsToken(ActionsToken),
}

impl Credential {
    pub fn kind(&self) -> CredentialKind {
        match self {
            Self::ApiKey(_) => CredentialKind::ApiKey,
            Self::ActionsToken(_) => CredentialKind::ActionsOidc,
        }
    }

    /// The raw bearer value. Named loudly so each call site is easy to audit;
    /// the only legitimate caller puts it into an `authorization` header.
    pub fn expose_secret(&self) -> &str {
        match self {
            Self::ApiKey(key) => key.expose_secret(),
            Self::ActionsToken(token) => token.expose_secret(),
        }
    }
}

/// Where the credential will come from, decided from the environment but not
/// yet obtained.
#[derive(Clone, Debug)]
pub enum CredentialSource {
    ApiKey(ApiKey),
    ActionsOidc(ActionsTokenRequest),
}

impl CredentialSource {
    pub fn from_env() -> Result<Self, CredentialError> {
        Self::from_lookup(|name| std::env::var(name))
    }

    /// [`from_env`](Self::from_env) over an arbitrary variable lookup, so tests
    /// can supply an environment without mutating the process's (which is
    /// `unsafe` in edition 2024 and racy across tests).
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Result<String, VarError>,
    ) -> Result<Self, CredentialError> {
        match lookup(API_KEY_ENV) {
            Err(VarError::NotPresent) => {}
            set => return Ok(Self::ApiKey(ApiKey::from_env_value(set)?)),
        }

        // Blank counts as absent: the runner never sets these empty, and an
        // empty URL or token cannot be used for anything.
        let present = |name: &str| lookup(name).ok().filter(|value| !value.trim().is_empty());
        if let (Some(url), Some(token)) = (present(REQUEST_URL_ENV), present(REQUEST_TOKEN_ENV)) {
            return Ok(Self::ActionsOidc(ActionsTokenRequest::parse(&url, &token)?));
        }

        if lookup(ACTIONS_ENV).is_ok_and(|value| value == "true") {
            return Err(CredentialError::IdTokenUnavailable);
        }
        Err(ApiKeyError::Missing.into())
    }

    /// Obtain the credential. Free for an API key; for an Actions token this
    /// is the single network request, made once per CLI invocation: the token
    /// lives for minutes and the CLI for seconds.
    pub async fn resolve(self) -> Result<Credential, CredentialError> {
        match self {
            Self::ApiKey(key) => Ok(Credential::ApiKey(key)),
            Self::ActionsOidc(request) => {
                debug!("Requesting a GitHub Actions OIDC token to authenticate");
                Ok(Credential::ActionsToken(request.fetch().await?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env::VarError;
    use std::time::Instant;

    use anyhow::{Context, Result};

    use super::*;
    use crate::config::actions_token::{
        ACTIONS_ENV, ActionsTokenError, REQUEST_TOKEN_ENV, REQUEST_URL_ENV,
    };
    use crate::config::fake_endpoint::{FakeEndpoint, Reply};
    use crate::config::{API_KEY_ENV, ApiKeyError};

    const JWT: &str = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJyZXBvOmFjbWUvd2lkZ2V0cyJ9.c2lnbmF0dXJl";
    const REQUEST_TOKEN: &str = "req-token-SECRET";
    const KEY: &str = "rk_live_abc123";

    /// A fake environment: tests cannot set real variables (`set_var` is
    /// `unsafe`, which this crate forbids, and would race across tests), so
    /// resolution takes a lookup function and tests hand it a map.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, VarError> + use<> {
        let vars: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        move |name| vars.get(name).cloned().ok_or(VarError::NotPresent)
    }

    fn token_body() -> String {
        format!(r#"{{"count":{},"value":"{JWT}"}}"#, JWT.len())
    }

    fn actions_env<'a>(
        endpoint: &'a FakeEndpoint,
        extra: &[(&'a str, &'a str)],
    ) -> Vec<(String, String)> {
        let mut vars = vec![
            (
                REQUEST_URL_ENV.to_string(),
                endpoint.url("/oidc?api-version=2.0"),
            ),
            (REQUEST_TOKEN_ENV.to_string(), REQUEST_TOKEN.to_string()),
        ];
        vars.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        vars
    }

    fn lookup(vars: &[(String, String)]) -> impl Fn(&str) -> Result<String, VarError> + use<> {
        let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        env(&pairs)
    }

    #[tokio::test]
    async fn an_api_key_wins_even_inside_github_actions() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(token_body()))?;
        let vars = actions_env(&endpoint, &[(API_KEY_ENV, KEY), (ACTIONS_ENV, "true")]);

        let credential = CredentialSource::from_lookup(lookup(&vars))?
            .resolve()
            .await?;

        assert_eq!(credential.kind(), CredentialKind::ApiKey);
        assert_eq!(credential.expose_secret(), KEY);
        assert!(
            endpoint.requests()?.is_empty(),
            "a configured key must not trigger a token request"
        );
        Ok(())
    }

    #[test]
    fn a_broken_api_key_is_an_error_not_a_reason_to_fall_back() -> Result<()> {
        // Falling back would hide that the secret is wired up wrong.
        let endpoint = FakeEndpoint::start(Reply::json(token_body()))?;
        for (value, expected) in [
            ("", ApiKeyError::Empty),
            ("two words", ApiKeyError::Malformed),
        ] {
            let vars = actions_env(&endpoint, &[(API_KEY_ENV, value), (ACTIONS_ENV, "true")]);
            let error = CredentialSource::from_lookup(lookup(&vars))
                .err()
                .with_context(|| format!("{value:?} should be rejected"))?;
            assert!(
                matches!(&error, CredentialError::ApiKey(e) if *e == expected),
                "{value:?}: {error:?}"
            );
        }
        assert!(endpoint.requests()?.is_empty());
        Ok(())
    }

    #[test]
    fn a_non_unicode_api_key_is_malformed_not_missing() {
        let error = CredentialSource::from_lookup(|name| {
            if name == API_KEY_ENV {
                Err(VarError::NotUnicode("x".into()))
            } else {
                Err(VarError::NotPresent)
            }
        })
        .err();
        assert!(
            matches!(error, Some(CredentialError::ApiKey(ApiKeyError::Malformed))),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn without_a_key_the_actions_token_is_requested_once_and_used() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(token_body()))?;
        let vars = actions_env(&endpoint, &[(ACTIONS_ENV, "true")]);

        let source = CredentialSource::from_lookup(lookup(&vars))?;
        assert!(
            endpoint.requests()?.is_empty(),
            "choosing a source must not touch the network"
        );

        let credential = source.resolve().await?;
        assert_eq!(credential.kind(), CredentialKind::ActionsOidc);
        assert_eq!(credential.expose_secret(), JWT);

        let seen = endpoint.requests()?;
        assert_eq!(seen.len(), 1);
        let request = seen.first().context("no request")?;
        assert_eq!(
            request.request_line,
            "GET /oidc?api-version=2.0&audience=rafn HTTP/1.1"
        );
        assert_eq!(
            request.header("authorization"),
            Some("bearer req-token-SECRET")
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_audience_is_appended_to_a_url_without_a_query_string() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::json(token_body()))?;
        let url = endpoint.url("/oidc");
        let vars = [
            (REQUEST_URL_ENV, url.as_str()),
            (REQUEST_TOKEN_ENV, REQUEST_TOKEN),
        ];

        CredentialSource::from_lookup(env(&vars))?.resolve().await?;

        let seen = endpoint.requests()?;
        assert_eq!(
            seen.first().context("no request")?.request_line,
            "GET /oidc?audience=rafn HTTP/1.1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn token_endpoint_failures_are_clear_and_never_echo_a_secret() -> Result<()> {
        for (reply, name) in [
            (Reply::status(401, "nope"), "401"),
            (Reply::status(403, "nope"), "403"),
            (Reply::status(404, "nope"), "404"),
            (Reply::status(500, "boom"), "500"),
            (Reply::status(503, "boom"), "503"),
        ] {
            let endpoint = FakeEndpoint::start(reply)?;
            let vars = actions_env(&endpoint, &[]);
            let error = CredentialSource::from_lookup(lookup(&vars))?
                .resolve()
                .await
                .err()
                .with_context(|| format!("{name} should fail"))?;

            assert!(
                matches!(
                    &error,
                    CredentialError::ActionsToken(ActionsTokenError::Rejected { .. })
                ),
                "{name}: {error:?}"
            );
            let message = format!("{error:#}");
            assert!(
                message.contains(&format!("HTTP status {name}")),
                "{message}"
            );
            assert!(message.contains("id-token: write"), "{message}");
            assert!(!message.contains(REQUEST_TOKEN), "{message}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn malformed_token_responses_are_rejected_without_echoing_them() -> Result<()> {
        for body in [
            "not json at all".to_string(),
            "{}".to_string(),
            r#"{"value":123}"#.to_string(),
            r#"{"value":{"jwt":"x"}}"#.to_string(),
            r#"["a","b"]"#.to_string(),
            String::new(),
        ] {
            let endpoint = FakeEndpoint::start(Reply::json(body.clone()))?;
            let vars = actions_env(&endpoint, &[]);
            let error = CredentialSource::from_lookup(lookup(&vars))?
                .resolve()
                .await
                .err()
                .with_context(|| format!("{body:?} should fail"))?;

            assert!(
                matches!(
                    &error,
                    CredentialError::ActionsToken(ActionsTokenError::BadResponse { .. })
                ),
                "{body:?}: {error:?}"
            );
            assert!(format!("{error:#}").contains("value"), "{error:#}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_value_that_is_not_a_jwt_is_rejected() -> Result<()> {
        for value in [
            "not-a-jwt",
            "a.b",
            "a.b.c.d",
            "a..c",
            "a.b.c==",
            "a b.c.d",
            "é.b.c",
        ] {
            let endpoint = FakeEndpoint::start(Reply::json(format!(r#"{{"value":"{value}"}}"#)))?;
            let vars = actions_env(&endpoint, &[]);
            let error = CredentialSource::from_lookup(lookup(&vars))?
                .resolve()
                .await
                .err()
                .with_context(|| format!("{value:?} should fail"))?;

            assert!(
                matches!(
                    &error,
                    CredentialError::ActionsToken(ActionsTokenError::Malformed)
                ),
                "{value:?}: {error:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_names_the_host() -> Result<()> {
        let url = FakeEndpoint::closed_url()?;
        let vars = [
            (REQUEST_URL_ENV, url.as_str()),
            (REQUEST_TOKEN_ENV, REQUEST_TOKEN),
        ];

        let error = CredentialSource::from_lookup(env(&vars))?
            .resolve()
            .await
            .err()
            .context("nothing is listening")?;

        assert!(
            matches!(
                &error,
                CredentialError::ActionsToken(ActionsTokenError::Request { .. })
            ),
            "{error:?}"
        );
        assert!(format!("{error:#}").contains("127.0.0.1"), "{error:#}");
        Ok(())
    }

    #[test]
    fn neither_key_nor_actions_is_the_missing_key_error() {
        for vars in [
            Vec::new(),
            // Only half of the pair is not enough to request anything.
            vec![(REQUEST_URL_ENV, "https://example.invalid/oidc")],
            vec![(REQUEST_TOKEN_ENV, REQUEST_TOKEN)],
            // Present but blank counts as absent.
            vec![(REQUEST_URL_ENV, " "), (REQUEST_TOKEN_ENV, "")],
            // Only the exact value `true` means Actions.
            vec![(ACTIONS_ENV, "false")],
            vec![(ACTIONS_ENV, "")],
        ] {
            let error = CredentialSource::from_lookup(env(&vars)).err();
            assert!(
                matches!(error, Some(CredentialError::ApiKey(ApiKeyError::Missing))),
                "{vars:?}: {error:?}"
            );
        }
    }

    #[test]
    fn github_actions_without_the_request_variables_says_the_job_lacks_id_token_write() {
        let error = CredentialSource::from_lookup(env(&[(ACTIONS_ENV, "true")])).err();
        assert!(
            matches!(error, Some(CredentialError::IdTokenUnavailable)),
            "{error:?}"
        );

        let message = CredentialError::IdTokenUnavailable.to_string();
        assert!(
            message.contains("permissions: id-token: write"),
            "{message}"
        );
        assert!(message.contains("RAFN_API_KEY"), "{message}");
        assert!(message.contains("forks"), "{message}");
        assert!(message.contains("type = \"local\""), "{message}");
    }

    #[test]
    fn the_missing_key_message_mentions_the_keyless_option() {
        let message = CredentialError::from(ApiKeyError::Missing).to_string();
        assert!(message.contains("id-token: write"), "{message}");
        assert!(message.contains("no key is needed"), "{message}");
    }

    #[test]
    fn an_invalid_request_url_fails_before_any_network_use() {
        for (url, expected) in [
            ("not a url", "not a usable URL"),
            ("ftp://example.invalid/oidc", "ftp"),
            ("http://example.invalid/oidc", "loopback"),
        ] {
            let error = CredentialSource::from_lookup(env(&[
                (REQUEST_URL_ENV, url),
                (REQUEST_TOKEN_ENV, REQUEST_TOKEN),
            ]))
            .err();
            assert!(
                matches!(
                    &error,
                    Some(CredentialError::ActionsToken(
                        ActionsTokenError::InvalidRequestUrl { .. }
                    ))
                ),
                "{url}: {error:?}"
            );
            let message = error.map(|e| e.to_string()).unwrap_or_default();
            assert!(message.contains(expected), "{url}: {message}");
        }
    }

    #[test]
    fn a_request_token_that_cannot_be_a_header_fails_before_any_network_use() {
        let error = CredentialSource::from_lookup(env(&[
            (REQUEST_URL_ENV, "https://example.invalid/oidc"),
            (REQUEST_TOKEN_ENV, "line\nbreak"),
        ]))
        .err();
        assert!(
            matches!(
                &error,
                Some(CredentialError::ActionsToken(
                    ActionsTokenError::InvalidRequestToken
                ))
            ),
            "{error:?}"
        );
        assert!(!format!("{error:?}").contains("line"), "{error:?}");
    }

    #[tokio::test]
    async fn a_hung_endpoint_times_out_instead_of_blocking_the_cli() -> Result<()> {
        let endpoint = FakeEndpoint::start(Reply::Hang)?;
        let url = endpoint.url("/oidc");
        let request = crate::config::ActionsTokenRequest::parse(&url, REQUEST_TOKEN)?;

        let started = Instant::now();
        let error = request
            .fetch_within(std::time::Duration::from_millis(150))
            .await
            .err()
            .context("a silent endpoint should time out")?;

        assert!(
            matches!(error, ActionsTokenError::TimedOut { .. }),
            "{error:?}"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        Ok(())
    }

    #[test]
    fn debug_output_never_contains_a_secret() -> Result<()> {
        let source = CredentialSource::from_lookup(env(&[
            (REQUEST_URL_ENV, "https://example.invalid/oidc"),
            (REQUEST_TOKEN_ENV, REQUEST_TOKEN),
        ]))?;
        let key = CredentialSource::from_lookup(env(&[(API_KEY_ENV, KEY)]))?;
        let token = Credential::ActionsToken(crate::config::ActionsToken::parse(JWT)?);

        for rendered in [
            format!("{source:?}"),
            format!("{source:#?}"),
            format!("{key:?}"),
            format!("{token:?}"),
            format!("{token:#?}"),
            format!("{:?}", Credential::ApiKey(ApiKey::parse(KEY)?)),
        ] {
            for secret in ["SECRET", "abc123", "eyJ", "c2lnbmF0dXJl"] {
                assert!(!rendered.contains(secret), "{secret} leaked: {rendered}");
            }
            assert!(rendered.contains("redacted"), "{rendered}");
        }
        Ok(())
    }
}
