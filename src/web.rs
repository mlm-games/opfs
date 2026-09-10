use crate::persistent::Error;
use futures_core::Stream;
use futures_util::StreamExt;
use js_sys::{ArrayBuffer, Uint8Array};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, stream::JsStream};
use web_sys::{
    FileSystemCreateWritableOptions, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetFileOptions, FileSystemReadWriteOptions, FileSystemRemoveOptions,
    FileSystemWritableFileStream,
};

type DirectoryEntry = crate::DirectoryEntry<DirectoryHandle, FileHandle>;

#[derive(Debug, Clone)]
pub struct DirectoryHandle(FileSystemDirectoryHandle);

#[derive(Debug, Clone)]
pub struct FileHandle {
    inner: FileSystemFileHandle,
    writer_active: Arc<AtomicBool>,
}

#[derive(Debug)]
pub struct WritableFileStream {
    inner: FileSystemWritableFileStream,
    writer_flag: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone)]
pub struct File(web_sys::File);

#[derive(Debug)]
pub struct SyncAccessHandle(web_sys::FileSystemSyncAccessHandle);

impl From<FileSystemDirectoryHandle> for DirectoryHandle {
    fn from(handle: FileSystemDirectoryHandle) -> Self {
        Self(handle)
    }
}

impl From<FileSystemFileHandle> for FileHandle {
    fn from(handle: FileSystemFileHandle) -> Self {
        Self {
            inner: handle,
            writer_active: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl From<FileSystemWritableFileStream> for WritableFileStream {
    fn from(handle: FileSystemWritableFileStream) -> Self {
        Self {
            inner: handle,
            writer_flag: None,
        }
    }
}

impl Drop for WritableFileStream {
    /// A stream dropped without `close()` frees the exclusive lock instead
    /// of leaking it.
    fn drop(&mut self) {
        if let Some(flag) = self.writer_flag.take() {
            flag.store(false, Ordering::SeqCst);
        }
    }
}

impl From<web_sys::File> for File {
    fn from(handle: web_sys::File) -> Self {
        Self(handle)
    }
}

impl crate::private::Sealed for DirectoryHandle {}
impl crate::private::Sealed for FileHandle {}
impl crate::private::Sealed for WritableFileStream {}
impl crate::private::Sealed for SyncAccessHandle {}

impl crate::SyncAccessHandle for SyncAccessHandle {
    type Error = Error;

    fn read(&self, buffer: &mut [u8], at: u64) -> Result<usize, Self::Error> {
        let options = FileSystemReadWriteOptions::new();
        options.set_at(at as f64);
        let n = self.0.read_with_u8_array_and_options(buffer, &options)?;
        Ok(n as usize)
    }

    fn write(&self, data: &[u8], at: u64) -> Result<usize, Self::Error> {
        let options = FileSystemReadWriteOptions::new();
        options.set_at(at as f64);
        let n = self.0.write_with_u8_array_and_options(data, &options)?;
        Ok(n as usize)
    }

    fn truncate(&self, size: u64) -> Result<(), Self::Error> {
        self.0.truncate_with_f64(size as f64)?;
        Ok(())
    }

    fn get_size(&self) -> Result<u64, Self::Error> {
        let size = self.0.get_size()?;
        Ok(size as u64)
    }

    fn flush(&self) -> Result<(), Self::Error> {
        self.0.flush()?;
        Ok(())
    }
}

impl crate::DirectoryHandle for DirectoryHandle {
    type Error = Error;
    type FileHandleT = FileHandle;

    async fn get_file_handle_with_options(
        &mut self,
        name: &str,
        options: &crate::GetFileHandleOptions,
    ) -> Result<Self::FileHandleT, Self::Error> {
        let fs_options = FileSystemGetFileOptions::new();
        fs_options.set_create(options.create);
        let file_system_file_handle = FileSystemFileHandle::from(
            JsFuture::from(self.0.get_file_handle_with_options(name, &fs_options)).await?,
        );
        Ok(FileHandle {
            inner: file_system_file_handle,
            writer_active: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn get_directory_handle_with_options(
        &mut self,
        name: &str,
        options: &crate::GetDirectoryHandleOptions,
    ) -> Result<Self, Self::Error> {
        use web_sys::FileSystemGetDirectoryOptions;

        let fs_options = FileSystemGetDirectoryOptions::new();
        fs_options.set_create(options.create);
        let file_system_directory_handle = FileSystemDirectoryHandle::from(
            JsFuture::from(self.0.get_directory_handle_with_options(name, &fs_options)).await?,
        );
        Ok(DirectoryHandle(file_system_directory_handle))
    }

    async fn remove_entry(&mut self, name: &str) -> Result<(), Self::Error> {
        JsFuture::from(self.0.remove_entry(name)).await?;
        Ok(())
    }

    async fn remove_entry_with_options(
        &mut self,
        name: &str,
        options: &crate::FileSystemRemoveOptions,
    ) -> Result<(), Self::Error> {
        let fs_options = FileSystemRemoveOptions::new();
        fs_options.set_recursive(options.recursive);
        JsFuture::from(self.0.remove_entry_with_options(name, &fs_options)).await?;
        Ok(())
    }

    async fn entries(
        &self,
    ) -> Result<impl Stream<Item = Result<(String, DirectoryEntry), Self::Error>>, Self::Error>
    {
        let entries_iterator = self.0.entries();
        let js_stream: JsStream = JsStream::from(entries_iterator);

        let stream = js_stream.map(|item| match item {
            Ok(js_array) => {
                let array = js_sys::Array::from(&js_array);
                let filename = array
                    .get(0)
                    .as_string()
                    .ok_or_else(|| JsValue::from_str("Invalid filename"))?;
                let handle = array.get(1);

                let entry = if handle.has_type::<FileSystemFileHandle>() {
                    DirectoryEntry::File(FileHandle {
                        inner: FileSystemFileHandle::from(handle),
                        writer_active: Arc::new(AtomicBool::new(false)),
                    })
                } else if handle.has_type::<FileSystemDirectoryHandle>() {
                    DirectoryEntry::Directory(DirectoryHandle(FileSystemDirectoryHandle::from(
                        handle,
                    )))
                } else {
                    return Err(Error::Msg("Unknown handle type".to_string()));
                };

                Ok((filename, entry))
            }
            Err(e) => Err(Error::from(e)),
        });

        Ok(stream)
    }
}

impl crate::FileHandle for FileHandle {
    type Error = Error;
    type WritableFileStreamT = WritableFileStream;
    type SyncAccessHandleT = SyncAccessHandle;

    async fn create_writable_with_options(
        &mut self,
        options: &crate::CreateWritableOptions,
    ) -> Result<Self::WritableFileStreamT, Self::Error> {
        if options.mode == crate::WritableMode::Exclusive
            && self.writer_active.swap(true, Ordering::SeqCst)
        {
            return Err(Error::Msg("File is already open for writing".into()));
        }
        let fs_options = FileSystemCreateWritableOptions::new();
        fs_options.set_keep_existing_data(options.keep_existing_data);
        let stream =
            match JsFuture::from(self.inner.create_writable_with_options(&fs_options)).await {
                Ok(js) => FileSystemWritableFileStream::unchecked_from_js(js),
                Err(e) => {
                    if options.mode == crate::WritableMode::Exclusive {
                        self.writer_active.store(false, Ordering::SeqCst);
                    }
                    return Err(Error::from(e));
                }
            };
        let writer_flag = if options.mode == crate::WritableMode::Exclusive {
            Some(self.writer_active.clone())
        } else {
            None
        };
        Ok(WritableFileStream {
            inner: stream,
            writer_flag,
        })
    }

    async fn read(&self) -> Result<Vec<u8>, Self::Error> {
        self.get_file().await?.read().await
    }

    async fn read_range<R: std::ops::RangeBounds<u64> + Send>(
        &self,
        range: R,
    ) -> Result<Vec<u8>, Self::Error> {
        let file = self.get_file().await?;
        file.read_range(range).await
    }

    async fn size(&self) -> Result<u64, Self::Error> {
        let size = self.get_file().await?.size();
        Ok(size)
    }

    #[cfg(web_sys_unstable_apis)]
    async fn create_sync_access_handle(&self) -> Result<Self::SyncAccessHandleT, Self::Error> {
        let handle = JsFuture::from(self.inner.create_sync_access_handle()).await?;
        let handle =
            wasm_bindgen::JsCast::unchecked_into::<web_sys::FileSystemSyncAccessHandle>(handle);
        Ok(SyncAccessHandle(handle))
    }
}

impl FileHandle {
    pub async fn get_file(&self) -> Result<File, Error> {
        let file: web_sys::File = JsFuture::from(self.inner.get_file()).await?.into();
        Ok(File(file))
    }
}

fn storage_manager() -> Result<web_sys::StorageManager, Error> {
    let window = web_sys::window().ok_or_else(|| Error::Msg("No window object".to_string()))?;
    Ok(window.navigator().storage())
}

/// Origin storage quota/usage via `navigator.storage.estimate()`.
///
/// Backs [`crate::AppFs::estimate`] on web.
pub async fn storage_estimate() -> Result<crate::StorageEstimate, Error> {
    let promise = storage_manager()?.estimate().map_err(Error::from)?;
    let value = JsFuture::from(promise).await?;
    let estimate = web_sys::StorageEstimate::unchecked_from_js(value);
    Ok(crate::StorageEstimate {
        quota: estimate.get_quota().map(|q| q as u64),
        usage: estimate.get_usage().map(|u| u as u64),
    })
}

/// Request persistent storage via `navigator.storage.persist()`.
///
/// Backs [`crate::AppFs::persist`] on web. Note: without user activation
/// (e.g. headless/automated Firefox) this promise may never settle; call it
/// from a user gesture in production code.
pub async fn storage_persist() -> Result<bool, Error> {
    let promise = storage_manager()?.persist().map_err(Error::from)?;
    let value = JsFuture::from(promise).await?;
    Ok(value.as_bool().unwrap_or(false))
}

/// Query persistent-storage status via `navigator.storage.persisted()`.
///
/// Backs [`crate::AppFs::persisted`] on web.
pub async fn storage_persisted() -> Result<bool, Error> {
    let promise = storage_manager()?.persisted().map_err(Error::from)?;
    let value = JsFuture::from(promise).await?;
    Ok(value.as_bool().unwrap_or(false))
}

impl crate::WritableFileStream for WritableFileStream {
    type Error = Error;

    async fn write_at_cursor_pos(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let uint8_array = js_sys::Uint8Array::from(data);
        let array = js_sys::Array::new();
        array.push(&uint8_array);
        let file = web_sys::File::new_with_u8_array_sequence(&array, "filename")?;

        JsFuture::from(self.inner.write_with_blob(&file)?).await?;
        Ok(())
    }

    async fn write_with_params(&mut self, params: &crate::WriteParams) -> Result<(), Self::Error> {
        use crate::WriteCommandType;
        use web_sys::{WriteCommandType as WebWriteCommandType, WriteParams as WebWriteParams};

        let web_params = WebWriteParams::new(match params.command_type {
            WriteCommandType::Write => WebWriteCommandType::Write,
            WriteCommandType::Seek => WebWriteCommandType::Seek,
            WriteCommandType::Truncate => WebWriteCommandType::Truncate,
        });

        if let Some(data) = &params.data {
            let uint8_array = js_sys::Uint8Array::from(data.as_slice());
            let array = js_sys::Array::new();
            array.push(&uint8_array);
            let file = web_sys::File::new_with_u8_array_sequence(&array, "filename")?;
            web_params.set_data(&file.into());
        }

        if let Some(position) = params.position {
            web_params.set_position(Some(position as f64));
        }

        if let Some(size) = params.size {
            web_params.set_size(Some(size as f64));
        }

        JsFuture::from(self.inner.write_with_write_params(&web_params)?).await?;
        Ok(())
    }

    async fn truncate(&mut self, size: u64) -> Result<(), Self::Error> {
        JsFuture::from(self.inner.truncate_with_f64(size as f64)?).await?;
        Ok(())
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        JsFuture::from(self.inner.close()).await?;
        if let Some(flag) = self.writer_flag.take() {
            flag.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    async fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        JsFuture::from(self.inner.seek_with_f64(offset as f64)?).await?;
        Ok(())
    }
}

impl File {
    fn size(&self) -> u64 {
        self.0.size() as u64
    }

    async fn read(&self) -> Result<Vec<u8>, Error> {
        let buffer = ArrayBuffer::unchecked_from_js(JsFuture::from(self.0.array_buffer()).await?);
        let uint8_array = Uint8Array::new(&buffer);
        let mut vec = vec![0; self.size() as usize];
        uint8_array.copy_to(&mut vec);
        Ok(vec)
    }

    async fn read_range<R: std::ops::RangeBounds<u64>>(&self, range: R) -> Result<Vec<u8>, Error> {
        use std::ops::Bound;
        use web_sys::Blob;

        let size = self.size();

        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => size,
        };

        if start >= size {
            return Ok(Vec::new());
        }

        let actual_end = end.min(size);
        if start >= actual_end {
            return Ok(Vec::new());
        }

        let blob: Blob = self
            .0
            .slice_with_f64_and_f64(start as f64, actual_end as f64)?;

        let buffer = ArrayBuffer::unchecked_from_js(JsFuture::from(blob.array_buffer()).await?);
        let uint8_array = Uint8Array::new(&buffer);
        let mut vec = vec![0; (actual_end - start) as usize];
        uint8_array.copy_to(&mut vec);
        Ok(vec)
    }

    #[allow(dead_code)]
    pub(crate) async fn text(&self) -> Result<String, Error> {
        JsFuture::from(self.0.text())
            .await?
            .as_string()
            .ok_or(Error::Msg("Failed to decode text".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::DirectoryHandle;
    use crate::{DirectoryHandle as _, FileSystemRemoveOptions, GetDirectoryHandleOptions};
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    static SUITE_NEXT: AtomicU64 = AtomicU64::new(0);

    /// Fresh isolated OPFS subdirectory per suite run, with stale suites
    /// from previous runs cleaned up best-effort.
    async fn setup_conformance() -> ((), DirectoryHandle) {
        let n = SUITE_NEXT.fetch_add(1, Ordering::SeqCst);
        let mut root = crate::persistent::app_specific_dir().await.unwrap();
        let stale: Vec<String> = match root.entries().await {
            Ok(stream) => stream
                .filter_map(|r| async { r.ok().map(|(name, _)| name) })
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .filter(|name| name.starts_with("ropfs-conf-"))
                .collect(),
            Err(_) => Vec::new(),
        };
        for name in stale {
            let _ = root
                .remove_entry_with_options(&name, &FileSystemRemoveOptions { recursive: true })
                .await;
        }
        let dir = root
            .get_directory_handle_with_options(
                &format!("ropfs-conf-{n}"),
                &GetDirectoryHandleOptions { create: true },
            )
            .await
            .unwrap();
        ((), dir)
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn conformance_suite() {
        crate::conformance::run_suite(setup_conformance).await;
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn storage_management() {
        let estimate = crate::persistent::storage_estimate().await.unwrap();
        assert!(estimate.usage.is_some());
        let _ = crate::persistent::storage_persisted().await.unwrap();
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    #[ignore = "navigator.storage.persist() never settles in headless Firefox (no user activation); run manually in Chrome"]
    async fn storage_persist_manual() {
        let _ = crate::persistent::storage_persist().await.unwrap();
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn backslash_name_allowed() {
        let (_guard, mut dir) = setup_conformance().await;
        dir.get_file_handle_with_options(
            "back\\slash",
            &crate::GetFileHandleOptions { create: true },
        )
        .await
        .unwrap();
    }
}
