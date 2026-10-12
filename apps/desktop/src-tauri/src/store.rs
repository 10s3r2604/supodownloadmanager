//! Durable JSON files with a last-good backup and explicit recovery.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

pub struct Loaded<T> {
    pub value: Option<T>,
    pub damaged: Option<PathBuf>,
    pub recovered_from: Option<PathBuf>,
}

impl<T> Loaded<T> {
    pub fn recovery_notice(&self, subject: &str) -> Option<String> {
        self.recovered_from.as_ref().map(|backup| {
            let mut message = format!(
                "Recovered {subject} from the last-good backup at {}.",
                backup.display()
            );
            if let Some(damaged) = &self.damaged {
                message.push_str(&format!(
                    " The damaged file was preserved at {}.",
                    damaged.display()
                ));
            }
            message
        })
    }
}

/// Only an absent primary and backup is an empty store.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    Ok(load_json_validated(path, |_| Ok(()))?.value)
}

pub fn load_json_validated<T: DeserializeOwned>(
    path: &Path,
    validate: impl Fn(&T) -> io::Result<()>,
) -> io::Result<Loaded<T>> {
    let primary = read_json(path, &validate);
    match primary {
        Ok(Some(value)) => {
            return Ok(Loaded {
                value: Some(value),
                damaged: None,
                recovered_from: None,
            });
        }
        Err(ref error) if error.kind() != io::ErrorKind::InvalidData => {
            return Err(context(
                path,
                io::Error::new(error.kind(), error.to_string()),
            ));
        }
        _ => {}
    }
    let backup = with_suffix(path, ".bak");
    let value = match read_json(&backup, &validate) {
        Ok(Some(value)) => value,
        Ok(None) if matches!(primary, Ok(None)) => {
            return Ok(Loaded {
                value: None,
                damaged: None,
                recovered_from: None,
            });
        }
        Ok(None) => return Err(context(path, primary.err().unwrap())),
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "Cannot recover {}: backup {} is unusable ({error}). The original file was preserved.",
                    path.display(),
                    backup.display()
                ),
            ));
        }
    };
    let bytes = std::fs::read(&backup)?;
    let damaged = if path.exists() {
        Some(preserve_damaged(path)?)
    } else {
        None
    };
    if let Err(error) = replace_bytes(path, &bytes) {
        if let Some(original) = &damaged {
            let _ = std::fs::rename(original, path);
        }
        return Err(context(path, error));
    }
    tracing::warn!(path = %path.display(), backup = %backup.display(), damaged = ?damaged,
        "recovered a JSON file from its last-good backup");
    Ok(Loaded {
        value: Some(value),
        damaged,
        recovered_from: Some(backup),
    })
}

fn read_json<T: DeserializeOwned>(
    path: &Path,
    validate: &impl Fn(&T) -> io::Result<()>,
) -> io::Result<Option<T>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    validate(&value)?;
    Ok(Some(value))
}

/// Saves the previous good document before publishing a synced replacement.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    save_bytes(path, &bytes, replace_bytes)
}

fn save_bytes(
    path: &Path,
    bytes: &[u8],
    publish: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let previous = match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Some(bytes)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(context(path, error)),
    };
    let backup = with_suffix(path, ".bak");
    let previous_backup = match std::fs::read(&backup) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(context(&backup, error)),
    };
    replace_bytes(&backup, previous.as_deref().unwrap_or(bytes))?;
    if let Err(error) = publish(path, bytes) {
        // In particular, a rejected first save must not leave a backup that
        // resurrects an unsaved download at the next startup.
        let rollback = match previous_backup {
            Some(bytes) => replace_bytes(&backup, &bytes),
            None => std::fs::remove_file(&backup).and_then(|_| sync_parent(&backup)),
        };
        if let Err(rollback) = rollback {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}; restoring backup {} also failed: {rollback}",
                    backup.display()
                ),
            ));
        }
        return Err(context(path, error));
    }
    Ok(())
}

/// A targeted settings repair retains the original document for the user.
pub fn repair_json<T: Serialize>(path: &Path, value: &T) -> io::Result<PathBuf> {
    let damaged = preserve_damaged(path)?;
    if let Err(error) = save_json(path, value) {
        let _ = std::fs::rename(&damaged, path);
        return Err(context(path, error));
    }
    Ok(damaged)
}

