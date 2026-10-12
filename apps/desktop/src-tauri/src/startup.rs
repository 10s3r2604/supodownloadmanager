//! Per-user login startup and the settings/OS registration transaction.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager as _};
#[cfg(not(windows))]
use tauri_plugin_autostart::ManagerExt;

use crate::manager::Manager;
use crate::settings::Settings;

pub(crate) const AUTOSTART_ARGUMENT: &str = "--autostart";
pub(crate) const STARTUP_ENTRY_NAME: &str = "Supo Download Manager";

pub(crate) fn is_autostart<S: AsRef<str>>(args: impl IntoIterator<Item = S>) -> bool {
    args.into_iter()
        .any(|argument| argument.as_ref() == AUTOSTART_ARGUMENT)
}

/// Serialize reads and saves so a UI poll cannot observe an OS change before
/// its corresponding durable settings transaction succeeds or is rolled back.
#[derive(Default)]
pub(crate) struct StartupSettings {
    transaction: Mutex<()>,
}

impl StartupSettings {
    pub(crate) fn read(&self, app: &AppHandle, manager: &Arc<Manager>) -> Result<Settings, String> {
        let _guard = self
            .transaction
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut settings = manager.settings();
        let backend = native::NativeStartup::new(app);
        let snapshot = backend
            .capture()
            .map_err(|error| startup_error(settings.resolved_language(), "read", &error))?;
        settings.start_with_windows = native::NativeStartup::snapshot_enabled(&snapshot)
            .map_err(|error| startup_error(settings.resolved_language(), "read", &error))?;
        Ok(settings)
    }

    pub(crate) fn save(
        &self,
        app: &AppHandle,
        manager: &Arc<Manager>,
        path: &Path,
        settings: &Settings,
    ) -> Result<(), String> {
        let _guard = self
            .transaction
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Reject invalid network configuration before touching OS startup.
        sdm_core::Downloader::new(settings.client_config()).map_err(|error| error.to_string())?;
        save_transaction(
            &native::NativeStartup::new(app),
            settings.start_with_windows,
            settings.resolved_language(),
            || {
                manager
                    .save_settings(path, settings.clone())
                    .map_err(|error| error.to_string())
            },
        )
    }
}

trait StartupBackend {
    type Snapshot;
    fn capture(&self) -> Result<Self::Snapshot, String>;
    fn snapshot_enabled(snapshot: &Self::Snapshot) -> Result<bool, String>;
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
    fn restore(&self, snapshot: &Self::Snapshot) -> Result<(), String>;
}

#[cfg(not(windows))]
mod native {
    use super::*;

