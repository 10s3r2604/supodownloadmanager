//! The small per-download progress windows, like the ones IDM shows.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const WIDTH: f64 = 500.0;
const HEIGHT: f64 = 268.0;

fn label(id: u64) -> String {
    format!("progress-{id}")
}

/// Whether a window belongs to a download instead of being the main window.
pub fn is_progress_window(label: &str) -> bool {
    label.starts_with("progress-")
}

/// Shows the progress window of a download, creating it when needed.
pub fn open_progress(app: &AppHandle, id: u64) {
    if let Some(window) = app.get_webview_window(&label(id)) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let url = WebviewUrl::App(format!("index.html#progress/{id}").into());
    let result = WebviewWindowBuilder::new(app, label(id), url)
        .title("Supo Download Manager")
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .center()
        .build();
    if let Err(error) = result {
        tracing::warn!(%error, id, "could not open the progress window");
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
}
