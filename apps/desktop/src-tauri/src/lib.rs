//! Supo Download Manager desktop app (Tauri).

mod bridge;
mod browsers;
mod category;
mod clipboard;
mod commands;
mod manager;
mod media;
mod settings;
mod store;
mod texts;
mod torrent;
mod tray;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager as _, RunEvent, Window, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::bridge::{BoxFuture, Bridge, BridgeConfig, BrowserDownload, Handler};
use crate::manager::{AddRequest, Manager, Notice};
use crate::settings::Settings;

/// Asks the user interface to show the "add download" dialog.
const ADD_PROMPT_EVENT: &str = "sdm:add-prompt";
/// Tells the user interface that a download link was copied.
const CLIPBOARD_EVENT: &str = "sdm:clipboard-link";
/// Asks the user interface to show the video quality picker.
const MEDIA_PROMPT_EVENT: &str = "sdm:media-prompt";
/// Progress of downloading yt-dlp or ffmpeg.
pub(crate) const TOOL_PROGRESS_EVENT: &str = "sdm:tool-progress";
/// Asks the user interface to show the "add torrent" dialog.
const TORRENT_PROMPT_EVENT: &str = "sdm:torrent-prompt";

pub struct AppState {
    pub manager: Arc<Manager>,
    pub bridge: Arc<Bridge>,
    pub bridge_port: u16,
    pub bridge_error: Mutex<Option<String>>,
    pub data_dir: PathBuf,
    /// yt-dlp and ffmpeg for videos from web pages.
    pub tools: Arc<media::Tools>,
    /// What yt-dlp found on video pages recently.
    pub prober: Arc<media::Prober>,
    /// What to do when running downloads finish, chosen in their windows.
    pub when_done: windows::WhenDoneChoices,
    /// Torrents SDM was started with, until the window asks for them.
    pub pending_torrents: Mutex<Vec<torrent::TorrentSource>>,
    quitting: AtomicBool,
}

/// A download offered by the browser, shown in the "add download" dialog.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AddPrompt {
    url: String,
    file_name: Option<String>,
    headers: Vec<(String, String)>,
}

pub fn run() {
    init_logging();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            show_main_window(app);
            // Opening a magnet link or .torrent file while SDM runs.
            for source in torrent_sources(args.iter().skip(1), Path::new(&cwd)) {
                let _ = app.emit(TORRENT_PROMPT_EVENT, source);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::list_downloads,
            commands::get_download,
            commands::open_progress_window,
            commands::get_when_done,
            commands::media_tools,
            commands::media_prepare,
            commands::media_probe,
            commands::media_probe_stage,
            commands::cancel_media_probe,
            commands::add_media,
            commands::inspect_torrent,
            commands::cancel_torrent_inspect,
            commands::add_torrent,
            commands::pick_torrent_file,
            commands::torrent_files,
            commands::take_pending_torrents,
            commands::magnet_handler,
            commands::register_magnet_handler,
            commands::set_when_done,
            commands::probe_url,
            commands::add_download,
            commands::pause_downloads,
            commands::resume_downloads,
            commands::remove_downloads,
            commands::pause_all,
            commands::resume_all,
            commands::open_download,
            commands::show_in_folder,
            commands::get_settings,
            commands::save_settings,
            commands::pick_folder,
            commands::list_browsers,
            commands::install_extension,
            commands::open_extension_folder,
            commands::app_info,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Supo Download Manager")
        .run(|app, event| match event {
            // Closing the last window should still save all progress.
            RunEvent::ExitRequested {
                api, code: None, ..
            } => {
                api.prevent_exit();
                quit(app);
            }
            // macOS hands over magnet links and .torrent files this way.
            #[cfg(target_os = "macos")]
            RunEvent::Opened { urls } => {
                let args: Vec<String> = urls
                    .iter()
                    .map(|url| match url.to_file_path() {
                        Ok(path) => path.display().to_string(),
                        Err(()) => url.to_string(),
                    })
                    .collect();
                for source in torrent_sources(args.iter(), Path::new("/")) {
                    show_main_window(app);
                    let _ = app.emit(TORRENT_PROMPT_EVENT, source);
                }
            }
            _ => {}
        });
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sdm_core=warn,sdm_desktop=info".into());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn setup(app: &AppHandle) -> anyhow::Result<()> {
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let mut settings = Settings::load(&data_dir.join("settings.json"));
    if settings.download_dir.as_os_str().is_empty() {
        settings.download_dir = app
            .path()
            .download_dir()
            .unwrap_or_else(|_| data_dir.join("Downloads"));
    }

    let notify_app = app.clone();
    let bundled_tools = app.path().resource_dir().ok().map(|dir| dir.join("tools"));
    let tools = Arc::new(media::Tools::new(&data_dir, bundled_tools));
    let torrents = torrent::TorrentEngine::new(data_dir.join("torrent"), settings.torrent_config());
    let manager = Manager::new(
        data_dir.join("downloads.json"),
        settings.clone(),
        tools.clone(),
        torrents,
        tauri::async_runtime::handle().inner().clone(),
        Box::new(move |notice| notify(&notify_app, notice)),
    )?;
    let bridge = Bridge::new(AppBridge {
        app: app.clone(),
        manager: manager.clone(),
    });
    app.manage(AppState {
        manager: manager.clone(),
        bridge: bridge.clone(),
        bridge_port: settings.bridge_port,
        bridge_error: Mutex::new(None),
        data_dir,
        tools: tools.clone(),
        prober: Arc::default(),
        when_done: windows::WhenDoneChoices::default(),
        pending_torrents: Mutex::new(torrent_sources(
            std::env::args().skip(1),
            &std::env::current_dir().unwrap_or_default(),
        )),
        quitting: AtomicBool::new(false),
    });
    tray::create(app, settings.resolved_language())?;

    // Sites change all the time; keep our copy of yt-dlp fresh.
    let update_manager = manager.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        tools.update_ytdlp(&update_manager.downloader()).await;
    });

    // Keep the stored list up to date and save it every few seconds.
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        let mut tick = 0u8;
        loop {
            ticker.tick().await;
            tick = (tick + 1) % 5;
            manager.tick(tick == 0);
        }
    });

    let bridge_app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = bridge.serve(settings.bridge_port).await {
            tracing::error!(%error, "the browser bridge could not start");
            if let Ok(mut slot) = bridge_app.state::<AppState>().bridge_error.lock() {
                *slot = Some(error.to_string());
            }
        }
    });

    tauri::async_runtime::spawn(watch_clipboard(app.clone()));
    Ok(())
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    if windows::is_progress_window(window.label()) {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let app = window.app_handle();
        if app.state::<AppState>().manager.settings().close_to_tray {
            let _ = window.hide();
        } else {
            quit(app);
        }
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Pauses all downloads so their progress is saved, then exits.
pub(crate) fn quit(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let manager = state.manager.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        manager.shutdown().await;
        app.exit(0);
    });
}

