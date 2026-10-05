//! Cross-process exclusive writer lock for file-backed databases.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use yydb_types::{Error, Result};

/// Exclusive writer lock held for the lifetime of a file-backed [`Connection`].
pub struct WriterLock {
    file: File,
}

impl WriterLock {
    /// Acquire `{db}-lock` or fail when another process already holds it.
    pub fn acquire(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_path = lock_sidecar_path(db_path);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        unlock_exclusive(&self.file);
    }
}

fn lock_sidecar_path(db_path: &Path) -> PathBuf {
    let mut sidecar = db_path.as_os_str().to_owned();
    sidecar.push("-lock");
    PathBuf::from(sidecar)
}

fn lock_exclusive(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let rc = libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB);
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return Err(writer_locked());
        }
        return Err(Error::Io(err));
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{
            GetLastError, ERROR_IO_PENDING, ERROR_LOCK_VIOLATION,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };
        let handle = file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        let ok = unsafe {
            let mut overlapped = std::mem::zeroed::<windows_sys::Win32::System::IO::OVERLAPPED>();
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let code = unsafe { GetLastError() };
        if code == ERROR_LOCK_VIOLATION || code == ERROR_IO_PENDING {
            return Err(writer_locked());
        }
        return Err(Error::Io(std::io::Error::from_raw_os_error(code as i32)));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(Error::Unsupported(
            "cross-process writer lock is not implemented on this platform",
        ))
    }
}

fn unlock_exclusive(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let _ = libc::flock(file.as_raw_fd(), libc::LOCK_UN);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        let handle = file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
        unsafe {
            let mut overlapped = std::mem::zeroed::<windows_sys::Win32::System::IO::OVERLAPPED>();
            let _ = UnlockFileEx(handle, 0, u32::MAX, u32::MAX, &mut overlapped);
        }
    }
}

fn writer_locked() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "database writer lock held by another process",
    ))
}
