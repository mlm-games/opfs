//! # OPFS - Origin Private File System
//!
//! A Rust implementation of the [Origin Private File System](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system) browser API.
//!
//! ## Usage
//!
//! The main entry point is the [`persistent`] module, which provides platform-agnostic
//! types that automatically resolve to the correct implementation:
//!
//! ```rust
//! use opfs::persistent::{DirectoryHandle, FileHandle, WritableFileStream, app_specific_dir};
//! use opfs::{GetFileHandleOptions, CreateWritableOptions};
//! use opfs::persistent;
//!
//! // you must import the traits to call methods on the types
//! use opfs::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};
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

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

use futures::Stream;
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
    ) -> impl std::future::Future<Output = Result<Self::WritableFileStreamT, Self::Error>> + sealed::MaybeSend;

    fn read(&self) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + sealed::MaybeSend;

    fn read_range<R: RangeBounds<u64> + Send>(
        &self,
        range: R,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + sealed::MaybeSend;

    fn size(&self) -> impl std::future::Future<Output = Result<u64, Self::Error>> + sealed::MaybeSend;

    /// Creates a synchronous access handle for high-performance read/write.
    ///
    /// On native and memory backends this is always available. On web (wasm32),
    /// this requires the `unstable_apis` feature flag:
    /// `RUSTFLAGS='--cfg web_sys_unstable_apis'` or
    /// ```toml
    /// # .cargo/config.toml
    /// [build]
    /// rustflags = ["--cfg", "web_sys_unstable_apis"]
    /// ```
    #[cfg(any(not(target_arch = "wasm32"), web_sys_unstable_apis))]
    fn create_sync_access_handle(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::SyncAccessHandleT, Self::Error>> + sealed::MaybeSend;
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

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;

    fn seek(&mut self, offset: u64)
    -> impl std::future::Future<Output = Result<(), Self::Error>> + sealed::MaybeSend;
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
