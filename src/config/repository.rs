//! Shared 3-part repository identity: forge, owner, and repository name.
//!
//! Mirrors the proto `RepositoryReference`/`SourceInformation` shape so the
//! same value can be used to populate either without re-deriving forge/owner.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::proto::pb;

fn default_forge() -> String {
    "github.com".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRef {
    #[serde(default = "default_forge")]
    pub forge: String,
    pub owner: String,
    pub repository: String,
}

impl fmt::Display for RepositoryRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.forge, self.owner, self.repository)
    }
}

impl RepositoryRef {
    pub fn to_proto(&self) -> pb::RepositoryReference {
        pb::RepositoryReference {
            forge: self.forge.clone(),
            owner: self.owner.clone(),
            repository: self.repository.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_formats_forge_owner_repository() {
        let r = RepositoryRef {
            forge: "github.com".to_string(),
            owner: "acme".to_string(),
            repository: "perf-suite".to_string(),
        };
        assert_eq!(r.to_string(), "github.com/acme/perf-suite");
    }

    #[test]
    fn to_proto_maps_fields() {
        let r = RepositoryRef {
            forge: "gitlab.com".to_string(),
            owner: "acme".to_string(),
            repository: "perf-suite".to_string(),
        };
        let proto = r.to_proto();
        assert_eq!(proto.forge, "gitlab.com");
        assert_eq!(proto.owner, "acme");
        assert_eq!(proto.repository, "perf-suite");
    }

    #[test]
    fn forge_defaults_to_github_when_absent() {
        let toml = r#"
owner = "acme"
repository = "perf-suite"
"#;
        let r: RepositoryRef = toml::from_str(toml).unwrap();
        assert_eq!(r.forge, "github.com");
    }

    #[test]
    fn forge_is_read_when_present() {
        let toml = r#"
forge = "gitlab.com"
owner = "acme"
repository = "perf-suite"
"#;
        let r: RepositoryRef = toml::from_str(toml).unwrap();
        assert_eq!(r.forge, "gitlab.com");
    }
}
