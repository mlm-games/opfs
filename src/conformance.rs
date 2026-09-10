//! Cross-backend conformance suite (test-only).
//!
//! Generic cases run against every backend — `memory` and `native` on host,
//! `memory` and `web` (real OPFS in headless Chromium) on wasm — through
//! [`run_suite`]. All cases only assume [`crate::DirectoryHandle`] with the
//! shared [`crate::persistent::Error`], so behavior asserted here is
//! portable by construction.

use crate::persistent::Error;
use crate::{
    CreateWritableOptions, DirectoryHandle, FileHandle, FileSystemRemoveOptions,
    GetDirectoryHandleOptions, GetFileHandleOptions, WritableFileStream, WritableMode,
};
use futures_util::StreamExt;

type Result<T> = crate::persistent::Result<T>;

type EntryItem<D> = Result<(
    String,
    crate::DirectoryEntry<D, <D as DirectoryHandle>::FileHandleT>,
)>;

async fn write_all(
    file: &mut impl FileHandle<Error = Error>,
    data: &[u8],
    keep_existing_data: bool,
) -> Result<()> {
    let mut writer = file
        .create_writable_with_options(&CreateWritableOptions {
            keep_existing_data,
            mode: WritableMode::Siloed,
        })
        .await?;
    writer.write_at_cursor_pos(data).await?;
    writer.close().await
}

async fn write_new<D: DirectoryHandle<Error = Error>>(
    dir: &mut D,
    name: &str,
    data: &[u8],
) -> Result<()> {
    let mut file = dir
        .get_file_handle_with_options(name, &GetFileHandleOptions { create: true })
        .await?;
    let mut writer = file
        .create_writable_with_options(&CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        })
        .await?;
    writer.write_at_cursor_pos(data).await?;
    writer.close().await
}

async fn collect_names<D: DirectoryHandle<Error = Error>>(dir: &D) -> Result<Vec<(String, bool)>> {
    let stream = dir.entries().await?;
    let items: Vec<EntryItem<D>> = stream.collect().await;
    let mut out = Vec::new();
    for item in items {
        let (name, entry) = item?;
        out.push((name, matches!(entry, crate::DirectoryEntry::Directory(_))));
    }
    out.sort();
    Ok(out)
}

pub(crate) async fn nested_create_read_write<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut sub = dir
        .get_directory_handle_with_options("sub", &GetDirectoryHandleOptions { create: true })
        .await?;
    let mut nested = sub
        .get_directory_handle_with_options("nested", &GetDirectoryHandleOptions { create: true })
        .await?;
    write_new(&mut nested, "hello.txt", b"hello").await?;
    let mut nested = sub
        .get_directory_handle_with_options("nested", &GetDirectoryHandleOptions { create: false })
        .await?;
    let file = nested
        .get_file_handle_with_options("hello.txt", &GetFileHandleOptions { create: false })
        .await?;
    assert_eq!(file.read().await?, b"hello");
    assert_eq!(file.size().await?, 5);
    Ok(())
}

pub(crate) async fn keep_existing_data_overwrites<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("data.txt", &GetFileHandleOptions { create: true })
        .await?;
    write_all(&mut file, b"Hello", false).await?;
    write_all(&mut file, b" World", true).await?;
    assert_eq!(file.read().await?, b" World");
    Ok(())
}

pub(crate) async fn siloed_write_replaces<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("data.txt", &GetFileHandleOptions { create: true })
        .await?;
    write_all(&mut file, b"Hello World", false).await?;
    write_all(&mut file, b"Hi", false).await?;
    assert_eq!(file.read().await?, b"Hi");
    Ok(())
}

pub(crate) async fn seek_beyond_eof_zero_fills<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("data.bin", &GetFileHandleOptions { create: true })
        .await?;
    let mut writer = file
        .create_writable_with_options(&CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        })
        .await?;
    writer.write_at_cursor_pos(b"Hello").await?;
    writer.seek(10).await?;
    writer.write_at_cursor_pos(b"!").await?;
    writer.close().await?;
    let data = file.read().await?;
    assert_eq!(data.len(), 11);
    assert_eq!(&data[..5], b"Hello");
    assert!(data[5..10].iter().all(|b| *b == 0));
    assert_eq!(data[10], b'!');
    Ok(())
}

