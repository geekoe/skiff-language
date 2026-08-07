//! Process pid-file self-defense (`dev-without-stack` D8).
//!
//! A process that runs with a `runDir` config option acquires an exclusive
//! pid file in that directory before starting listeners:
//!
//! - file absent -> create with O_EXCL, write own pid;
//! - file present with a **live** pid -> refuse to start (`AlreadyRunning`);
//! - file present with a **dead** pid -> take over (remove, recreate).
//!
//! The returned guard removes the file on graceful shutdown (Drop); a
//! SIGKILL leaves a stale file which the next start takes over. This makes it
//! impossible for two processes to share one run directory regardless of who
//! starts them (manual, launchd, deployment supervisor).
//!
//! When no `runDir` is configured the process simply skips pid acquisition
//! (backward compatible with existing deployments).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidLockError {
    AlreadyRunning { pid: u32 },
    Io(String),
}

impl std::fmt::Display for PidLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PidLockError::AlreadyRunning { pid } => {
                write!(formatter, "another process is already running (pid {pid})")
            }
            PidLockError::Io(message) => write!(formatter, "pid file operation failed: {message}"),
        }
    }
}

impl std::error::Error for PidLockError {}

/// Owns one exclusive pid file; removes it on Drop.
#[derive(Debug)]
pub struct PidFileGuard {
    path: PathBuf,
}

impl PidFileGuard {
    /// Acquire `<run_dir>/<name>.pid` with O_EXCL semantics.
    pub fn acquire(run_dir: &Path, name: &str) -> Result<PidFileGuard, PidLockError> {
        fs::create_dir_all(run_dir)
            .map_err(|error| PidLockError::Io(error.to_string()))?;
        let path = run_dir.join(format!("{name}.pid"));
        match Self::create_exclusive(&path) {
            Ok(()) => Ok(PidFileGuard { path }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if let Some(pid) = read_pid(&path) {
                    if pid_is_alive(pid) {
                        return Err(PidLockError::AlreadyRunning { pid });
                    }
                }
                fs::remove_file(&path)
                    .map_err(|error| PidLockError::Io(error.to_string()))?;
                Self::create_exclusive(&path)
                    .map_err(|error| PidLockError::Io(error.to_string()))?;
                Ok(PidFileGuard { path })
            }
            Err(error) => Err(PidLockError::Io(error.to_string())),
        }
    }

    fn create_exclusive(path: &Path) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;
        Ok(())
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn read_pid(path: &Path) -> Option<u32> {
    let content = fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

fn pid_is_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) performs a liveness probe only; it never signals.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    let error = io::Error::last_os_error();
    error.raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_creates_pid_file_with_own_pid() {
        let dir = tempdir();
        let guard = PidFileGuard::acquire(&dir, "router").expect("acquire");
        let content = fs::read_to_string(guard.path()).expect("read pid file");
        assert_eq!(content.trim(), std::process::id().to_string());
    }

    #[test]
    fn second_acquire_while_running_is_refused() {
        let dir = tempdir();
        let _guard = PidFileGuard::acquire(&dir, "runtime").expect("first acquire");
        let error = PidFileGuard::acquire(&dir, "runtime").expect_err("second acquire");
        assert!(matches!(error, PidLockError::AlreadyRunning { .. }));
    }

    #[test]
    fn drop_releases_the_lock() {
        let dir = tempdir();
        let first = PidFileGuard::acquire(&dir, "router").expect("first acquire");
        drop(first);
        let second = PidFileGuard::acquire(&dir, "router").expect("re-acquire after drop");
        drop(second);
    }

    #[test]
    fn stale_file_with_dead_pid_is_taken_over() {
        let dir = tempdir();
        let path = dir.join("router.pid");
        fs::write(&path, "999999999\n").expect("write stale pid");
        let guard = PidFileGuard::acquire(&dir, "router").expect("takeover");
        let content = fs::read_to_string(&path).expect("read pid file");
        assert_eq!(content.trim(), std::process::id().to_string());
        drop(guard);
    }

    #[test]
    fn different_names_do_not_conflict() {
        let dir = tempdir();
        let router = PidFileGuard::acquire(&dir, "router").expect("router lock");
        let runtime = PidFileGuard::acquire(&dir, "runtime").expect("runtime lock");
        drop(router);
        drop(runtime);
    }

    #[test]
    fn malformed_stale_file_is_taken_over() {
        let dir = tempdir();
        let path = dir.join("router.pid");
        fs::write(&path, "not-a-pid\n").expect("write malformed pid");
        let guard = PidFileGuard::acquire(&dir, "router").expect("takeover malformed");
        drop(guard);
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "skiff-pid-lock-test-{}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}

impl PidFileGuard {
    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }
}
