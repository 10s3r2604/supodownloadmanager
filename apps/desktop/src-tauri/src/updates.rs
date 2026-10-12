//! Signed stable updates, downloaded in the background and installed only
//! after an idle download manager has durably saved its history.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager as _};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::manager::{Manager, UpdatePreparationError};

const EVENT: &str = "sdm:update";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePhase {
    Disabled,
    Idle,
    Checking,
    Downloading,
    Ready,
    Waiting,
    Installing,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub phase: UpdatePhase,
    pub supported: bool,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub downloaded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked: Option<u64>,
    /// Stable localization codes, never raw URLs or proxy credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct VerifiedUpdate {
    update: Update,
    /// Only `Update::download`, which verifies the configured Minisign
    /// signature, can create these bytes. They are never loaded from disk.
    bytes: Vec<u8>,
}

struct Inner {
    state: UpdateState,
    pending: Option<VerifiedUpdate>,
    next_check: Instant,
    /// An automatic install failure waits for an explicit user retry.
    install_failed: bool,
}

pub struct UpdateService<R: tauri::Runtime = tauri::Wry> {
    app: AppHandle<R>,
    manager: Arc<Manager>,
    inner: Mutex<Inner>,
    operation: tokio::sync::Mutex<()>,
    /// Only the mock-runtime tests can substitute a local fixture endpoint.
    #[cfg(test)]
    test_endpoint: Option<url::Url>,
}

impl<R: tauri::Runtime> UpdateService<R> {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state(&self) -> UpdateState {
        let mut state = self.lock().state.clone();
        if state.supported
            && !self.manager.settings().auto_update
            && matches!(state.phase, UpdatePhase::Idle | UpdatePhase::Disabled)
        {
            state.phase = UpdatePhase::Disabled;
        }
        state
    }

    fn change(&self, change: impl FnOnce(&mut Inner)) {
        let state = {
            let mut inner = self.lock();
            change(&mut inner);
            inner.state.clone()
        };
        let _ = self.app.emit(EVENT, state);
    }

    fn fail(&self, code: &'static str) {
        self.change(|inner| {
            inner.state.phase = UpdatePhase::Error;
            inner.state.error = Some(code.into());
        });
    }