fn notify(app: &AppHandle, notice: Notice) {
    let state = app.state::<AppState>();
    let settings = state.manager.settings();
    let texts = texts::texts(settings.resolved_language());
    let (title, body) = match notice {
        Notice::Started { id } => {
            if settings.progress_window {
                windows::open_progress(app, id);
            }
            return;
        }
        Notice::Completed {
            id,
            file_name,
            path,
        } => {
            let when_done = state.when_done.take(id, &settings);
            if when_done.open_file
                && let Err(error) = app.opener().open_path(path.to_string_lossy(), None::<&str>)
            {
                tracing::warn!(%error, "could not open the finished file");
            }
            if when_done.show_dialog {
                // The dialog says it all; no need for a notification too.
                windows::open_progress(app, id);
                return;
            }
            windows::close_progress(app, id);
            (texts.completed, file_name)
        }
        Notice::Failed { file_name, error } => (texts.failed, format!("{file_name}\n{error}")),
    };
    if !settings.notifications {
        return;
    }
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(%error, "could not show a notification");
    }
}

/// Offers links copied to the clipboard for download.
async fn watch_clipboard(app: AppHandle) {
    let mut last = app.clipboard().read_text().unwrap_or_default();
    let mut ticker = tokio::time::interval(Duration::from_millis(1500));
    loop {
        ticker.tick().await;
        let settings = app.state::<AppState>().manager.settings();
        if !settings.clipboard_monitor {
            continue;
        }
        let Ok(text) = app.clipboard().read_text() else {
            continue;
        };
        if text == last {
            continue;
        }
        if let Some(link) = clipboard::downloadable_link(&text, &settings.capture_extensions) {
            let _ = app.emit(CLIPBOARD_EVENT, link);
        }
        last = text;
    }
}

/// Connects the browser bridge to the download list.
struct AppBridge {
    app: AppHandle,
    manager: Arc<Manager>,
}

impl Handler for AppBridge {
    fn config(&self) -> BridgeConfig {
        let settings = self.manager.settings();
        BridgeConfig {
            enabled: settings.browser_integration,
            capture_extensions: settings.capture_extensions,
        }
    }

