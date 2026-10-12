//! User settings, stored as `settings.json` in the app data folder.

use std::path::{Path, PathBuf};

use sdm_core::{ClientConfig, Downloader, ProxyConfig};
use serde::{Deserialize, Serialize};

use crate::category::{Category, default_capture_extensions};
use crate::store;

/// Port of the local bridge the browser extension talks to.
pub const DEFAULT_BRIDGE_PORT: u16 = 17360;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Where downloads are saved.
    pub download_dir: PathBuf,
    /// Persistent temporary data for unfinished HTTP and media downloads.
    /// Changing this affects new downloads; existing records retain their root.
    pub temp_dir: PathBuf,
    /// Save into `Video`, `Music`, ... sub-folders.
    pub category_folders: bool,
    /// Connections per download.
    pub connections: u16,
    /// Downloads running at the same time; the rest wait in the queue.
    pub max_parallel: u16,
    /// Total speed limit in bytes per second, `0` for unlimited.
    pub speed_limit: u64,
    /// Empty for the system proxy, `none` for no proxy, or a proxy URL.
    pub proxy: String,
    /// `auto`, `en` or `vi`.
    pub language: String,
    /// Look of the app: `minimal` or `neon`.
    pub skin: String,
    /// `system`, `light` or `dark` (Neon is always dark).
    pub theme: String,
    /// Open a progress window for every download that starts.
    pub progress_window: bool,
    /// Look of the progress window: `graph`, `ring` or `classic`.
    pub progress_style: String,
    /// Show a "download complete" dialog when a download finishes.
    pub complete_dialog: bool,
    /// Keep running in the tray when the window is closed.
    pub close_to_tray: bool,
    /// Start in the tray when this user signs in to the computer.
    pub start_with_windows: bool,
    /// Check for and install verified stable releases when no download runs.
    pub auto_update: bool,
    /// Show a notification when a download finishes.
    pub notifications: bool,
    /// Offer to download links copied to the clipboard.
    pub clipboard_monitor: bool,
    /// Accept downloads from the browser extension.
    pub browser_integration: bool,
    /// Show the "add download" dialog for downloads from the browser.
    pub confirm_browser_downloads: bool,
    /// Port of the local bridge for the browser extension.
    pub bridge_port: u16,
    /// File types the browser extension hands over to SDM.
    pub capture_extensions: Vec<String>,
    /// Share finished torrents until this many percent of their size were
    /// uploaded: `0` for not at all, `65535` until SDM quits.
    pub seed_ratio: u16,
    /// Upload limit for torrents in bytes per second, `0` for unlimited.
    pub upload_limit: u64,
    /// Port for incoming Torrent connections, `0` to pick one.
    pub torrent_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: PathBuf::new(),
            temp_dir: PathBuf::new(),
            category_folders: true,
            connections: 8,
            max_parallel: 3,
            speed_limit: 0,
            proxy: String::new(),
            language: "en".into(),
            skin: "minimal".into(),
            theme: "dark".into(),
            progress_window: true,
            progress_style: "graph".into(),
            complete_dialog: true,
            close_to_tray: true,
            start_with_windows: false,
            auto_update: true,
            notifications: true,
            clipboard_monitor: true,
            browser_integration: true,
            confirm_browser_downloads: true,
            bridge_port: DEFAULT_BRIDGE_PORT,
            capture_extensions: default_capture_extensions(),
            seed_ratio: 100,
            upload_limit: 0,
            torrent_port: 0,
        }
    }
}

