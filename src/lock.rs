//! Machine-global lock ensuring only one `rafn bench` run executes at a time
//! per host.
//!
//! On Linux the lock is a process-shared, **robust** `pthread` mutex living in
//! a named POSIX shared-memory object (`shm_open`). Being in shared memory
//! keyed by a fixed name makes it machine-global and independent of the
//! filesystem/`TMPDIR` — every process on the host maps the same mutex. Being
//! *robust* is what makes it safe for a benchmark runner: if a run is killed or
//! crashes while holding the lock, the kernel flags the next acquirer with
//! `EOWNERDEAD` instead of leaving the lock wedged forever. That acquirer calls
//! `pthread_mutex_consistent` and proceeds, so a dead owner never blocks the
//! machine.
//!
//! On non-Linux platforms the robust/`timedlock` primitives aren't uniformly
//! available, so the lock degrades to a no-op with a warning.
//!
//! ## Scope
//!
//! The guarantee is "one run per host *namespace*". POSIX shared-memory
//! objects live in the per-mount-namespace `/dev/shm`, so sessions in fully
//! isolated containers (separate mount/IPC namespaces) map different objects
//! and won't serialize against each other. This is inherent to any in-host
//! primitive — a named POSIX semaphore, SysV IPC, or an `flock` file all share
//! the same limitation — and there is no way to serialize across isolated
//! containers without a resource they share. To serialize containerized
//! sessions, run them in the host IPC/mount namespace (e.g. a shared
//! `/dev/shm`, `--ipc=host`).

#[cfg(target_os = "linux")]
pub use linux::BenchLock;

#[cfg(not(target_os = "linux"))]
pub use fallback::BenchLock;

#[cfg(not(target_os = "linux"))]
mod fallback {
    use anyhow::Result;
    use std::time::Duration;
    use tracing::warn;

    /// No-op lock guard for platforms without a robust process-shared mutex.
    pub struct BenchLock;

