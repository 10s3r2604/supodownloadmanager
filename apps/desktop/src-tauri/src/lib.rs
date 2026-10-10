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
mod tray;
mod windows;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager as _, RunEvent, Window, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::bridge::{Bridge, BridgeConfig, BrowserDownload, Handler};
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

pub struct AppState {
    pub manager: Arc<Manager>,
    pub bridge: Arc<Bridge>,
    pub bridge_port: u16,
    pub bridge_error: Mutex<Option<String>>,
    pub data_dir: PathBuf,
    /// yt-dlp and ffmpeg for videos from web pages.
    pub tools: Arc<media::Tools>,
    /// What to do when running downloads finish, chosen in their windows.
    pub when_done: windows::WhenDoneChoices,
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
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
            commands::add_media,
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
        .run(|app, event| {
            // Closing the last window should still save all progress.
            if let RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
                quit(app);
            }
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
    let tools = Arc::new(media::Tools::new(&data_dir));
    let manager = Manager::new(
        data_dir.join("downloads.json"),
        settings.clone(),
        tools.clone(),
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
        when_done: windows::WhenDoneChoices::default(),
        quitting: AtomicBool::new(false),
    });
    tray::create(app, settings.resolved_language())?;

    // Sites change all the time; keep our copy of yt-dlp fresh.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        tools.update_ytdlp().await;
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
}
