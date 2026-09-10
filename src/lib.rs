//! # ropfs - Origin Private File System
//!
//! A Rust implementation of the [Origin Private File System](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system) browser API.
//!
//! ## Usage
//!
//! The main entry point is the [`persistent`] module, which provides platform-agnostic
//! types that automatically resolve to the correct implementation:
//!
//! ```rust
//! use ropfs::persistent::{DirectoryHandle, FileHandle, WritableFileStream, app_specific_dir};
//! use ropfs::{GetFileHandleOptions, CreateWritableOptions};
//! use ropfs::persistent;
//!
//! // you must import the traits to call methods on the types
//! use ropfs::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};
//!
//! // This code works on both native and web platforms
//! async fn example(mut dir: DirectoryHandle) -> persistent::Result<()> {
//!     let options = GetFileHandleOptions { create: true };
//!     let mut file = dir.get_file_handle_with_options("example.txt", &options).await?;
//!
//!     let write_options = CreateWritableOptions { keep_existing_data: false, mode: Default::default() };
//!     let mut writer = file.create_writable_with_options(&write_options).await?;
//!
//!     writer.write_at_cursor_pos(b"Hello, world!").await?;
//!     writer.close().await?;
//!
//!     let data = file.read().await?;
//!     println!("File contents: {:?}", String::from_utf8(data));
//!
//!     Ok(())
//! }
//!
//! async fn use_example() -> persistent::Result<()> {
//!     let directory: DirectoryHandle = app_specific_dir().await?;
//!     example(directory).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Platform-Specific Modules
//!
//! For advanced use cases, you can also access platform-specific implementations directly:
//!
//! - [`native`] - Native file system operations using `tokio::fs`
//! - [`web`] - Web platform operations using OPFS APIs
//! - [`memory`] - In-memory filesystem for use in tests (or when persistence isn't necessary)

pub mod memory;
pub mod persistent;
pub mod sync;

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(test)]
mod conformance;

use futures_core::Stream;
use std::fmt::Debug;
use std::ops::RangeBounds;

mod sealed {
    #[cfg(not(target_arch = "wasm32"))]
    pub trait MaybeSend: Send {}
    #[cfg(not(target_arch = "wasm32"))]
    impl<T: Send> MaybeSend for T {}

    #[cfg(target_arch = "wasm32")]
    pub trait MaybeSend {}
    #[cfg(target_arch = "wasm32")]
    impl<T> MaybeSend for T {}
}

pub struct GetFileHandleOptions {
    pub create: bool,
}

pub struct GetDirectoryHandleOptions {
    pub create: bool,
}

pub struct CreateWritableOptions {
    pub keep_existing_data: bool,
    pub mode: WritableMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritableMode {
    #[default]
    Siloed,
    Exclusive,
}

pub struct FileSystemRemoveOptions {
    pub recursive: bool,
}

#[derive(Debug, Clone)]
pub enum WriteCommandType {
    Write,
    Seek,
    Truncate,
}

#[derive(Debug, Clone)]
pub struct WriteParams {
    pub command_type: WriteCommandType,
    pub data: Option<Vec<u8>>,
    pub position: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum DirectoryEntry<Directory, File> {
    File(File),
    Directory(Directory),
}

pub trait DirectoryHandle: Debug + Sized + private::Sealed {
    type Error: Debug;
    type FileHandleT: FileHandle<Error = Self::Error>;

    fn get_file_handle_with_options(
        &mut self,
        name: &str,
        options: &GetFileHandleOptions,
    ) -> impl std::future::Future<Output = Result<Self::FileHandleT, Self::Error>> + sealed::MaybeSend;

    fn get_directory_handle_with_options(
        &mut self,
        name: &str,
        options: &GetDirectoryHandleOptions,
    ) -> impl std::future::Future<Output = Result<Self, Self::Error>> + sealed::MaybeSend;