    pub(super) struct NativeStartup<'a>(&'a AppHandle);

    impl<'a> NativeStartup<'a> {
        pub(super) fn new(app: &'a AppHandle) -> Self {
            Self(app)
        }
    }

    impl StartupBackend for NativeStartup<'_> {
        type Snapshot = bool;
        fn capture(&self) -> Result<bool, String> {
            self.0
                .autolaunch()
                .is_enabled()
                .map_err(|error| error.to_string())
        }
        fn snapshot_enabled(snapshot: &bool) -> Result<bool, String> {
            Ok(*snapshot)
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), String> {
            let startup = self.0.autolaunch();
            let result = if enabled {
                startup.enable()
            } else {
                startup.disable()
            };
            result.map_err(|error| error.to_string())
        }
        fn restore(&self, snapshot: &bool) -> Result<(), String> {
            self.set_enabled(*snapshot)
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_registry::{CURRENT_USER, Type, Value};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    /// Preserve raw types and bytes, including a Task Manager disable override,
    /// so a failed save restores the exact registration rather than recreating
    /// a merely similar enabled/disabled state.
    pub(super) struct Snapshot {
        run: Option<Value>,
        approved: Option<Value>,
    }

    pub(super) struct NativeStartup;
    impl NativeStartup {
        pub(super) fn new(_app: &AppHandle) -> Self {
            Self
        }
    }

    impl StartupBackend for NativeStartup {
        type Snapshot = Snapshot;
        fn capture(&self) -> Result<Snapshot, String> {
            Ok(Snapshot {
                run: read_value(RUN_KEY)?,
                approved: read_value(APPROVED_KEY)?,
            })
        }
        fn snapshot_enabled(snapshot: &Snapshot) -> Result<bool, String> {
            let Some(run) = &snapshot.run else {
                return Ok(false);
            };
            let command = String::try_from(run.clone()).map_err(|error| error.to_string())?;
            if command.trim().is_empty() {
                return Ok(false);
            }
            match &snapshot.approved {
                None => Ok(true),
                Some(value) if value.ty() == Type::Bytes => approval_enabled(value),
                Some(_) => {
                    Err("The Windows StartupApproved entry has an invalid registry type".into())
                }
            }
        }
        fn set_enabled(&self, enabled: bool) -> Result<(), String> {
            // Do not use the plugin's Windows registration: its underlying
            // default can write HKLM when elevated and does not quote paths.
            // This option belongs solely to the currently signed-in user.
            if enabled {
                let executable = std::env::current_exe().map_err(|error| error.to_string())?;
                let command = startup_command(&executable)?;
                CURRENT_USER
                    .create(RUN_KEY)
                    .and_then(|key| key.set_string(STARTUP_ENTRY_NAME, command))
                    .map_err(|error| error.to_string())?;
                CURRENT_USER
                    .create(APPROVED_KEY)
                    .and_then(|key| {
                        key.set_bytes(STARTUP_ENTRY_NAME, Type::Bytes, &APPROVED_ENABLED)
                    })
                    .map_err(|error| error.to_string())?;
            } else {
                remove_value(RUN_KEY)?;
                remove_value(APPROVED_KEY)?;
            }
            Ok(())
        }
        fn restore(&self, snapshot: &Snapshot) -> Result<(), String> {
            // Attempt both restorations even if the first fails.
            let run = restore_value(RUN_KEY, snapshot.run.as_ref());
            let approved = restore_value(APPROVED_KEY, snapshot.approved.as_ref());
            match (run, approved) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                (Err(run), Err(approved)) => {
                    Err(format!("Run: {run}; StartupApproved: {approved}"))
                }
            }
        }
    }

    fn read_value(path: &str) -> Result<Option<Value>, String> {
        match CURRENT_USER
            .open(path)
            .and_then(|key| key.get_value(STARTUP_ENTRY_NAME))
        {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code().0 as u32 == 0x8007_0002 => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn remove_value(path: &str) -> Result<(), String> {
        match CURRENT_USER
            .options()
            .write()
            .open(path)
            .and_then(|key| key.remove_value(STARTUP_ENTRY_NAME))
        {
            Ok(()) => Ok(()),
            Err(error) if error.code().0 as u32 == 0x8007_0002 => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    fn restore_value(path: &str, value: Option<&Value>) -> Result<(), String> {
        match value {
            Some(value) => CURRENT_USER
                .create(path)
                .and_then(|key| key.set_value(STARTUP_ENTRY_NAME, value))
                .map_err(|error| error.to_string()),
            None => remove_value(path),
        }
    }
}

#[cfg(any(windows, test))]
fn startup_command(executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or("The startup executable path is not valid Unicode")?;
    if executable.is_empty() || executable.contains(['\0', '"']) {
        return Err("The startup executable path contains an invalid character".into());
    }
    Ok(format!("\"{executable}\" {AUTOSTART_ARGUMENT}"))
}

#[cfg(any(windows, test))]
fn approval_enabled(bytes: &[u8]) -> Result<bool, String> {
    if bytes.len() < 12 {
        return Err("The Windows StartupApproved entry is incomplete".into());
    }
    let status: [u8; 4] = bytes
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("The Windows StartupApproved entry is incomplete")?;
    match u32::from_le_bytes(status) {
        2 | 6 => Ok(true),
        3 | 7 => Ok(false),
        _ => Err("The Windows StartupApproved entry has an unknown status".into()),
    }
}

/// The OS operation happens before saving; Manager only applies running
/// configuration after durable persistence succeeds. A failed OS operation or
/// persistence restores the original registration, with any rollback failure
/// included in the error rather than pretending the settings were saved.
fn save_transaction<B: StartupBackend>(
    backend: &B,
    wanted: bool,
    language: &str,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let snapshot = backend
        .capture()
        .map_err(|error| startup_error(language, "read", &error))?;
    let original =
        B::snapshot_enabled(&snapshot).map_err(|error| startup_error(language, "read", &error))?;
    let changed = wanted != original;
    if changed && let Err(error) = backend.set_enabled(wanted) {
        let message = startup_error(language, "change", &error);
        return Err(with_rollback_error(
            message,
            backend.restore(&snapshot),
            language,
        ));
    }
    if let Err(error) = persist() {
        return Err(if changed {
            with_rollback_error(error, backend.restore(&snapshot), language)
        } else {
            error
        });
    }
    Ok(())
}

fn startup_error(language: &str, action: &str, error: &str) -> String {
    let message = match (language, action) {
        ("vi", "read") => "Không thể đọc trạng thái khởi động cùng máy tính",
        ("vi", _) => "Không thể thay đổi tùy chọn khởi động cùng máy tính. Cấu hình chưa được lưu",
        (_, "read") => "Could not read the computer startup setting",
        _ => "Could not change the computer startup setting. Settings were not saved",
    };
    format!("{message}: {error}")
}

fn with_rollback_error(error: String, rollback: Result<(), String>, language: &str) -> String {
    match rollback {
        Ok(()) => error,
        Err(rollback) => {
            let message = if language == "vi" {
                "Không thể khôi phục tùy chọn khởi động trước đó; hãy mở Cài đặt để kiểm tra lại"
            } else {
                "Could not restore the previous startup setting; reopen Settings to check it"
            };
            format!("{error}. {message}: {rollback}")
        }
    }
}

/// Keep the first main window inside the current monitor's usable area,
/// including taskbars and display scaling. Position refers to the outer frame;
/// reserve space for that frame instead of centering against the whole screen.
pub(crate) fn fit_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let work_width = f64::from(area.size.width) / scale;
    let work_height = f64::from(area.size.height) / scale;
    let (width, height, min_width, min_height) = main_window_size(work_width, work_height);
    // GTK may apply set_size after this setup callback returns. Derive frame
    // insets from the old dimensions rather than centering the requested
    // window using an outer_size that still belongs to its old small size.
    let frame = window
        .outer_size()
        .ok()
        .zip(window.inner_size().ok())
        .map(|(outer, inner)| {
            (
                width + f64::from(outer.width.saturating_sub(inner.width)) / scale,
                height + f64::from(outer.height.saturating_sub(inner.height)) / scale,
            )
        })
        .unwrap_or((width, height + 32.0));
    let _ = window.set_min_size(Some(LogicalSize::new(min_width, min_height)));
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(LogicalPosition::new(
        f64::from(area.position.x) / scale + ((work_width - frame.0) / 2.0).max(0.0),
        f64::from(area.position.y) / scale + ((work_height - frame.1) / 2.0).max(0.0),
    ));
}

fn main_window_size(work_width: f64, work_height: f64) -> (f64, f64, f64, f64) {
    let width = (work_width - 32.0).clamp(1.0, 880.0);
    let height = (work_height - 64.0).clamp(1.0, 560.0);
    (width, height, width.min(660.0), height.min(420.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct FakeStartup {
        enabled: Cell<bool>,
        registration: RefCell<Vec<u8>>,
        changes: RefCell<Vec<bool>>,
        failures: RefCell<Vec<bool>>,
        read_error: Cell<bool>,
    }

    struct FakeSnapshot {
        enabled: bool,
        registration: Vec<u8>,
    }

    impl StartupBackend for FakeStartup {
        type Snapshot = FakeSnapshot;
        fn capture(&self) -> Result<FakeSnapshot, String> {
            if self.read_error.get() {
                Err("registry read denied".into())
            } else {
                Ok(FakeSnapshot {
                    enabled: self.enabled.get(),
                    registration: self.registration.borrow().clone(),
                })
            }
        }
        fn snapshot_enabled(snapshot: &FakeSnapshot) -> Result<bool, String> {
            Ok(snapshot.enabled)
        }
        fn set_enabled(&self, value: bool) -> Result<(), String> {
            self.changes.borrow_mut().push(value);
            // An OS implementation may partially change state before failing.
            self.enabled.set(value);
            *self.registration.borrow_mut() = vec![u8::from(value)];
            if self.failures.borrow_mut().pop().unwrap_or(false) {
                Err("registry write denied".into())
            } else {
                Ok(())
            }
        }
        fn restore(&self, snapshot: &FakeSnapshot) -> Result<(), String> {
            self.set_enabled(snapshot.enabled)?;
            self.registration.replace(snapshot.registration.clone());
            Ok(())
        }
    }

    #[test]
    fn accepts_only_the_exact_autostart_argument() {
        assert!(is_autostart(["--autostart"]));
        assert!(!is_autostart([
            "--autostart-other",
            "https://example.org/--autostart"
        ]));
        assert!(!is_autostart(std::iter::empty::<&str>()));
    }

    #[test]
    fn quotes_installed_executable_paths_and_passes_one_fixed_argument() {
        assert_eq!(
            startup_command(Path::new(
                r"C:\Program Files\Supo Download Manager\sdm-desktop.exe"
            ))
            .unwrap(),
            r#""C:\Program Files\Supo Download Manager\sdm-desktop.exe" --autostart"#
        );
        assert!(startup_command(Path::new("bad\"path.exe")).is_err());
        assert!(startup_command(Path::new("bad\0path.exe")).is_err());
    }

    #[test]
    fn respects_task_manager_disabled_startup_entries() {
        assert!(approval_enabled(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap());
        assert!(!approval_enabled(&[3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]).unwrap());
        assert!(!approval_enabled(&[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap());
        assert!(approval_enabled(&[1]).is_err());
        assert!(approval_enabled(&[3, 0, 0, 0]).is_err());
        assert!(approval_enabled(&[99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn failed_persistence_restores_previous_os_registration() {
        for wanted in [true, false] {
            let backend = FakeStartup::default();
            backend.enabled.set(!wanted);
            *backend.registration.borrow_mut() = vec![3, 9, 8, 7, 6];
            let error = save_transaction(&backend, wanted, "en", || Err("disk is full".into()))
                .unwrap_err();
            assert_eq!(error, "disk is full");
            assert_eq!(backend.enabled.get(), !wanted);
            assert_eq!(*backend.registration.borrow(), [3, 9, 8, 7, 6]);
            assert_eq!(*backend.changes.borrow(), [wanted, !wanted]);
        }
    }

    #[test]
    fn failed_os_change_rolls_back_and_never_persists() {
        let backend = FakeStartup::default();
        *backend.failures.borrow_mut() = vec![false, true];
        let persisted = Cell::new(false);
        let error = save_transaction(&backend, true, "vi", || {
            persisted.set(true);
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("Cấu hình chưa được lưu"));
        assert!(!persisted.get());
        assert!(!backend.enabled.get());
        assert_eq!(*backend.changes.borrow(), [true, false]);
    }

    #[test]
    fn reports_a_failed_rollback_instead_of_claiming_previous_state() {
        let backend = FakeStartup::default();
        *backend.failures.borrow_mut() = vec![true, false];
        let error =
            save_transaction(&backend, true, "en", || Err("disk is full".into())).unwrap_err();
        assert!(error.contains("disk is full"));
        assert!(error.contains("Could not restore the previous startup setting"));
    }

    #[test]
    fn unreadable_os_state_does_not_persist_or_change_registration() {
        let backend = FakeStartup::default();
        backend.read_error.set(true);
        let error = save_transaction(&backend, true, "en", || panic!("must not save")).unwrap_err();
        assert!(error.contains("Could not read"));
        assert!(backend.changes.borrow().is_empty());
    }

    #[test]
    fn unchanged_os_state_does_not_issue_a_write() {
        let backend = FakeStartup::default();
        save_transaction(&backend, false, "en", || Ok(())).unwrap();
        assert!(backend.changes.borrow().is_empty());
    }

    #[test]
    fn keeps_both_enabling_and_disabling_after_a_successful_save() {
        let backend = FakeStartup::default();
        save_transaction(&backend, true, "en", || Ok(())).unwrap();
        assert!(backend.enabled.get());
        save_transaction(&backend, false, "en", || Ok(())).unwrap();
        assert!(!backend.enabled.get());
        assert_eq!(*backend.changes.borrow(), [true, false]);
    }

    #[test]
    fn reduces_window_and_minimum_on_a_small_scaled_work_area() {
        assert_eq!(
            main_window_size(1920.0, 1040.0),
            (880.0, 560.0, 660.0, 420.0)
        );
        assert_eq!(main_window_size(640.0, 400.0), (608.0, 336.0, 608.0, 336.0));
    }
}
