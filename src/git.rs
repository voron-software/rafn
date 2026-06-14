//! Git repository detection utilities.

use anyhow::{Context as _, Result};
use std::path::PathBuf;
use std::process::Command;
use url::Url;

use crate::config::RepositoryRef;

/// Git repository information.
pub struct GitInfo {
    pub repository: Option<RepositoryRef>,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
}

impl GitInfo {
    /// Merge CLI-provided overrides with autodetected git info.
    /// Returns resolved (commit, branch) with an error for a missing commit.
    pub fn resolve(
        commit: Option<String>,
        branch: Option<String>,
    ) -> (Result<String>, Option<String>) {
        let git_info = detect_git_info();

        let commit_sha = commit
            .or(git_info.commit_sha)
            .context("Could not detect commit. Use --commit or set RAFN_COMMIT");
        let branch_resolved = branch.or(git_info.branch);

        (commit_sha, branch_resolved)
    }
}

/// Detect git repository information from the current directory.
pub fn detect_git_info() -> GitInfo {
    GitInfo {
        repository: detect_repository(),
        commit_sha: detect_commit_sha(),
        branch: detect_branch(),
    }
}

/// Detect the repository identity from the `origin` remote URL, parsing the
/// host as `forge` and the remaining path as `owner/repository`.
pub fn detect_repository() -> Option<RepositoryRef> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    extract_repo_from_url(&url)
}

fn detect_commit_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Absolute path of the git repository root containing the cwd, via
/// `git rev-parse --show-toplevel`. `None` when not inside a git repo.
pub fn detect_git_root() -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn detect_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Parse a git remote URL into a [`RepositoryRef`], taking the host as
/// `forge` and the remaining path as `owner/repository`.
fn extract_repo_from_url(url: &str) -> Option<RepositoryRef> {
    // Git's SCP-like syntax (`[user@]host:path`, e.g.
    // `git@github.com:owner/repo.git`) isn't a URL; rewrite it to the
    // equivalent `ssh://` form so it can go through the same parser as
    // `https://`/`ssh://` remotes.
    let normalized = if url.contains("://") {
        url.to_string()
    } else {
        let (host_part, path_part) = url.split_once(':')?;
        format!("ssh://{host_part}/{path_part}")
    };

    let parsed = Url::parse(&normalized).ok()?;
    let forge = parsed.host_str()?.to_string();

    let path = parsed
        .path()
        .trim_start_matches('/')
        .trim_end_matches(".git");
    let (owner, repository) = path.split_once('/')?;

    Some(RepositoryRef {
        forge,
        owner: owner.to_string(),
        repository: repository.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_ssh() {
        assert_eq!(
            extract_repo_from_url("git@github.com:owner/repo.git"),
            Some(RepositoryRef {
                forge: "github.com".to_string(),
                owner: "owner".to_string(),
                repository: "repo".to_string(),
            })
        );
    }

    #[test]
    fn test_extract_repo_https() {
        assert_eq!(
            extract_repo_from_url("https://github.com/owner/repo.git"),
            Some(RepositoryRef {
                forge: "github.com".to_string(),
                owner: "owner".to_string(),
                repository: "repo".to_string(),
            })
        );
    }

    #[test]
    fn test_extract_repo_https_no_git_suffix() {
        assert_eq!(
            extract_repo_from_url("https://github.com/owner/repo"),
            Some(RepositoryRef {
                forge: "github.com".to_string(),
                owner: "owner".to_string(),
                repository: "repo".to_string(),
            })
        );
    }

    #[test]
    fn test_extract_repo_https_with_credentials() {
        assert_eq!(
            extract_repo_from_url("https://x-access-token@github.com/owner/repo.git"),
            Some(RepositoryRef {
                forge: "github.com".to_string(),
                owner: "owner".to_string(),
                repository: "repo".to_string(),
            })
        );
    }

    #[test]
    fn test_extract_repo_gitlab_ssh() {
        assert_eq!(
            extract_repo_from_url("git@gitlab.com:owner/repo.git"),
            Some(RepositoryRef {
                forge: "gitlab.com".to_string(),
                owner: "owner".to_string(),
                repository: "repo".to_string(),
            })
        );
    }
}