pub(crate) async fn truncate_smaller_and_larger<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("data.bin", &GetFileHandleOptions { create: true })
        .await?;
    let mut writer = file
        .create_writable_with_options(&CreateWritableOptions {
            keep_existing_data: false,
            mode: WritableMode::Siloed,
        })
        .await?;
    writer.write_at_cursor_pos(b"Hello, World!").await?;
    writer.truncate(5).await?;
    writer.close().await?;
    assert_eq!(file.read().await?, b"Hello");

    let mut writer = file
        .create_writable_with_options(&CreateWritableOptions {
            keep_existing_data: true,
            mode: WritableMode::Siloed,
        })
        .await?;
    writer.truncate(8).await?;
    writer.close().await?;
    let data = file.read().await?;
    assert_eq!(data.len(), 8);
    assert_eq!(&data[..5], b"Hello");
    assert!(data[5..].iter().all(|b| *b == 0));
    Ok(())
}

pub(crate) async fn recursive_remove<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut sub = dir
        .get_directory_handle_with_options("sub", &GetDirectoryHandleOptions { create: true })
        .await?;
    write_new(&mut sub, "inner.txt", b"x").await?;
    let err = dir
        .remove_entry_with_options("sub", &FileSystemRemoveOptions { recursive: false })
        .await
        .unwrap_err();
    assert!(!err.is_not_found(), "expected non-empty error, got {err:?}");
    dir.remove_entry_with_options("sub", &FileSystemRemoveOptions { recursive: true })
        .await?;
    let err = dir
        .get_directory_handle_with_options("sub", &GetDirectoryHandleOptions { create: false })
        .await
        .unwrap_err();
    assert!(err.is_not_found(), "expected NotFound, got {err:?}");
    Ok(())
}

pub(crate) async fn list_names_and_types<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    write_new(&mut dir, "b.txt", b"b").await?;
    write_new(&mut dir, "a.txt", b"a").await?;
    dir.get_directory_handle_with_options("sub", &GetDirectoryHandleOptions { create: true })
        .await?;
    let names = collect_names(&dir).await?;
    assert_eq!(
        names,
        vec![
            ("a.txt".to_string(), false),
            ("b.txt".to_string(), false),
            ("sub".to_string(), true),
        ]
    );
    Ok(())
}

pub(crate) async fn invalid_names_rejected<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    for name in ["", ".", "..", "no/slash"] {
        let file_res = dir
            .get_file_handle_with_options(name, &GetFileHandleOptions { create: true })
            .await;
        let dir_res = dir
            .get_directory_handle_with_options(name, &GetDirectoryHandleOptions { create: true })
            .await;
        let (file_err, dir_err) = match (file_res, dir_res) {
            (Err(f), Err(d)) => (f, d),
            (f, d) => panic!("{name:?} must be rejected (file: {f:?}, dir: {d:?})"),
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(
                matches!(file_err, Error::InvalidName(_)),
                "file {name:?}: expected InvalidName, got {file_err:?}"
            );
            assert!(
                matches!(dir_err, Error::InvalidName(_)),
                "dir {name:?}: expected InvalidName, got {dir_err:?}"
            );
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (file_err, dir_err);
        }
    }
    Ok(())
}

pub(crate) async fn missing_entries_are_not_found<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let err = dir
        .get_file_handle_with_options("missing.txt", &GetFileHandleOptions { create: false })
        .await
        .unwrap_err();
    assert!(err.is_not_found(), "expected NotFound, got {err:?}");
    let err = dir.remove_entry("missing.txt").await.unwrap_err();
    assert!(err.is_not_found(), "expected NotFound, got {err:?}");
    Ok(())
}

pub(crate) async fn exclusive_same_handle_rejected<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("ex.txt", &GetFileHandleOptions { create: true })
        .await?;
    let opts = CreateWritableOptions {
        keep_existing_data: false,
        mode: WritableMode::Exclusive,
    };
    let _writer = file.create_writable_with_options(&opts).await?;
    assert!(
        file.create_writable_with_options(&opts).await.is_err(),
        "second exclusive writer must be rejected"
    );
    Ok(())
}