fn preserve_damaged(path: &Path) -> io::Result<PathBuf> {
    let damaged = unique_path(path, ".damaged");
    std::fs::rename(path, &damaged)?;
    if let Err(error) = sync_parent(path) {
        let _ = std::fs::rename(&damaged, path);
        return Err(error);
    }
    Ok(damaged)
}

fn replace_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    sync_parent(path)?;
    let previous = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let tmp = unique_path(path, ".tmp");
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        if let Err(error) = sync_parent(path) {
            let rollback = match &previous {
                Some(bytes) => write_rollback(path, bytes),
                None => std::fs::remove_file(path),
            };
            if let Err(rollback) = rollback {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "{error}; restoring {} also failed: {rollback}",
                        path.display()
                    ),
                ));
            }
            return Err(error);
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(tmp);
    result.map_err(|error| context(path, error))
}

fn write_rollback(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = unique_path(path, ".rollback");
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    let _ = std::fs::remove_file(tmp);
    result
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn context(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "Cannot read or save {}: {error}. Existing data was preserved.",
            path.display()
        ),
    )
}

fn unique_path(path: &Path, suffix: &str) -> PathBuf {
    with_suffix(
        path,
        &format!(
            "{suffix}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ),
    )
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
    fn missing_and_round_trip_are_distinct_from_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/value.json");
        assert_eq!(load_json::<Vec<u32>>(&path).unwrap(), None);
        save_json(&path, &vec![1u32, 2, 3]).unwrap();
        assert_eq!(load_json::<Vec<u32>>(&path).unwrap(), Some(vec![1, 2, 3]));
        let damaged = dir.path().join("bad.json");
        std::fs::write(&damaged, "[1, 2").unwrap();
        assert_eq!(
            load_json::<Vec<u32>>(&damaged).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(std::fs::read_to_string(damaged).unwrap(), "[1, 2");
    }

    #[test]
    fn recovers_previous_snapshot_and_retains_damaged_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("value.json");
        save_json(&path, &vec![1u32, 2]).unwrap();
        save_json(&path, &vec![1u32, 2, 3]).unwrap();
        std::fs::write(&path, "[broken").unwrap();
        let recovered = load_json_validated::<Vec<u32>>(&path, |_| Ok(())).unwrap();
        assert_eq!(recovered.value, Some(vec![1, 2]));
        assert_eq!(
            std::fs::read_to_string(recovered.damaged.unwrap()).unwrap(),
            "[broken"
        );
        assert_eq!(load_json::<Vec<u32>>(&path).unwrap(), Some(vec![1, 2]));
    }

    #[test]
    fn failed_backup_save_keeps_current_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("value.json");
        save_json(&path, &vec![1u32]).unwrap();
        std::fs::remove_file(with_suffix(&path, ".bak")).unwrap();
        std::fs::create_dir(with_suffix(&path, ".bak")).unwrap();
        assert!(save_json(&path, &vec![2u32]).is_err());
        assert_eq!(load_json::<Vec<u32>>(&path).unwrap(), Some(vec![1]));
    }

    #[test]
    fn io_errors_and_unusable_backups_are_not_empty_stores() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_json::<Vec<u32>>(dir.path()).is_err());
        let path = dir.path().join("value.json");
        std::fs::write(&path, "broken primary").unwrap();
        std::fs::write(with_suffix(&path, ".bak"), "broken backup").unwrap();
        assert!(load_json::<Vec<u32>>(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken primary");
    }

    #[test]
    fn failed_primary_publication_restores_backup_and_cannot_resurrect_an_add() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("value.json");
        let fail = |_: &Path, _: &[u8]| Err(io::Error::other("injected publication failure"));
        assert!(save_bytes(&path, b"[1]", fail).is_err());
        assert!(!path.exists());
        assert!(!with_suffix(&path, ".bak").exists());
        assert_eq!(load_json::<Vec<u32>>(&path).unwrap(), None);

        save_json(&path, &vec![1u32]).unwrap();
        save_json(&path, &vec![2u32]).unwrap();
        let previous = std::fs::read(&path).unwrap();
        let previous_backup = std::fs::read(with_suffix(&path, ".bak")).unwrap();
        assert!(save_bytes(&path, b"[3]", fail).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(
            std::fs::read(with_suffix(&path, ".bak")).unwrap(),
            previous_backup
        );
    }
}
