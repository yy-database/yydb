//! Cross-process single-writer lock with a fencing epoch for file-backed databases.

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use yydb_formats::crc32c;

/// How a [`WriterLock`] was acquired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    /// Exclusive lock. Only one writer may hold it and it advances the fence.
    Writer,
    /// Shared lock. Any number of readers may hold it while no writer does.
    Reader,
}

const LOCK_MAGIC: &[u8; 4] = b"YYLK";
const LOCK_VERSION: u8 = 0;
const LOCK_HEADER_BYTES: usize = 17;

/// File lock held for the lifetime of a database handle.
///
/// Writers take an exclusive lock and persist a monotonically increasing fence
/// epoch in `{db}-lock`. Readers take a shared lock and observe the current
/// epoch without rewriting it. The OS releases the lock when the process exits,
/// so a crashed writer does not block the next process.
pub struct WriterLock {
    file: File,
    mode: LockMode,
    fence: u64,
}

impl WriterLock {
    /// Acquire the exclusive writer lock or fail when another holder exists.
    pub fn acquire(db_path: &Path) -> Result<Self> {
        let (file, fence) = open_and_lock(db_path, LockMode::Writer)?;
        Ok(Self { file, mode: LockMode::Writer, fence })
    }

    /// Acquire a shared reader lock. Readers do not advance the fence epoch.
    pub fn acquire_reader(db_path: &Path) -> Result<Self> {
        let (file, fence) = open_and_lock(db_path, LockMode::Reader)?;
        Ok(Self { file, mode: LockMode::Reader, fence })
    }

    /// Acquisition mode.
    pub fn mode(&self) -> LockMode {
        self.mode
    }

    /// Fence epoch observed or issued by this acquisition.
    pub fn fence(&self) -> u64 {
        self.fence
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

fn open_and_lock(db_path: &Path, mode: LockMode) -> Result<(File, u64)> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = lock_sidecar_path(db_path);
    let mut file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&lock_path)?;
    match mode {
        LockMode::Writer => lock_exclusive(&file)?,
        LockMode::Reader => lock_shared(&file)?,
    }
    let observed = read_fence(&mut file)?;
    if mode == LockMode::Reader {
        return Ok((file, observed.unwrap_or(0)));
    }
    let fence = observed.unwrap_or(0) + 1;
    write_fence(&mut file, fence)?;
    Ok((file, fence))
}

/// Read the persisted fence. `None` means the metadata was never completed.
fn read_fence(file: &mut File) -> Result<Option<u64>> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(None);
    }
    if len < LOCK_HEADER_BYTES as u64 {
        let mut head = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut head)?;
        if head.starts_with(LOCK_MAGIC) {
            // An acquisition died before its header reached stable storage.
            return Ok(None);
        }
        return Err(Error::Corrupt("writer lock metadata is not YYLK"));
    }
    let mut header = [0_u8; LOCK_HEADER_BYTES];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    if &header[..4] != LOCK_MAGIC {
        return Err(Error::Corrupt("writer lock metadata is not YYLK"));
    }
    if header[4] != LOCK_VERSION {
        return Err(Error::Unsupported("writer lock version"));
    }
    let stored = u32::from_le_bytes(header[13..17].try_into().unwrap());
    if stored != crc32c(&header[..13]) {
        return Err(Error::Corrupt("writer lock checksum mismatch"));
    }
    Ok(Some(u64::from_le_bytes(header[5..13].try_into().unwrap())))
}

/// Persist a new fence. The file is reset first so a crash leaves either empty
/// metadata or a complete header, never a new header followed by old bytes.
fn write_fence(file: &mut File, fence: u64) -> Result<()> {
    let mut header = [0_u8; LOCK_HEADER_BYTES];
    header[..4].copy_from_slice(LOCK_MAGIC);
    header[4] = LOCK_VERSION;
    header[5..13].copy_from_slice(&fence.to_le_bytes());
    let checksum = crc32c(&header[..13]);
    header[13..17].copy_from_slice(&checksum.to_le_bytes());

    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    file.sync_all()?;
    Ok(())
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
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
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
        lock_file_ex(file, true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(Error::Unsupported("cross-process writer lock is not implemented on this platform"))
    }
}

fn lock_shared(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
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
        lock_file_ex(file, false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(Error::Unsupported("cross-process reader lock is not implemented on this platform"))
    }
}

#[cfg(windows)]
fn lock_file_ex(file: &File, exclusive: bool) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{ERROR_IO_PENDING, ERROR_LOCK_VIOLATION, GetLastError},
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
    };
    let handle = file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let flags = if exclusive { LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY } else { LOCKFILE_FAIL_IMMEDIATELY };
    let ok = unsafe {
        let mut overlapped = std::mem::zeroed::<windows_sys::Win32::System::IO::OVERLAPPED>();
        LockFileEx(handle, flags, 0, u32::MAX, u32::MAX, &mut overlapped)
    };
    if ok != 0 {
        return Ok(());
    }
    let code = unsafe { GetLastError() };
    if code == ERROR_LOCK_VIOLATION || code == ERROR_IO_PENDING {
        return Err(writer_locked());
    }
    Err(Error::Io(std::io::Error::from_raw_os_error(code as i32)))
}

fn unlock_file(file: &File) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
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
    Error::Io(std::io::Error::new(std::io::ErrorKind::WouldBlock, "database writer lock held by another holder"))
}
