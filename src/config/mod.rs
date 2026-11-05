//! Configuration models and loading helpers.
//!
//! Credentials are not configuration: they come from the environment only. An
//! API key is read from `RAFN_API_KEY` (see [`ApiKey`]); inside GitHub Actions
//! the job's OIDC token can stand in for it (see [`CredentialSource`]).

mod actions_token;
mod api_key;
mod credential;
mod effective;
#[cfg(test)]
pub(crate) mod fake_endpoint;
mod loopback;
mod repo;
mod repository;
mod user;

pub use actions_token::{ActionsToken, ActionsTokenError, ActionsTokenRequest};
pub use api_key::{API_KEY_ENV, ApiKey, ApiKeyError};
pub use credential::{Credential, CredentialError, CredentialKind, CredentialSource};
pub use effective::EffectiveConfig;
pub use repo::{BackendSection, BackendType, BenchConfig, CloudConfig, ProjectConfig, RepoConfig};
pub use repository::RepositoryRef;
pub use user::{Config, UserCloudConfig};

pub(crate) use loopback::is_loopback;
