#[cfg(target_arch = "wasm32")]
pub use crate::web::{DirectoryHandle, FileHandle, WritableFileStream};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native::{DirectoryHandle, FileHandle, WritableFileStream};

pub type Error = <DirectoryHandle as crate::DirectoryHandle>::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// Returns a directory handle for app-specific data storage.
///
/// On native platforms, this returns a subdirectory named after the
/// executable within the platform's application data directory
/// (e.g. `~/.local/share/<app-name>/` on Linux). On web platforms,
/// this returns the per-origin OPFS root (sandboxed, invisible to
/// the OS file manager).
#[cfg(target_arch = "wasm32")]
pub async fn app_specific_dir() -> Result<DirectoryHandle> {
    use wasm_bindgen_futures::JsFuture;
    use web_sys::FileSystemDirectoryHandle;

    let window = web_sys::window().ok_or_else(|| {
        let msg = wasm_bindgen::JsValue::from_str("No window object");
        let err: <DirectoryHandle as crate::DirectoryHandle>::Error = msg;
        err
    })?;
    let navigator = window.navigator();

    let root_directory_handle =
        FileSystemDirectoryHandle::from(JsFuture::from(navigator.storage().get_directory()).await?);

    Ok(DirectoryHandle::from(root_directory_handle))
}

/// Returns a directory handle for app-specific data storage.
///
/// On native platforms, this returns a subdirectory named after the
/// executable within the platform's application data directory
/// (e.g. `~/.local/share/<app-name>/` on Linux). On web platforms,
/// this returns the per-origin OPFS root (sandboxed, invisible to
/// the OS file manager).
#[cfg(not(target_arch = "wasm32"))]
pub async fn app_specific_dir() -> Result<DirectoryHandle> {
    let data_dir = dirs::data_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find user data directory",
        )
    })?;

    let app_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "opfs".to_string());

    let app_dir = data_dir.join(&app_name);

    tokio::fs::create_dir_all(&app_dir).await?;

    Ok(DirectoryHandle::from(app_dir))
}
