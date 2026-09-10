//! Synchronous filesystem mirroring `std::fs`.
//!
//! **Backend note:** on native targets this delegates to the real filesystem.
//! On `wasm32` this is a `localStorage`-backed shim — it is **not** OPFS
//! (no `FileSystemSyncAccessHandle`, ~5 MB quota, string-only storage with
//! base64-encoded values). Do not mistake it for durable/private
//! origin storage; prefer the async [`crate::web`] OPFS backend on the web.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use web_time::{SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
use base64::Engine as _;

#[derive(Debug, Clone, Default)]
struct MemoryInner {
    files: HashMap<PathBuf, Vec<u8>>,
    dirs: HashSet<PathBuf>,
    mtimes: HashMap<PathBuf, u64>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Synchronous storage that mirrors `std::fs` on native and `localStorage` on wasm.
///
/// **Wasm caveat:** the browser backend persists through `localStorage`
/// (base64-encoded), not OPFS. It is suitable for small settings-like blobs,
/// not for large or strictly durable data.
/// Clone shares the same underlying map (like `FsStorage` sharing the real FS).
#[derive(Debug, Clone, Default)]
pub struct Fs {
    inner: Arc<RwLock<MemoryInner>>,
}

impl Fs {
    pub fn new() -> Self {
        let fs = Self::default();
        #[cfg(target_arch = "wasm32")]
        fs.hydrate_from_local_storage();
        fs
    }

    fn normalize(p: &Path) -> PathBuf {
        p.to_path_buf()
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_prefix() -> &'static str {
        "ropfs:"
    }

    #[cfg(target_arch = "wasm32")]
    fn to_ls_key(path: &Path) -> String {
        let s = path.to_string_lossy().replace('\\', "/");
        let s = s
            .trim_start_matches("./")
            .trim_start_matches('/')
            .to_string();
        format!("{}{}", Self::ls_prefix(), s)
    }

    #[cfg(target_arch = "wasm32")]
    fn local_storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    #[cfg(target_arch = "wasm32")]
    fn hydrate_from_local_storage(&self) {
        let Some(ls) = Self::local_storage() else {
            return;
        };
        let len = ls.length().ok().unwrap_or(0);
        for i in 0..len {
            let Ok(Some(key)) = ls.key(i) else {
                continue;
            };
            if !key.starts_with(Self::ls_prefix()) {
                continue;
            }
            if key.ends_with("/__dir__") {
                let dir_path =
                    PathBuf::from(&key[Self::ls_prefix().len()..key.len() - "/__dir__".len()]);
                let _ = self.create_dir_all(&dir_path);
                continue;
            }
            if let Ok(Some(v)) = ls.get_item(&key) {
                let path = PathBuf::from(&key[Self::ls_prefix().len()..]);
                let _ = self.write(&path, &Self::ls_decode(&v));
            }
        }
    }

