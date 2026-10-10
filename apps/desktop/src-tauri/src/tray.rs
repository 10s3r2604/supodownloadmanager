//! The icon in the system tray (notification area / menu bar).

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::AppState;
use crate::texts::texts;

const TRAY_ID: &str = "main";

pub fn create(app: &AppHandle, language: &str) -> tauri::Result<()> {
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Supo Download Manager")
        .menu(&menu(app, language)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => crate::show_main_window(app),
            "pause_all" => app.state::<AppState>().manager.pause_all(),
            "resume_all" => app.state::<AppState>().manager.resume_all(),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Rebuilds the menu after the language changed.
pub fn set_language(app: &AppHandle, language: &str) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), menu(app, language)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn menu(app: &AppHandle, language: &str) -> tauri::Result<Menu<Wry>> {
    let t = texts(language);
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "show", t.show, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "pause_all", t.pause_all, true, None::<&str>)?,
            &MenuItem::with_id(app, "resume_all", t.resume_all, true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", t.quit, true, None::<&str>)?,
        ],
    )
}
