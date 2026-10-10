//! User settings, stored as `settings.json` in the app data folder.

use std::path::{Path, PathBuf};

use sdm_core::{ClientConfig, ProxyConfig};
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
    /// `minimal`; old `neon` settings are migrated when loaded.
    pub skin: String,
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// Open a progress window for every download that starts.
    pub progress_window: bool,
    /// Look of the progress window: `graph`, `ring` or `classic`.
    pub progress_style: String,
    /// Show a "download complete" dialog when a download finishes.
    pub complete_dialog: bool,
    /// Keep running in the tray when the window is closed.
    pub close_to_tray: bool,
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
    /// Port for incoming BitTorrent connections, `0` to pick one.
    pub torrent_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: PathBuf::new(),
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
    /// Reads the settings, falling back to defaults when the file is
    /// missing or broken.
    pub fn load(path: &Path) -> Self {
        store::load_json::<Self>(path)
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        store::save_json(path, self)
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
        // Neon always displayed dark, including when its saved theme was light.
        if self.skin == "neon" {
            self.theme = "dark".into();
        }
        self.skin = "minimal".into();
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
    }

    #[test]
    fn migrates_neon_without_changing_download_preferences() {
        for theme in ["system", "light", "dark"] {
            let old = Settings {
                skin: "neon".into(),
                theme: theme.into(),
                download_dir: PathBuf::from("downloads"),
                connections: 16,
                max_parallel: 5,
                proxy: "none".into(),
                progress_window: false,
                ..Settings::default()
            };
            let migrated = old.clone().sanitized();
            assert_eq!(migrated.skin, "minimal");
            assert_eq!(migrated.theme, "dark");
            assert_eq!(migrated.download_dir, old.download_dir);
            assert_eq!(migrated.connections, old.connections);
            assert_eq!(migrated.max_parallel, old.max_parallel);
            assert_eq!(migrated.proxy, old.proxy);
            assert_eq!(migrated.progress_window, old.progress_window);
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
}
