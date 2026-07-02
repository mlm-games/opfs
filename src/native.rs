use crate::persistent::Error;
use futures::Stream;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

type DirectoryEntry = crate::DirectoryEntry<DirectoryHandle, FileHandle>;

#[derive(Clone, Debug)]
pub struct DirectoryHandle(PathBuf);

#[derive(Clone, Debug)]
pub struct FileHandle {
    path: PathBuf,
    writer_active: Arc<AtomicBool>,
}

#[derive(Debug)]
pub struct WritableFileStream {
    file: Option<tokio::fs::File>,
    target_path: PathBuf,
    temp: Option<tempfile::NamedTempFile>,
    writer_flag: Option<Arc<AtomicBool>>,
}

#[derive(Debug)]
pub struct SyncAccessHandle(std::fs::File);

impl From<PathBuf> for DirectoryHandle {
    fn from(handle: PathBuf) -> Self {
        Self(handle)
    }
}

impl From<PathBuf> for FileHandle {
    fn from(path: PathBuf) -> Self {
        Self {
            path,
            writer_active: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl crate::private::Sealed for DirectoryHandle {}
impl crate::private::Sealed for FileHandle {}
impl crate::private::Sealed for WritableFileStream {}
impl crate::private::Sealed for SyncAccessHandle {}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::Msg("name must not be empty".into()));
    }
    if name == "." || name == ".." {
        return Err(Error::Msg(format!("'{}' is not a valid name", name)));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(Error::Msg(format!(
            "'{}' contains path separators",
            name
        )));
    }
    Ok(())
}

impl crate::SyncAccessHandle for SyncAccessHandle {
    type Error = Error;

    fn read(&self, buffer: &mut [u8], at: u64) -> Result<usize, Self::Error> {
        use std::io::{Read, Seek};
        let mut file = &self.0;
        file.seek(SeekFrom::Start(at))?;
        let n = file.read(buffer)?;
        Ok(n)
    }

    fn write(&self, data: &[u8], at: u64) -> Result<usize, Self::Error> {
        use std::io::{Seek, Write};
        let mut file = &self.0;
        file.seek(SeekFrom::Start(at))?;
        file.write_all(data)?;
        Ok(data.len())
    }

    fn truncate(&self, size: u64) -> Result<(), Self::Error> {
        self.0.set_len(size)?;
        Ok(())
    }

    fn get_size(&self) -> Result<u64, Self::Error> {
        Ok(self.0.metadata()?.len())
    }