impl Settings {
    /// Defaults are used only for a missing file. A malformed proxy from an
    /// older release is repaired without discarding the user's other settings.
    pub fn load(path: &Path) -> std::io::Result<(Self, Option<String>)> {
        match store::load_json_validated::<Self>(path, |settings| settings.validate()) {
            Ok(loaded) => {
                let notice = loaded.recovery_notice("settings");
                Ok((loaded.value.unwrap_or_default().sanitized(), notice))
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                // Reading without proxy validation still rejects broken JSON
                // and I/O failures. Only the invalid proxy field is repaired.
                let Some(mut settings) = store::load_json::<Self>(path)? else {
                    return Err(error);
                };
                settings = settings.sanitized();
                if settings.validate().is_ok() {
                    return Err(error);
                }
                settings.proxy.clear();
                settings.validate()?;
                let damaged = store::repair_json(path, &settings)?;
                Ok((
                    settings,
                    Some(format!(
                        "The saved proxy address was invalid and has been reset to the system proxy. Other settings were kept. The original settings were preserved at {}. Open Settings to choose a proxy.",
                        damaged.display()
                    )),
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        self.validate()?;
        store::save_json(path, self)
    }

    fn validate(&self) -> std::io::Result<()> {
        Downloader::new(self.client_config())
            .map(|_| ())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    /// Brings every value into its valid range.
    pub fn sanitized(mut self) -> Self {
        self.connections = self.connections.clamp(1, 32);
        self.max_parallel = self.max_parallel.clamp(1, 16);
        if self.bridge_port < 1024 {
            self.bridge_port = DEFAULT_BRIDGE_PORT;
        }
        if !["auto", "en", "vi"].contains(&self.language.as_str()) {
            self.language = "en".into();
        }
        if !["minimal", "neon"].contains(&self.skin.as_str()) {
            self.skin = "minimal".into();
        }
        if !["graph", "ring", "classic"].contains(&self.progress_style.as_str()) {
            self.progress_style = "graph".into();
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            self.theme = "dark".into();
        }
        self.proxy = self.proxy.trim().to_owned();
        if self.torrent_port < 1024 {
            self.torrent_port = 0;
        }
        let mut extensions: Vec<String> = self
            .capture_extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .collect();
        extensions.sort();
        extensions.dedup();
        self.capture_extensions = extensions;
        self
    }

    pub fn speed_limit(&self) -> Option<u64> {
        (self.speed_limit > 0).then_some(self.speed_limit)
    }

    pub fn torrent_config(&self) -> crate::torrent::EngineConfig {
        crate::torrent::EngineConfig {
            port: self.torrent_port,
            download_limit: self.speed_limit(),
            upload_limit: (self.upload_limit > 0).then_some(self.upload_limit),
            proxy: self.proxy.clone(),
            seed_ratio: self.seed_ratio,
            local_only: false,
        }
    }

    pub fn client_config(&self) -> ClientConfig {
        ClientConfig {
            proxy: match self.proxy.as_str() {
                "" => ProxyConfig::System,
                "none" => ProxyConfig::Disabled,
                url => ProxyConfig::Url(url.to_owned()),
            },
            speed_limit: self.speed_limit(),
            ..ClientConfig::default()
        }
    }

    /// Folder for new downloads of the given category.
    pub fn directory_for(&self, category: Category) -> PathBuf {
        match category.folder() {
            Some(folder) if self.category_folders => self.download_dir.join(folder),
            _ => self.download_dir.clone(),
        }
    }

    /// The language to use for texts produced by the backend.
    pub fn resolved_language(&self) -> &'static str {
        match self.language.as_str() {
            "vi" => "vi",
            "en" => "en",
            _ => match sys_locale::get_locale() {
                Some(locale) if locale.to_ascii_lowercase().starts_with("vi") => "vi",
                _ => "en",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_values() {
        let settings = Settings {
            connections: 0,
            max_parallel: 99,
            bridge_port: 80,
            language: "fr".into(),
            skin: "glossy".into(),
            progress_style: "fancy".into(),
            capture_extensions: vec![".ZIP".into(), "zip".into(), "".into(), "a b".into()],
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(settings.connections, 1);
        assert_eq!(settings.max_parallel, 16);
        assert_eq!(settings.bridge_port, DEFAULT_BRIDGE_PORT);
        assert_eq!(settings.language, "en");
        assert_eq!(settings.skin, "minimal");
        assert_eq!(settings.progress_style, "graph");
        assert_eq!(settings.capture_extensions, ["zip"]);
    }

    #[test]
    fn category_folders() {
        let settings = Settings {
            download_dir: PathBuf::from("dl"),
            ..Settings::default()
        };
        assert_eq!(
            settings.directory_for(Category::Video),
            Path::new("dl").join("Video")
        );
        assert_eq!(settings.directory_for(Category::Other), Path::new("dl"));
        let flat = Settings {
            category_folders: false,
            ..settings
        };
        assert_eq!(flat.directory_for(Category::Video), Path::new("dl"));
    }

    #[test]
    fn missing_fields_use_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"connections": 4}"#).unwrap();
        assert_eq!(settings.connections, 4);
        assert_eq!(settings.max_parallel, 3);
        assert!(settings.browser_integration);
        assert_eq!(settings.skin, "minimal");
        assert_eq!(settings.theme, "dark");
        assert!(!settings.start_with_windows);
        assert!(settings.auto_update);
    }

    #[test]
    fn keeps_both_looks_and_their_themes() {
        for theme in ["system", "light", "dark"] {
            let neon = Settings {
                skin: "neon".into(),
                theme: theme.into(),
                ..Settings::default()
            }
            .sanitized();
            assert_eq!(neon.skin, "neon");
            assert_eq!(neon.theme, theme);
        }
        for theme in ["light", "system"] {
            let settings = Settings {
                theme: theme.into(),
                ..Settings::default()
            }
            .sanitized();
            assert_eq!(settings.theme, theme);
        }
    }

    #[test]
    fn proxy_modes() {
        let mut settings = Settings::default();
        assert_eq!(settings.client_config().proxy, ProxyConfig::System);
        settings.proxy = "none".into();
        assert_eq!(settings.client_config().proxy, ProxyConfig::Disabled);
        settings.proxy = "socks5://127.0.0.1:1080".into();
        assert_eq!(
            settings.client_config().proxy,
            ProxyConfig::Url("socks5://127.0.0.1:1080".into())
        );
    }

    #[test]
    fn invalid_saved_proxy_recovers_valid_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let good = Settings {
            proxy: "none".into(),
            connections: 19,
            language: "vi".into(),
            ..Settings::default()
        }
        .sanitized();
        good.save(&path).unwrap();
        let bad = Settings {
            proxy: "http://[".into(),
            ..good.clone()
        };
        let bytes = serde_json::to_vec(&bad).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (loaded, notice) = Settings::load(&path).unwrap();
        assert_eq!(loaded, good);
        assert!(notice.unwrap().contains("preserved"));
        let damaged = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(".damaged-"))
            .unwrap();
        assert_eq!(std::fs::read(damaged).unwrap(), bytes);
    }

    #[test]
    fn invalid_proxy_without_backup_repairs_only_proxy_and_retains_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings {
            proxy: "http://[".into(),
            connections: 19,
            language: "vi".into(),
            temp_dir: dir.path().join("custom-temp"),
            ..Settings::default()
        }
        .sanitized();
        let bytes = serde_json::to_vec(&settings).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (loaded, notice) = Settings::load(&path).unwrap();
        settings.proxy.clear();
        assert_eq!(loaded, settings);
        assert!(notice.unwrap().contains("system proxy"));
        let damaged = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(".damaged-"))
            .unwrap();
        assert_eq!(std::fs::read(damaged).unwrap(), bytes);
        assert_eq!(Settings::load(&path).unwrap().0, settings);
    }

    #[test]
    fn unreadable_settings_are_preserved_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"{broken").unwrap();
        let error = Settings::load(&path).unwrap_err();
        assert!(error.to_string().contains("settings.json"));
        assert_eq!(std::fs::read(path).unwrap(), b"{broken");
    }
}