    fn remove_entry(
        &mut self,
        name: &str,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn remove_entry_with_options(
        &mut self,
        name: &str,
        options: &FileSystemRemoveOptions,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    #[allow(clippy::type_complexity)]
    fn entries(
        &self,
    ) -> impl std::future::Future<
        Output = Result<
            impl Stream<Item = Result<(String, DirectoryEntry<Self, Self::FileHandleT>), Self::Error>>,
            Self::Error,
        >,
    > + sealed::MaybeSend;
}

pub trait FileHandle: Debug + private::Sealed {
    type Error: Debug;
    type WritableFileStreamT: WritableFileStream<Error = Self::Error>;
    type SyncAccessHandleT: SyncAccessHandle<Error = Self::Error>;

    fn create_writable_with_options(
        &mut self,
        options: &CreateWritableOptions,
    ) -> impl std::future::Future<Output = Result<Self::WritableFileStreamT, Self::Error>>
    + sealed::MaybeSend;

    fn read(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + sealed::MaybeSend;

    fn read_range<R: RangeBounds<u64> + Send>(
        &self,
        range: R,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + sealed::MaybeSend;

    fn size(
        &self,
    ) -> impl std::future::Future<Output = Result<u64, Self::Error>> + sealed::MaybeSend;

    /// Creates a synchronous access handle for high-performance read/write.
    ///
    /// On native and memory backends this is always available. On web (wasm32),
    /// this requires the `unstable_apis` cargo feature
    /// (`--features unstable_apis`), which enables the
    /// `web_sys_unstable_apis` cfg required by `web-sys` sync-access bindings.
    /// Alternatively pass `RUSTFLAGS='--cfg web_sys_unstable_apis'` directly.
    #[cfg(any(not(target_arch = "wasm32"), web_sys_unstable_apis))]
    fn create_sync_access_handle(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::SyncAccessHandleT, Self::Error>>
    + sealed::MaybeSend;
}

pub trait WritableFileStream: Debug + private::Sealed {
    type Error: Debug;

    fn write_at_cursor_pos(
        &mut self,
        data: &[u8],
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn write_with_params(
        &mut self,
        params: &WriteParams,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn truncate(
        &mut self,
        size: u64,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn close(
        &mut self,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn seek(
        &mut self,
        offset: u64,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;
}

pub trait SyncAccessHandle: Debug + private::Sealed {
    type Error: Debug;

    fn read(&self, buffer: &mut [u8], at: u64) -> Result<usize, Self::Error>;

    fn write(&self, data: &[u8], at: u64) -> Result<usize, Self::Error>;

    fn truncate(&self, size: u64) -> Result<(), Self::Error>;

    fn get_size(&self) -> Result<u64, Self::Error>;

    fn flush(&self) -> Result<(), Self::Error>;
}

mod private {
    pub trait Sealed {}
}

/// Traits re-exported for method resolution.
pub mod prelude {
    pub use crate::DirectoryHandle as _;
    pub use crate::FileHandle as _;
    #[cfg(any(not(target_arch = "wasm32"), web_sys_unstable_apis))]
    pub use crate::SyncAccessHandle as _;
    pub use crate::WritableFileStream as _;
}

/// Provides simple read/write/remove/exists/list operations without
/// needing to import the `DirectoryHandle` and `FileHandle` traits.
///
/// Paths are `/`-delimited (leading/trailing slashes are tolerated) and
/// automatically create parent directories on write. Reads and existence
/// checks traverse existing directories without creating them.
///
/// Beyond the basics this offers string helpers, append, metadata
/// queries (`metadata`/`is_file`/`is_dir`), directory removal,
/// recursive `rename`/`copy`/`list_recursive`, `clear`, and storage
/// management (`estimate`/`persist`/`persisted`). Multi-step operations
/// such as `rename` and `copy` are **not atomic**: they copy first and
/// delete last, so a failure can leave both source and destination behind.
///
/// ```no_run
/// # async fn example() {
/// use ropfs::AppFs;
///
/// let mut app = AppFs::new().await.unwrap();
///
/// // Flat paths work as before
/// app.write("hello.txt", b"world").await.unwrap();
/// let data = app.read("hello.txt").await.unwrap();
///
/// // Nested paths create subdirectories automatically
/// app.write("config/settings.json", b"{}").await.unwrap();
/// assert!(app.exists("config/settings.json").await.unwrap());
///
/// // Ensure a directory hierarchy exists
/// app.ensure_dir("cache/audio").await.unwrap();
/// # }
/// ```
/// Quota/usage information for the app's storage area.
///
/// On web this comes from `navigator.storage.estimate()` (both fields are
/// usually present). On native platforms `quota` is `None` (no API reports
/// it portably) and `usage` is the recursively summed size of the app
/// directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageEstimate {
    /// Total bytes available, if the platform reports it.
    pub quota: Option<u64>,
    /// Bytes currently used, if the platform reports it.
    pub usage: Option<u64>,
}

/// Metadata for a path in [`AppFs`], mirroring `std::fs::metadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    pub is_file: bool,
    pub is_dir: bool,
    /// Byte length for files, `None` for directories.
    pub len: Option<u64>,
}

/// An opened path: either a file or a directory handle.
enum OpenEntry {
    File(persistent::FileHandle),
    Dir(persistent::DirectoryHandle),
}

pub struct AppFs {
    dir: persistent::DirectoryHandle,
}

impl AppFs {
    /// Open the app-specific directory for file operations.
    pub async fn new() -> persistent::Result<Self> {
        let dir = persistent::app_specific_dir().await?;
        Ok(Self { dir })
    }

    /// Open the app-specific directory for `info` (see
    /// [`persistent::app_specific_dir_for`]).
    pub async fn new_for(info: &persistent::AppInfo) -> persistent::Result<Self> {
        let dir = persistent::app_specific_dir_for(info).await?;
        Ok(Self { dir })
    }

    /// Split a `/`-delimited path into parent segments and leaf name.
    ///
    /// Leading/trailing slashes are tolerated (`"/a/b/"` behaves like
    /// `"a/b"`); interior empty segments are skipped during navigation.
    fn split_path(path: &str) -> (Vec<&str>, &str) {
        let mut parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let file_name = parts.pop().unwrap_or("");
        (parts, file_name)
    }

    /// Navigate to a subdirectory, optionally creating missing segments.
    async fn navigate_to(
        &self,
        segments: &[&str],
        create: bool,
    ) -> persistent::Result<persistent::DirectoryHandle> {
        use crate::DirectoryHandle as _;

        let mut dir = self.dir.clone();
        for &segment in segments {
            if !segment.is_empty() {
                let opts = GetDirectoryHandleOptions { create };
                dir = dir
                    .get_directory_handle_with_options(segment, &opts)
                    .await?;
            }
        }
        Ok(dir)
    }

    /// Read the full contents of a file at the given path.
    ///
    /// Parent directories are traversed (not created). Returns an error
    /// if the filge or any parent directory does not exist.
    pub async fn read(&mut self, path: &str) -> persistent::Result<Vec<u8>> {
        use crate::DirectoryHandle as _;
        use crate::FileHandle as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, false).await?;
        let opts = GetFileHandleOptions { create: false };
        let file = dir.get_file_handle_with_options(file_name, &opts).await?;
        file.read().await
    }

    /// Write data to a file at the given path, creating or truncating it.
    ///
    /// Parent directories are created automatically if they don't exist.
    pub async fn write(&mut self, path: &str, data: &[u8]) -> persistent::Result<()> {
        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, true).await?;
        Self::write_file_to(&mut dir, file_name, data).await
    }

    /// Write `data` to `name` inside an already-opened directory.
    async fn write_file_to(
        dir: &mut persistent::DirectoryHandle,
        name: &str,
        data: &[u8],
    ) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;
        use crate::FileHandle as _;
        use crate::WritableFileStream as _;

        let opts = GetFileHandleOptions { create: true };
        let mut file = dir.get_file_handle_with_options(name, &opts).await?;
        let write_opts = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file.create_writable_with_options(&write_opts).await?;
        writer.write_at_cursor_pos(data).await?;
        writer.close().await
    }

    /// Open `path` and classify it as a file or directory handle.
    ///
    /// Missing paths (including missing parents) yield
    /// [`persistent::Error::NotFound`]; other failures propagate.
    async fn open_entry(&mut self, path: &str) -> persistent::Result<OpenEntry> {
        use crate::DirectoryHandle as _;

        let (parents, leaf) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, false).await?;
        let file_res = dir
            .get_file_handle_with_options(leaf, &GetFileHandleOptions { create: false })
            .await;
        let dir_res = dir
            .get_directory_handle_with_options(leaf, &GetDirectoryHandleOptions { create: false })
            .await;
        match (file_res, dir_res) {
            (_, Ok(dir)) => Ok(OpenEntry::Dir(dir)),
            (Ok(file), Err(_)) => Ok(OpenEntry::File(file)),
            (Err(file_err), Err(dir_err)) => {
                if file_err.is_not_found() && dir_err.is_not_found() {
                    Err(persistent::Error::NotFound(path.to_string()))
                } else if file_err.is_not_found() {
                    Err(dir_err)
                } else {
                    Err(file_err)
                }
            }
        }
    }

    /// Remove a file at the given path.
    pub async fn remove(&mut self, path: &str) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, false).await?;
        dir.remove_entry(file_name).await
    }

    /// Check whether a file exists at the given path.
    ///
    /// Returns `Ok(false)` only when the file or a parent directory is
    /// missing. Other failures (invalid names, type mismatches, I/O errors,
    /// quota, ...) are propagated to the caller instead of being silently
    /// reported as "does not exist".
    pub async fn exists(&mut self, path: &str) -> persistent::Result<bool> {
        use crate::DirectoryHandle as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = match self.navigate_to(&parents, false).await {
            Ok(d) => d,
            Err(e) if e.is_not_found() => return Ok(false),
            Err(e) => return Err(e),
        };
        let opts = GetFileHandleOptions { create: false };
        match dir.get_file_handle_with_options(file_name, &opts).await {
            Ok(_) => Ok(true),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Ensure a directory hierarchy exists, creating missing segments.
    ///
    /// Accepts a `/`-delimited path. All segments are created if absent.
    pub async fn ensure_dir(&mut self, path: &str) -> persistent::Result<()> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.navigate_to(&segments, true).await?;
        Ok(())
    }

    /// Read the full contents of a file as a UTF-8 string.
    pub async fn read_to_string(&mut self, path: &str) -> persistent::Result<String> {
        let data = self.read(path).await?;
        String::from_utf8(data)
            .map_err(|e| persistent::Error::Msg(format!("'{path}' is not valid UTF-8: {e}")))
    }

    /// Write a UTF-8 string to a file, creating or truncating it.
    pub async fn write_string(&mut self, path: &str, data: &str) -> persistent::Result<()> {
        self.write(path, data.as_bytes()).await
    }

    /// Append data to a file, creating it (and parent directories) if needed.
    pub async fn append(&mut self, path: &str, data: &[u8]) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;
        use crate::FileHandle as _;
        use crate::WritableFileStream as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, true).await?;
        let opts = GetFileHandleOptions { create: true };
        let mut file = dir.get_file_handle_with_options(file_name, &opts).await?;
        let end = file.size().await?;
        let write_opts = CreateWritableOptions {
            keep_existing_data: true,
            mode: WritableMode::Siloed,
        };
        let mut writer = file.create_writable_with_options(&write_opts).await?;
        writer.seek(end).await?;
        writer.write_at_cursor_pos(data).await?;
        writer.close().await
    }

    /// Metadata for the file or directory at `path`.
    ///
    /// Errors with [`persistent::Error::NotFound`] when nothing exists there.
    pub async fn metadata(&mut self, path: &str) -> persistent::Result<Metadata> {
        use crate::FileHandle as _;

        match self.open_entry(path).await? {
            OpenEntry::File(file) => Ok(Metadata {
                is_file: true,
                is_dir: false,
                len: Some(file.size().await?),
            }),
            OpenEntry::Dir(_) => Ok(Metadata {
                is_file: false,
                is_dir: true,
                len: None,
            }),
        }
    }

    /// Whether `path` names an existing file (`Ok(false)` when missing).
    pub async fn is_file(&mut self, path: &str) -> persistent::Result<bool> {
        match self.open_entry(path).await {
            Ok(OpenEntry::File(_)) => Ok(true),
            Ok(OpenEntry::Dir(_)) => Ok(false),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Whether `path` names an existing directory (`Ok(false)` when missing).
    pub async fn is_dir(&mut self, path: &str) -> persistent::Result<bool> {
        match self.open_entry(path).await {
            Ok(OpenEntry::Dir(_)) => Ok(true),
            Ok(OpenEntry::File(_)) => Ok(false),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// List all entries in the root app-specific directory.
    pub async fn list(&self) -> persistent::Result<Vec<String>> {
        Self::entry_names(&self.dir).await
    }

    /// Collect entry names of an open directory (per-item errors skipped,
    /// mirroring [`AppFs::list`]).
    async fn entry_names(dir: &persistent::DirectoryHandle) -> persistent::Result<Vec<String>> {
        use crate::DirectoryHandle as _;
        use futures_util::StreamExt;

        let entries = dir.entries().await?;
        let names = entries
            .filter_map(|r| async { r.ok().map(|(name, _)| name) })
            .collect()
            .await;
        Ok(names)
    }

    /// List entry names inside the directory at `path`.
    pub async fn list_dir(&self, path: &str) -> persistent::Result<Vec<String>> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let dir = self.navigate_to(&segments, false).await?;
        Self::entry_names(&dir).await
    }

    /// List all files and directories under `path`, recursively.
    ///
    /// Returned paths are relative to `path`, use `/` as separator, and
    /// directories carry a trailing `/`.
    pub async fn list_recursive(&self, path: &str) -> persistent::Result<Vec<String>> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let dir = self.navigate_to(&segments, false).await?;
        let mut out = Vec::new();
        Self::list_recursive_in(&dir, String::new(), &mut out).await?;
        Ok(out)
    }

    async fn list_recursive_in(
        dir: &persistent::DirectoryHandle,
        prefix: String,
        out: &mut Vec<String>,
    ) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;
        use futures_util::StreamExt;

        let stream = dir.entries().await?;
        let items: Vec<
            persistent::Result<(
                String,
                crate::DirectoryEntry<persistent::DirectoryHandle, persistent::FileHandle>,
            )>,
        > = stream.collect().await;
        for item in items {
            let (name, entry) = item?;
            match entry {
                crate::DirectoryEntry::File(_) => out.push(format!("{prefix}{name}")),
                crate::DirectoryEntry::Directory(sub) => {
                    out.push(format!("{prefix}{name}/"));
                    Box::pin(Self::list_recursive_in(
                        &sub,
                        format!("{prefix}{name}/"),
                        out,
                    ))
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// Remove the (empty) directory at `path`.
    ///
    /// Fails when the directory is not empty; use [`AppFs::remove_dir_all`]
    /// for recursive deletion.
    pub async fn remove_dir(&mut self, path: &str) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, false).await?;
        dir.remove_entry(file_name).await
    }

    /// Remove the file or directory at `path`, recursing into directories.
    pub async fn remove_dir_all(&mut self, path: &str) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;

        let (parents, file_name) = Self::split_path(path);
        let mut dir = self.navigate_to(&parents, false).await?;
        let opts = FileSystemRemoveOptions { recursive: true };
        dir.remove_entry_with_options(file_name, &opts).await
    }

    /// Remove every entry in the root app-specific directory.
    pub async fn clear(&mut self) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;

        let mut dir = self.dir.clone();
        for name in Self::entry_names(&dir).await? {
            let opts = FileSystemRemoveOptions { recursive: true };
            dir.remove_entry_with_options(&name, &opts).await?;
        }
        Ok(())
    }

    /// Move the file or directory at `from` to `to`.
    ///
    /// Parent directories of `to` are created as needed. Directories move
    /// recursively; moving a directory into itself is rejected. This is
    /// **not atomic** (copy first, delete last).
    pub async fn rename(&mut self, from: &str, to: &str) -> persistent::Result<()> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        if from == to {
            return Ok(());
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(persistent::Error::Msg(format!(
                "cannot move '{from}' into itself"
            )));
        }
        match self.open_entry(from).await? {
            OpenEntry::File(_) => {
                let data = self.read(from).await?;
                self.write(to, &data).await?;
                self.remove(from).await?;
            }
            OpenEntry::Dir(src) => {
                self.ensure_dir(to).await?;
                let (to_parents, to_leaf) = Self::split_path(to);
                let mut dst = self.open_child_dir(&to_parents, to_leaf, true).await?;
                Self::copy_dir_recursive(&src, &mut dst).await?;
                self.remove_dir_all(from).await?;
            }
        }
        Ok(())
    }

    /// Copy the file or directory at `from` to `to`.
    ///
    /// Semantics mirror [`AppFs::rename`] without deleting the source. Like
    /// `rename`, this is **not atomic**.
    pub async fn copy(&mut self, from: &str, to: &str) -> persistent::Result<()> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        if from == to {
            return Ok(());
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(persistent::Error::Msg(format!(
                "cannot copy '{from}' into itself"
            )));
        }
        match self.open_entry(from).await? {
            OpenEntry::File(_) => {
                let data = self.read(from).await?;
                self.write(to, &data).await?;
            }
            OpenEntry::Dir(src) => {
                self.ensure_dir(to).await?;
                let (to_parents, to_leaf) = Self::split_path(to);
                let mut dst = self.open_child_dir(&to_parents, to_leaf, true).await?;
                Self::copy_dir_recursive(&src, &mut dst).await?;
            }
        }
        Ok(())
    }