    impl BenchLock {
        pub fn acquire(_timeout: Duration) -> Result<Self> {
            warn!(
                "The machine-global benchmark lock is only implemented on Linux; \
                 proceeding without locking"
            );
            Ok(BenchLock)
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{Result, bail};
    use std::ffi::CStr;
    use std::io::Error as IoError;
    use std::mem;
    use std::os::raw::{c_char, c_void};
    use std::ptr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tracing::info;

    /// Fixed name of the shared-memory object. Leading slash per `shm_open(3)`.
    const SHM_NAME: &CStr = c"/rafn-bench.lock";

    /// Marker written once the mutex has been initialized, so processes that
    /// attach after the creator wait until the mutex is usable.
    const READY_MAGIC: u32 = 0x7261_666e; // "rafn"

    /// Layout of the shared-memory region.
    #[repr(C)]
    struct Shared {
        /// `READY_MAGIC` once `mutex` is initialized. Release/acquire handshake
        /// with whichever process initialized the region under the init lock.
        ready: AtomicU32,
        _pad: u32,
        mutex: libc::pthread_mutex_t,
    }

    /// Held for the lifetime of a benchmark run. Unlocks and unmaps on drop;
    /// the robust mutex is also auto-recovered if this process dies while
    /// holding it.
    ///
    /// Intentionally neither `Send` nor `Sync`: a `pthread` mutex is owned by
    /// the acquiring thread and must be unlocked by it, so the guard must not
    /// cross threads (holding it across an `.await` in a `Send` future would be
    /// a bug and won't compile).
    pub struct BenchLock {
        shared: *mut Shared,
        map_len: usize,
    }

    impl BenchLock {
        /// Acquire the machine-global lock, waiting up to `timeout`. Emits an
        /// INFO message if it has to wait, and returns an error if `timeout`
        /// elapses first.
        pub fn acquire(timeout: Duration) -> Result<Self> {
            Self::acquire_named(SHM_NAME, timeout)
        }

        /// Same as [`acquire`](Self::acquire) but against an explicit shm name,
        /// so tests can use isolated locks instead of the shared global one.
        fn acquire_named(name: &CStr, timeout: Duration) -> Result<Self> {
            let (shared, map_len) = map_shared(name)?;
            let mutex = unsafe { ptr::addr_of_mut!((*shared).mutex) };

            // The guard is only constructed once we actually hold the mutex, so
            // Drop never unlocks a mutex we don't own (undefined for a robust
            // NORMAL mutex). On the failure paths we unmap explicitly instead.

            // Fast path: take it without blocking or logging.
            match unsafe { libc::pthread_mutex_trylock(mutex) } {
                0 => return Ok(Self { shared, map_len }),
                libc::EOWNERDEAD => {
                    unsafe { libc::pthread_mutex_consistent(mutex) };
                    return Ok(Self { shared, map_len });
                }
                libc::EBUSY => {} // contended — fall through to the timed wait
                err => {
                    unmap(shared, map_len);
                    bail!(
                        "Failed to acquire global benchmark lock: {}",
                        IoError::from_raw_os_error(err)
                    );
                }
            }

            info!(
                "Another benchmark run holds the global lock; waiting up to {}s",
                timeout.as_secs()
            );

            let deadline = abs_deadline(timeout);
            match unsafe { libc::pthread_mutex_timedlock(mutex, &deadline) } {
                0 => Ok(Self { shared, map_len }),
                libc::EOWNERDEAD => {
                    unsafe { libc::pthread_mutex_consistent(mutex) };
                    Ok(Self { shared, map_len })
                }
                libc::ETIMEDOUT => {
                    unmap(shared, map_len);
                    bail!(
                        "Timed out after {}s waiting for the global benchmark lock",
                        timeout.as_secs()
                    )
                }
                err => {
                    unmap(shared, map_len);
                    bail!(
                        "Failed to acquire global benchmark lock: {}",
                        IoError::from_raw_os_error(err)
                    )
                }
            }
        }
    }

    impl Drop for BenchLock {
        fn drop(&mut self) {
            unsafe {
                libc::pthread_mutex_unlock(ptr::addr_of_mut!((*self.shared).mutex));
            }
            unmap(self.shared, self.map_len);
        }
    }

    /// Unmap a shared region. Does not touch the mutex, so it is safe on paths
    /// where we never acquired.
    fn unmap(shared: *mut Shared, len: usize) {
        unsafe { libc::munmap(shared as *mut c_void, len) };
    }

    /// Absolute `CLOCK_REALTIME` deadline `timeout` from now, saturating rather
    /// than overflowing on absurdly large timeouts.
    fn abs_deadline(timeout: Duration) -> libc::timespec {
        let mut now: libc::timespec = unsafe { mem::zeroed() };
        unsafe {
            libc::clock_gettime(libc::CLOCK_REALTIME, &mut now);
        }
        let mut secs = now.tv_sec as i128 + timeout.as_secs() as i128;
        let mut nanos = now.tv_nsec as i128 + timeout.subsec_nanos() as i128;
        if nanos >= 1_000_000_000 {
            secs += 1;
            nanos -= 1_000_000_000;
        }
        let tv_sec = secs.min(libc::time_t::MAX as i128) as libc::time_t;
        libc::timespec {
            tv_sec,
            tv_nsec: nanos as _,
        }
    }

    /// Open (creating if needed) the shared-memory object and return a pointer
    /// to its [`Shared`] region, ensuring the robust mutex inside is
    /// initialized exactly once.
    ///
    /// Initialization is serialized with an advisory `flock` on the object's
    /// descriptor. `flock` is released automatically when the holder's fd is
    /// closed or the process dies, so a run that crashes mid-initialization
    /// cannot leave the object permanently un-ready — the next process simply
    /// takes the init lock and finishes (or redoes) the setup.
    fn map_shared(name: &CStr) -> Result<(*mut Shared, usize)> {
        let size = mem::size_of::<Shared>();

        let fd = unsafe {
            libc::shm_open(
                name.as_ptr() as *const c_char,
                libc::O_CREAT | libc::O_RDWR,
                0o600,
            )
        };
        if fd < 0 {
            bail!("shm_open for lock {name:?}: {}", IoError::last_os_error());
        }

        if unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            let e = IoError::last_os_error();
            unsafe { libc::close(fd) };
            bail!("flock (init) lock shm {name:?}: {e}");
        }

        let result = init_region(name, fd, size);

        // Release the init lock and drop the fd; the mapping stays valid.
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
            libc::close(fd);
        }
        result.map(|shared| (shared, size))
    }

