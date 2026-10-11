//! Functions the user interface calls with `invoke(...)`.

use std::path::PathBuf;

use sdm_core::RemoteInfo;
use serde::Serialize;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::AppState;
use crate::browsers::{self, Browser, Engine, InstallInfo};
use crate::category::Category;
use crate::manager::{AddRequest, DownloadView, State as DownloadState};
use crate::media::{AddMediaRequest, MediaInfo, MediaSource, ToolStatus};
use crate::settings::Settings;
use crate::torrent::{AddTorrentRequest, TorrentFile, TorrentInfo, TorrentSource};
use crate::tray;
use crate::windows::WhenDone;

type Result<T> = std::result::Result<T, String>;

fn text_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub async fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadView>> {
    // Background jobs may hold the manager lock while saving to disk.
    // Keep frequent UI polls off the Windows message loop.
    Ok(state.manager.list())
}

#[tauri::command]
pub async fn get_download(state: State<'_, AppState>, id: u64) -> Result<Option<DownloadView>> {
    Ok(state.manager.get(id))
}

/// Opens the progress window of a download.
#[tauri::command]
pub async fn open_progress_window(app: AppHandle, id: u64) {
    crate::windows::open_progress(&app, id);
}

/// Which of yt-dlp and ffmpeg are ready.
#[tauri::command]
pub async fn media_tools(state: State<'_, AppState>) -> Result<ToolStatus> {
    let tools = state.tools.clone();
    tauri::async_runtime::spawn_blocking(move || tools.status())
        .await
        .map_err(text_error)
}