    async fn check(self: &Arc<Self>) {
        let Ok(_operation) = self.operation.try_lock() else {
            return;
        };
        if !self.state().supported {
            return;
        }
        if self.lock().pending.is_some() {
            self.change(|inner| {
                inner.state.phase = UpdatePhase::Ready;
                inner.state.error = None;
                inner.install_failed = false;
            });
            return;
        }
        self.change(|inner| {
            inner.next_check = Instant::now() + CHECK_INTERVAL;
            inner.state.phase = UpdatePhase::Checking;
            inner.state.error = None;
            inner.state.version = None;
            inner.state.downloaded = 0;
            inner.state.total = None;
            inner.state.last_checked = Some(now_ms());
        });

        let settings = self.manager.settings();
        let builder = self
            .app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .configure_client(|client| client.https_only(true))
            .version_comparator(|current, release| {
                release.version.pre.is_empty() && release.version > current
            });
        #[cfg(test)]
        let builder = match self.test_endpoint.as_ref() {
            Some(endpoint) => match builder
                .configure_client(|client| client.https_only(false))
                // The real Linux installer replaces executable_path. Never
                // let a signed fixture point at the running test executable.
                .executable_path(
                    std::env::temp_dir()
                        .join(format!(
                            "sdm-updater-test-{}-{}",
                            std::process::id(),
                            now_ms()
                        ))
                        .join("Missing.app/Contents/MacOS/sdm"),
                )
                .endpoints(vec![endpoint.clone()])
            {
                Ok(builder) => builder,
                Err(_) => {
                    self.fail("manifest");
                    return;
                }
            },
            None => builder,
        };
        let builder = match settings.proxy.as_str() {
            "" => builder,
            "none" => builder.no_proxy(),
            proxy => match url::Url::parse(proxy) {
                Ok(proxy) => builder.proxy(proxy),
                Err(_) => {
                    self.fail("network");
                    return;
                }
            },
        };
        let updater = match builder.build() {
            Ok(updater) => updater,
            Err(_) => {
                self.fail("manifest");
                return;
            }
        };
        let result = tokio::time::timeout(CHECK_TIMEOUT, updater.check()).await;
        let mut update = match result {
            Ok(Ok(Some(update))) => update,
            Ok(Ok(None)) | Ok(Err(tauri_plugin_updater::Error::ReleaseNotFound)) => {
                // Before the first signed release there is no latest.json.
                // This is a normal initial state, not a repeating error toast.
                self.change(|inner| inner.state.phase = UpdatePhase::Idle);
                return;
            }
            Ok(Err(error)) => {
                self.fail(error_code(&error));
                return;
            }
            Err(_) => {
                self.fail("timeout");
                return;
            }
        };
        let insecure = update.download_url.scheme() != "https";
        #[cfg(test)]
        let insecure = insecure && self.test_endpoint.is_none();
        if insecure {
            self.fail("transport");
            return;
        }
        update.timeout = Some(DOWNLOAD_TIMEOUT);
        self.change(|inner| {
            inner.state.phase = UpdatePhase::Downloading;
            inner.state.version = Some(update.version.clone());
        });

        let too_large = tokio::sync::Notify::new();
        let mut last_event = Instant::now();
        let download = update.download(
            |count, total| {
                let state = {
                    let mut inner = self.lock();
                    inner.state.downloaded = inner.state.downloaded.saturating_add(count as u64);
                    inner.state.total = total;
                    if inner.state.downloaded > MAX_PACKAGE_BYTES
                        || total.is_some_and(|size| size > MAX_PACKAGE_BYTES)
                    {
                        too_large.notify_one();
                    }
                    if last_event.elapsed() >= Duration::from_millis(250) {
                        last_event = Instant::now();
                        Some(inner.state.clone())
                    } else {
                        None
                    }
                };
                if let Some(state) = state {
                    let _ = self.app.emit(EVENT, state);
                }
            },
            || {},
        );
        let bytes = tokio::select! {
            result = tokio::time::timeout(DOWNLOAD_TIMEOUT, download) => match result {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(error)) => {
                    self.fail(error_code(&error));
                    return;
                }
                Err(_) => {
                    self.fail("timeout");
                    return;
                }
            },
            _ = too_large.notified() => {
                self.fail("manifest");
                return;
            }
        };
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            self.fail("manifest");
            return;
        }
        self.change(|inner| {
            inner.state.downloaded = bytes.len() as u64;
            inner.state.phase = UpdatePhase::Ready;
            inner.pending = Some(VerifiedUpdate { update, bytes });
            inner.install_failed = false;
        });
    }

    /// Returns true only if installation succeeded. On Windows the official
    /// plugin exits after launching NSIS; failures keep the verified package
    /// available and release the download manager's scheduling reservation.
    async fn install(&self) -> bool {
        let Ok(_operation) = self.operation.try_lock() else {
            return false;
        };
        if self.lock().pending.is_none() {
            return false;
        }
        match self.manager.prepare_update() {
            Ok(()) => {}
            Err(UpdatePreparationError::Busy) => {
                self.change(|inner| {
                    inner.state.phase = UpdatePhase::Waiting;
                    inner.state.error = None;
                });
                return false;
            }
            Err(UpdatePreparationError::Persistence(error)) => {
                tracing::warn!(%error, "update installation blocked by history persistence");
                self.change(|inner| inner.install_failed = true);
                self.fail("storage");
                return false;
            }
        }
        let pending = self.lock().pending.take().expect("pending update reserved");
        self.change(|inner| {
            inner.state.phase = UpdatePhase::Installing;
            inner.state.error = None;
        });
        let installed = tauri::async_runtime::spawn_blocking(move || {
            let result = pending.update.install(&pending.bytes);
            (pending, result)
        })
        .await;
        match installed {
            Ok((_, Ok(()))) => {
                // Other supported desktop platforms must relaunch explicitly.
                self.app.restart();
            }
            Ok((pending, Err(_))) => {
                self.manager.cancel_update();
                self.change(|inner| {
                    inner.pending = Some(pending);
                    inner.install_failed = true;
                });
                self.fail("installer");
            }
            Err(_) => {
                self.manager.cancel_update();
                self.change(|inner| inner.install_failed = true);
                self.fail("installer");
            }
        }
        false
    }
}

