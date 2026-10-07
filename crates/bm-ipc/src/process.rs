//! Process plumbing for the core side: private stdout, ignored console interrupts, parent watchdog, single-core
//! lock.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Hands out the process's real stdout and points fd 1 (Windows: the std output handle) at stderr, so later prints
/// land on stderr instead of the frame stream.
pub fn take_stdout() -> io::Result<File> {
    imp::take_stdout()
}

/// Ctrl+C in the server console reaches the whole process group; only the shim may stop the core.
pub fn ignore_interrupts() {
    imp::ignore_interrupts()
}

pub fn parent_alive(pid: u32) -> bool {
    imp::alive(pid)
}

/// Blocks until process `pid` is gone (returns at once if it already is).
pub fn wait_for_parent_exit(pid: u32) {
    imp::wait_exit(pid)
}

/// This process's resident set size in bytes (`/bluemap status`, docs/13 §5: the core shares the container limit).
pub fn resident_memory() -> Option<u64> {
    imp::rss()
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another BlueMap core (pid {pid:?}) is running on this folder")]
    Held { pid: Option<u32> },
    #[error("core lock {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

/// Exclusive lock on `<folder>/.core.lock` plus our pid in `<folder>/.core.pid` (Windows locks are mandatory, so
/// the pid can't live in the locked file). Released on drop or process exit.
pub struct CoreLock {
    _file: File,
    pid_file: PathBuf,
}

impl CoreLock {
    pub fn acquire(folder: &Path) -> Result<Self, LockError> {
        let lock = folder.join(".core.lock");
        let pid_file = folder.join(".core.pid");
        let err = |path: &Path| {
            let path = path.to_owned();
            move |source| LockError::Io { path, source }
        };
        std::fs::create_dir_all(folder).map_err(err(folder))?;
        let file = OpenOptions::new().create(true).truncate(false).write(true).open(&lock).map_err(err(&lock))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let pid = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse().ok());
                return Err(LockError::Held { pid });
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(err(&lock)(e)),
        }
        std::fs::write(&pid_file, std::process::id().to_string()).map_err(err(&pid_file))?;
        Ok(Self { _file: file, pid_file })
    }
}

impl Drop for CoreLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.pid_file);
    }
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::fd::FromRawFd;
    use std::time::Duration;

    pub fn take_stdout() -> io::Result<File> {
        // SAFETY: plain fd syscalls on fds 1 and 2, which every process has open
        unsafe {
            let fd = libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3);
            if fd < 0 || libc::dup2(2, 1) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(File::from_raw_fd(fd))
        }
    }

    pub fn ignore_interrupts() {
        // SAFETY: installing SIG_IGN has no preconditions
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
    }

    pub fn alive(pid: u32) -> bool {
        // SAFETY: signal 0 only checks for existence
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    pub fn rss() -> Option<u64> {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Some(pages * u64::try_from(page).ok()?)
    }

    pub fn wait_exit(pid: u32) {
        // a dead parent also shows as reparenting
        // SAFETY: getppid has no preconditions
        while alive(pid) && unsafe { libc::getppid() } as u32 == pid {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::{FromRawHandle, RawHandle};

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler, SetStdHandle,
    };
    use windows_sys::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    pub fn take_stdout() -> io::Result<File> {
        // SAFETY: std handle getters/setters; ownership of the original stdout handle moves into the File
        unsafe {
            let out = GetStdHandle(STD_OUTPUT_HANDLE);
            if out.is_null() || out == INVALID_HANDLE_VALUE {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no stdout handle"));
            }
            if SetStdHandle(STD_OUTPUT_HANDLE, GetStdHandle(STD_ERROR_HANDLE)) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(File::from_raw_handle(out as RawHandle))
        }
    }

    pub fn ignore_interrupts() {
        // SAFETY: a null handler with TRUE makes this process ignore Ctrl+C
        unsafe {
            SetConsoleCtrlHandler(None, 1);
        }
    }

    pub fn alive(pid: u32) -> bool {
        // SAFETY: the handle is checked and closed
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            let running = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
            CloseHandle(h);
            running
        }
    }

    pub fn rss() -> Option<u64> {
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: c is a properly sized out-struct; the pseudo handle needs no closing
        let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (ok != 0).then_some(c.WorkingSetSize as u64)
    }

    pub fn wait_exit(pid: u32) {
        // SAFETY: the handle is checked and closed
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return;
            }
            WaitForSingleObject(h, INFINITE);
            CloseHandle(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_records_pid() {
        let dir = std::env::temp_dir().join(format!("bm-ipc-lock-{}", std::process::id()));
        let first = CoreLock::acquire(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(".core.pid")).unwrap(), std::process::id().to_string());
        match CoreLock::acquire(&dir) {
            Err(LockError::Held { pid }) => assert_eq!(pid, Some(std::process::id())),
            other => panic!("expected Held, got {:?}", other.err()),
        }
        drop(first);
        assert!(!dir.join(".core.pid").exists());
        drop(CoreLock::acquire(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn own_process_is_alive() {
        assert!(parent_alive(std::process::id()));
        assert!(!parent_alive(u32::MAX - 1));
    }
}
