//! Reading and writing JSON files safely.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Reads a JSON file. Returns `None` when it is missing or cannot be parsed.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "ignoring unreadable file");
            None
        }
    }
}

/// Writes a JSON file atomically, so a crash never leaves half a file.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let tmp = with_suffix(path, ".tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("value.json");
        assert_eq!(load_json::<Vec<u32>>(&path), None);
        save_json(&path, &vec![1u32, 2, 3]).unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path), Some(vec![1, 2, 3]));
        std::fs::write(&path, "[1, 2").unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path), None);
    }
}