pub(crate) async fn exclusive_drop_releases<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let mut file = dir
        .get_file_handle_with_options("ex.txt", &GetFileHandleOptions { create: true })
        .await?;
    let opts = CreateWritableOptions {
        keep_existing_data: false,
        mode: WritableMode::Exclusive,
    };
    let writer = file.create_writable_with_options(&opts).await?;
    drop(writer);
    assert!(
        file.create_writable_with_options(&opts).await.is_ok(),
        "dropped exclusive writer must release the lock"
    );
    Ok(())
}

pub(crate) async fn binary_roundtrip<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    let data: Vec<u8> = (0u8..=255u8).collect();
    write_new(&mut dir, "blob.bin", &data).await?;
    let file = dir
        .get_file_handle_with_options("blob.bin", &GetFileHandleOptions { create: false })
        .await?;
    let back = file.read().await?;
    assert_eq!(back, data);
    assert_eq!(file.size().await?, 256);
    Ok(())
}

pub(crate) async fn read_range_basics<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    write_new(&mut dir, "range.txt", b"Hello, World!").await?;
    let file = dir
        .get_file_handle_with_options("range.txt", &GetFileHandleOptions { create: false })
        .await?;
    assert_eq!(file.read_range(0..5).await?, b"Hello");
    assert_eq!(file.read_range(7..).await?, b"World!");
    assert_eq!(file.read_range(..).await?, b"Hello, World!");
    assert!(file.read_range(100..).await?.is_empty());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sync_access_handle_roundtrip<D>(mut dir: D) -> Result<()>
where
    D: DirectoryHandle<Error = Error>,
{
    use crate::SyncAccessHandle as _;

    let mut file = dir
        .get_file_handle_with_options("sync.bin", &GetFileHandleOptions { create: true })
        .await?;
    write_all(&mut file, b"Hello, World!", false).await?;
    let sync_handle = file.create_sync_access_handle().await?;
    let mut buf = vec![0u8; 5];
    assert_eq!(sync_handle.read(&mut buf, 0)?, 5);
    assert_eq!(&buf, b"Hello");
    assert_eq!(sync_handle.write(b"12345", 7)?, 5);
    sync_handle.flush()?;
    assert_eq!(sync_handle.get_size()?, 13);
    sync_handle.truncate(5)?;
    assert_eq!(sync_handle.get_size()?, 5);
    let data = file.read().await?;
    assert_eq!(data, b"Hello");
    Ok(())
}

/// Run every portable case against a backend.
///
/// `make` is invoked fresh for each case and returns a guard (kept alive
/// for the case; e.g. a `TempDir`) plus the directory under test.
pub(crate) async fn run_suite<G, D, Make, Fut>(make: Make)
where
    D: DirectoryHandle<Error = Error>,
    Make: Fn() -> Fut,
    Fut: core::future::Future<Output = (G, D)>,
{
    nested_create_read_write(make().await.1)
        .await
        .expect("conformance: nested_create_read_write");
    keep_existing_data_overwrites(make().await.1)
        .await
        .expect("conformance: keep_existing_data_overwrites");
    siloed_write_replaces(make().await.1)
        .await
        .expect("conformance: siloed_write_replaces");
    seek_beyond_eof_zero_fills(make().await.1)
        .await
        .expect("conformance: seek_beyond_eof_zero_fills");
    truncate_smaller_and_larger(make().await.1)
        .await
        .expect("conformance: truncate_smaller_and_larger");
    recursive_remove(make().await.1)
        .await
        .expect("conformance: recursive_remove");
    list_names_and_types(make().await.1)
        .await
        .expect("conformance: list_names_and_types");
    invalid_names_rejected(make().await.1)
        .await
        .expect("conformance: invalid_names_rejected");
    missing_entries_are_not_found(make().await.1)
        .await
        .expect("conformance: missing_entries_are_not_found");
    exclusive_same_handle_rejected(make().await.1)
        .await
        .expect("conformance: exclusive_same_handle_rejected");
    exclusive_drop_releases(make().await.1)
        .await
        .expect("conformance: exclusive_drop_releases");
    binary_roundtrip(make().await.1)
        .await
        .expect("conformance: binary_roundtrip");
    read_range_basics(make().await.1)
        .await
        .expect("conformance: read_range_basics");
    #[cfg(not(target_arch = "wasm32"))]
    sync_access_handle_roundtrip(make().await.1)
        .await
        .expect("conformance: sync_access_handle_roundtrip");
}