    /// Decode a `localStorage` value. Current values are base64; very old
    /// entries written as raw lossy-UTF8 fall back to their raw bytes so a
    /// stale cache never becomes unreadable.
    #[cfg(target_arch = "wasm32")]
    fn ls_decode(s: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .unwrap_or_else(|_| s.as_bytes().to_vec())
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_set(path: &Path, data: &[u8]) {
        if let Some(ls) = Self::local_storage() {
            let key = Self::to_ls_key(path);
            let s = base64::engine::general_purpose::STANDARD.encode(data);
            let _ = ls.set_item(&key, &s);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_remove(path: &Path) {
        if let Some(ls) = Self::local_storage() {
            let key = Self::to_ls_key(path);
            let _ = ls.remove_item(&key);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_remove_prefix(prefix: &Path) {
        if let Some(ls) = Self::local_storage() {
            let prefix_key = Self::to_ls_key(prefix);
            let dir_prefix = format!("{}/__dir__", prefix_key);
            let file_prefix = format!("{}/", prefix_key);
            let mut to_remove = Vec::new();
            let len = ls.length().ok().unwrap_or(0);
            for i in 0..len {
                if let Ok(Some(k)) = ls.key(i) {
                    if k == prefix_key
                        || k.starts_with(&file_prefix)
                        || k == dir_prefix
                        || k.starts_with(&format!("{}/", dir_prefix))
                    {
                        to_remove.push(k);
                    }
                }
            }
            for k in to_remove {
                let _ = ls.remove_item(&k);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_exists(path: &Path) -> bool {
        let Some(ls) = Self::local_storage() else {
            return false;
        };
        let key = Self::to_ls_key(path);
        if ls.get_item(&key).ok().flatten().is_some() {
            return true;
        }
        let dir_key = format!("{}/__dir__", key);
        if ls.get_item(&dir_key).ok().flatten().is_some() {
            return true;
        }
        let prefix = format!("{}/", key);
        let len = ls.length().ok().unwrap_or(0);
        for i in 0..len {
            if let Ok(Some(k)) = ls.key(i) {
                if k.starts_with(&prefix) {
                    return true;
                }
            }
        }
        false
    }

    #[cfg(target_arch = "wasm32")]
    fn ls_is_dir(path: &Path) -> bool {
        let Some(ls) = Self::local_storage() else {
            return false;
        };
        let key = Self::to_ls_key(path);
        let dir_key = format!("{}/__dir__", key);
        if ls.get_item(&dir_key).ok().flatten().is_some() {
            return true;
        }
        let prefix = format!("{}/", key);
        let len = ls.length().ok().unwrap_or(0);
        for i in 0..len {
            if let Ok(Some(k)) = ls.key(i) {
                if k.starts_with(&prefix) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Fs {
    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        // Also keep memory for tests, but delegate to real fs
        {
            let mut g = self.inner.write().unwrap();
            let mut cur = PathBuf::new();
            for comp in path.components() {
                cur.push(comp);
                g.dirs.insert(cur.clone());
            }
            g.dirs.insert(path.to_path_buf());
        }
        std::fs::create_dir_all(path)
    }
    pub fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(b) => {
                // mirror to memory for read_dir etc
                let mut g = self.inner.write().unwrap();
                g.files.insert(Self::normalize(path), b.clone());
                g.mtimes.insert(Self::normalize(path), now_secs());
                Ok(Some(b))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, data)?;
        let mut g = self.inner.write().unwrap();
        g.files.insert(Self::normalize(path), data.to_vec());
        g.mtimes.insert(Self::normalize(path), now_secs());
        if let Some(parent) = path.parent() {
            g.dirs.insert(parent.to_path_buf());
        }
        Ok(())
    }
    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)?;
        // mirror memory
        let mut g = self.inner.write().unwrap();
        let from_n = Self::normalize(from);
        let to_n = Self::normalize(to);
        if let Some(data) = g.files.remove(&from_n) {
            let mtime = g.mtimes.remove(&from_n).unwrap_or(now_secs());
            g.files.insert(to_n.clone(), data);
            g.mtimes.insert(to_n.clone(), mtime);
        }
        Ok(())
    }
    pub fn copy(&self, from: &Path, to: &Path) -> io::Result<u64> {
        let n = std::fs::copy(from, to)?;
        if let Ok(data) = std::fs::read(to) {
            let mut g = self.inner.write().unwrap();
            g.files.insert(Self::normalize(to), data.clone());
            g.mtimes.insert(Self::normalize(to), now_secs());
        }
        Ok(n)
    }
    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)?;
        let mut g = self.inner.write().unwrap();
        g.files.remove(&Self::normalize(path));
        g.mtimes.remove(&Self::normalize(path));
        Ok(())
    }
    pub fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)?;
        let mut g = self.inner.write().unwrap();
        let p = Self::normalize(path);
        g.files.retain(|k, _| !k.starts_with(&p));
        g.mtimes.retain(|k, _| !k.starts_with(&p));
        g.dirs.retain(|d| !d.starts_with(&p) && d != &p);
        Ok(())
    }
    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    pub fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
    pub fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    pub fn metadata_len(&self, path: &Path) -> Option<u64> {
        std::fs::metadata(path).ok().map(|m| m.len())
    }
    pub fn mtime_secs(&self, path: &Path) -> Option<u64> {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    }
    pub fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(path)? {
            out.push(e?.path());
        }
        Ok(out)
    }
    pub fn sync_file(&self, path: &Path) {
        if let Ok(f) = std::fs::File::open(path) {
            let _ = f.sync_all();
        }
    }
    pub fn sync_dir(&self, path: &Path) {
        if let Ok(f) = std::fs::File::open(path) {
            let _ = f.sync_all();
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Fs {
    pub fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        {
            let mut g = self.inner.write().unwrap();
            let mut cur = PathBuf::new();
            for comp in path.components() {
                cur.push(comp);
                g.dirs.insert(cur.clone());
            }
            g.dirs.insert(path.to_path_buf());
        }
        if let Some(ls) = Self::local_storage() {
            let key = format!("{}/__dir__", Self::to_ls_key(path));
            let _ = ls.set_item(&key, "1");
        }
        Ok(())
    }
    pub fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        {
            let g = self.inner.read().unwrap();
            if let Some(v) = g.files.get(&Self::normalize(path)) {
                return Ok(Some(v.clone()));
            }
        }
        if let Some(ls) = Self::local_storage() {
            let key = Self::to_ls_key(path);
            if let Ok(Some(s)) = ls.get_item(&key) {
                let b = Self::ls_decode(&s);
                let mut g = self.inner.write().unwrap();
                g.files.insert(Self::normalize(path), b.clone());
                g.mtimes.insert(Self::normalize(path), now_secs());
                return Ok(Some(b));
            }
        }
        Ok(None)
    }
    pub fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        {
            let mut g = self.inner.write().unwrap();
            if let Some(parent) = path.parent() {
                drop(g);
                self.create_dir_all(parent)?;
                g = self.inner.write().unwrap();
            }
            g.files.insert(Self::normalize(path), data.to_vec());
            g.mtimes.insert(Self::normalize(path), now_secs());
            if let Some(parent) = path.parent() {
                g.dirs.insert(parent.to_path_buf());
            }
        }
        Self::ls_set(path, data);
        Ok(())
    }
    pub fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        {
            let mut g = self.inner.write().unwrap();
            let from_n = Self::normalize(from);
            let to_n = Self::normalize(to);
            if let Some(data) = g.files.remove(&from_n) {
                let mtime = g.mtimes.remove(&from_n).unwrap_or(now_secs());
                g.files.insert(to_n.clone(), data);
                g.mtimes.insert(to_n.clone(), mtime);
                g.dirs.insert(to_n.clone());
                if let Some(parent) = to_n.parent() {
                    g.dirs.insert(parent.to_path_buf());
                }
            } else if !g.dirs.contains(&from_n) && !g.files.keys().any(|k| k.starts_with(&from_n)) {
                // try hydrate from ls if not in memory
                if Self::ls_exists(from) {
                    if let Some(ls) = Self::local_storage() {
                        let key = Self::to_ls_key(from);
                        if let Ok(Some(s)) = ls.get_item(&key) {
                            g.files.insert(to_n.clone(), s.into_bytes());
                            g.mtimes.insert(to_n.clone(), now_secs());
                        }
                    }
                } else {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
                }
            } else {
                // dir rename: move prefixed files
                let mut to_move = Vec::new();
                for k in g.files.keys().cloned().collect::<Vec<_>>() {
                    if k.starts_with(&from_n) {
                        to_move.push(k);
                    }
                }
                for k in to_move {
                    if let Some(v) = g.files.remove(&k) {
                        let rel = k.strip_prefix(&from_n).unwrap();
                        let new_k = if rel.as_os_str().is_empty() {
                            to_n.clone()
                        } else {
                            to_n.join(rel)
                        };
                        let mt = g.mtimes.remove(&k).unwrap_or(now_secs());
                        g.files.insert(new_k.clone(), v);
                        g.mtimes.insert(new_k, mt);
                    }
                }
                let mut dirs_to_move = Vec::new();
                for d in g.dirs.iter().cloned().collect::<Vec<_>>() {
                    if d.starts_with(&from_n) {
                        dirs_to_move.push(d);
                    }
                }
                for d in dirs_to_move {
                    let rel = d.strip_prefix(&from_n).unwrap();
                    let new_d = if rel.as_os_str().is_empty() {
                        to_n.clone()
                    } else {
                        to_n.join(rel)
                    };
                    g.dirs.remove(&d);
                    g.dirs.insert(new_d.clone());
                    if let Some(mt) = g.mtimes.remove(&d) {
                        g.mtimes.insert(new_d, mt);
                    }
                }
                g.dirs.remove(&from_n);
                g.dirs.insert(to_n.clone());
                if let Some(parent) = to_n.parent() {
                    g.dirs.insert(parent.to_path_buf());
                }
            }
        }
        // mirror localStorage
        if let Some(ls) = Self::local_storage() {
            let from_key = Self::to_ls_key(from);
            let to_key = Self::to_ls_key(to);
            if let Ok(Some(v)) = ls.get_item(&from_key) {
                let _ = ls.set_item(&to_key, &v);
                let _ = ls.remove_item(&from_key);
            }
            let from_dir = format!("{}/__dir__", from_key);
            let to_dir = format!("{}/__dir__", to_key);
            if let Ok(Some(v)) = ls.get_item(&from_dir) {
                let _ = ls.set_item(&to_dir, &v);
                let _ = ls.remove_item(&from_dir);
            }
            let prefix_from = format!("{}/", from_key);
            let prefix_to = format!("{}/", to_key);
            let mut moves = Vec::new();
            let len = ls.length().ok().unwrap_or(0);
            for i in 0..len {
                if let Ok(Some(k)) = ls.key(i) {
                    if k.starts_with(&prefix_from) {
                        moves.push(k);
                    }
                }
            }
            for k in moves {
                if let Ok(Some(v)) = ls.get_item(&k) {
                    let rest = &k[prefix_from.len()..];
                    let new_k = format!("{}{}", prefix_to, rest);
                    let _ = ls.set_item(&new_k, &v);
                    let _ = ls.remove_item(&k);
                }
            }
        }
        Ok(())
    }
    pub fn copy(&self, from: &Path, to: &Path) -> io::Result<u64> {
        let data = self
            .read(from)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "copy source not found"))?;
        let len = data.len() as u64;
        self.write(to, &data)?;
        Ok(len)
    }
    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut g = self.inner.write().unwrap();
        g.files.remove(&Self::normalize(path));
        g.mtimes.remove(&Self::normalize(path));
        Self::ls_remove(path);
        Ok(())
    }
    pub fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        {
            let mut g = self.inner.write().unwrap();
            let p = Self::normalize(path);
            g.files.retain(|k, _| !k.starts_with(&p));
            g.mtimes.retain(|k, _| !k.starts_with(&p));
            g.dirs.retain(|d| !d.starts_with(&p) && d != &p);
        }
        Self::ls_remove_prefix(path);
        Ok(())
    }
    pub fn exists(&self, path: &Path) -> bool {
        let g = self.inner.read().unwrap();
        let p = Self::normalize(path);
        if g.files.contains_key(&p) || g.dirs.contains(&p) {
            return true;
        }
        drop(g);
        Self::ls_exists(path)
    }
    pub fn is_dir(&self, path: &Path) -> bool {
        let g = self.inner.read().unwrap();
        if g.dirs.contains(&Self::normalize(path)) {
            return true;
        }
        drop(g);
        Self::ls_is_dir(path)
    }
    pub fn is_file(&self, path: &Path) -> bool {
        let g = self.inner.read().unwrap();
        if g.files.contains_key(&Self::normalize(path)) {
            return true;
        }
        drop(g);
        if let Some(ls) = Self::local_storage() {
            let key = Self::to_ls_key(path);
            return ls.get_item(&key).ok().flatten().is_some();
        }
        false
    }
    pub fn metadata_len(&self, path: &Path) -> Option<u64> {
        let g = self.inner.read().unwrap();
        if let Some(v) = g.files.get(&Self::normalize(path)) {
            return Some(v.len() as u64);
        }
        drop(g);
        if let Some(ls) = Self::local_storage() {
            let key = Self::to_ls_key(path);
            if let Ok(Some(v)) = ls.get_item(&key) {
                return Some(Self::ls_decode(&v).len() as u64);
            }
        }
        None
    }
    pub fn mtime_secs(&self, path: &Path) -> Option<u64> {
        let g = self.inner.read().unwrap();
        if let Some(v) = g.mtimes.get(&Self::normalize(path)) {
            return Some(*v);
        }
        drop(g);
        if Self::ls_exists(path) {
            return Some(now_secs());
        }
        None
    }
    pub fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let g = self.inner.read().unwrap();
        let p = Self::normalize(path);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mem_has_dir = g.dirs.contains(&p);
        for k in g.files.keys().chain(g.dirs.iter()) {
            if let Ok(rel) = k.strip_prefix(&p) {
                if let Some(first) = rel.components().next() {
                    let child = p.join(first);
                    if seen.insert(child.clone()) {
                        out.push(child);
                    }
                }
            }
        }
        drop(g);
        // Merge from localStorage
        if let Some(ls) = Self::local_storage() {
            let prefix = format!("{}/", Self::to_ls_key(path));
            let base_key = Self::to_ls_key(path);
            let len = ls.length().ok().unwrap_or(0);
            for i in 0..len {
                if let Ok(Some(k)) = ls.key(i) {
                    if k == base_key {
                        continue;
                    }
                    if k.starts_with(&prefix) {
                        let rest = &k[prefix.len()..];
                        if let Some(first) = rest.split('/').next() {
                            if first.is_empty() || first == "__dir__" {
                                continue;
                            }
                            let child = path.join(first);
                            if seen.insert(child.clone()) {
                                out.push(child);
                            }
                        }
                    }
                    if k.ends_with("/__dir__") {
                        let dir_path_str = &k[Self::ls_prefix().len()..k.len() - "/__dir__".len()];
                        let dir_path = PathBuf::from(dir_path_str);
                        if let Ok(rel) = dir_path.strip_prefix(path) {
                            if let Some(first) = rel.components().next() {
                                let child = path.join(first);
                                if seen.insert(child.clone()) {
                                    out.push(child);
                                }
                            }
                        }
                    }
                }
            }
        }
        if out.is_empty() && !mem_has_dir && !Self::ls_is_dir(path) && !Self::ls_exists(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "dir not found"));
        }
        Ok(out)
    }
    pub fn sync_file(&self, _path: &Path) {}
    pub fn sync_dir(&self, _path: &Path) {}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn test_binary_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let fs = Fs::new();
        let path = dir.path().join("blob.bin");
        let data: Vec<u8> = (0u8..=255u8).collect();
        fs.write(&path, &data).unwrap();
        assert_eq!(fs.read(&path).unwrap(), Some(data.clone()));
        assert_eq!(fs.metadata_len(&path), Some(256));

        let renamed = dir.path().join("renamed.bin");
        fs.rename(&path, &renamed).unwrap();
        assert_eq!(fs.read(&renamed).unwrap(), Some(data));
        assert!(fs.read(&path).unwrap().is_none());

        fs.remove_file(&renamed).unwrap();
        let err = fs.remove_file(&renamed).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
