//! Machine-global advisory lock: only one `rafn bench` run at a time per host.
//!
//! The lock is an advisory `flock` on a fixed path in the system temp dir, so
//! it is shared across every repository and session on the machine. That is
//! what makes it machine-global: parallel agentic sessions benchmarking
//! different repos still serialize against one another, keeping timings free of
//! CPU contention. Because it is an OS advisory lock, it is released
//! automatically when the holding process exits or crashes — no stale lock
//! files to clean up.

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tracing::info;

/// How often to retry acquiring the lock while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Held for the lifetime of a benchmark run. Dropping (or process exit)
/// releases the advisory lock.
pub struct BenchLock {
    _file: File,
}

impl BenchLock {
    /// Acquire the machine-global lock, waiting up to `timeout` for any other
    /// run to release it. Emits an INFO message if it has to wait, and returns
    /// an error if `timeout` elapses first.
    pub fn acquire(timeout: Duration) -> Result<Self> {
        Self::acquire_at(&lock_path(), timeout)
    }

    fn acquire_at(path: &Path, timeout: Duration) -> Result<Self> {
        let file = File::create(path)
            .with_context(|| format!("Failed to open lock file {}", path.display()))?;

        // Fast path: the lock is free, take it without any waiting noise.
        if file.try_lock_exclusive().is_ok() {
            return Ok(Self { _file: file });
        }

        info!(
            "Another benchmark run holds the global lock ({}); waiting up to {}s",
            path.display(),
            timeout.as_secs()
        );

        let deadline = Instant::now() + timeout;
        loop {
            if file.try_lock_exclusive().is_ok() {
                return Ok(Self { _file: file });
            }
            if Instant::now() >= deadline {
                bail!(
                    "Timed out after {}s waiting for the global benchmark lock ({})",
                    timeout.as_secs(),
                    path.display()
                );
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

/// Fixed machine-wide path for the lock file.
fn lock_path() -> PathBuf {
    std::env::temp_dir().join("rafn-bench.lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquires_when_free() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn-bench.lock");
        let lock = BenchLock::acquire_at(&path, Duration::from_secs(1));
        assert!(lock.is_ok());
    }

    #[test]
    fn times_out_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn-bench.lock");

        let held = BenchLock::acquire_at(&path, Duration::from_secs(1)).unwrap();

        // A second acquisition on the same path should give up after the
        // timeout rather than block forever.
        let contended = BenchLock::acquire_at(&path, Duration::from_millis(200));
        assert!(contended.is_err());

        drop(held);
    }

    #[test]
    fn reacquires_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn-bench.lock");

        let first = BenchLock::acquire_at(&path, Duration::from_secs(1)).unwrap();
        drop(first);

        let second = BenchLock::acquire_at(&path, Duration::from_secs(1));
        assert!(second.is_ok());
    }
}
