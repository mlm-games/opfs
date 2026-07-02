//! "in-memory" filesystem for use in tests or when persistence isn't necessary

use futures::Stream;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// An entry in a virtual directory in the in-memory filesystem.
pub type DirectoryEntry = crate::DirectoryEntry<DirectoryHandle, FileHandle>;

/// A virtual directory in the in-memory filesystem.
#[derive(Debug, Clone)]
pub struct DirectoryHandle(Arc<RwLock<HashMap<String, DirectoryEntry>>>);

/// A virtual file in the in-memory filesystem.
#[derive(Debug, Clone)]
pub struct FileHandle(Arc<RwLock<Vec<u8>>>);

/// A writable file stream in the in-memory filesystem.
///
/// Writes go to a staging buffer. On [`close`](WritableFileStream::close),
/// the staging buffer atomically replaces the target file's data.
#[derive(Debug)]
pub struct WritableFileStream {
    cursor_pos: u64,
    staging: Vec<u8>,
    target: Arc<RwLock<Vec<u8>>>,
    closed: bool,
}

impl crate::private::Sealed for DirectoryHandle {}
impl crate::private::Sealed for FileHandle {}
impl crate::private::Sealed for WritableFileStream {}

impl FileHandle {
    fn new() -> Self {
        Self(Arc::new(RwLock::new(Vec::new())))
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err(format!("'{}' is not a valid name", name));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("'{}' contains path separators", name));
    }
    Ok(())
}

impl crate::DirectoryHandle for DirectoryHandle {
    type Error = String;
    type FileHandleT = FileHandle;

    async fn get_file_handle_with_options(
        &self,
        name: &str,
        options: &crate::GetFileHandleOptions,
    ) -> Result<Self::FileHandleT, Self::Error> {
        validate_name(name)?;
        let mut directory = self.0.write().unwrap();
        let entry = match directory.entry(name.to_string()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if options.create {
                    let file_handle = FileHandle::new();
                    entry.insert(DirectoryEntry::File(file_handle.clone()));
                    DirectoryEntry::File(file_handle)
                } else {
                    return Err(format!("'{name}' does not exist"));
                }
            }
        };

        match entry {
            DirectoryEntry::Directory(_) => Err(format!("'{name}' is a directory")),
            DirectoryEntry::File(file) => Ok(file),
        }
    }

    async fn get_directory_handle_with_options(
        &self,
        name: &str,
        options: &crate::GetDirectoryHandleOptions,
    ) -> Result<Self, Self::Error> {
        validate_name(name)?;
        let mut directory = self.0.write().unwrap();
        let entry = match directory.entry(name.to_string()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if options.create {
                    let dir_handle = DirectoryHandle::default();
                    entry.insert(DirectoryEntry::Directory(dir_handle.clone()));
                    DirectoryEntry::Directory(dir_handle)
                } else {
                    return Err(format!("'{name}' does not exist"));
                }
            }
        };

        match entry {
            DirectoryEntry::File(_) => Err(format!("'{name}' is a file")),
            DirectoryEntry::Directory(dir) => Ok(dir),
        }
    }

    async fn remove_entry(&mut self, name: &str) -> Result<(), Self::Error> {
        validate_name(name)?;
        let mut directory = self.0.write().unwrap();
        if directory.remove(name).is_none() {
            return Err(format!("'{name}' does not exist"));
        }
        Ok(())
    }

    async fn remove_entry_with_options(
        &mut self,
        name: &str,
        options: &crate::FileSystemRemoveOptions,
    ) -> Result<(), Self::Error> {
        validate_name(name)?;
        let mut directory = self.0.write().unwrap();

        if let Some(entry) = directory.get(name) {
            match entry {
                DirectoryEntry::Directory(dir) if !options.recursive => {
                    if !dir.0.read().unwrap().is_empty() {
                        return Err(format!("Directory '{}' is not empty", name));
                    }
                }
                _ => {}
            }
        }

        if directory.remove(name).is_none() {
            return Err(format!("'{name}' does not exist"));
        }
        Ok(())
    }

    async fn entries(
        &self,
    ) -> Result<impl Stream<Item = Result<(String, DirectoryEntry), Self::Error>>, Self::Error>
    {
        let directory = self.0.read().unwrap();
        let entries: Vec<_> = directory
            .iter()
            .map(|(name, entry)| Ok((name.clone(), entry.clone())))
            .collect();
        drop(directory);
        Ok(futures::stream::iter(entries))
    }
}
impl Default for DirectoryHandle {
    fn default() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
}

