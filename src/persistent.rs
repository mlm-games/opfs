#[cfg(target_arch = "wasm32")]
pub use crate::web::{DirectoryHandle, FileHandle, WritableFileStream};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native::{DirectoryHandle, FileHandle, WritableFileStream};

#[cfg(target_arch = "wasm32")]
pub use crate::web::SyncAccessHandle;

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native::SyncAccessHandle;

#[derive(Debug)]
pub enum Error {
    #[cfg(not(target_arch = "wasm32"))]
    Io(std::io::Error),
    #[cfg(target_arch = "wasm32")]
    Js(wasm_bindgen::JsValue),
    Msg(String),
    Closed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Error::Io(e) => write!(f, "I/O error: {}", e),
            #[cfg(target_arch = "wasm32")]
            Error::Js(e) => write!(f, "JavaScript error: {:?}", e),
            Error::Msg(msg) => write!(f, "{}", msg),
            Error::Closed => write!(f, "stream is closed"),
        }
    }
}

impl std::error::Error for Error {
    #[cfg(not(target_arch = "wasm32"))]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::Msg(msg)
    }
}

impl From<&str> for Error {
    fn from(msg: &str) -> Self {
        Error::Msg(msg.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

#[cfg(target_arch = "wasm32")]
impl From<wasm_bindgen::JsValue> for Error {
    fn from(e: wasm_bindgen::JsValue) -> Self {
        Error::Js(e)
    }
}

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
        Error::Msg("No window object".to_string())
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
