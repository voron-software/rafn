//! Machine-global advisory lock: only one `rafn bench` run at a time per host.
//!
//! The lock is an advisory `flock` on a *fixed*, machine-wide path so it is
//! shared across every repository and session on the machine. That is what
//! makes it machine-global: parallel agentic sessions benchmarking different
//! repos still serialize against one another, keeping timings free of CPU
//! contention. Because it is an OS advisory lock, it is released automatically
//! when the holding process exits or crashes — no stale lock files to clean up.

use anyhow::{Context, Result, bail};
use fs2::{FileExt, lock_contended_error};
use std::fs::{File, OpenOptions};
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
        let file = open_lock_file(path)?;

        // Fast path: the lock is free, take it without any waiting noise.
        if try_lock(&file)? {
            return Ok(Self { _file: file });
        }

        info!(
            "Another benchmark run holds the global lock ({}); waiting up to {}s",
            path.display(),
            timeout.as_secs()
        );

        let deadline = Instant::now() + timeout;
        loop {
            if try_lock(&file)? {
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

/// Attempt to take the exclusive lock once.
///
/// Returns `Ok(true)` when the lock was acquired and `Ok(false)` when another
/// process holds it (contention — worth retrying). Any other error (an
/// unsupported filesystem, exhausted lock resources, ...) is *not* contention
/// and is returned as `Err` so it surfaces immediately rather than spinning for
/// the full timeout behind a misleading "still waiting" message.
fn try_lock(file: &File) -> Result<bool> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == lock_contended_error().kind() => Ok(false),
        Err(e) => Err(anyhow::Error::new(e).context("Failed to acquire global benchmark lock")),
    }
}

/// Open (creating if absent) the lock file.
///
/// The file is opened for writing but deliberately *not* truncated and never
/// written to — we only need a stable inode to `flock`. Skipping truncation
/// avoids clobbering whatever an existing path points at (e.g. a symlink
/// planted in a world-writable temp dir).
fn open_lock_file(path: &Path) -> Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // World read/write (subject to umask) so any user sharing the host can
        // open the lock file. Only takes effect when the file is first created.
        opts.mode(0o666);
    }
    opts.open(path)
        .with_context(|| format!("Failed to open lock file {}", path.display()))
}

/// Fixed, machine-wide path for the lock file.
///
/// On Unix this is a hardcoded path under the shared, sticky `/tmp` rather than
/// `std::env::temp_dir()`, so sessions with differing `TMPDIR`/`TEMP`/`TMP`
/// still contend for the *same* lock — which is the whole point of the feature.
/// The sticky bit on `/tmp` keeps other users from replacing the file once it
/// exists.
fn lock_path() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp/rafn-bench.lock")
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("rafn-bench.lock")
    }
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

    #[test]
    fn does_not_truncate_existing_file() {
        // The lock file's contents must survive being opened for locking, so a
        // pre-existing path is never clobbered.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rafn-bench.lock");
        std::fs::write(&path, b"sentinel").unwrap();

        let lock = BenchLock::acquire_at(&path, Duration::from_secs(1)).unwrap();
        drop(lock);

        assert_eq!(std::fs::read(&path).unwrap(), b"sentinel");
    }
}
