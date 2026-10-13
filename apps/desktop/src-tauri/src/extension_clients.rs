//! Remember observed extensions separately from their live connection heartbeat.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Client {
    pub version: Option<String>,
}

pub struct Clients {
    path: PathBuf,
    records: Mutex<BTreeMap<String, Client>>,
    dirty: AtomicBool,
    persistence_allowed: bool,
}

pub fn version(value: &str) -> Option<Vec<u32>> {
    let parts: Vec<_> = value.split('.').collect();
    if !(1..=4).contains(&parts.len()) || value.len() > 24 {
        return None;
    }
    parts
        .into_iter()
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u32>().ok().filter(|part| *part <= 65535)
        })
        .collect()
}

fn supported(browser: &str) -> bool {
    matches!(
        browser,
        "chrome" | "edge" | "chromium" | "coccoc" | "brave" | "opera" | "vivaldi" | "firefox"
    )
}

pub fn newer(available: &[u32], installed: &[u32]) -> bool {
    let normalized = |version: &[u32]| {
        std::array::from_fn::<_, 4, _>(|index| version.get(index).copied().unwrap_or(0))
    };
    normalized(available) > normalized(installed)
}

impl Clients {
    pub fn load(path: PathBuf) -> Arc<Self> {
        let loaded =
            crate::store::load_json_validated::<BTreeMap<String, Client>>(&path, |records| {
                if records.len() > 8
                    || records.iter().any(|(browser, client)| {
                        !supported(browser)
                            || client
                                .version
                                .as_deref()
                                .is_some_and(|v| version(v).is_none())
                    })
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid extension status",
                    ));
                }
                Ok(())
            });
        let (loaded, persistence_allowed) = match loaded {
            Ok(loaded) => (loaded, true),
            Err(error) => {
                tracing::warn!("Could not read optional browser extension status: {error}");
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let damaged = path.with_extension(format!("json.damaged-{nonce}"));
                let preserved = error.kind() == io::ErrorKind::InvalidData
                    && std::fs::rename(&path, damaged).is_ok();
                (
                    crate::store::Loaded {
                        value: None,
                        damaged: None,
                        recovered_from: None,
                    },
                    preserved,
                )
            }
        };
        if let Some(notice) = loaded.recovery_notice("browser extension status") {
            tracing::warn!("{notice}");
        }
        Arc::new(Self {
            path,
            records: Mutex::new(loaded.value.unwrap_or_default()),
            dirty: AtomicBool::new(false),
            persistence_allowed,
        })
    }

    pub fn get(&self, browser: &str) -> Option<Client> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(browser)
            .cloned()
    }

    pub fn observe(&self, browser: &str, reported: Option<&str>) {
        if !supported(browser) {
            return;
        }
        let reported = reported
            .filter(|value| version(value).is_some())
            .map(str::to_owned);
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let previous = records.get(browser);
        let version = reported.or_else(|| previous.and_then(|client| client.version.clone()));
        if previous.is_some_and(|client| client.version == version)
            && !self.dirty.load(Ordering::Acquire)
        {
            return;
        }
        records.insert(browser.to_owned(), Client { version });
        if !self.persistence_allowed {
            return;
        }
        if let Err(error) = crate::store::save_json(&self.path, &*records) {
            self.dirty.store(true, Ordering::Release);
            tracing::warn!("Could not save browser extension status: {error}");
        } else {
            self.dirty.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_installation_survives_restart_and_old_clients_do_not_erase_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let clients = Clients::load(path.clone());
        clients.observe("chrome", Some("0.6.5"));
        clients.observe("chrome", None);
        drop(clients);
        let clients = Clients::load(path);
        assert_eq!(
            clients.get("chrome").unwrap().version.as_deref(),
            Some("0.6.5")
        );
        assert!(clients.get("edge").is_none());
    }

    #[test]
    fn invalid_versions_and_browser_names_cannot_grow_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let clients = Clients::load(dir.path().join("extensions.json"));
        clients.observe("website", Some("0.6.6"));
        clients.observe("chrome", Some("../../private"));
        assert!(clients.get("website").is_none());
        assert!(clients.get("chrome").unwrap().version.is_none());
        assert_eq!(version("0.6.10").unwrap(), vec![0, 6, 10]);
        assert!(newer(&[0, 6, 10], &[0, 6, 9]));
        assert!(!newer(&[0, 6, 6], &[0, 6, 6, 0]));
        for invalid in ["", "0..6", "1.2.3.4.5", "65536", "1-beta"] {
            assert!(version(invalid).is_none());
        }
    }

    #[test]
    fn damaged_optional_status_is_preserved_without_blocking_startup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        std::fs::write(&path, "not-json").unwrap();
        let clients = Clients::load(path.clone());
        assert!(clients.get("chrome").is_none());
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("damaged")
        }));
        clients.observe("chrome", Some("0.6.6"));
        assert_eq!(
            Clients::load(path)
                .get("chrome")
                .unwrap()
                .version
                .as_deref(),
            Some("0.6.6")
        );
    }
}
