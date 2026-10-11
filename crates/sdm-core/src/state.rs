//! Remembering download progress on disk so it survives restarts.
//!
//! While `movie.mp4` downloads, the data goes into `movie.mp4.sdm-part` and the
//! progress into `movie.mp4.sdm-state`. Both disappear once the download is
//! complete.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::probe::RemoteInfo;

const STATE_VERSION: u32 = 1;

/// Files owned by one unfinished download. A configured temp root isolates
/// them by destination, so equal filenames in different folders never collide.
#[derive(Debug, Clone)]
pub struct PartialFiles {
    pub data: PathBuf,
    pub state: PathBuf,
    directory: Option<PathBuf>,
}

impl PartialFiles {
    pub fn new(path: &Path, temp_dir: Option<&Path>) -> Self {
        let directory = temp_dir.map(|root| {
            let absolute = if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            };
            let digest = Sha256::digest(absolute.as_os_str().as_encoded_bytes());
            root.join(format!("download-{digest:x}"))
        });
        let base = directory.as_ref().map_or_else(
            || path.to_owned(),
            |dir| {
                dir.join(
                    path.file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("download")),
                )
            },
        );
        Self {
            data: part_path(&base),
            state: state_path(&base),
            directory,
        }
    }

    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    pub(crate) async fn prepare(&self, path: &Path) -> Result<()> {
        if let Some(directory) = &self.directory {
            tokio::fs::create_dir_all(directory).await?;
            // Old app versions wrote next to the final file. Keep both halves
            // recoverable if an offline migration or resume is interrupted.
            for (old, new) in [
                (part_path(path), self.data.clone()),
                (state_path(path), self.state.clone()),
                (
                    with_suffix(&state_path(path), ".tmp"),
                    with_suffix(&self.state, ".tmp"),
                ),
            ] {
                if tokio::fs::try_exists(&old).await? && !tokio::fs::try_exists(&new).await? {
                    move_partial(&old, &new).await?;
                }
            }
        }
        Ok(())
    }

    /// Removes only this download's files, never the configured temp root.
    pub async fn cleanup(&self) -> Result<()> {
        for path in [
            &self.data,
            &self.state,
            &with_suffix(&self.state, ".tmp"),
            &with_suffix(&self.data, ".migrating"),
            &with_suffix(&self.state, ".migrating"),
        ] {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(directory) = &self.directory {
            match tokio::fs::remove_dir(directory).await {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

async fn move_partial(source: &Path, destination: &Path) -> Result<()> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            let staging = with_suffix(destination, ".migrating");
            tokio::fs::copy(source, &staging).await?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&staging)
                .await?
                .sync_all()
                .await?;
            tokio::fs::rename(&staging, destination).await?;
            tokio::fs::remove_file(source).await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// The file that holds the data of an unfinished download of `path`.
pub fn part_path(path: &Path) -> PathBuf {
    with_suffix(path, ".sdm-part")
}

/// The file that holds the progress of an unfinished download of `path`.
pub fn state_path(path: &Path) -> PathBuf {
    with_suffix(path, ".sdm-state")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    name.into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedState {
    pub version: u32,
    pub url: String,
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: Vec<SavedSegment>,
}

/// A segment as stored on disk: bytes `start..pos` are downloaded,
/// `pos..end` are still missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedSegment {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
}

impl SavedState {
    pub(crate) fn new(info: &RemoteInfo, size: u64, segments: Vec<SavedSegment>) -> Self {
        Self {
            version: STATE_VERSION,
            url: info.url.clone(),
            size,
            etag: info.etag.clone(),
            last_modified: info.last_modified.clone(),
            segments,
        }
    }

    pub(crate) async fn load(path: &Path) -> Result<Option<Self>> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| Error::State(e.to_string())),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the state atomically, so a crash never leaves half a file.
    pub(crate) async fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| Error::State(e.to_string()))?;
        let tmp = with_suffix(path, ".tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// Whether the saved segments make sense.
    pub(crate) fn is_valid(&self) -> bool {
        self.version == STATE_VERSION
            && !self.segments.is_empty()
            && self
                .segments
                .iter()
                .all(|s| s.start <= s.pos && s.pos <= s.end && s.end <= self.size)
    }

    /// Whether the file on the server still looks like the one we started
    /// downloading.
    pub(crate) fn matches(&self, info: &RemoteInfo) -> bool {
        fn same(saved: &Option<String>, remote: &Option<String>) -> bool {
            match (saved, remote) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
        }
        info.resumable
            && info.size == Some(self.size)
            && same(&self.etag, &info.etag)
            && same(&self.last_modified, &info.last_modified)
    }
}

/// Points the unfinished download of `path` at a new address of the same
/// file (an expired link replaced by a fresh one), so it continues instead
/// of starting over. Returns the bytes already downloaded, or `None` when
/// there is no saved progress.
///
/// Fails when the new address serves a file of another size, or one that
/// cannot be resumed.
pub async fn retarget_partial(path: &Path, info: &RemoteInfo) -> Result<Option<u64>> {
    retarget_partial_in(path, None, info).await
}

/// Changes an unfinished download's link in its configured temp directory.
pub async fn retarget_partial_in(
    path: &Path,
    temp_dir: Option<&Path>,
    info: &RemoteInfo,
) -> Result<Option<u64>> {
    let files = PartialFiles::new(path, temp_dir);
    files.prepare(path).await?;
    let state = files.state;
    let Some(mut saved) = SavedState::load(&state).await? else {
        return Ok(None);
    };
    if !saved.is_valid() {
        return Ok(None);
    }
    if info.size != Some(saved.size) {
        return Err(Error::NotSameFile(match info.size {
            Some(size) => format!(
                "the new link is for a different file ({size} bytes instead of {})",
                saved.size
            ),
            None => "the new link does not say how big the file is".into(),
        }));
    }
    if !info.resumable {
        return Err(Error::NotSameFile(
            "the new link cannot continue a download that was started".into(),
        ));
    }
    saved.url.clone_from(&info.url);
    saved.etag.clone_from(&info.etag);
    saved.last_modified.clone_from(&info.last_modified);
    saved.save(&state).await?;
    Ok(Some(
        saved.segments.iter().map(|s| s.pos - s.start).sum::<u64>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(size: u64, etag: Option<&str>) -> RemoteInfo {
        RemoteInfo {
            url: "https://example.com/a.bin".into(),
            final_url: "https://example.com/a.bin".into(),
            size: Some(size),
            resumable: true,
            file_name: "a.bin".into(),
            content_type: None,
            etag: etag.map(Into::into),
            last_modified: None,
        }
    }

    #[test]
    fn side_files_keep_the_full_name() {
        let path = Path::new("dir/movie.mp4");
        assert_eq!(part_path(path), Path::new("dir/movie.mp4.sdm-part"));
        assert_eq!(state_path(path), Path::new("dir/movie.mp4.sdm-state"));
    }

    #[test]
    fn detects_remote_changes() {
        let segments = vec![SavedSegment {
            start: 0,
            pos: 5,
            end: 10,
        }];
        let state = SavedState::new(&info(10, Some("\"v1\"")), 10, segments);
        assert!(state.is_valid());
        assert!(state.matches(&info(10, Some("\"v1\""))));
        assert!(state.matches(&info(10, None)));
        assert!(!state.matches(&info(10, Some("\"v2\""))));
        assert!(!state.matches(&info(11, Some("\"v1\""))));
    }

    #[test]
    fn rejects_nonsense_segments() {
        let segments = vec![SavedSegment {
            start: 0,
            pos: 12,
            end: 10,
        }];
        assert!(!SavedState::new(&info(10, None), 10, segments).is_valid());
    }

    #[tokio::test]
    async fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin.sdm-state");
        assert_eq!(SavedState::load(&path).await.unwrap(), None);

        let segments = vec![SavedSegment {
            start: 0,
            pos: 5,
            end: 10,
        }];
        let state = SavedState::new(&info(10, None), 10, segments);
        state.save(&path).await.unwrap();
        assert_eq!(SavedState::load(&path).await.unwrap(), Some(state));

        tokio::fs::write(&path, b"{not json").await.unwrap();
        assert!(matches!(
            SavedState::load(&path).await,
            Err(Error::State(_))
        ));
    }

    #[tokio::test]
    async fn moves_progress_to_a_new_address() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        assert_eq!(
            retarget_partial(&path, &info(10, None)).await.unwrap(),
            None
        );

        let segments = vec![
            SavedSegment {
                start: 0,
                pos: 3,
                end: 5,
            },
            SavedSegment {
                start: 5,
                pos: 9,
                end: 10,
            },
        ];
        SavedState::new(&info(10, Some("\"old\"")), 10, segments)
            .save(&state_path(&path))
            .await
            .unwrap();

        let mut fresh = info(10, Some("\"new\""));
        fresh.url = "https://mirror.example.net/a.bin?token=2".into();
        assert!(matches!(
            retarget_partial(&path, &info(11, None)).await,
            Err(Error::NotSameFile(_))
        ));
        let mut whole = fresh.clone();
        whole.resumable = false;
        assert!(matches!(
            retarget_partial(&path, &whole).await,
            Err(Error::NotSameFile(_))
        ));

        assert_eq!(retarget_partial(&path, &fresh).await.unwrap(), Some(7));
        let saved = SavedState::load(&state_path(&path)).await.unwrap().unwrap();
        assert_eq!(saved.url, fresh.url);
        assert!(saved.matches(&fresh));
    }
}
