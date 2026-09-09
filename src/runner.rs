//! Subprocess execution for benchmarks.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use tracing::info;

use crate::framework::ProcessCommand;

/// Result of running a benchmark command.
#[allow(dead_code)]
#[derive(Debug)]
pub struct RunResult {
    pub exit_status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Run a benchmark command, streaming output in real-time.
pub fn run_benchmark(command: &ProcessCommand) -> Result<RunResult> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.current_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn: {}", command.display()))?;

    // `Stdio::piped()` above guarantees these are `Some` on first `take()`.
    let stdout_pipe = child
        .stdout
        .take()
        .context("child stdout pipe missing despite Stdio::piped()")?;
    let stderr_pipe = child
        .stderr
        .take()
        .context("child stderr pipe missing despite Stdio::piped()")?;

    // Stream stdout in a separate thread.
    let stdout_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout_pipe);
        let mut captured = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    info!("{line}");
                    captured.push_str(&line);
                    captured.push('\n');
                }
                Err(_) => break,
            }
        }
        captured
    });

    // Stream stderr in a separate thread.
    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr_pipe);
        let mut captured = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    info!("{line}");
                    captured.push_str(&line);
                    captured.push('\n');
                }
                Err(_) => break,
            }
        }
        captured
    });

    let status = child.wait().context("Failed to wait for child process")?;
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    Ok(RunResult {
        exit_status: status,
        stdout,
        stderr,
    })
}

// The tests drive a real subprocess through `sh`, so they only apply on unix.
#[cfg(all(test, unix))]
mod tests {
    use anyhow::Result;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use super::*;

    fn shell_command(script: &str, current_dir: PathBuf) -> ProcessCommand {
        ProcessCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            current_dir,
        }
    }

    #[test]
    fn captures_stdout_and_stderr_separately() -> Result<()> {
        let tmp = TempDir::new()?;
        let command = shell_command(
            "echo to-stdout; echo to-stderr >&2",
            tmp.path().to_path_buf(),
        );

        let result = run_benchmark(&command)?;

        assert!(result.exit_status.success());
        assert_eq!(result.stdout, "to-stdout\n");
        assert_eq!(result.stderr, "to-stderr\n");
        Ok(())
    }

    #[test]
    fn reports_non_zero_exit_status_without_failing() -> Result<()> {
        let tmp = TempDir::new()?;
        let command = shell_command("exit 3", tmp.path().to_path_buf());

        let result = run_benchmark(&command)?;

        assert!(!result.exit_status.success());
        assert_eq!(result.exit_status.code(), Some(3));
        Ok(())
    }

    #[test]
    fn spawn_failure_is_reported_as_error() -> Result<()> {
        let tmp = TempDir::new()?;
        let command = shell_command("true", tmp.path().join("does-not-exist"));

        let err = run_benchmark(&command)
            .err()
            .context("expected spawn to fail")?;

        assert!(err.to_string().contains("Failed to spawn"));
        Ok(())
    }
}