    /// Size, map, and (if not already done) initialize the shared region.
    /// Caller must hold the init `flock` on `fd`.
    fn init_region(name: &CStr, fd: i32, size: usize) -> Result<*mut Shared> {
        let mut st: libc::stat = unsafe { mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } != 0 {
            bail!("fstat lock shm {name:?}: {}", IoError::last_os_error());
        }
        // Grow to the needed size only if smaller, so an already-initialized
        // object is never re-truncated (which would zero the live mutex).
        if (st.st_size as usize) < size && unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0
        {
            bail!("ftruncate lock shm {name:?}: {}", IoError::last_os_error());
        }

        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            bail!("mmap lock shm {name:?}: {}", IoError::last_os_error());
        }
        let shared = ptr as *mut Shared;

        // Initialize the robust mutex once. Guarded by the init flock, so no
        // other process observes a half-initialized region.
        if unsafe { (*shared).ready.load(Ordering::Acquire) } != READY_MAGIC {
            let mutex = unsafe { ptr::addr_of_mut!((*shared).mutex) };
            if let Err(e) = init_robust_mutex(mutex) {
                unsafe { libc::munmap(ptr, size) };
                return Err(e);
            }
            unsafe { (*shared).ready.store(READY_MAGIC, Ordering::Release) };
        }
        Ok(shared)
    }

    /// Initialize a process-shared, robust mutex in place.
    fn init_robust_mutex(mutex: *mut libc::pthread_mutex_t) -> Result<()> {
        unsafe {
            let mut attr: libc::pthread_mutexattr_t = mem::zeroed();
            check(
                "pthread_mutexattr_init",
                libc::pthread_mutexattr_init(&mut attr),
            )?;

            let result = (|| {
                check(
                    "pthread_mutexattr_setpshared",
                    libc::pthread_mutexattr_setpshared(&mut attr, libc::PTHREAD_PROCESS_SHARED),
                )?;
                check(
                    "pthread_mutexattr_setrobust",
                    libc::pthread_mutexattr_setrobust(&mut attr, libc::PTHREAD_MUTEX_ROBUST),
                )?;
                check("pthread_mutex_init", libc::pthread_mutex_init(mutex, &attr))
            })();

            libc::pthread_mutexattr_destroy(&mut attr);
            result
        }
    }

    /// Turn a nonzero `pthread` return code into an error.
    fn check(what: &str, rc: i32) -> Result<()> {
        if rc == 0 {
            Ok(())
        } else {
            bail!("{what}: {}", IoError::from_raw_os_error(rc))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::CString;

        /// A unique shm name per call so concurrent tests don't collide with
        /// each other or with any real lock on the machine.
        fn unique_name() -> CString {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            CString::new(format!("/rafn-test-{}-{}.lock", std::process::id(), n)).unwrap()
        }

        fn unlink(name: &CStr) {
            unsafe { libc::shm_unlink(name.as_ptr() as *const c_char) };
        }

        #[test]
        fn acquires_when_free() {
            let name = unique_name();
            let lock = BenchLock::acquire_named(&name, Duration::from_secs(1));
            assert!(lock.is_ok());
            drop(lock);
            unlink(&name);
        }

        #[test]
        fn reacquires_after_release() {
            let name = unique_name();
            let first = BenchLock::acquire_named(&name, Duration::from_secs(1)).unwrap();
            drop(first);
            let second = BenchLock::acquire_named(&name, Duration::from_secs(1));
            assert!(second.is_ok());
            drop(second);
            unlink(&name);
        }

        #[test]
        fn times_out_while_held() {
            let name = unique_name();
            let held = BenchLock::acquire_named(&name, Duration::from_secs(1)).unwrap();

            // A second acquisition of the same lock must give up after the
            // timeout rather than block forever.
            let contended = BenchLock::acquire_named(&name, Duration::from_millis(200));
            assert!(contended.is_err());

            drop(held);
            unlink(&name);
        }

        #[test]
        fn recovers_from_dead_owner() {
            let name = unique_name();

            // A thread takes the lock and dies without releasing it (the guard
            // is leaked so Drop never runs), simulating a crashed benchmark.
            let holder_name = name.clone();
            std::thread::spawn(move || {
                let guard = BenchLock::acquire_named(&holder_name, Duration::from_secs(1)).unwrap();
                std::mem::forget(guard);
            })
            .join()
            .unwrap();

            // The robust mutex must let the next run recover instead of hanging.
            let recovered = BenchLock::acquire_named(&name, Duration::from_secs(1));
            assert!(recovered.is_ok());

            drop(recovered);
            unlink(&name);
        }
    }
}
