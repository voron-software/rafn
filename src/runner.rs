//! Subprocess execution for benchmarks.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};

use crate::framework::ProcessCommand;

/// Result of running a benchmark command.
#[allow(dead_code)]
pub struct RunResult {
    pub exit_status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Run a benchmark command, streaming output in real-time.
pub fn run_benchmark(command: &ProcessCommand, verbose: bool) -> Result<RunResult> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.current_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn: {}", command.display()))?;

    let stdout_pipe = child.stdout.take().unwrap();
    let stderr_pipe = child.stderr.take().unwrap();

    // Stream stdout in a separate thread
    let stdout_verbose = verbose;
    let stdout_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout_pipe);
        let mut captured = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if stdout_verbose {
                        println!("{}", line);
                    }
                    captured.push_str(&line);
                    captured.push('\n');
                }
                Err(_) => break,
            }
        }
        captured
    });

    // Stream stderr in a separate thread
    let stderr_verbose = verbose;
    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr_pipe);
        let mut captured = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if stderr_verbose {
                        eprintln!("{}", line);
                    }
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
