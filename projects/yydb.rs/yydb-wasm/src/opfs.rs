//! OPFS host I/O adapter (host glue only).
//!
//! The browser hands over `FileSystemSyncAccessHandle` objects for the main
//! file and its sidecars. This module only moves bytes through those handles.
//! Publish ordering, image validation, quota accounting and the generation it
//! advances all live in `database.storage::local_io::volume`.
//!
//! A synchronized wasm surface cannot create an OPFS handle by itself: the file
//! system API is promise based and sync access handles only exist inside a
//! worker. When the host cannot supply one, every entry point answers
//! [`Error::NativeCapabilityMissing`] instead of emulating a durable publish in
//! process memory.

use std::sync::Mutex;

use database::{
    storage::local_io::{HostBlobDirectory, HostVolumeStore, VolumePublisher},
    types::{Error, Result},
};
use wasm_bindgen::JsValue;

/// Persistent storage layout selected from the logical path suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistentStorageMode {
    /// Single-file `.yydb` layout.
    Yydb,
    /// Multi-file `.yydx` layout with a sibling blob directory.
    Yydx,
}

impl PersistentStorageMode {
    /// Infer the layout from `path`.
    pub fn from_path(path: &str) -> Result<Self> {
        if path.ends_with(".yydx") {
            return Ok(Self::Yydx);
        }
        if path.ends_with(".yydb") {
            return Ok(Self::Yydb);
        }
        Err(Error::Unsupported("persistent path must end with .yydb or .yydx"))
    }
}

/// Primitives an OPFS host must actually provide for a durable volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpfsCapabilities {
    /// A `FileSystemSyncAccessHandle` was created for the main file.
    pub sync_access_handle: bool,
    /// `read(buffer, { at })` and `write(buffer, { at })` are available.
    pub range_read_write: bool,
    /// `flush()` is available as the durability barrier.
    pub durability_sync: bool,
    /// The host holds the exclusive handle, so no other context can write.
    pub exclusive_writer: bool,
    /// The host resolved the `.yydx` blob directory facade.
    pub blob_directory: bool,
}

impl OpfsCapabilities {
    /// Profile of a host that already opened every sync handle a `.yydb` needs.
    pub fn host_sync_handles() -> Self {
        Self {
            sync_access_handle: true,
            range_read_write: true,
            durability_sync: true,
            exclusive_writer: true,
            blob_directory: false,
        }
    }

    /// Profile of a host that also resolved the `.yydx` blob directory facade.
    pub fn host_sync_handles_with_directory() -> Self {
        Self { blob_directory: true, ..Self::host_sync_handles() }
    }

    /// Profile of a host that provided nothing.
    pub fn unavailable() -> Self {
        Self::default()
    }
}

/// Reject a volume whose host cannot satisfy the required primitives.
pub fn validate_opfs_capabilities(caps: &OpfsCapabilities, mode: PersistentStorageMode) -> Result<()> {
    if !caps.sync_access_handle {
        return Err(Error::NativeCapabilityMissing { capability: "opfs.sync_access_handle" });
    }
    if !caps.range_read_write {
        return Err(Error::NativeCapabilityMissing { capability: "opfs.range_read_write" });
    }
    if !caps.durability_sync {
        return Err(Error::NativeCapabilityMissing { capability: "opfs.durability_sync" });
    }
    if !caps.exclusive_writer {
        return Err(Error::NativeCapabilityMissing { capability: "opfs.exclusive_writer" });
    }
    if mode == PersistentStorageMode::Yydx && !caps.blob_directory {
        // Enumerating and unlinking blobs needs the directory API, which is
        // asynchronous. Sync handles alone cannot serve a `.yydx` volume.
        return Err(Error::NativeCapabilityMissing { capability: "opfs.blob_directory" });
    }
    Ok(())
}

/// The upstream publisher bound to a host byte store, behind one worker-local lock.
pub struct PersistentVolume {
    inner: Mutex<VolumePublisher<Box<dyn HostVolumeStore>>>,
}

