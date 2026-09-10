#[cfg(target_arch = "wasm32")]
pub use crate::web::{DirectoryHandle, FileHandle, WritableFileStream};

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native::{DirectoryHandle, FileHandle, WritableFileStream};

#[cfg(target_arch = "wasm32")]
pub use crate::web::SyncAccessHandle;

#[cfg(not(target_arch = "wasm32"))]
pub use crate::native::SyncAccessHandle;

#[cfg(target_arch = "wasm32")]
pub use crate::web::{storage_estimate, storage_persist, storage_persisted};

#[derive(Debug)]
pub enum Error {
    /// A file or directory in the path does not exist.
    NotFound(String),
    /// A file or directory already exists where creation was requested.
    AlreadyExists(String),
    /// A handle of the wrong type (file vs directory) was requested.
    TypeMismatch(String),
    /// An entry name is empty, `.`/`..`, or contains a path separator.
    InvalidName(String),
    Io(std::io::Error),
    #[cfg(target_arch = "wasm32")]
    Js(wasm_bindgen::JsValue),
    Msg(String),
    Closed,
}

impl Error {
    /// Returns `true` for missing-file/directory errors, so callers can
    /// distinguish "does not exist" from real failures (permission, quota,
    /// type mismatch, internal errors, ...).
    pub fn is_not_found(&self) -> bool {
        match self {
            Error::NotFound(_) => true,
            Error::Io(e) => e.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotFound(name) => write!(f, "'{name}' does not exist"),
            Error::AlreadyExists(name) => write!(f, "'{name}' already exists"),
            Error::TypeMismatch(msg) => write!(f, "{msg}"),
            Error::InvalidName(msg) => write!(f, "{msg}"),
            Error::Io(e) => write!(f, "I/O error: {}", e),
            #[cfg(target_arch = "wasm32")]
            Error::Js(e) => write!(f, "JavaScript error: {:?}", e),
            Error::Msg(msg) => write!(f, "{}", msg),
            Error::Closed => write!(f, "stream is closed"),
        }
    }
}

impl std::error::Error for Error {
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

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound(e.to_string())
        } else {
            Error::Io(e)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<wasm_bindgen::JsValue> for Error {
    fn from(e: wasm_bindgen::JsValue) -> Self {
        if let Some(name) = js_sys::Reflect::get(&e, &wasm_bindgen::JsValue::from_str("name"))
            .ok()
            .and_then(|v| v.as_string())
        {
            let message = js_sys::Reflect::get(&e, &wasm_bindgen::JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| name.clone());
            if name == "NotFoundError" {
                return Error::NotFound(message);
            }
            if name == "TypeMismatchError" {
                return Error::TypeMismatch(message);
            }
        }
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

    let window = web_sys::window().ok_or_else(|| Error::Msg("No window object".to_string()))?;
    let navigator = window.navigator();

    let root_directory_handle =
        FileSystemDirectoryHandle::from(JsFuture::from(navigator.storage().get_directory()).await?);

    Ok(DirectoryHandle::from(root_directory_handle))
}

/// Identifies an application for [`app_specific_dir_for`].
///
/// The triple follows the [`directories::ProjectDirs`] convention
/// (`qualifier`, `organization`, `application`), e.g.
/// `AppInfo { qualifier: "com", organization: "Example", application: "MyApp" }`
/// resolves to `~/.local/share/MyApp/` on Linux (when the organization does
/// not affect the Linux layout), `~/Library/Application Support/com.example.MyApp`
/// on macOS, and the roaming app-data dir on Windows.
#[derive(Debug, Clone, Copy)]
pub struct AppInfo {
    pub qualifier: &'static str,
    pub organization: &'static str,
    pub application: &'static str,
}

/// Legacy native location: `<data-dir>/<current-exe-stem>/`.
///
/// Kept as the behavior of [`app_specific_dir`] and as the migration source
/// for [`app_specific_dir_for`].
#[cfg(not(target_arch = "wasm32"))]
fn legacy_app_dir() -> std::path::PathBuf {
    let data_dir = dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("."));

    let app_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "ropfs".to_string());

    data_dir.join(&app_name)
}

/// Returns a directory handle for app-specific data storage.
///
/// On native platforms, this returns a subdirectory named after the
/// executable within the platform's application data directory
/// (e.g. `~/.local/share/<app-name>/` on Linux). This is the **legacy**
/// location: it is stable, but the executable-stem naming is fragile
/// (renaming the binary orphans the data). New applications should prefer
/// [`app_specific_dir_for`], which uses OS-conventional project paths and
/// migrates legacy data forward. On web platforms, this returns the
/// per-origin OPFS root (sandboxed, invisible to the OS file manager).
#[cfg(not(target_arch = "wasm32"))]
pub async fn app_specific_dir() -> Result<DirectoryHandle> {
    if dirs::data_dir().is_none() {
        return Err(Error::NotFound(
            "Could not find user data directory".to_string(),
        ));
    }
    let app_dir = legacy_app_dir();

    tokio::fs::create_dir_all(&app_dir).await?;

    Ok(DirectoryHandle::from(app_dir))
}

