//! Drag completed downloads to the operating system's file manager or another app.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{State, WebviewWindow};

use crate::AppState;
use crate::manager::State as DownloadState;

type Result<T> = std::result::Result<T, String>;
type Reply = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<bool>>>>>;

// Accept a download ID, never an arbitrary path supplied by the WebView. Resolve
// outside the UI thread and release the manager lock before the native drag loop.
#[tauri::command]
pub async fn drag_download(
    window: WebviewWindow,
    state: State<'_, AppState>,
    id: u64,
) -> Result<bool> {
    let path = available_path(state.manager.path_of(id))?;
    let (send, receive) = tokio::sync::oneshot::channel();
    let reply = Arc::new(Mutex::new(Some(send)));
    let source = window.clone();
    window
        .run_on_main_thread(move || {
            let completion = reply.clone();
            let result = start_drag(&source, path, move |dropped| {
                finish(&completion, Ok(dropped));
            });
            if let Err(error) = result {
                finish(&reply, Err(error));
            }
        })
        .map_err(|error| error.to_string())?;
    receive
        .await
        .map_err(|_| "the drag window is no longer available".to_owned())?
}

fn finish(reply: &Reply, result: Result<bool>) {
    if let Some(send) = reply
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        let _ = send.send(result);
    }
}

fn available_path(entry: Option<(PathBuf, DownloadState)>) -> Result<PathBuf> {
    let (path, state) = entry.ok_or("unknown download")?;
    if state != DownloadState::Completed || !path.exists() {
        return Err("the file is not available".into());
    }
    std::fs::canonicalize(path).map_err(|error| error.to_string())
}

fn start_drag(
    window: &WebviewWindow,
    path: PathBuf,
    on_done: impl Fn(bool) + Send + 'static,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    let native = window.gtk_window().map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "linux"))]
    let native = window;

    drag::start_drag(
        &native,
        drag::DragItem::Files(vec![path]),
        drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec()),
        move |outcome, _| on_done(matches!(outcome, drag::DragResult::Dropped)),
        // Copy keeps the original download and history available. Cancelling
        // the native operation is normal, and never removes the source file.
        drag::Options {
            mode: drag::DragMode::Copy,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_file_with_spaces_and_unicode_resolves_to_the_existing_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Tải xong (1).zip");
        std::fs::write(&path, b"completed bytes").unwrap();
        let resolved = available_path(Some((path.clone(), DownloadState::Completed))).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, std::fs::canonicalize(&path).unwrap());
        assert_eq!(std::fs::read(path).unwrap(), b"completed bytes");
    }

    #[test]
    fn unfinished_unknown_and_removed_downloads_cannot_be_dragged() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("file.zip");
        std::fs::write(&path, b"partial bytes").unwrap();
        for state in [
            DownloadState::Downloading,
            DownloadState::Paused,
            DownloadState::Failed,
        ] {
            assert!(available_path(Some((path.clone(), state))).is_err());
        }
        assert!(available_path(None).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(available_path(Some((path, DownloadState::Completed))).is_err());
    }

    #[test]
    fn completed_torrent_folder_is_an_available_drag_item() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("Torrent files");
        std::fs::create_dir(&path).unwrap();
        assert!(
            available_path(Some((path, DownloadState::Completed)))
                .unwrap()
                .is_dir()
        );
    }
}
