//! The `RAFN_API_KEY` contract.
//!
//! The cloud backend authenticates with an API key that is read **only** from
//! the `RAFN_API_KEY` environment variable. It is deliberately not a field of
//! `rafn.toml` or the user config file: both are routinely committed, synced
//! or pasted into bug reports, and an env var is what CI systems (GitHub
//! Actions secrets, in particular) already know how to inject safely.
//!
//! Only the cloud backend needs a key. `[backend] type = "local"` never reads
//! it, and `rafn push --dry-run` does not either because it never contacts
//! the service.

use std::env::VarError;
use std::fmt;

use thiserror::Error;

/// Environment variable the API key is read from.
pub const API_KEY_ENV: &str = "RAFN_API_KEY";

/// Why `RAFN_API_KEY` could not be turned into an [`ApiKey`]. Messages never
/// include the offending value, since it may be a (mis-pasted) secret.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum ApiKeyError {
    #[error(
        "RAFN_API_KEY is not set. The cloud backend needs an API key: \
         export RAFN_API_KEY=<your key> (in GitHub Actions, pass a secret via the \
         `api-key` input). To use rafn without the cloud, set `type = \"local\"` \
         under [backend] in rafn.toml."
    )]
    Missing,

    // An unset GitHub secret expands to an empty string rather than to
    // nothing, so this is the failure mode CI users actually hit.
    #[error(
        "RAFN_API_KEY is set but empty. In GitHub Actions this usually means \
         the secret is not available to this run (for example on pull requests \
         from forks)."
    )]
    Empty,

    #[error(
        "RAFN_API_KEY is not a valid API key: it must be a single token of \
         visible ASCII characters. Check for stray quotes, spaces or a \
         mis-pasted value."
    )]
    Malformed,
}

/// A validated rafn API key.
///
/// Validation happens once, here, so that the code attaching the key to an
/// RPC never has to deal with a value that cannot be sent as a header.
///
/// `Debug` is redacted and there is deliberately no `Display`, so the secret
/// cannot leak through `{:?}` of an enclosing struct or through error chains.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Validate a raw key. Surrounding whitespace is dropped because secrets
    /// piped in from files or CI variables commonly carry a trailing newline
    /// and no real key contains whitespace.
    pub fn parse(raw: &str) -> Result<Self, ApiKeyError> {
        let key = raw.trim();
        if key.is_empty() {
            return Err(ApiKeyError::Empty);
        }
        // Visible ASCII is exactly what is safe inside an HTTP header value
        // without escaping; anything else would be a paste accident.
        if !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err(ApiKeyError::Malformed);
        }
        Ok(Self(key.to_string()))
    }

    /// Interpret the result of reading [`API_KEY_ENV`]. Split from
    /// [`from_env`](Self::from_env) so tests need not mutate the process
    /// environment (which is `unsafe` in edition 2024 and racy across tests).
    pub fn from_env_value(value: Result<String, VarError>) -> Result<Self, ApiKeyError> {
        match value {
            Ok(raw) => Self::parse(&raw),
            Err(VarError::NotPresent) => Err(ApiKeyError::Missing),
            Err(VarError::NotUnicode(_)) => Err(ApiKeyError::Malformed),
        }
    }

    pub fn from_env() -> Result<Self, ApiKeyError> {
        Self::from_env_value(std::env::var(API_KEY_ENV))
    }

    /// The raw secret. Named loudly so each call site is easy to audit; the
    /// only legitimate caller puts it into an `authorization` header.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn parse_keeps_the_token() -> Result<()> {
        assert_eq!(
            ApiKey::parse("rk_live_abc123")?.expose_secret(),
            "rk_live_abc123"
        );
        Ok(())
    }

    #[test]
    fn parse_trims_surrounding_whitespace() -> Result<()> {
        assert_eq!(
            ApiKey::parse("  rk_live_abc123\n")?.expose_secret(),
            "rk_live_abc123"
        );
        Ok(())
    }

    #[test]
    fn parse_rejects_empty_and_blank() {
        assert_eq!(ApiKey::parse("").err(), Some(ApiKeyError::Empty));
        assert_eq!(ApiKey::parse(" \t\n").err(), Some(ApiKeyError::Empty));
    }

    #[test]
    fn parse_rejects_values_that_are_not_a_single_ascii_token() {
        for bad in [
            "two words",
            "tab\tinside",
            "nul\0byte",
            "caf\u{e9}",
            "line\nbreak",
        ] {
            assert_eq!(
                ApiKey::parse(bad).err(),
                Some(ApiKeyError::Malformed),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn from_env_value_maps_each_env_failure() {
        assert_eq!(
            ApiKey::from_env_value(Err(VarError::NotPresent)).err(),
            Some(ApiKeyError::Missing)
        );
        assert_eq!(
            ApiKey::from_env_value(Err(VarError::NotUnicode("x".into()))).err(),
            Some(ApiKeyError::Malformed)
        );
        assert!(ApiKey::from_env_value(Ok("rk_live_abc123".to_string())).is_ok());
    }

    #[test]
    fn missing_key_message_names_the_variable_the_action_input_and_the_local_escape_hatch() {
        let message = ApiKeyError::Missing.to_string();
        assert!(message.contains("RAFN_API_KEY"), "{message}");
        assert!(message.contains("`api-key`"), "{message}");
        assert!(message.contains("type = \"local\""), "{message}");
    }

    #[test]
    fn error_messages_never_echo_the_value() {
        // The error enum carries no payload, so a leak would need a new field;
        // this pins the property for the value most likely to be mis-pasted.
        let message = ApiKey::parse("secret value")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(!message.is_empty());
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn debug_is_redacted_even_when_pretty_printed() -> Result<()> {
        let key = ApiKey::parse("rk_live_supersecret")?;
        assert_eq!(format!("{key:?}"), "ApiKey(<redacted>)");
        assert!(!format!("{key:#?}").contains("supersecret"));
        Ok(())
    }

    #[test]
    fn debug_of_an_enclosing_struct_stays_redacted() -> Result<()> {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            key: ApiKey,
        }

        let rendered = format!(
            "{:?}",
            Holder {
                key: ApiKey::parse("rk_live_supersecret")?
            }
        );
        assert!(!rendered.contains("supersecret"), "leaked: {rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
        Ok(())
    }
}
