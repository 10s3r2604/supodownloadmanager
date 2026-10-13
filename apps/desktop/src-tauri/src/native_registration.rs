//! Per-user browser host registration. No browser profile or policy changes.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const HOST: &str = "com.supodownloadmanager.bridge";
pub const FIREFOX_ID: &str = "sdm@supodownloadmanager.github.io";
pub const IDENTIFIER: &str = "com.supodownloadmanager.app";
static REGISTER_LOCK: Mutex<()> = Mutex::new(());

pub fn data_dir() -> io::Result<PathBuf> {
    dirs::data_dir()
        .map(|path| path.join(IDENTIFIER))
        .ok_or_else(|| io::Error::other("The application data folder is unavailable"))
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

/// Chromium hashes the native path bytes, uppercasing the Windows drive letter.
/// Keep unpacked identities unchanged, including their existing preferences.
pub fn unpacked_id(path: &Path) -> String {
    #[cfg(windows)]
    let bytes = {
        use std::os::windows::ffi::OsStrExt;
        let path = path.to_string_lossy();
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
        let mut units: Vec<u16> = std::ffi::OsStr::new(path).encode_wide().collect();
        if units.len() > 1
            && units[1] == u16::from(b':')
            && (u16::from(b'a')..=u16::from(b'z')).contains(&units[0])
        {
            units[0] -= u16::from(b'a' - b'A');
        }
        units
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    Sha256::digest(bytes)[..16]
        .iter()
        .flat_map(|byte| {
            [
                char::from(b'a' + (byte >> 4)),
                char::from(b'a' + (byte & 15)),
            ]
        })
        .collect()
}

pub fn manifest_path(data: &Path, firefox: bool) -> PathBuf {
    data.join("native-messaging")
        .join(if firefox { "firefox" } else { "chromium" })
        .join(format!("{HOST}.json"))
}

pub fn allowed_origin(data: &Path, origin: &str) -> bool {
    std::fs::read(manifest_path(data, false))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["allowed_origins"].as_array().cloned())
        .is_some_and(|origins| origins.iter().any(|item| item.as_str() == Some(origin)))
}

pub(super) fn executable() -> io::Result<PathBuf> {
    // An AppImage mount disappears when the app exits. Register its stable path.
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("APPIMAGE") {
        let path = PathBuf::from(path);
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
    }
    std::env::current_exe()
}

fn manifests(data: &Path, executable: &Path, extra_id: Option<&str>) -> io::Result<(Value, Value)> {
    let mut ids = Vec::<String>::new();
    if let Ok(bytes) = std::fs::read(manifest_path(data, false))
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        && let Some(origins) = value["allowed_origins"].as_array()
    {
        for origin in origins.iter().filter_map(Value::as_str) {
            if let Some(id) = origin
                .strip_prefix("chrome-extension://")
                .and_then(|s| s.strip_suffix('/'))
                && valid_id(id)
                && !ids.iter().any(|saved| saved == id)
            {
                ids.push(id.to_owned());
            }
        }
    }
    let folder = data.join("extension/chromium");
    for id in [
        Some(unpacked_id(&folder)),
        folder.canonicalize().ok().map(|p| unpacked_id(&p)),
        extra_id.map(str::to_owned),
    ]
    .into_iter()
    .flatten()
    {
        if !valid_id(&id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid extension identity",
            ));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    // Bound stale identities without dropping the current bundle or new client.
    if ids.len() > 32 {
        ids.drain(..ids.len() - 32);
    }
    let base = json!({"name":HOST,"description":"Supo Download Manager browser connection", "path":executable,"type":"stdio"});
    let mut chromium = base.clone();
    chromium["allowed_origins"] = json!(
        ids.iter()
            .map(|id| format!("chrome-extension://{id}/"))
            .collect::<Vec<_>>()
    );
    let mut firefox = base;
    firefox["allowed_extensions"] = json!([FIREFOX_ID]);
    Ok((chromium, firefox))
}

pub fn register(data: &Path, extra_id: Option<&str>) -> io::Result<()> {
    let _lock = REGISTER_LOCK
        .lock()
        .map_err(|_| io::Error::other("Browser registration failed"))?;
    let (chromium, firefox) = manifests(data, &executable()?, extra_id)?;
    crate::store::save_json(&manifest_path(data, false), &chromium)?;
    crate::store::save_json(&manifest_path(data, true), &firefox)?;
    #[cfg(windows)]
    for vendor in [
        r"Google\Chrome",
        r"Chromium",
        r"Microsoft\Edge",
        r"BraveSoftware\Brave-Browser",
        r"CocCoc\Browser",
        r"Opera Software",
        r"Vivaldi",
    ] {
        let key = windows_registry::CURRENT_USER
            .create(format!(r"Software\{vendor}\NativeMessagingHosts\{HOST}"))?;
        key.set_string("", manifest_path(data, false).to_string_lossy().as_ref())?;
    }
    #[cfg(windows)]
    {
        let key = windows_registry::CURRENT_USER
            .create(format!(r"Software\Mozilla\NativeMessagingHosts\{HOST}"))?;
        key.set_string("", manifest_path(data, true).to_string_lossy().as_ref())?;
    }
    #[cfg(unix)]
    {
        let home =
            dirs::home_dir().ok_or_else(|| io::Error::other("The user folder is unavailable"))?;
        #[cfg(target_os = "macos")]
        let root = home.join("Library/Application Support");
        #[cfg(not(target_os = "macos"))]
        let root = dirs::config_dir()
            .ok_or_else(|| io::Error::other("The configuration folder is unavailable"))?;
        #[cfg(target_os = "macos")]
        let vendors = [
            "Google/Chrome",
            "Chromium",
            "Microsoft Edge",
            "BraveSoftware/Brave-Browser",
            "CocCoc",
            "com.operasoftware.Opera",
            "Vivaldi",
        ];
        #[cfg(not(target_os = "macos"))]
        let vendors = [
            "google-chrome",
            "chromium",
            "microsoft-edge",
            "BraveSoftware/Brave-Browser",
            "coccoc-browser",
            "opera",
            "vivaldi",
        ];
        for vendor in vendors {
            crate::store::save_json(
                &root
                    .join(vendor)
                    .join("NativeMessagingHosts")
                    .join(format!("{HOST}.json")),
                &chromium,
            )?;
        }
        #[cfg(target_os = "macos")]
        let firefox_dir = root.join("Mozilla/NativeMessagingHosts");
        #[cfg(not(target_os = "macos"))]
        let firefox_dir = home.join(".mozilla/native-messaging-hosts");
        crate::store::save_json(&firefox_dir.join(format!("{HOST}.json")), &firefox)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_valid_unpackaged_identities_and_rejects_arbitrary_ids() {
        let dir = tempfile::tempdir().unwrap();
        let id = "abcdefghijklmnopabcdefghijklmnop";
        let (chrome, firefox) = manifests(dir.path(), Path::new("/fixture/sdm"), Some(id)).unwrap();
        assert!(
            chrome["allowed_origins"]
                .as_array()
                .unwrap()
                .contains(&json!(format!("chrome-extension://{id}/")))
        );
        assert_eq!(firefox["allowed_extensions"], json!([FIREFOX_ID]));
        crate::store::save_json(&manifest_path(dir.path(), false), &chrome).unwrap();
        let (next, _) = manifests(dir.path(), Path::new("/new/sdm"), None).unwrap();
        assert_eq!(next["allowed_origins"], chrome["allowed_origins"]);
        assert_eq!(next["path"], "/new/sdm");
        assert!(manifests(dir.path(), Path::new("/sdm"), Some("../../other")).is_err());
        assert!(allowed_origin(
            dir.path(),
            &format!("chrome-extension://{id}/")
        ));
        assert!(!allowed_origin(dir.path(), "https://example.com"));
    }
}