/// Returns a directory handle for app-specific data storage at an
/// OS-conventional project path.
///
/// On native platforms this resolves via [`directories::ProjectDirs`] and,
/// on first use, migrates any data found at the legacy [`app_specific_dir`]
/// location (copied recursively; the legacy directory is left in place so
/// the migration is non-destructive — delete it once you have verified the
/// move). Migration is skipped when the project directory already exists
/// and is non-empty, so it never overwrites newer data. When no project
/// directory can be determined, it falls back to the legacy location.
///
/// On web platforms the origin has a single OPFS root, so `info` is ignored
/// and this behaves exactly like [`app_specific_dir`].
#[cfg(not(target_arch = "wasm32"))]
pub async fn app_specific_dir_for(info: &AppInfo) -> Result<DirectoryHandle> {
    let app_dir =
        directories::ProjectDirs::from(info.qualifier, info.organization, info.application)
            .map(|project| project.data_dir().to_path_buf())
            .unwrap_or_else(legacy_app_dir);

    let legacy = legacy_app_dir();
    if app_dir != legacy {
        migrate_legacy_dir(&legacy, &app_dir).await?;
    }

    tokio::fs::create_dir_all(&app_dir).await?;

    Ok(DirectoryHandle::from(app_dir))
}

/// One-time, non-destructive migration from the legacy app directory.
///
/// Copies recursively when the destination does not exist or is empty and
/// the legacy directory exists and is non-empty. The legacy directory is
/// intentionally left behind.
#[cfg(not(target_arch = "wasm32"))]
async fn migrate_legacy_dir(legacy: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    if !legacy.is_dir() {
        return Ok(());
    }
    if dest.is_dir() && !is_empty_dir(dest).await? {
        return Ok(());
    }
    if is_empty_dir(legacy).await? {
        return Ok(());
    }

    async fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
        tokio::fs::create_dir_all(dst).await?;
        let mut entries = tokio::fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let target = dst.join(entry.file_name());
            if file_type.is_dir() {
                Box::pin(copy_tree(&entry.path(), &target)).await?;
            } else if file_type.is_file() {
                tokio::fs::copy(entry.path(), &target).await?;
            }
        }
        Ok(())
    }

    copy_tree(legacy, dest).await
}

#[cfg(not(target_arch = "wasm32"))]
async fn is_empty_dir(path: &std::path::Path) -> Result<bool> {
    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => Ok(entries.next_entry().await?.is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e.into()),
    }
}

/// Returns a directory handle for app-specific data storage at an
/// OS-conventional project path.
///
/// On web platforms the origin has a single OPFS root, so `info` is ignored
/// and this behaves exactly like [`app_specific_dir`]. See the native
/// documentation for the migration behavior on other platforms.
#[cfg(target_arch = "wasm32")]
pub async fn app_specific_dir_for(_info: &AppInfo) -> Result<DirectoryHandle> {
    app_specific_dir().await
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrates_legacy_tree_once() {
        let legacy = tempfile::TempDir::new().unwrap();
        let dest_root = tempfile::TempDir::new().unwrap();
        tokio::fs::create_dir_all(legacy.path().join("sub"))
            .await
            .unwrap();
        tokio::fs::write(legacy.path().join("sub").join("f.txt"), b"v")
            .await
            .unwrap();

        let dest = dest_root.path().join("app");
        migrate_legacy_dir(legacy.path(), &dest).await.unwrap();
        assert_eq!(
            tokio::fs::read(dest.join("sub").join("f.txt"))
                .await
                .unwrap(),
            b"v"
        );
        assert!(legacy.path().join("sub").join("f.txt").exists());
        tokio::fs::write(legacy.path().join("new.txt"), b"new")
            .await
            .unwrap();
        migrate_legacy_dir(legacy.path(), &dest).await.unwrap();
        assert!(!dest.join("new.txt").exists());
    }

    #[tokio::test]
    async fn skips_missing_or_empty_legacy() {
        let root = tempfile::TempDir::new().unwrap();
        migrate_legacy_dir(&root.path().join("nope"), &root.path().join("d"))
            .await
            .unwrap();
        assert!(!root.path().join("d").exists());

        let empty = tempfile::TempDir::new().unwrap();
        migrate_legacy_dir(empty.path(), &root.path().join("d2"))
            .await
            .unwrap();
        assert!(!root.path().join("d2").exists());
    }

    #[tokio::test]
    async fn app_specific_dir_for_is_usable() {
        let info = AppInfo {
            qualifier: "com",
            organization: "ropfs-tests",
            application: "ropfs-migration-test",
        };
        let mut app = crate::AppFs::new_for(&info).await.unwrap();
        app.write("probe.txt", b"1").await.unwrap();
        assert_eq!(app.read("probe.txt").await.unwrap(), b"1");

        let expected =
            directories::ProjectDirs::from(info.qualifier, info.organization, info.application)
                .map(|p| p.data_dir().to_path_buf())
                .unwrap_or_else(legacy_app_dir);
        if expected.to_string_lossy().contains("ropfs-migration-test") {
            let _ = tokio::fs::remove_dir_all(&expected).await;
        }
    }
}
