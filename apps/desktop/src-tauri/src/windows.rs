//! The small per-download progress windows.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::AppState;
use crate::settings::Settings;

fn label(id: u64) -> String {
    format!("progress-{id}")
}

/// Whether a window belongs to a download instead of being the main window.
pub fn is_progress_window(label: &str) -> bool {
    label.starts_with("progress-")
}

/// Starting size of each window style. The page then fits the height to
/// what it shows.
fn size_of(style: &str) -> (f64, f64) {
    match style {
        "ring" => (460.0, 214.0),
        "classic" => (520.0, 392.0),
        _ => (480.0, 318.0),
    }
}

/// Shows the progress window of a download, creating it when needed.
pub fn open_progress(app: &AppHandle, id: u64) {
    if let Some(window) = app.get_webview_window(&label(id)) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let style = app.state::<AppState>().manager.settings().progress_style;
    let (width, height) = size_of(&style);
    let url = WebviewUrl::App(format!("index.html#progress/{id}").into());
    // No system title bar: the page draws its own, so the window looks the
    // same everywhere. The shadow also gives rounded corners on Windows 11.
    let result = WebviewWindowBuilder::new(app, label(id), url)
        .title("Supo Download Manager")
        .inner_size(width, height)
        // Resizable only so the page can fit the height (GTK refuses to
        // shrink fixed windows); min = max keeps users from resizing it.
        .min_inner_size(width, height)
        .max_inner_size(width, height)
        .maximizable(false)
        .decorations(false)
        .shadow(true)
        .center()
        .build();
    if let Err(error) = result {
        tracing::warn!(%error, id, "could not open the progress window");
    }
}

/// Closes the progress window of a download, if it is open.
pub fn close_progress(app: &AppHandle, id: u64) {
    if let Some(window) = app.get_webview_window(&label(id)) {
        let _ = window.close();
    }
}

/// What to do when a download finishes, chosen in its progress window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhenDone {
    /// Show the "download complete" dialog.
    pub show_dialog: bool,
    /// Open the file.
    pub open_file: bool,
}

impl WhenDone {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            show_dialog: settings.complete_dialog,
            open_file: false,
        }
    }
}

/// The choices made for downloads that are still running.
#[derive(Default)]
pub struct WhenDoneChoices(Mutex<HashMap<u64, WhenDone>>);

impl WhenDoneChoices {
    pub fn get(&self, id: u64, settings: &Settings) -> WhenDone {
        self.lock()
            .get(&id)
            .copied()
            .unwrap_or_else(|| WhenDone::from_settings(settings))
    }

    pub fn set(&self, id: u64, choice: WhenDone) {
        self.lock().insert(id, choice);
    }

    /// Returns the choice for a finished download and forgets it.
    pub fn take(&self, id: u64, settings: &Settings) -> WhenDone {
        self.lock()
            .remove(&id)
            .unwrap_or_else(|| WhenDone::from_settings(settings))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, WhenDone>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_progress_windows() {
        assert!(is_progress_window(&label(7)));
        assert!(!is_progress_window("main"));
    }

    #[test]
    fn remembers_choices_until_the_download_is_done() {
        let settings = Settings {
            complete_dialog: false,
            ..Settings::default()
        };
        let choices = WhenDoneChoices::default();
        assert_eq!(
            choices.get(1, &settings),
            WhenDone::from_settings(&settings)
        );

        let chosen = WhenDone {
            show_dialog: true,
            open_file: true,
        };
        choices.set(1, chosen);
        assert_eq!(choices.get(1, &settings), chosen);
        assert_eq!(choices.take(1, &settings), chosen);
        assert!(!choices.take(1, &settings).show_dialog);
    }
}