    /// Open the child `leaf` of `parents` (navigated from the root).
    async fn open_child_dir(
        &self,
        parents: &[&str],
        leaf: &str,
        create: bool,
    ) -> persistent::Result<persistent::DirectoryHandle> {
        use crate::DirectoryHandle as _;

        let mut dir = self.navigate_to(parents, create).await?;
        let opts = GetDirectoryHandleOptions { create };
        dir.get_directory_handle_with_options(leaf, &opts).await
    }

    /// Recursively copy the contents of `src` into the existing `dst`.
    async fn copy_dir_recursive(
        src: &persistent::DirectoryHandle,
        dst: &mut persistent::DirectoryHandle,
    ) -> persistent::Result<()> {
        use crate::DirectoryHandle as _;
        use futures_util::StreamExt;

        let src = src.clone();
        let stream = src.entries().await?;
        let items: Vec<
            persistent::Result<(
                String,
                crate::DirectoryEntry<persistent::DirectoryHandle, persistent::FileHandle>,
            )>,
        > = stream.collect().await;
        for item in items {
            let (name, entry) = item?;
            match entry {
                crate::DirectoryEntry::File(file) => {
                    use crate::FileHandle as _;

                    let data = file.read().await?;
                    Self::write_file_to(dst, &name, &data).await?;
                }
                crate::DirectoryEntry::Directory(sub) => {
                    let mut sub_dst = dst
                        .get_directory_handle_with_options(
                            &name,
                            &GetDirectoryHandleOptions { create: true },
                        )
                        .await?;
                    Box::pin(Self::copy_dir_recursive(&sub, &mut sub_dst)).await?;
                }
            }
        }
        Ok(())
    }