    fn download(&self, download: BrowserDownload) {
        let headers = download.headers();
        let file_name = download.file_name.filter(|name| !name.trim().is_empty());
        if torrent::is_magnet(&download.url)
            || torrent::is_torrent_file_name(&download.url)
            || file_name
                .as_deref()
                .is_some_and(torrent::is_torrent_file_name)
        {
            open_torrent(
                &self.app,
                torrent::TorrentSource {
                    link: Some(download.url),
                    path: None,
                    headers,
                },
            );
            return;
        }
        if self.manager.settings().confirm_browser_downloads {
            show_main_window(&self.app);
            let _ = self.app.emit(
                ADD_PROMPT_EVENT,
                AddPrompt {
                    url: download.url,
                    file_name,
                    headers,
                },
            );
            return;
        }
        let manager = self.manager.clone();
        tauri::async_runtime::spawn(async move {
            let request = AddRequest {
                url: download.url,
                headers,
                file_name,
                directory: None,
                start: true,
            };
            if let Err(error) = manager.add(request).await {
                tracing::warn!(%error, "could not add a download from the browser");
            }
        });
    }

    fn media(&self, source: media::MediaSource) {
        show_main_window(&self.app);
        let _ = self.app.emit(MEDIA_PROMPT_EVENT, source);
    }

    fn probe_media(
        &self,
        source: media::MediaSource,
    ) -> BoxFuture<Result<media::MediaInfo, String>> {
        let app = self.app.clone();
        Box::pin(async move { probe_page(&app, &source).await })
    }

    fn add_media(
        &self,
        source: media::MediaSource,
        option: String,
    ) -> BoxFuture<Result<u64, String>> {
        let app = self.app.clone();
        Box::pin(async move {
            let info = probe_page(&app, &source).await?;
            let option = info
                .options
                .iter()
                .find(|o| o.id == option)
                .cloned()
                .ok_or("this quality is no longer offered")?;
            let state = app.state::<AppState>();
            if option.needs_ffmpeg && state.tools.ffmpeg().is_none() {
                if !state.tools.status().ffmpeg_installable {
                    return Err("this quality needs ffmpeg; install it first".into());
                }
                let downloader = state.manager.downloader();
                state.tools.install_ffmpeg(&downloader, |_| {}).await?;
            }
            state.manager.add_media(media::AddMediaRequest {
                source,
                option,
                title: info.title,
                directory: None,
                start: true,
            })
        })
    }
}

/// What yt-dlp finds on a page, getting yt-dlp first if needed.
pub(crate) async fn probe_page(
    app: &AppHandle,
    source: &media::MediaSource,
) -> Result<media::MediaInfo, String> {
    let state = app.state::<AppState>();
    if state.tools.ytdlp().is_none() {
        let downloader = state.manager.downloader();
        state.tools.install_ytdlp(&downloader, |_| {}).await?;
    }
    let proxy = state.manager.settings().proxy;
    state.prober.probe(&state.tools, source, &proxy).await
}

/// Magnet links and `.torrent` files among command line arguments.
fn torrent_sources<S: AsRef<str>>(
    args: impl Iterator<Item = S>,
    cwd: &Path,
) -> Vec<torrent::TorrentSource> {
    args.filter_map(|arg| {
        let arg = arg.as_ref().trim();
        if torrent::is_magnet(arg) {
            return Some(torrent::TorrentSource {
                link: Some(arg.to_owned()),
                ..torrent::TorrentSource::default()
            });
        }
        let path = cwd.join(arg);
        (torrent::is_torrent_file_name(arg) && path.is_file()).then(|| torrent::TorrentSource {
            path: Some(path),
            ..torrent::TorrentSource::default()
        })
    })
    .collect()
}

/// Shows the "add torrent" dialog, or adds the whole torrent right away
/// when the user does not want to confirm downloads.
pub(crate) fn open_torrent(app: &AppHandle, source: torrent::TorrentSource) {
    let manager = app.state::<AppState>().manager.clone();
    if manager.settings().confirm_browser_downloads {
        show_main_window(app);
        let _ = app.emit(TORRENT_PROMPT_EVENT, source);
        return;
    }
    tauri::async_runtime::spawn(async move {
        let added = match manager.inspect_torrent(source).await {
            Ok(info) if info.existing.is_some() => return,
            Ok(info) => manager.add_torrent(torrent::AddTorrentRequest {
                info_hash: info.info_hash,
                start: true,
                ..torrent::AddTorrentRequest::default()
            }),
            Err(error) => Err(error),
        };
        if let Err(error) = added {
            tracing::warn!(%error, "could not add a torrent from the browser");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_torrents_in_arguments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Ubuntu.torrent"), b"d4:infod...").unwrap();
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
        let args = [
            "--minimized",
            magnet,
            "Ubuntu.torrent",
            "missing.torrent",
            "notes.txt",
        ];
        let found = torrent_sources(args.iter(), dir.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].link.as_deref(), Some(magnet));
        assert_eq!(
            found[1].path.as_deref(),
            Some(dir.path().join("Ubuntu.torrent").as_path())
        );
    }
}
