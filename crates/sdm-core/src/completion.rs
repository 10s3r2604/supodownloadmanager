//! A durable publication intent bridges final-file rename and application
//! history persistence. Its digest distinguishes a published download from a
//! same-size file that already occupied the destination before publication.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::checksum::sha256_file;
use crate::engine::PathGuard;
use crate::error::{Error, Result};
use crate::state::{PartialFiles, durable_write, sync_parent};

/// A verified final file whose completion has not yet been acknowledged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReceipt {
    pub completion_id: String,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    completion_id: String,
    path: PathBuf,
    size: u64,
    sha256: String,
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    })
}

async fn load(path: &Path) -> Result<Option<Journal>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::State(e.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn prepare_completion(
    path: &Path,
    temp_dir: Option<&Path>,
    completion_id: &str,
    size: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    let files = PartialFiles::new(path, temp_dir);
    // Cancelling the hash is safe: its detached blocking task only reads.
    // Once journal I/O begins, await it fully before cleanup so a cancelled
    // filesystem task cannot recreate a temporary file after cancellation.
    let sha256 = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        result = sha256_file(&files.data) => result?,
    };
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let journal = Journal {
        version: 1,
        completion_id: completion_id.to_owned(),
        path: absolute(path)?,
        size,
        sha256,
    };
    let bytes = serde_json::to_vec(&journal).map_err(|e| Error::State(e.to_string()))?;
    durable_write(&files.completion, &bytes).await
}

/// Checks an unacknowledged publication without contacting the server. Returns
/// a receipt only when the caller's identity, destination, length, and SHA-256
/// match. A publication intent alone never establishes that the file completed.
/// Persist the application's Completed history before acknowledging this receipt.
pub async fn reconcile_completion(
    path: &Path,
    temp_dir: Option<&Path>,
    completion_id: &str,
) -> Result<Option<CompletionReceipt>> {
    let _guard = PathGuard::try_new(path).ok_or_else(|| Error::InUse(path.to_owned()))?;
    let files = PartialFiles::new(path, temp_dir);
    let Some(journal) = load(&files.completion).await? else {
        return Ok(None);
    };
    if journal.version != 1
        || journal.completion_id != completion_id
        || journal.path != absolute(path)?
    {
        return Ok(None);
    }
    let metadata = match tokio::fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file()
        || metadata.len() != journal.size
        || sha256_file(path).await? != journal.sha256
    {
        return Ok(None);
    }
    Ok(Some(CompletionReceipt {
        completion_id: journal.completion_id,
        path: path.to_owned(),
        size: journal.size,
    }))
}

/// Deletes this caller's journal after its Completed history has been saved.
/// Missing journals are harmless. A journal owned by another caller or target
/// returns [`Error::State`] and preserves every file, so callers cannot mistake
/// a different publication for successful acknowledgement and then clean it up.
/// Call only after the transfer has ended, or during startup reconciliation.
pub async fn acknowledge_completion(
    path: &Path,
    temp_dir: Option<&Path>,
    completion_id: &str,
) -> Result<()> {
    let _guard = PathGuard::try_new(path).ok_or_else(|| Error::InUse(path.to_owned()))?;
    let files = PartialFiles::new(path, temp_dir);
    let Some(journal) = load(&files.completion).await? else {
        return Ok(());
    };
    if journal.version != 1
        || journal.completion_id != completion_id
        || journal.path != absolute(path)?
    {
        return Err(Error::State(
            "completion receipt belongs to another download or destination".into(),
        ));
    }
    tokio::fs::remove_file(&files.completion).await?;
    sync_parent(&files.completion).await?;
    files.cleanup_transfer().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovers_crash_immediately_after_publication_and_preserves_receipt_until_ack() {
        let output = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = output.path().join("file.bin");
        let files = PartialFiles::new(&path, Some(temp.path()));
        files.prepare(&path).await.unwrap();
        tokio::fs::write(&files.data, b"complete payload")
            .await
            .unwrap();
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(&files.data)
            .await
            .unwrap()
            .sync_all()
            .await
            .unwrap();
        prepare_completion(
            &path,
            Some(temp.path()),
            "row-1",
            16,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            reconcile_completion(&path, Some(temp.path()), "row-1")
                .await
                .unwrap(),
            None
        );

        // Simulate a crash at the first instruction after final publication:
        // progress cleanup and the application's history write never happened.
        tokio::fs::rename(&files.data, &path).await.unwrap();
        sync_parent(&path).await.unwrap();
        let receipt = reconcile_completion(&path, Some(temp.path()), "row-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receipt.size, 16);
        files.cleanup_transfer().await.unwrap();
        assert!(files.completion.exists());
        assert!(matches!(
            acknowledge_completion(&path, Some(temp.path()), "other-row").await,
            Err(Error::State(_))
        ));
        assert!(files.completion.exists());
        acknowledge_completion(&path, Some(temp.path()), "row-1")
            .await
            .unwrap();
        acknowledge_completion(&path, Some(temp.path()), "row-1")
            .await
            .unwrap();
        assert!(!files.directory().unwrap().exists());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"complete payload");
    }

    #[tokio::test]
    async fn unpublished_intent_does_not_accept_same_size_destination_or_corruption() {
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("file.bin");
        let files = PartialFiles::new(&path, None);
        tokio::fs::write(&files.data, b"correct").await.unwrap();
        tokio::fs::write(&path, b"oldfile").await.unwrap();
        prepare_completion(&path, None, "row-1", 7, &CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(
            acknowledge_completion(&path, None, "another-row").await,
            Err(Error::State(_))
        ));
        assert_eq!(tokio::fs::read(&files.data).await.unwrap(), b"correct");
        assert!(files.completion.exists());
        assert_eq!(
            reconcile_completion(&path, None, "row-1").await.unwrap(),
            None
        );
        tokio::fs::rename(&files.data, &path).await.unwrap();
        assert!(
            reconcile_completion(&path, None, "row-1")
                .await
                .unwrap()
                .is_some()
        );
        tokio::fs::write(&path, b"corrupt").await.unwrap();
        assert_eq!(
            reconcile_completion(&path, None, "row-1").await.unwrap(),
            None
        );
        tokio::fs::write(&path, b"short").await.unwrap();
        assert_eq!(
            reconcile_completion(&path, None, "row-1").await.unwrap(),
            None
        );
        tokio::fs::write(&files.completion, b"{broken")
            .await
            .unwrap();
        assert!(matches!(
            reconcile_completion(&path, None, "row-1").await,
            Err(Error::State(_))
        ));
    }
}
