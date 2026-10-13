// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = sdm_desktop::native_mode() {
        std::process::exit(code);
    }
    sdm_desktop::run();
}