impl crate::FileHandle for FileHandle {
    type Error = String;
    type WritableFileStreamT = WritableFileStream;

    async fn create_writable_with_options(
        &mut self,
        options: &crate::CreateWritableOptions,
    ) -> Result<Self::WritableFileStreamT, Self::Error> {
        let staging = if options.keep_existing_data {
            self.0.read().unwrap().clone()
        } else {
            Vec::new()
        };
        Ok(WritableFileStream {
            cursor_pos: 0,
            staging,
            target: self.0.clone(),
            closed: false,
        })
    }

    async fn read(&self) -> Result<Vec<u8>, Self::Error> {
        Ok(self.0.read().unwrap().clone())
    }

    async fn read_range<R: std::ops::RangeBounds<u64> + Send>(
        &self,
        range: R,
    ) -> Result<Vec<u8>, Self::Error> {
        use std::ops::Bound;

        let data = self.0.read().unwrap();
        let len = data.len() as u64;

        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => len,
        };

        if start >= len {
            return Ok(Vec::new());
        }

        let actual_end = end.min(len);
        if start > actual_end {
            return Ok(Vec::new());
        }

        Ok(data[start as usize..actual_end as usize].to_vec())
    }

    async fn size(&self) -> Result<u64, Self::Error> {
        Ok(self.0.read().unwrap().len() as u64)
    }
}

impl crate::WritableFileStream for WritableFileStream {
    type Error = String;

    async fn write_at_cursor_pos(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        if self.closed {
            return Err("stream is closed".to_string());
        }
        let data_len = data.len() as u64;
        let needed_len = self.cursor_pos + data_len;
        if needed_len > self.staging.len() as u64 {
            self.staging.resize(needed_len as usize, 0);
        }
        let start = self.cursor_pos as usize;
        self.staging[start..start + data.len()].copy_from_slice(data);
        self.cursor_pos += data_len;
        Ok(())
    }

    async fn write_with_params(&mut self, params: &crate::WriteParams) -> Result<(), Self::Error> {
        if self.closed {
            return Err("stream is closed".to_string());
        }
        use crate::WriteCommandType;

        match params.command_type {
            WriteCommandType::Write => {
                if let Some(data) = &params.data {
                    if let Some(position) = params.position {
                        let data_len = data.len() as u64;
                        let needed_len = position + data_len;
                        if needed_len > self.staging.len() as u64 {
                            self.staging.resize(needed_len as usize, 0);
                        }
                        let start = position as usize;
                        self.staging[start..start + data.len()].copy_from_slice(data);
                        self.cursor_pos = position + data_len;
                    } else {
                        self.write_at_cursor_pos(data).await?;
                    }
                } else {
                    return Err("Write command requires data".to_string());
                }
            }
            WriteCommandType::Seek => {
                if let Some(position) = params.position {
                    self.seek(position).await?;
                } else {
                    return Err("Seek command requires position".to_string());
                }
            }
            WriteCommandType::Truncate => {
                if let Some(size) = params.size {
                    self.truncate(size).await?;
                } else {
                    return Err("Truncate command requires size".to_string());
                }
            }
        }
        Ok(())
    }

    async fn truncate(&mut self, size: u64) -> Result<(), Self::Error> {
        if self.closed {
            return Err("stream is closed".to_string());
        }
        self.staging.resize(size as usize, 0);
        if self.cursor_pos > size {
            self.cursor_pos = size;
        }
        Ok(())
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        if self.closed {
            return Err("stream is closed".to_string());
        }
        self.closed = true;
        let staging = std::mem::take(&mut self.staging);
        *self.target.write().unwrap() = staging;
        Ok(())
    }