    fn flush(&self) -> Result<(), Self::Error> {
        self.0.sync_all()?;
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
        validate_name(name)?;
        let mut path = self.0.clone();
        path.push(name);

        if options.create {
            tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .await?;
        } else {
            tokio::fs::metadata(&path).await?;
        }

        Ok(FileHandle {
            path,
            writer_active: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn get_directory_handle_with_options(
        &mut self,
        name: &str,
        options: &crate::GetDirectoryHandleOptions,
    ) -> Result<Self, Self::Error> {
        validate_name(name)?;
        let mut path = self.0.clone();
        path.push(name);

        if options.create {
            tokio::fs::create_dir_all(&path).await?;
        } else {
            let metadata = tokio::fs::metadata(&path).await?;
            if !metadata.is_dir() {
                return Err(Error::Msg(format!("'{}' is not a directory", name)));
            }
        }

        Ok(DirectoryHandle(path))
    }

    async fn remove_entry(&mut self, name: &str) -> Result<(), Self::Error> {
        validate_name(name)?;
        let mut path = self.0.clone();
        path.push(name);

        let metadata = tokio::fs::metadata(&path).await?;
        if metadata.is_file() {
            tokio::fs::remove_file(&path).await?;
        } else if metadata.is_dir() {
            tokio::fs::remove_dir(&path).await?;
        }

        Ok(())
    }

    async fn remove_entry_with_options(
        &mut self,
        name: &str,
        options: &crate::FileSystemRemoveOptions,
    ) -> Result<(), Self::Error> {
        validate_name(name)?;
        let mut path = self.0.clone();
        path.push(name);

        let metadata = tokio::fs::metadata(&path).await?;
        if metadata.is_file() {
            tokio::fs::remove_file(&path).await?;
        } else if metadata.is_dir() {
            if options.recursive {
                tokio::fs::remove_dir_all(&path).await?;
            } else {
                tokio::fs::remove_dir(&path).await?;
            }
        }

        Ok(())
    }

    async fn entries(
        &self,
    ) -> Result<impl Stream<Item = Result<(String, DirectoryEntry), Self::Error>>, Self::Error>
    {
        let read_dir = tokio::fs::read_dir(&self.0).await?;
        let stream = futures::stream::unfold(read_dir, |mut read_dir| async {
            loop {
                match read_dir.next_entry().await {
                    Ok(Some(entry)) => {
                        let name = match entry.file_name().into_string() {
                            Ok(n) => n,
                            Err(os_string) => {
                                return Some((
                                    Err(Error::Msg(format!(
                                        "Invalid filename: {:?}",
                                        os_string
                                    ))),
                                    read_dir,
                                ));
                            }
                        };
                        let path = entry.path();
                        let metadata = match entry.metadata().await {
                            Ok(m) => m,
                            Err(e) => return Some((Err(e.into()), read_dir)),
                        };
                        let dir_entry = if metadata.is_file() {
                            DirectoryEntry::File(FileHandle {
                                path: path.clone(),
                                writer_active: Arc::new(AtomicBool::new(false)),
                            })
                        } else if metadata.is_dir() {
                            DirectoryEntry::Directory(DirectoryHandle(path))
                        } else {
                            continue;
                        };
                        return Some((Ok((name, dir_entry)), read_dir));
                    }
                    Ok(None) => return None,
                    Err(e) => return Some((Err(e.into()), read_dir)),
                }
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
        if options.mode == crate::WritableMode::Exclusive
            && self.writer_active.swap(true, Ordering::SeqCst)
        {
            return Err(Error::Msg("File is already open for writing".into()));
        }
        let parent = self.path.parent().unwrap_or(Path::new("."));
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        let mut file = tokio::fs::File::from_std(temp.as_file().try_clone()?);

        if options.keep_existing_data {
            if let Ok(src) = tokio::fs::File::open(&self.path).await {
                let mut src = src;
                tokio::io::copy(&mut src, &mut file).await?;
            }
            file.seek(SeekFrom::Start(0)).await?;
        }

        let flag = if options.mode == crate::WritableMode::Exclusive {
            Some(self.writer_active.clone())
        } else {
            None
        };

        Ok(WritableFileStream {
            file: Some(file),
            target_path: self.path.clone(),
            temp: Some(temp),
            writer_flag: flag,
        })
    }

    async fn read(&self) -> Result<Vec<u8>, Self::Error> {
        use tokio::io::AsyncReadExt;

        let mut file = tokio::fs::File::open(&self.path).await?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await?;
        Ok(buffer)
    }

    async fn read_range<R: std::ops::RangeBounds<u64> + Send>(
        &self,
        range: R,
    ) -> Result<Vec<u8>, Self::Error> {
        use std::ops::Bound;
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let mut file = tokio::fs::File::open(&self.path).await?;
        let file_size = file.metadata().await?.len();

        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };

        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => file_size,
        };

        if start >= file_size {
            return Ok(Vec::new());
        }

        let actual_end = end.min(file_size);
        let bytes_to_read = actual_end.saturating_sub(start);

        if bytes_to_read == 0 {
            return Ok(Vec::new());
        }

        file.seek(SeekFrom::Start(start)).await?;
        let mut buffer = vec![0; bytes_to_read as usize];
        file.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    async fn size(&self) -> Result<u64, Self::Error> {
        let metadata = tokio::fs::metadata(&self.path).await?;
        Ok(metadata.len())
    }

    async fn create_sync_access_handle(&self) -> Result<Self::SyncAccessHandleT, Self::Error> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        Ok(SyncAccessHandle(file))
    }
}

impl crate::WritableFileStream for WritableFileStream {
    type Error = Error;

    async fn write_at_cursor_pos(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        match self.file.as_mut() {
            Some(file) => {
                file.write_all(data).await?;
                Ok(())
            }
            None => Err(Error::Closed),
        }
    }

    async fn write_with_params(&mut self, params: &crate::WriteParams) -> Result<(), Self::Error> {
        use crate::WriteCommandType;

        let file = self.file.as_mut().ok_or(Error::Closed)?;

        match params.command_type {
            WriteCommandType::Write => {
                if let Some(data) = &params.data {
                    if let Some(position) = params.position {
                        file.seek(SeekFrom::Start(position)).await?;
                    }
                    file.write_all(data).await?;
                } else {
                    return Err(Error::Msg("Write command requires data".into()));
                }
            }
            WriteCommandType::Seek => {
                if let Some(position) = params.position {
                    file.seek(SeekFrom::Start(position)).await?;
                } else {
                    return Err(Error::Msg("Seek command requires position".into()));
                }
            }
            WriteCommandType::Truncate => {
                if let Some(size) = params.size {
                    file.set_len(size).await?;
                    let pos = file.seek(SeekFrom::Current(0)).await?;
                    if pos > size {
                        file.seek(SeekFrom::Start(size)).await?;
                    }
                } else {
                    return Err(Error::Msg("Truncate command requires size".into()));
                }
            }
        }
        Ok(())
    }

    async fn truncate(&mut self, size: u64) -> Result<(), Self::Error> {
        match self.file.as_mut() {
            Some(file) => {
                file.set_len(size).await?;
                let pos = file.seek(SeekFrom::Current(0)).await?;
                if pos > size {
                    file.seek(SeekFrom::Start(size)).await?;
                }
                Ok(())
            }
            None => Err(Error::Closed),
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        if let Some(file) = self.file.take() {
            file.sync_all().await?;
            drop(file);
        }
        if let Some(temp) = self.temp.take() {
            temp.persist(&self.target_path).map_err(|e| e.error)?;
        }
        if let Some(flag) = self.writer_flag.take() {
            flag.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    async fn seek(&mut self, offset: u64) -> Result<(), Self::Error> {
        match self.file.as_mut() {
            Some(file) => {
                file.seek(SeekFrom::Start(offset)).await?;
                Ok(())
            }
            None => Err(Error::Closed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CreateWritableOptions, DirectoryHandle as _, FileHandle as _, GetFileHandleOptions,
        SyncAccessHandle as _, WritableFileStream as _, WritableMode,
    };
    use futures::StreamExt;
    use tempfile::TempDir;

    async fn setup_temp_dir() -> (TempDir, DirectoryHandle) {
        let temp_dir = TempDir::new().unwrap();
        let dir_handle = DirectoryHandle(temp_dir.path().to_path_buf());
        (temp_dir, dir_handle)
    }

    #[tokio::test]
    async fn test_create_and_read_file() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
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
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: false };

        let result = dir
            .get_file_handle_with_options("nonexistent.txt", &options)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_remove_entry() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
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
    async fn test_entries_empty() {
        let (_temp_dir, dir) = setup_temp_dir().await;
        let entries_stream = dir.entries().await.unwrap();
        let entries: Vec<_> = entries_stream.collect().await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_entries_with_files() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
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
    async fn test_entries_with_subdirectory() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let _file = dir
            .get_file_handle_with_options("file.txt", &options)
            .await
            .unwrap();

        let mut subdir_path = dir.0.clone();
        subdir_path.push("subdir");
        tokio::fs::create_dir(&subdir_path).await.unwrap();

        let entries_stream = dir.entries().await.unwrap();
        let entries: Vec<_> = entries_stream.collect().await;

        assert_eq!(entries.len(), 2);

        let mut items: Vec<_> = entries
            .into_iter()
            .map(|r| {
                let (name, entry) = r.unwrap();
                let is_dir = matches!(entry, DirectoryEntry::Directory(_));
                (name, is_dir)
            })
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(items[0].0, "file.txt");
        assert!(!items[0].1);
        assert_eq!(items[1].0, "subdir");
        assert!(items[1].1);
    }

    #[tokio::test]
    async fn test_seek_and_write() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
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
    async fn test_keep_existing_data() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello").await.unwrap();
        writer.close().await.unwrap();

        let keep_options = CreateWritableOptions {
            keep_existing_data: true,
            mode: WritableMode::Siloed,
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
    async fn test_truncate_existing_data() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello World").await.unwrap();
        writer.close().await.unwrap();

        let truncate_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer2 = file
            .create_writable_with_options(&truncate_options)
            .await
            .unwrap();
        writer2.write_at_cursor_pos(b"Hi").await.unwrap();
        writer2.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"Hi");
    }

    #[tokio::test]
    async fn test_read_range() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.write_at_cursor_pos(b"Hello, World!").await.unwrap();
        writer.close().await.unwrap();

        assert_eq!(file.read_range(0..5).await.unwrap(), b"Hello");
        assert_eq!(file.read_range(7..).await.unwrap(), b"World!");
        assert_eq!(file.read_range(2..9).await.unwrap(), b"llo, Wo");
        assert_eq!(file.read_range(100..).await.unwrap(), b"");
        assert_eq!(file.read_range(0..=4).await.unwrap(), b"Hello");
        assert_eq!(file.read_range(..).await.unwrap(), b"Hello, World!");
    }

    #[tokio::test]
    async fn test_truncate_and_write_params() {
        use crate::{WriteCommandType, WriteParams};

        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        writer.write_at_cursor_pos(b"Hello, World!").await.unwrap();

        let truncate_params = WriteParams {
            command_type: WriteCommandType::Truncate,
            data: None,
            position: None,
            size: Some(5),
        };
        writer.write_with_params(&truncate_params).await.unwrap();

        writer.close().await.unwrap();

        let data = file.read().await.unwrap();
        assert_eq!(data, b"Hello");
    }

    #[tokio::test]
    async fn test_sync_access_handle() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file_handle = dir
            .get_file_handle_with_options("test.bin", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let mut writer = file_handle
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer
            .write_at_cursor_pos(b"Hello, World!")
            .await
            .unwrap();
        writer.close().await.unwrap();

        let sync_handle = file_handle.create_sync_access_handle().await.unwrap();

        let mut buf = vec![0u8; 5];
        let n = sync_handle.read(&mut buf, 0).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"Hello");

        let n = sync_handle.write(b"12345", 7).unwrap();
        assert_eq!(n, 5);

        sync_handle.flush().unwrap();

        let data = file_handle.read().await.unwrap();
        assert_eq!(&data[0..7], b"Hello, ");
        assert_eq!(&data[7..12], b"12345");
        assert_eq!(&data[12..13], b"!");
    }

    #[tokio::test]
    async fn test_exclusive_writer_rejects_second() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Exclusive,
        };
        let _writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        let result = file.create_writable_with_options(&write_options).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_siloed_writer_allows_second() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        };
        let _writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();

        let result = file.create_writable_with_options(&write_options).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_exclusive_writer_releases_on_close() {
        let (_temp_dir, mut dir) = setup_temp_dir().await;
        let options = GetFileHandleOptions { create: true };

        let mut file = dir
            .get_file_handle_with_options("test.txt", &options)
            .await
            .unwrap();

        let write_options = CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Exclusive,
        };
        let mut writer = file
            .create_writable_with_options(&write_options)
            .await
            .unwrap();
        writer.close().await.unwrap();

        let result = file.create_writable_with_options(&write_options).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_error_matches_portably() {
        let mut dir = DirectoryHandle(PathBuf::from("/nonexistent/path"));
        let result = dir
            .get_file_handle_with_options("test.txt", &GetFileHandleOptions { create: false })
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::Io(_) => {}
            _ => panic!("expected Io error, got: {:?}", err),
        }
    }
}