/// Downloads yt-dlp (and ffmpeg when `ffmpeg` is set) if they are missing,
/// reporting progress with `sdm:tool-progress` events.
#[tauri::command]
pub async fn media_prepare(
    app: AppHandle,
    state: State<'_, AppState>,
    ffmpeg: bool,
) -> Result<ToolStatus> {
    let downloader = state.manager.downloader();
    let report = |progress| {
        let _ = app.emit(crate::TOOL_PROGRESS_EVENT, progress);
    };
    state.tools.install_ytdlp(&downloader, report).await?;
    if ffmpeg && state.tools.ffmpeg().is_none() {
        state.tools.install_ffmpeg(&downloader, report).await?;
    }
    Ok(state.tools.status())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbe {
    #[serde(flatten)]
    info: MediaInfo,
    /// Where videos and audio go unless the user picks another folder.
    video_dir: PathBuf,
    audio_dir: PathBuf,
}

/// Asks yt-dlp which qualities a page offers.
#[tauri::command]
pub async fn media_probe(
    app: AppHandle,
    state: State<'_, AppState>,
    source: MediaSource,
) -> Result<MediaProbe> {
    let settings = state.manager.settings();
    let info = crate::probe_page(&app, &source).await?;
    Ok(MediaProbe {
        info,
        video_dir: settings.directory_for(Category::Video),
        audio_dir: settings.directory_for(Category::Music),
    })
}

/// What yt-dlp is doing while it reads a page.
#[tauri::command]
pub async fn media_probe_stage(
    state: State<'_, AppState>,
    page_url: String,
) -> Result<Option<String>> {
    Ok(state.prober.stage(&page_url))
}

/// Stops discovery for this page, including any browser prefetch sharing it.
#[tauri::command]
pub async fn cancel_media_probe(state: State<'_, AppState>, page_url: String) -> Result<()> {
    state.prober.cancel(&page_url);
    Ok(())
}

/// Adds a video in the chosen quality to the download list.
/// Scheduling may create a WebView2 window, so this must run off the UI thread.
#[tauri::command]
pub async fn add_media(state: State<'_, AppState>, request: AddMediaRequest) -> Result<u64> {
    state.manager.add_media(request)
}

/// Reads a magnet link or `.torrent` file for the "add torrent" dialog.
#[tauri::command]
pub async fn inspect_torrent(
    state: State<'_, AppState>,
    source: TorrentSource,
) -> Result<TorrentInfo> {
    state.manager.inspect_torrent(source).await
}

/// Stops reading magnet links (the dialog was closed).
#[tauri::command]
pub fn cancel_torrent_inspect(state: State<'_, AppState>) {
    state.manager.cancel_torrent_inspections();
}

/// Adds the chosen files of a torrent to the download list.
#[tauri::command]
pub async fn add_torrent(state: State<'_, AppState>, request: AddTorrentRequest) -> Result<u64> {
    state.manager.add_torrent(request)
}

/// What happens when a download finishes.
#[tauri::command]
pub fn get_when_done(state: State<'_, AppState>, id: u64) -> WhenDone {
    state.when_done.get(id, &state.manager.settings())
}

#[tauri::command]
pub fn set_when_done(state: State<'_, AppState>, id: u64, choice: WhenDone) {
    state.when_done.set(id, choice);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    #[serde(flatten)]
    info: RemoteInfo,
    category: Category,
    directory: PathBuf,
}

/// Asks the server about a link, for the "add download" dialog.
#[tauri::command]
pub async fn probe_url(
    state: State<'_, AppState>,
    url: String,
    headers: Vec<(String, String)>,
) -> Result<ProbeResult> {
    let manager = state.manager.clone();
    let info = manager
        .downloader()
        .probe(&url, &headers)
        .await
        .map_err(text_error)?;
    let category = Category::from_file_name(&info.file_name);
    let directory = manager.settings().directory_for(category);
    Ok(ProbeResult {
        info,
        category,
        directory,
    })
}

#[tauri::command]
pub async fn add_download(state: State<'_, AppState>, request: AddRequest) -> Result<u64> {
    state.manager.clone().add(request).await
}

#[tauri::command]
pub fn pause_downloads(state: State<'_, AppState>, ids: Vec<u64>) {
    state.manager.pause(&ids);
}

#[tauri::command]
pub async fn resume_downloads(state: State<'_, AppState>, ids: Vec<u64>) -> Result<()> {
    state.manager.resume(&ids);
    Ok(())
}

#[tauri::command]
pub fn remove_downloads(state: State<'_, AppState>, ids: Vec<u64>, delete_files: bool) {
    state.manager.remove(&ids, delete_files);
}

#[tauri::command]
pub fn pause_all(state: State<'_, AppState>) {
    state.manager.pause_all();
}

#[tauri::command]
pub async fn resume_all(state: State<'_, AppState>) -> Result<()> {
    state.manager.resume_all();
    Ok(())
}

/// Opens a finished file with its default program.
#[tauri::command]
pub fn open_download(app: AppHandle, state: State<'_, AppState>, id: u64) -> Result<()> {
    let (path, download_state) = state.manager.path_of(id).ok_or("unknown download")?;
    if download_state != DownloadState::Completed || !path.exists() {
        return Err("the file is not available".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(text_error)
}

/// Shows the file (or its folder, while it is unfinished) in the file manager.
#[tauri::command]
pub fn show_in_folder(app: AppHandle, state: State<'_, AppState>, id: u64) -> Result<()> {
    let (path, _) = state.manager.path_of(id).ok_or("unknown download")?;
    if path.exists() {
        return app.opener().reveal_item_in_dir(&path).map_err(text_error);
    }
    let folder = path.parent().ok_or("the folder does not exist")?;
    std::fs::create_dir_all(folder).map_err(text_error)?;
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(text_error)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings> {
    Ok(state.manager.settings())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings> {
    let settings = settings.sanitized();
    settings
        .save(&state.data_dir.join("settings.json"))
        .map_err(text_error)?;
    state
        .manager
        .update_settings(settings.clone())
        .map_err(text_error)?;
    tray::set_language(&app, settings.resolved_language());
    Ok(settings)
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = sender.send(folder);
    });
    let folder = receiver.await.ok().flatten()?;
    folder.into_path().ok().map(|p| p.display().to_string())
}

/// Asks for a `.torrent` file.
#[tauri::command]
pub async fn pick_torrent_file(app: AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("BitTorrent", &["torrent"])
        .pick_file(move |file| {
            let _ = sender.send(file);
        });
    let file = receiver.await.ok().flatten()?;
    file.into_path().ok().map(|p| p.display().to_string())
}

/// Magnet links and .torrent files SDM was started with.
#[tauri::command]
pub fn take_pending_torrents(state: State<'_, AppState>) -> Vec<TorrentSource> {
    state
        .pending_torrents
        .lock()
        .map(|mut list| std::mem::take(&mut *list))
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MagnetHandler {
    /// The system lets SDM make itself the program for magnet links.
    supported: bool,
    registered: bool,
}

/// Whether SDM opens magnet links outside the browser.
#[tauri::command]
pub fn magnet_handler(app: AppHandle) -> MagnetHandler {
    use tauri_plugin_deep_link::DeepLinkExt;
    match app.deep_link().is_registered("magnet") {
        Ok(registered) => MagnetHandler {
            supported: true,
            registered,
        },
        Err(_) => MagnetHandler {
            supported: false,
            registered: false,
        },
    }
}

/// Makes SDM the program that opens magnet links.
#[tauri::command]
pub fn register_magnet_handler(app: AppHandle) -> Result<()> {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.deep_link().register("magnet").map_err(text_error)
}

/// The files a torrent download gets, in torrent order.
#[tauri::command]
pub fn torrent_files(state: State<'_, AppState>, id: u64) -> Vec<TorrentFile> {
    state.manager.torrent_files(id).unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    #[serde(flatten)]
    browser: Browser,
    /// The extension in this browser talked to SDM recently.
    connected: bool,
}

#[tauri::command]
pub fn list_browsers(state: State<'_, AppState>) -> Vec<BrowserStatus> {
    let seen = state.bridge.connected();
    browsers::detect()
        .into_iter()
        .map(|browser| {
            let connected = seen.get(&browser.id).is_some_and(|secs| *secs < 120);
            BrowserStatus { browser, connected }
        })
        .collect()
}

#[tauri::command]
pub fn install_extension(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<InstallInfo> {
    let browser = browsers::detect()
        .into_iter()
        .find(|b| b.id == id)
        .ok_or("this browser was not found")?;
    let bundled = app
        .path()
        .resolve("extension", BaseDirectory::Resource)
        .map_err(text_error)?;
    browsers::install(&browser, &bundled, &state.data_dir).map_err(text_error)
}

/// Opens the folder that holds the unpacked extension.
#[tauri::command]
pub fn open_extension_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    engine: Engine,
) -> Result<()> {
    let folder = state
        .data_dir
        .join("extension")
        .join(engine.extension_folder());
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(text_error)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    bridge_port: u16,
    bridge_error: Option<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        bridge_port: state.bridge_port,
        bridge_error: state.bridge_error.lock().ok().and_then(|e| e.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_results_match_the_interface() {
        let result = ProbeResult {
            info: RemoteInfo {
                url: "u".into(),
                final_url: "f".into(),
                size: Some(3),
                resumable: true,
                file_name: "movie.mkv".into(),
                content_type: Some("video/x-matroska".into()),
                etag: None,
                last_modified: None,
            },
            category: Category::Video,
            directory: PathBuf::from("dl"),
        };
        let json = serde_json::to_value(result).unwrap();
        for key in [
            "url",
            "finalUrl",
            "size",
            "resumable",
            "fileName",
            "contentType",
            "category",
            "directory",
        ] {
            assert!(json.get(key).is_some(), "missing {key} in {json}");
        }
        assert_eq!(json["category"], "video");
    }
}