impl PersistentVolume {
    /// Bind `path` to a host byte store. The store owns every read and write.
    pub fn open(path: &str, store: Box<dyn HostVolumeStore>) -> Result<Self> {
        Ok(Self { inner: Mutex::new(VolumePublisher::open(path, store)?) })
    }

    /// Bind `path` to a host byte store and a `.yydx` blob directory port.
    pub fn open_with_directory(
        path: &str,
        store: Box<dyn HostVolumeStore>,
        directory: Box<dyn HostBlobDirectory>,
    ) -> Result<Self> {
        let publisher = VolumePublisher::open(path, store)?.with_blob_directory(directory);
        Ok(Self { inner: Mutex::new(publisher) })
    }

    /// Logical database path.
    pub fn path(&self) -> String {
        self.publisher().path().to_owned()
    }

    /// Generation of the last durable main-file publication.
    pub fn generation(&self) -> u64 {
        self.publisher().generation()
    }

    /// Durable main-file image, when the host already published one.
    pub fn read_main(&self) -> Result<Option<Vec<u8>>> {
        self.publisher().read_main()
    }

    /// Publish a complete `YDPG` main-file image through the upstream contract.
    pub fn publish_main(&self, body: &[u8]) -> Result<()> {
        self.publisher().publish_main(body).map(|_| ())
    }

    /// Read back the `YYWL` sidecar, when the host published one.
    pub fn read_wal(&self) -> Result<Option<Vec<u8>>> {
        self.publisher().read_wal()
    }

    /// Publish the `YYWL` sidecar behind the host durability barrier.
    pub fn publish_wal(&self, body: &[u8]) -> Result<()> {
        self.publisher().publish_wal(body)
    }

    /// Remove the `YYWL` sidecar.
    pub fn remove_wal(&self) -> Result<()> {
        self.publisher().remove_wal()
    }

    /// Read back the `YYSH` sidecar, when the host published one.
    pub fn read_shm(&self) -> Result<Option<Vec<u8>>> {
        self.publisher().read_shm()
    }

    /// Publish the `YYSH` sidecar behind the host durability barrier.
    pub fn publish_shm(&self, body: &[u8]) -> Result<()> {
        self.publisher().publish_shm(body)
    }

    /// Remove the `YYSH` sidecar.
    pub fn remove_shm(&self) -> Result<()> {
        self.publisher().remove_shm()
    }

    /// Hashes of the blob segments the host directory currently holds.
    pub fn list_blobs(&self) -> Result<Vec<String>> {
        self.publisher().list_blobs()
    }

    /// Read one blob segment from the host store.
    pub fn read_blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.publisher().read_blob(hash)
    }

    /// Publish one immutable blob segment through the host store.
    pub fn publish_blob(&self, hash: &str, body: &[u8]) -> Result<()> {
        self.publisher().publish_blob(hash, body)
    }

    /// Delete one blob segment through the host directory (or the byte store).
    pub fn remove_blob(&self, hash: &str) -> Result<()> {
        self.publisher().remove_blob(hash)
    }

    fn publisher(&self) -> std::sync::MutexGuard<'_, VolumePublisher<Box<dyn HostVolumeStore>>> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Build a volume from host-supplied OPFS objects.
///
/// `handles` is `{ main, wal?, shm?, blobs?, directory? }`:
/// - `main`/`wal`/`shm` are `FileSystemSyncAccessHandle`s the host opened in a
///   worker.
/// - `blobs` maps a known blob hash to its sync handle.
/// - `directory` is the synchronous facade the host built from the
///   asynchronous `FileSystemDirectoryHandle` pair `values()` / `removeEntry()`,
///   with `list(): string[]` and `remove(hash: string)`.
///
/// The capability profile is derived from the objects the host actually handed
/// over, so a handle missing `flush` reports `opfs.durability_sync` instead of
/// opening a volume that cannot make a durability promise.
pub fn open_volume_from_handles(path: &str, handles: JsValue) -> Result<PersistentVolume> {
    let mode = PersistentStorageMode::from_path(path)?;
    let (store, capabilities, directory) = host_store::from_handles(handles)?;
    validate_opfs_capabilities(&capabilities, mode)?;
    match directory {
        Some(directory) => PersistentVolume::open_with_directory(path, store, directory),
        None => PersistentVolume::open(path, store),
    }
}

