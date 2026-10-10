//! Remembering download progress on disk so it survives restarts.
//!
//! While `movie.mp4` downloads, the data goes into `movie.mp4.sdm-part` and the
//! progress into `movie.mp4.sdm-state`. Both disappear once the download is
//! complete.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::probe::RemoteInfo;

const STATE_VERSION: u32 = 1;

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
}
