use crate::persistent::Error;
use futures::Stream;
use futures::StreamExt;
use js_sys::{ArrayBuffer, AsyncIterator, Uint8Array};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, stream::JsStream};
use web_sys::{
    FileSystemCreateWritableOptions, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetFileOptions, FileSystemRemoveOptions, FileSystemWritableFileStream,
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
pub struct WritableFileStream(FileSystemWritableFileStream);

#[derive(Debug, Clone)]
pub struct File(web_sys::File);

#[derive(Debug)]
pub struct SyncAccessHandle;

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
        Self(handle)
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

    fn read(&self, _buffer: &mut [u8], _at: u64) -> Result<usize, Self::Error> {
        Err(Error::Msg(
            "SyncAccessHandle requires --cfg web_sys_unstable_apis".into(),
        ))
    }

    fn write(&self, _data: &[u8], _at: u64) -> Result<usize, Self::Error> {
        Err(Error::Msg(
            "SyncAccessHandle requires --cfg web_sys_unstable_apis".into(),
        ))
    }

    fn truncate(&self, _size: u64) -> Result<(), Self::Error> {
        Err(Error::Msg(
            "SyncAccessHandle requires --cfg web_sys_unstable_apis".into(),
        ))
    }

    fn get_size(&self) -> Result<u64, Self::Error> {
        Err(Error::Msg(
            "SyncAccessHandle requires --cfg web_sys_unstable_apis".into(),
        ))
    }

    fn flush(&self) -> Result<(), Self::Error> {
        Err(Error::Msg(
            "SyncAccessHandle requires --cfg web_sys_unstable_apis".into(),
        ))
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
        let async_iterator = AsyncIterator::from(entries_iterator);
        let js_stream: JsStream = JsStream::from(async_iterator);

        let stream = js_stream.map(|item| {
            match item {
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
                        DirectoryEntry::Directory(DirectoryHandle(
                            FileSystemDirectoryHandle::from(handle),
                        ))
                    } else {
                        return Err(Error::Msg("Unknown handle type".to_string()));
                    };

                    Ok((filename, entry))
                }
                Err(e) => Err(Error::from(e)),
            }
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
        if options.mode == crate::WritableMode::Exclusive {
            if self.writer_active.swap(true, Ordering::SeqCst) {
                return Err(Error::Msg("File is already open for writing".into()));
            }
        }
        let fs_options = FileSystemCreateWritableOptions::new();
        fs_options.set_keep_existing_data(options.keep_existing_data);
        let file_system_writable_file_stream = FileSystemWritableFileStream::unchecked_from_js(
            JsFuture::from(self.inner.create_writable_with_options(&fs_options)).await?,
        );
        Ok(WritableFileStream(file_system_writable_file_stream))
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
        let _ = JsFuture::from(self.inner.create_sync_access_handle()).await?;
        Err(Error::Msg(
            "SyncAccessHandle: real web implementation pending web-sys bindings".into(),
        ))
    }
}

impl FileHandle {
    pub async fn get_file(&self) -> Result<File, Error> {
        let file: web_sys::File = JsFuture::from(self.inner.get_file()).await?.into();
        Ok(File(file))
    }
}

impl crate::WritableFileStream for WritableFileStream {
    type Error = Error;

    async fn write_at_cursor_pos(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let uint8_array = js_sys::Uint8Array::from(data);
        let array = js_sys::Array::new();
        array.push(&uint8_array);
        let file = web_sys::File::new_with_u8_array_sequence(&array, "filename")?;

        JsFuture::from(self.0.write_with_blob(&file)?).await?;
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

        JsFuture::from(self.0.write_with_write_params(&web_params)?).await?;
        Ok(())
    }

    async fn truncate(&mut self, size: u64) -> Result<(), Self::Error> {
        JsFuture::from(self.0.truncate_with_f64(size as f64)?).await?;
        Ok(())
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        JsFuture::from(self.0.close()).await?;
        Ok(())
    }

    async fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        JsFuture::from(self.0.seek_with_f64(offset as f64)?).await?;
        Ok(())
    }
}

impl File {
    fn size(&self) -> u64 {
        self.0.size() as u64
    }

    async fn read(&self) -> Result<Vec<u8>, Error> {
        let buffer =
            ArrayBuffer::unchecked_from_js(JsFuture::from(self.0.array_buffer()).await?);
        let uint8_array = Uint8Array::new(&buffer);
        let mut vec = vec![0; self.size() as usize];
        uint8_array.copy_to(&mut vec);
        Ok(vec)
    }

    async fn read_range<R: std::ops::RangeBounds<u64>>(
        &self,
        range: R,
    ) -> Result<Vec<u8>, Error> {
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

        let buffer =
            ArrayBuffer::unchecked_from_js(JsFuture::from(blob.array_buffer()).await?);
        let uint8_array = Uint8Array::new(&buffer);
        let mut vec = vec![0; (actual_end - start) as usize];
        uint8_array.copy_to(&mut vec);
        Ok(vec)
    }

    pub(crate) async fn text(&self) -> Result<String, Error> {
        JsFuture::from(self.0.text())
            .await?
            .as_string()
            .ok_or(Error::Msg("Failed to decode text".into()))
    }
}