#[cfg(target_arch = "wasm32")]
mod host_store {
    use std::collections::BTreeMap;

    use js_sys::{Array, Function, Object, Reflect, Uint8Array};
    use wasm_bindgen::{JsCast, JsValue};

    use database::{
        storage::local_io::{HostBlobDirectory, HostVolumeStore, VolumeFile},
        types::{Error, Result},
    };

    use super::OpfsCapabilities;

    /// Members the volume contract needs: sized range reads and writes plus a
    /// durability barrier. `FileSystemSyncAccessHandle` provides all of them.
    const RANGE_METHODS: [&str; 4] = ["getSize", "read", "write", "truncate"];
    const DURABILITY_METHOD: &str = "flush";

    struct SyncHandle {
        value: JsValue,
    }

    impl SyncHandle {
        fn new(value: JsValue) -> Result<Self> {
            if value.is_null() || value.is_undefined() {
                return Err(Error::NativeCapabilityMissing { capability: "opfs.sync_access_handle" });
            }
            Ok(Self { value })
        }

        fn has(&self, name: &str) -> Result<bool> {
            let member = Reflect::get(&self.value, &JsValue::from_str(name)).map_err(js_error)?;
            Ok(member.is_function())
        }

        /// Report which primitives this specific host object actually exposes.
        ///
        /// A sync access handle is exclusive by specification, so holding one
        /// is what rules out a concurrent writer.
        fn capabilities(&self) -> Result<OpfsCapabilities> {
            let mut range_read_write = true;
            for name in RANGE_METHODS {
                range_read_write &= self.has(name)?;
            }
            Ok(OpfsCapabilities {
                sync_access_handle: true,
                range_read_write,
                durability_sync: self.has(DURABILITY_METHOD)?,
                exclusive_writer: true,
                blob_directory: false,
            })
        }

        fn call(&self, name: &str, args: &[JsValue]) -> Result<JsValue> {
            let function: Function = Reflect::get(&self.value, &JsValue::from_str(name)).map_err(js_error)?.into();
            let list = js_sys::Array::new();
            for arg in args {
                list.push(arg);
            }
            function.apply(&self.value, &list).map_err(js_error)
        }

        fn size(&self) -> Result<u64> {
            let size = self.call("getSize", &[])?;
            let size = size.as_f64().ok_or(Error::NativeCapabilityMissing { capability: "opfs.sync_access_handle" })?;
            Ok(size.max(0.0) as u64)
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
            let view = Uint8Array::new_with_length(buf.len() as u32);
            let read = self.call("read", &[JsValue::from(view.clone()), at_object(offset)?])?;
            let read = read.as_f64().unwrap_or(0.0).max(0.0) as usize;
            let read = read.min(buf.len());
            view.subarray(0, read as u32).copy_to(&mut buf[..read]);
            Ok(read)
        }

        fn write_at(&self, offset: u64, body: &[u8]) -> Result<()> {
            let view = Uint8Array::new_with_length(body.len() as u32);
            view.copy_from(body);
            let written = self.call("write", &[JsValue::from(view), at_object(offset)?])?;
            let written = written.as_f64().unwrap_or(0.0).max(0.0) as usize;
            if written != body.len() {
                return Err(Error::Io(std::io::Error::other(format!(
                    "OPFS short write at {offset}: {written} of {} bytes",
                    body.len()
                ))));
            }
            Ok(())
        }

        fn truncate(&self, len: u64) -> Result<()> {
            self.call("truncate", &[JsValue::from_f64(len as f64)]).map(|_| ())
        }

        fn flush(&self) -> Result<()> {
            self.call("flush", &[]).map(|_| ())
        }
    }

