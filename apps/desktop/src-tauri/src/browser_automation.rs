//! Bounded, opt-in UI installation in the selected browser's normal profile.

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::Browser;

pub(super) fn load(browser: &Browser, directory: &std::path::Path) -> bool {
    let Some(system) = std::env::var_os("SystemRoot") else {
        return false;
    };
    let executable =
        std::path::PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let input = serde_json::json!({
        "executable": browser.executable,
        "page": browser.extensions_page,
        "engine": browser.engine,
        "directory": directory,
        "version": super::extension_version(directory)
            .unwrap_or_default()
            .iter().map(u32::to_string).collect::<Vec<_>>().join("."),
    });
    let Ok(mut child) = Command::new(executable)
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(include_str!("browser_install.ps1"))
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if child
        .stdin
        .take()
        .is_none_or(|mut stream| stream.write_all(input.to_string().as_bytes()).is_err())
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(24);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .is_ok_and(|output| output.status.success() && output.stdout == b"loaded");
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
