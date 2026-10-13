//! Bounded, opt-in UI installation in the selected browser's normal profile.

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::Browser;

pub(super) fn load(browser: &Browser, directory: &std::path::Path) -> Result<(), &'static str> {
    let Some(system) = std::env::var_os("SystemRoot") else {
        return Err("unavailable");
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
        .args(["-NoProfile", "-NonInteractive", "-Mta", "-Command"])
        .arg(include_str!("browser_install.ps1"))
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Err("unavailable");
    };
    if child
        .stdin
        .take()
        .is_none_or(|mut stream| stream.write_all(input.to_string().as_bytes()).is_err())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err("unavailable");
    }
    let deadline = Instant::now() + Duration::from_secs(24);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return match child.wait_with_output() {
                    Ok(output) if output.status.success() => match output.stdout.as_slice() {
                        b"loaded" => Ok(()),
                        b"manual:locate" => Err("locate"),
                        b"manual:locate-window" => Err("locate-window"),
                        b"manual:locate-address" => Err("locate-address"),
                        b"manual:locate-url" => Err("locate-url"),
                        b"manual:developer" => Err("developer"),
                        b"manual:load" => Err("load"),
                        b"manual:picker" => Err("picker"),
                        b"manual:confirm" => Err("confirm"),
                        b"manual:verify" => Err("verify"),
                        _ => Err("unavailable"),
                    },
                    _ => Err("unavailable"),
                };
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timeout");
            }
        }
    }
}