    fn at_object(offset: u64) -> Result<JsValue> {
        let options = Object::new();
        Reflect::set(&options, &JsValue::from_str("at"), &JsValue::from_f64(offset as f64)).map_err(js_error)?;
        Ok(options.into())
    }

    fn js_error(value: JsValue) -> Error {
        Error::Io(std::io::Error::other(format!("OPFS host call failed: {value:?}")))
    }

    /// OPFS byte store: one sync handle per logical file, no process state.
    pub(crate) struct OpfsVolumeStore {
        main: SyncHandle,
        wal: Option<SyncHandle>,
        shm: Option<SyncHandle>,
        blobs: BTreeMap<String, SyncHandle>,
    }

    pub(crate) fn from_handles(
        handles: JsValue,
    ) -> Result<(Box<dyn HostVolumeStore>, OpfsCapabilities, Option<Box<dyn HostBlobDirectory>>)> {
        if handles.is_null() || handles.is_undefined() {
            return Err(Error::NativeCapabilityMissing { capability: "opfs.sync_access_handle" });
        }
        let main = SyncHandle::new(Reflect::get(&handles, &JsValue::from_str("main")).map_err(js_error)?)?;
        let wal = optional_handle(&handles, "wal")?;
        let shm = optional_handle(&handles, "shm")?;
        let blobs = blob_handles(&handles)?;
        let directory = blob_directory(&handles)?;
        let mut capabilities = main.capabilities()?;
        capabilities.blob_directory = directory.is_some();
        Ok((Box::new(OpfsVolumeStore { main, wal, shm, blobs }), capabilities, directory))
    }

    fn optional_handle(handles: &JsValue, key: &str) -> Result<Option<SyncHandle>> {
        let value = Reflect::get(handles, &JsValue::from_str(key)).map_err(js_error)?;
        if value.is_null() || value.is_undefined() {
            return Ok(None);
        }
        SyncHandle::new(value).map(Some)
    }

    /// Sync handles the host already opened for blob segments it knows about.
    fn blob_handles(handles: &JsValue) -> Result<BTreeMap<String, SyncHandle>> {
        let value = Reflect::get(handles, &JsValue::from_str("blobs")).map_err(js_error)?;
        let mut out = BTreeMap::new();
        if value.is_null() || value.is_undefined() {
            return Ok(out);
        }
        if !value.is_object() {
            return Err(Error::NativeCapabilityMissing { capability: "opfs.blob_directory" });
        }
        for key in Object::keys(&Object::from(value.clone())).iter() {
            let Some(name) = key.as_string()
            else {
                continue;
            };
            let handle = Reflect::get(&value, &key).map_err(js_error)?;
            out.insert(name, SyncHandle::new(handle)?);
        }
        Ok(out)
    }

    /// The `.yydx` blob directory facade the host handed over.
    ///
    /// `FileSystemDirectoryHandle.values()` and `removeEntry()` return promises,
    /// so a synchronized wasm call cannot drive them directly. The host awaits
    /// them in JS and exposes the resolved pair as `list()` / `remove()`, which
    /// this adapter calls through `Reflect`.
    fn blob_directory(handles: &JsValue) -> Result<Option<Box<dyn HostBlobDirectory>>> {
        let value = Reflect::get(handles, &JsValue::from_str("directory")).map_err(js_error)?;
        if value.is_null() || value.is_undefined() {
            return Ok(None);
        }
        BlobDirectoryHandle::new(value).map(|handle| Some(Box::new(handle) as Box<dyn HostBlobDirectory>))
    }

    /// Synchronous facade over the host's asynchronous directory operations.
    pub(crate) struct BlobDirectoryHandle {
        value: JsValue,
    }

