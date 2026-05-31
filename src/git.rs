//! Git repository detection utilities.

use anyhow::{Context as _, Result};
use std::path::PathBuf;
use std::process::Command;

/// Git repository information.
pub struct GitInfo {
    pub repository: Option<String>,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
}

impl GitInfo {
    /// Merge CLI-provided overrides with autodetected git info.
    /// Returns resolved (repository, commit, branch) with errors for missing required fields.
    pub fn resolve(
        repo: Option<String>,
        commit: Option<String>,
        branch: Option<String>,
    ) -> (Result<String>, Result<String>, Option<String>) {
        let git_info = detect_git_info();

        let repository = repo
            .or(git_info.repository)
            .context("Could not detect repository. Use --repo or set PERFSCOPE_REPO");
        let commit_sha = commit
            .or(git_info.commit_sha)
            .context("Could not detect commit. Use --commit or set PERFSCOPE_COMMIT");
        let branch_resolved = branch.or(git_info.branch);

        (repository, commit_sha, branch_resolved)
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

fn detect_repository() -> Option<String> {
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

fn extract_repo_from_url(url: &str) -> Option<String> {
    // Handle SSH format: git@github.com:owner/repo.git
    if url.starts_with("git@") {
        let parts: Vec<&str> = url.split(':').collect();
        if parts.len() == 2 {
            return Some(parts[1].trim_end_matches(".git").to_string());
        }
    }

    // Handle HTTPS format: https://github.com/owner/repo.git
    if url.starts_with("https://") || url.starts_with("http://") {
        let path: String = url
            .split('/')
            .skip(3)
            .collect::<Vec<_>>()
            .join("/")
            .trim_end_matches(".git")
            .to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_repo_ssh() {
        assert_eq!(
            extract_repo_from_url("git@github.com:owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_extract_repo_https() {
        assert_eq!(
            extract_repo_from_url("https://github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_extract_repo_https_no_git_suffix() {
        assert_eq!(
            extract_repo_from_url("https://github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
    }
}
