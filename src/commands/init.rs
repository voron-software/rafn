//! `rafn init` — scaffold a repo for cloud-backed benchmark tracking.
//!
//! Cloud is the compiled default backend (`src/config/repo.rs`), so this
//! writes just enough `rafn.toml` to make that explicit; local-first
//! scaffolding is out of scope for this command.

use anyhow::{Context, Result, bail};
use clap::Args;
use std::fs;
use std::path::Path;
use std::process::Command;

const RAFN_TOML_CONTENTS: &str = "[backend]\ntype = \"cloud\"\n";
const GITIGNORE_ENTRY: &str = ".rafn/snapshots";

#[derive(Args)]
pub struct InitCommand;

impl InitCommand {
    pub async fn execute(self) -> Result<()> {
        init_in(&std::env::current_dir()?)
    }
}

fn init_in(dir: &Path) -> Result<()> {
    let rafn_toml_path = dir.join("rafn.toml");
    if rafn_toml_path.exists() {
        bail!(
            "{} already exists; refusing to overwrite",
            rafn_toml_path.display()
        );
    }

    fs::write(&rafn_toml_path, RAFN_TOML_CONTENTS)
        .with_context(|| format!("Failed to write {}", rafn_toml_path.display()))?;
    println!("Created rafn.toml (backend: cloud)");

    if is_git_repo(dir) {
        if update_gitignore(dir)? {
            println!("Added {GITIGNORE_ENTRY} to .gitignore");
        }
    } else {
        println!("Not inside a git repository; skipping .gitignore update");
    }

    println!("\nRun `rafn bench` to record benchmark results.");
    Ok(())
}

/// Whether `dir` is inside a git working tree. Checked directly against
/// `dir` (rather than the process's cwd) so this stays independently
/// testable.
fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Add `.rafn/snapshots` to `dir`'s `.gitignore`, creating the file if it
/// doesn't exist. Returns whether the file was changed.
fn update_gitignore(dir: &Path) -> Result<bool> {
    let gitignore_path = dir.join(".gitignore");

    let existing = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)
            .with_context(|| format!("Failed to read {}", gitignore_path.display()))?
    } else {
        String::new()
    };

    if existing.lines().any(|line| line.trim() == GITIGNORE_ENTRY) {
        return Ok(false);
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(GITIGNORE_ENTRY);
    updated.push('\n');

    fs::write(&gitignore_path, updated)
        .with_context(|| format!("Failed to write {}", gitignore_path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_init(dir: &Path) {
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_creates_rafn_toml_with_cloud_backend() {
        let dir = tempfile::tempdir().unwrap();
        init_in(dir.path()).unwrap();

        let contents = fs::read_to_string(dir.path().join("rafn.toml")).unwrap();
        assert_eq!(contents, "[backend]\ntype = \"cloud\"\n");
    }

    #[test]
    fn test_refuses_to_overwrite_existing_rafn_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn.toml");
        fs::write(&path, "[backend]\ntype = \"local\"\n").unwrap();

        let result = init_in(dir.path());
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[backend]\ntype = \"local\"\n"
        );
    }

    #[test]
    fn test_skips_gitignore_in_non_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        init_in(dir.path()).unwrap();

        assert!(!dir.path().join(".gitignore").exists());
    }

    #[test]
    fn test_creates_gitignore_in_git_directory() {
        let dir = tempfile::tempdir().unwrap();
        git_init(dir.path());

        init_in(dir.path()).unwrap();

        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(contents, ".rafn/snapshots\n");
    }

    #[test]
    fn test_appends_to_existing_gitignore_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "/target").unwrap();

        assert!(update_gitignore(dir.path()).unwrap());

        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(contents, "/target\n.rafn/snapshots\n");
    }

    #[test]
    fn test_gitignore_update_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "/target\n.rafn/snapshots\n").unwrap();

        assert!(!update_gitignore(dir.path()).unwrap());

        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(contents, "/target\n.rafn/snapshots\n");
    }

    #[test]
    fn test_is_git_repo_false_for_plain_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
    }

    #[test]
    fn test_is_git_repo_true_after_git_init() {
        let dir = tempfile::tempdir().unwrap();
        git_init(dir.path());
        assert!(is_git_repo(dir.path()));
    }
}