    impl BlobDirectoryHandle {
        fn new(value: JsValue) -> Result<Self> {
            if !value.is_object() {
                return Err(Error::NativeCapabilityMissing { capability: "opfs.blob_directory" });
            }
            let handle = Self { value };
            // A raw `FileSystemDirectoryHandle` only exposes the asynchronous
            // pair. Accepting it would mean pretending an unresolved promise
            // answered the listing.
            if !handle.has("list")? || !handle.has("remove")? {
                return Err(Error::NativeCapabilityMissing { capability: "opfs.blob_directory" });
            }
            Ok(handle)
        }

        fn has(&self, name: &str) -> Result<bool> {
            let member = Reflect::get(&self.value, &JsValue::from_str(name)).map_err(js_error)?;
            Ok(member.is_function())
        }

        fn call(&self, name: &str, args: &[JsValue]) -> Result<JsValue> {
            let function: Function = Reflect::get(&self.value, &JsValue::from_str(name)).map_err(js_error)?.into();
            let list = Array::new();
            for arg in args {
                list.push(arg);
            }
            function.apply(&self.value, &list).map_err(js_error)
        }
    }

    impl HostBlobDirectory for BlobDirectoryHandle {
        fn list(&mut self) -> Result<Vec<String>> {
            let value = self.call("list", &[])?;
            let entries: Array = value
                .dyn_into()
                .map_err(|_| Error::Io(std::io::Error::other("blob directory list() must return an array")))?;
            let mut out = Vec::with_capacity(entries.length() as usize);
            for entry in entries.iter() {
                let Some(hash) = entry.as_string()
                else {
                    return Err(Error::Io(std::io::Error::other("blob directory list() entries must be strings")));
                };
                out.push(hash);
            }
            Ok(out)
        }

        fn remove(&mut self, hash: &str) -> Result<()> {
            self.call("remove", &[JsValue::from_str(hash)]).map(|_| ())
        }
    }

    impl OpfsVolumeStore {
        fn handle(&self, file: &VolumeFile) -> Result<&SyncHandle> {
            match file {
                VolumeFile::Main => Ok(&self.main),
                VolumeFile::Wal => self.wal.as_ref().ok_or(Error::NativeCapabilityMissing { capability: "opfs.wal_sidecar" }),
                VolumeFile::Shm => self.shm.as_ref().ok_or(Error::NativeCapabilityMissing { capability: "opfs.shm_sidecar" }),
                VolumeFile::Blob(hash) => {
                    self.blobs.get(hash).ok_or(Error::NativeCapabilityMissing { capability: "opfs.blob_directory" })
                }
            }
        }
    }

    impl HostVolumeStore for OpfsVolumeStore {
        fn len(&mut self, file: &VolumeFile) -> Result<Option<u64>> {
            let size = self.handle(file)?.size()?;
            Ok((size > 0).then_some(size))
        }

        fn read_at(&mut self, file: &VolumeFile, offset: u64, buf: &mut [u8]) -> Result<usize> {
            self.handle(file)?.read_at(offset, buf)
        }

        fn write_at(&mut self, file: &VolumeFile, offset: u64, body: &[u8]) -> Result<()> {
            self.handle(file)?.write_at(offset, body)
        }

        fn truncate(&mut self, file: &VolumeFile, len: u64) -> Result<()> {
            self.handle(file)?.truncate(len)
        }

        fn sync(&mut self, file: &VolumeFile) -> Result<()> {
            self.handle(file)?.flush()
        }

        fn remove(&mut self, file: &VolumeFile) -> Result<()> {
            // A sync access handle cannot unlink. Truncating to zero makes the
            // file read as absent, which is the same observable volume state.
            self.handle(file)?.truncate(0)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod host_store {
    use wasm_bindgen::JsValue;

    use database::{
        storage::local_io::{HostBlobDirectory, HostVolumeStore},
        types::{Error, Result},
    };

    use super::OpfsCapabilities;

    pub(crate) fn from_handles(
        _handles: JsValue,
    ) -> Result<(Box<dyn HostVolumeStore>, OpfsCapabilities, Option<Box<dyn HostBlobDirectory>>)> {
        Err(Error::NativeCapabilityMissing { capability: "opfs.sync_access_handle" })
    }
}