pub fn start(app: AppHandle, manager: Arc<Manager>) {
    let service = Arc::new(UpdateService {
        inner: Mutex::new(Inner {
            state: UpdateState {
                phase: UpdatePhase::Idle,
                supported: cfg!(target_os = "windows"),
                current_version: app.package_info().version.to_string(),
                version: None,
                downloaded: 0,
                total: None,
                last_checked: None,
                error: None,
            },
            pending: None,
            next_check: Instant::now(),
            install_failed: false,
        }),
        app: app.clone(),
        manager,
        operation: tokio::sync::Mutex::new(()),
        #[cfg(test)]
        test_endpoint: None,
    });
    app.manage(service.clone());
    tauri::async_runtime::spawn(async move {
        // Let the main view, tray and recovery notices become ready first.
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            if service.state().supported && service.manager.settings().auto_update {
                let (pending, due, failed) = {
                    let inner = service.lock();
                    (
                        inner.pending.is_some(),
                        Instant::now() >= inner.next_check,
                        inner.install_failed,
                    )
                };
                if pending && !failed && all_windows_hidden(&service.app) {
                    service.install().await;
                } else if !pending && due {
                    service.check().await;
                }
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
}

fn all_windows_hidden<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    app.webview_windows()
        .values()
        .all(|window| matches!(window.is_visible(), Ok(false)))
}

/// Called before normal quit. A manual check with automatic updates disabled
/// never opts the user into installation on exit.
pub async fn install_on_exit(app: &AppHandle) -> bool {
    let Some(service) = app.try_state::<Arc<UpdateService>>() else {
        return false;
    };
    if !service.manager.settings().auto_update || service.lock().install_failed {
        return false;
    }
    service.install().await
}

#[tauri::command]
pub fn get_update_state(service: tauri::State<'_, Arc<UpdateService>>) -> UpdateState {
    service.state()
}

#[tauri::command]
pub fn check_for_updates(service: tauri::State<'_, Arc<UpdateService>>) -> UpdateState {
    let task = service.inner().clone();
    tauri::async_runtime::spawn(async move { task.check().await });
    service.state()
}

#[tauri::command]
pub async fn install_update(
    service: tauri::State<'_, Arc<UpdateService>>,
) -> Result<UpdateState, String> {
    service.install().await;
    Ok(service.state())
}

fn error_code(error: &tauri_plugin_updater::Error) -> &'static str {
    use tauri_plugin_updater::Error;
    match error {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => "signature",
        Error::Serialization(_)
        | Error::Semver(_)
        | Error::TargetNotFound(_)
        | Error::TargetsNotFound(_)
        | Error::EmptyEndpoints => "manifest",
        Error::InsecureTransportProtocol => "transport",
        Error::UnsupportedArch | Error::UnsupportedOs => "unsupported",
        _ => "network",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::{Json, Router, routing::get};
    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};

    use super::*;
    use crate::{media::Tools, settings::Settings, torrent::TorrentEngine};

    // A disposable fixture key signed these bytes, then its private half was
    // destroyed. These public values cannot authorize production updates.
    const BYTES: &[u8] = b"SDM signed updater fixture\n";
    const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDA4N0VERTFEMkJBRDg1QzUKUldURmhhMHJIZDUrQ0d6Tk5Ncit3eWlFeGxYSVRnM0ovRU5langzZGlRUzhjT1YwKzNSUEx1aEwK";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVURmhhMHJIZDUrQ0c4RW5BVi84NEdCSElBK3lkbW51ZkJEdi9SYmd5SkFnclFtRkJkRUZnZWE0OWpJeVdwR1I1UDUrRFl6OWVNWUFPb3ZrUUxudzZRVzlIK0pjeVJnelFJPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNzYyOTk5CWZpbGU6c2lnbmVkLXBhY2thZ2UJdmVyc2lvbjo5OS4wLjAKQjdaQUZ2YmRRYTBGdGphekZjL2ViQ05rdE1tWDd2M1BzZDRFdUVQSmNxeGNqaXc5RWVwM2RkVktrR1dTbjFuTWxBTFA3aWxkVmVSci9EOWlPb09wQmc9PQo=";

    struct Fixture {
        _app: tauri::App<MockRuntime>,
        dir: tempfile::TempDir,
        service: Arc<UpdateService<MockRuntime>>,
        manifest_requests: Arc<AtomicUsize>,
        package_requests: Arc<AtomicUsize>,
        manifest_entered: Arc<tokio::sync::Notify>,
        manifest_release: Arc<tokio::sync::Notify>,
    }

    impl Fixture {
        async fn new(version: &str, bytes: &[u8], hold_manifest: bool) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let manifest_requests = Arc::new(AtomicUsize::new(0));
            let package_requests = Arc::new(AtomicUsize::new(0));
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let manifest = serde_json::json!({
                "version": version,
                "url": format!("{base}/package"),
                "signature": SIGNATURE,
                "pub_date": "2026-10-12T00:00:00Z",
            });
            let requests = manifest_requests.clone();
            let ready = entered.clone();
            let blocked = release.clone();
            let package_count = package_requests.clone();
            let package = bytes.to_vec();
            let routes = Router::new()
                .route(
                    "/latest.json",
                    get(move || {
                        let requests = requests.clone();
                        let ready = ready.clone();
                        let blocked = blocked.clone();
                        let manifest = manifest.clone();
                        async move {
                            requests.fetch_add(1, Ordering::SeqCst);
                            ready.notify_one();
                            if hold_manifest {
                                blocked.notified().await;
                            }
                            Json(manifest)
                        }
                    }),
                )
                .route(
                    "/package",
                    get(move || {
                        let requests = package_count.clone();
                        let bytes = package.clone();
                        async move {
                            requests.fetch_add(1, Ordering::SeqCst);
                            bytes
                        }
                    }),
                )
                .route("/paused.zip", get(|| async { BYTES }));
            tokio::spawn(async move {
                axum::serve(listener, routes).await.unwrap();
            });

            let mut context = mock_context(noop_assets());
            context.config_mut().plugins.0.insert(
                "updater".into(),
                serde_json::json!({
                    "pubkey": PUBKEY,
                    "requireSignedVersion": true,
                    "dangerousInsecureTransportProtocol": true,
                }),
            );
            let app = mock_builder()
                .plugin(tauri_plugin_updater::Builder::new().build())
                .build(context)
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let settings = Settings {
                download_dir: dir.path().join("Downloads"),
                proxy: "none".into(),
                ..Settings::default()
            };
            let manager = Manager::new(
                dir.path().join("downloads.json"),
                settings,
                Arc::new(Tools::new(dir.path(), None)),
                TorrentEngine::new(
                    dir.path().join("torrent"),
                    crate::torrent::EngineConfig {
                        local_only: true,
                        ..crate::torrent::EngineConfig::default()
                    },
                ),
                tokio::runtime::Handle::current(),
                Box::new(|_| {}),
            )
            .unwrap();
            let service = Arc::new(UpdateService {
                app: app.handle().clone(),
                manager,
                inner: Mutex::new(Inner {
                    state: UpdateState {
                        phase: UpdatePhase::Idle,
                        supported: true,
                        current_version: "0.1.0".into(),
                        version: None,
                        downloaded: 0,
                        total: None,
                        last_checked: None,
                        error: None,
                    },
                    pending: None,
                    next_check: Instant::now(),
                    install_failed: false,
                }),
                operation: tokio::sync::Mutex::new(()),
                test_endpoint: Some(format!("{base}/latest.json").parse().unwrap()),
            });
            Self {
                _app: app,
                dir,
                service,
                manifest_requests,
                package_requests,
                manifest_entered: entered,
                manifest_release: release,
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn verifies_signed_package_before_making_an_update_ready() {
        let fixture = Fixture::new("99.0.0", BYTES, false).await;
        fixture.service.check().await;
        let state = fixture.service.state();
        assert_eq!(state.phase, UpdatePhase::Ready);
        assert_eq!(state.version.as_deref(), Some("99.0.0"));
        assert_eq!(state.downloaded, BYTES.len() as u64);
        assert!(state.last_checked.is_some());
        assert_eq!(
            fixture.service.lock().pending.as_ref().unwrap().bytes,
            BYTES
        );
        assert_eq!(fixture.package_requests.load(Ordering::SeqCst), 1);
        // Repeated checks preserve the verified package without redownloading.
        fixture.service.check().await;
        assert_eq!(fixture.package_requests.load(Ordering::SeqCst), 1);
        fixture.service.manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_tampered_bytes_and_manifest_version_mismatch() {
        for (version, bytes) in [("99.0.0", &b"tampered"[..]), ("99.0.1", BYTES)] {
            let fixture = Fixture::new(version, bytes, false).await;
            fixture.service.check().await;
            assert_eq!(fixture.service.state().phase, UpdatePhase::Error);
            assert_eq!(fixture.service.state().error.as_deref(), Some("signature"));
            assert!(fixture.service.lock().pending.is_none());
            fixture.service.manager.shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stable_channel_rejects_prereleases_and_downgrades() {
        for version in ["99.0.0-rc.1", "0.0.1"] {
            let fixture = Fixture::new(version, BYTES, false).await;
            fixture.service.check().await;
            assert_eq!(fixture.service.state().phase, UpdatePhase::Idle);
            assert_eq!(fixture.package_requests.load(Ordering::SeqCst), 0);
            assert!(fixture.service.lock().pending.is_none());
            fixture.service.manager.shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overlapping_checks_use_one_request_and_one_download() {
        let fixture = Fixture::new("99.0.0", BYTES, true).await;
        let service = fixture.service.clone();
        let check = tokio::spawn(async move {
            service.check().await;
        });
        tokio::time::timeout(Duration::from_secs(5), fixture.manifest_entered.notified())
            .await
            .unwrap();
        fixture.service.check().await;
        assert_eq!(fixture.service.state().phase, UpdatePhase::Checking);
        assert_eq!(fixture.manifest_requests.load(Ordering::SeqCst), 1);
        fixture.manifest_release.notify_one();
        check.await.unwrap();
        assert_eq!(fixture.service.state().phase, UpdatePhase::Ready);
        assert_eq!(fixture.package_requests.load(Ordering::SeqCst), 1);
        fixture.service.manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn storage_and_invalid_installer_failures_retain_package_and_allow_retry() {
        let fixture = Fixture::new("99.0.0", BYTES, false).await;
        let request = crate::manager::AddRequest {
            url: fixture
                .service
                .test_endpoint
                .as_ref()
                .unwrap()
                .join("/paused.zip")
                .unwrap()
                .to_string(),
            file_name: Some("paused.zip".into()),
            ..crate::manager::AddRequest::default()
        };
        let id = fixture.service.manager.add(request).await.unwrap();
        fixture.service.check().await;
        let backup = fixture.dir.path().join("downloads.json.bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        assert!(!fixture.service.install().await);
        assert_eq!(fixture.service.state().error.as_deref(), Some("storage"));
        assert!(fixture.service.lock().pending.is_some());
        assert!(fixture.service.lock().install_failed);
        std::fs::remove_dir(&backup).unwrap();
        fixture.service.check().await;
        assert_eq!(fixture.service.state().phase, UpdatePhase::Ready);
        assert!(
            !fixture.service.install().await,
            "fixture is signed data, never an executable installer"
        );
        assert_eq!(fixture.service.state().error.as_deref(), Some("installer"));
        assert!(fixture.service.lock().pending.is_some());
        assert_eq!(fixture.package_requests.load(Ordering::SeqCst), 1);
        // The failed installer released the gate, permitting ordinary work.
        fixture.service.manager.resume(&[id]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while fixture.service.manager.get(id).unwrap().state != crate::manager::State::Completed
            && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            fixture.service.manager.get(id).unwrap().state,
            crate::manager::State::Completed
        );
        assert_eq!(
            std::fs::read(fixture.service.manager.get(id).unwrap().path).unwrap(),
            BYTES
        );
        fixture.service.manager.shutdown().await;
    }
}