    async fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        if self.closed {
            return Err("stream is closed".to_string());
        }
        self.cursor_pos = offset;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CreateWritableOptions, DirectoryHandle as _, FileHandle as _, GetFileHandleOptions,
        WritableFileStream as _,
    };
    use futures::StreamExt;

    #[tokio::test]
    async fn test_create_and_read_file() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        let data = b"Hello, world!";
        writer.write_at_cursor_pos(data).await.unwrap();
        writer.close().await.unwrap();

        let read_data = file.read().await.unwrap();
        assert_eq!(read_data, data);
        assert_eq!(file.size().await.unwrap(), data.len() as u64);
    }

    #[tokio::test]
    async fn test_file_not_found() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: false };

        let result = dir
            .get_file_handle_with_options("nonexistent.txt", &options)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn test_remove_entry() {
        let mut dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let _file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        dir.remove_entry("test.txt").await.unwrap();

        let result = dir
            .get_file_handle_with_options("test.txt", &GetFileHandleOptions { create: false })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_entry_missing() {
        let mut dir = DirectoryHandle::default();
        let result = dir.remove_entry("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn test_entries_empty() {
        let dir = DirectoryHandle::default();
        let entries_stream = dir.entries().await.unwrap();
        let entries: Vec<_> = entries_stream.collect().await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_entries_with_files() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let _file1 = dir
            .get_file_handle_with_options("file1.txt", &options)
            .await
            .unwrap();
        let _file2 = dir
            .get_file_handle_with_options("file2.txt", &options)
            .await
            .unwrap();

        let entries_stream = dir.entries().await.unwrap();
        let entries: Vec<_> = entries_stream.collect().await;

        assert_eq!(entries.len(), 2);

        let mut names: Vec<_> = entries.into_iter().map(|r| r.unwrap().0).collect();
        names.sort();
        assert_eq!(names, vec!["file1.txt", "file2.txt"]);
    }

    #[tokio::test]
    async fn test_seek_and_write() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        writer.write_at_cursor_pos(b"Hello").await.unwrap();
        writer.seek(0).await.unwrap();
        writer.write_at_cursor_pos(b"Hi").await.unwrap();
        writer.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"Hillo");
    }

    #[tokio::test]
    async fn test_seek_beyond_end() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        writer.write_at_cursor_pos(b"Hello").await.unwrap();
        writer.seek(10).await.unwrap();
        writer.write_at_cursor_pos(b"!").await.unwrap();
        writer.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"Hello\0\0\0\0\0!");
        assert_eq!(file.size().await.unwrap(), 11);
    }

    #[tokio::test]
    async fn test_keep_existing_data() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello").await.unwrap();
        writer.close().await.unwrap();

        let keep_options = CreateWritableOptions {
            keep_existing_data: true,
        };
        let mut writer2 = file
            .create_writable_with_options(&keep_options)
            .await
            .unwrap();
        writer2.write_at_cursor_pos(b" World").await.unwrap();
        writer2.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b" World");
    }

    #[tokio::test]
    async fn test_read_range() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello, World!").await.unwrap();
        writer.close().await.unwrap();

        let data = file.read_range(0..5).await.unwrap();
        assert_eq!(data, b"Hello");

        let data = file.read_range(7..).await.unwrap();
        assert_eq!(data, b"World!");

        let data = file.read_range(2..9).await.unwrap();
        assert_eq!(data, b"llo, Wo");

        let data = file.read_range(100..).await.unwrap();
        assert_eq!(data, b"");

        let data = file.read_range(0..=4).await.unwrap();
        assert_eq!(data, b"Hello");

        let data = file.read_range(..).await.unwrap();
        assert_eq!(data, b"Hello, World!");
    }

    #[tokio::test]
    async fn test_truncate() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello, World!").await.unwrap();
        writer.truncate(5).await.unwrap();
        writer.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"Hello");
        assert_eq!(file.size().await.unwrap(), 5);
    }

    #[tokio::test]
    async fn test_write_with_params() {
        use crate::{WriteCommandType, WriteParams};

        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        let params = WriteParams {
            command_type: WriteCommandType::Write,
            data: Some(b"Hello".to_vec()),
            position: None,
            size: None,
        };
        writer.write_with_params(&params).await.unwrap();

        let params = WriteParams {
            command_type: WriteCommandType::Seek,
            data: None,
            position: Some(2),
            size: None,
        };
        writer.write_with_params(&params).await.unwrap();

        let params = WriteParams {
            command_type: WriteCommandType::Write,
            data: Some(b"XXX".to_vec()),
            position: None,
            size: None,
        };
        writer.write_with_params(&params).await.unwrap();

        let params = WriteParams {
            command_type: WriteCommandType::Write,
            data: Some(b"!".to_vec()),
            position: Some(0),
            size: None,
        };
        writer.write_with_params(&params).await.unwrap();

        writer.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"!eXXX");
    }

    #[tokio::test]
    async fn test_write_after_close_errors() {
        let dir = DirectoryHandle::default();
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        writer.write_at_cursor_pos(b"data").await.unwrap();
        writer.close().await.unwrap();

        let result = writer.write_at_cursor_pos(b"more").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("closed"));
    }
}