    /// Quota/usage information for the app's storage area.
    ///
    /// Web: `navigator.storage.estimate()`. Native: recursively summed
    /// directory size (`quota` is `None`).
    pub async fn estimate(&self) -> persistent::Result<StorageEstimate> {
        #[cfg(target_arch = "wasm32")]
        {
            persistent::storage_estimate().await
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(StorageEstimate {
                quota: None,
                usage: Some(self.dir.storage_usage().await?),
            })
        }
    }

    /// Request persistent storage (`navigator.storage.persist()` on web;
    /// always `Ok(true)` on native, where storage is durable by nature).
    pub async fn persist(&self) -> persistent::Result<bool> {
        #[cfg(target_arch = "wasm32")]
        {
            persistent::storage_persist().await
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(true)
        }
    }

    /// Query persistent-storage status (`navigator.storage.persisted()` on
    /// web; always `Ok(true)` on native).
    pub async fn persisted(&self) -> persistent::Result<bool> {
        #[cfg(target_arch = "wasm32")]
        {
            persistent::storage_persisted().await
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(true)
        }
    }
}

/// Read the full contents of a file in the app-specific directory.
///
/// Convenience shorthand for a single `AppFs::new()` + `read()` call.
pub async fn read(name: &str) -> persistent::Result<Vec<u8>> {
    AppFs::new().await?.read(name).await
}

/// Write data to a file in the app-specific directory, creating or truncating it.
///
/// Convenience shorthand for a single `AppFs::new()` + `write()` call.
pub async fn write(name: &str, data: &[u8]) -> persistent::Result<()> {
    AppFs::new().await?.write(name, data).await
}

/// Remove a file from the app-specific directory.
///
/// Convenience shorthand for a single `AppFs::new()` + `remove()` call.
pub async fn remove(name: &str) -> persistent::Result<()> {
    AppFs::new().await?.remove(name).await
}

/// Check whether a file exists in the app-specific directory.
///
/// Convenience shorthand for a single `AppFs::new()` + `exists()` call.
pub async fn exists(name: &str) -> persistent::Result<bool> {
    AppFs::new().await?.exists(name).await
}

/// Ensure a directory hierarchy exists, creating missing segments.
///
/// Convenience shorthand for a single `AppFs::new()` + `ensure_dir()` call.
pub async fn ensure_dir(path: &str) -> persistent::Result<()> {
    AppFs::new().await?.ensure_dir(path).await
}

/// Read a file in the app-specific directory as a UTF-8 string.
pub async fn read_to_string(path: &str) -> persistent::Result<String> {
    AppFs::new().await?.read_to_string(path).await
}

/// Write a UTF-8 string to a file in the app-specific directory.
pub async fn write_string(path: &str, data: &str) -> persistent::Result<()> {
    AppFs::new().await?.write_string(path, data).await
}

/// Append data to a file in the app-specific directory.
pub async fn append(path: &str, data: &[u8]) -> persistent::Result<()> {
    AppFs::new().await?.append(path, data).await
}

/// Metadata for a file or directory in the app-specific directory.
pub async fn metadata(path: &str) -> persistent::Result<Metadata> {
    AppFs::new().await?.metadata(path).await
}

/// Whether `path` names an existing file in the app-specific directory.
pub async fn is_file(path: &str) -> persistent::Result<bool> {
    AppFs::new().await?.is_file(path).await
}

/// Whether `path` names an existing directory in the app-specific directory.
pub async fn is_dir(path: &str) -> persistent::Result<bool> {
    AppFs::new().await?.is_dir(path).await
}

/// Remove the (empty) directory at `path` in the app-specific directory.
pub async fn remove_dir(path: &str) -> persistent::Result<()> {
    AppFs::new().await?.remove_dir(path).await
}

/// Remove the file or directory at `path`, recursing into directories.
pub async fn remove_dir_all(path: &str) -> persistent::Result<()> {
    AppFs::new().await?.remove_dir_all(path).await
}

/// Move the file or directory at `from` to `to` (not atomic).
pub async fn rename(from: &str, to: &str) -> persistent::Result<()> {
    AppFs::new().await?.rename(from, to).await
}

/// Copy the file or directory at `from` to `to` (not atomic).
pub async fn copy(from: &str, to: &str) -> persistent::Result<()> {
    AppFs::new().await?.copy(from, to).await
}

/// List entry names inside the directory at `path`.
pub async fn list_dir(path: &str) -> persistent::Result<Vec<String>> {
    AppFs::new().await?.list_dir(path).await
}

/// List all files and directories under `path`, recursively.
pub async fn list_recursive(path: &str) -> persistent::Result<Vec<String>> {
    AppFs::new().await?.list_recursive(path).await
}

/// Remove every entry in the root app-specific directory.
pub async fn clear() -> persistent::Result<()> {
    AppFs::new().await?.clear().await
}

/// Quota/usage information for the app's storage area.
pub async fn estimate() -> persistent::Result<StorageEstimate> {
    AppFs::new().await?.estimate().await
}

/// Request persistent storage.
pub async fn persist() -> persistent::Result<bool> {
    AppFs::new().await?.persist().await
}

/// Query persistent-storage status.
pub async fn persisted() -> persistent::Result<bool> {
    AppFs::new().await?.persisted().await
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod appfs_tests {
    use super::*;

    /// `AppFs` rooted at a temp dir (never touches the real app directory).
    fn test_fs() -> (tempfile::TempDir, AppFs) {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = persistent::DirectoryHandle::from(tmp.path().to_path_buf());
        (tmp, AppFs { dir })
    }

    #[tokio::test]
    async fn strings_append_metadata() {
        let (_tmp, mut app) = test_fs();
        app.write_string("a/b.txt", "hello").await.unwrap();
        assert_eq!(app.read_to_string("a/b.txt").await.unwrap(), "hello");
        app.append("a/b.txt", b" world").await.unwrap();
        assert_eq!(app.read_to_string("a/b.txt").await.unwrap(), "hello world");
        let meta = app.metadata("a/b.txt").await.unwrap();
        assert!(meta.is_file && !meta.is_dir && meta.len == Some(11));
        assert!(app.is_file("a/b.txt").await.unwrap());
        assert!(!app.is_dir("a/b.txt").await.unwrap());
        assert!(app.is_dir("a").await.unwrap());
        assert!(!app.is_file("a").await.unwrap());
        assert!(!app.is_file("nope.txt").await.unwrap());
        assert!(!app.is_dir("nope.txt").await.unwrap());
        assert!(app.metadata("nope.txt").await.is_err());
        assert!(app.read_to_string("a/b.txt").await.is_ok());
    }

    #[tokio::test]
    async fn rename_copy_remove() {
        let (_tmp, mut app) = test_fs();
        app.write("d1/f.txt", b"data").await.unwrap();
        app.copy("d1/f.txt", "d2/g.txt").await.unwrap();
        assert_eq!(app.read("d1/f.txt").await.unwrap(), b"data");
        assert_eq!(app.read("d2/g.txt").await.unwrap(), b"data");
        app.rename("d1/f.txt", "d1/f.txt").await.unwrap();
        app.rename("d1/f.txt", "d3/h.txt").await.unwrap();
        assert!(app.exists("d3/h.txt").await.unwrap());
        assert!(!app.exists("d1/f.txt").await.unwrap());
        app.rename("d2", "d4").await.unwrap();
        assert_eq!(app.read("d4/g.txt").await.unwrap(), b"data");
        assert!(!app.is_dir("d2").await.unwrap());
        assert!(app.rename("d4", "d4/sub").await.is_err());
        assert!(app.copy("d4", "d4/sub2").await.is_err());
        app.remove_dir_all("d4").await.unwrap();
        assert!(!app.is_dir("d4").await.unwrap());
        app.ensure_dir("empty").await.unwrap();
        app.remove_dir("empty").await.unwrap();
        assert!(!app.is_dir("empty").await.unwrap());
        app.write("full/f.txt", b"x").await.unwrap();
        assert!(app.remove_dir("full").await.is_err());
    }

    #[tokio::test]
    async fn list_and_clear_and_estimate() {
        let (_tmp, mut app) = test_fs();
        app.write("x.txt", b"1").await.unwrap();
        app.write("sub/y.txt", b"22").await.unwrap();
        let mut root = app.list().await.unwrap();
        root.sort();
        assert_eq!(root, vec!["sub".to_string(), "x.txt".to_string()]);
        assert_eq!(
            app.list_dir("sub").await.unwrap(),
            vec!["y.txt".to_string()]
        );
        let mut rec = app.list_recursive("").await.unwrap();
        rec.sort();
        assert_eq!(
            rec,
            vec![
                "sub/".to_string(),
                "sub/y.txt".to_string(),
                "x.txt".to_string(),
            ]
        );
        let est = app.estimate().await.unwrap();
        assert_eq!(est.usage, Some(3));
        assert_eq!(est.quota, None);
        assert!(app.persist().await.unwrap());
        assert!(app.persisted().await.unwrap());
        app.clear().await.unwrap();
        assert!(app.list().await.unwrap().is_empty());
        let est = app.estimate().await.unwrap();
        assert_eq!(est.usage, Some(0));
    }
}
