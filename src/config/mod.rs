//! Configuration models and loading helpers.
//!
//! Credentials are not configuration: the cloud API key is read from the
//! `RAFN_API_KEY` environment variable only, see [`ApiKey`].

mod api_key;
mod effective;
mod repo;
mod repository;
mod user;

pub use api_key::{API_KEY_ENV, ApiKey, ApiKeyError};
pub use effective::EffectiveConfig;
pub use repo::{BackendSection, BackendType, BenchConfig, CloudConfig, ProjectConfig, RepoConfig};
pub use repository::RepositoryRef;
pub use user::{Config, UserCloudConfig};
