//! Where the app writes down what it did.
//!
//! A packaged Windows build has no console to print to, so every warning and
//! error used to be thrown away exactly when a user needs them. Everything
//! goes to one file in the application data folder, next to the saved
//! download list, and the previous file is kept when it grows too large.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A file this big is rotated aside on the next start.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const CURRENT: &str = "sdm-desktop.log";
const PREVIOUS: &str = "sdm-desktop.previous.log";

pub fn directory() -> io::Result<PathBuf> {
    Ok(crate::native_registration::data_dir()?.join("logs"))
}

/// Opens the log, keeping the last one when it has grown too large.
fn open(directory: &Path) -> io::Result<File> {
    std::fs::create_dir_all(directory)?;
    let current = directory.join(CURRENT);
    if std::fs::metadata(&current).is_ok_and(|meta| meta.len() > MAX_BYTES) {
        let _ = std::fs::rename(&current, directory.join(PREVIOUS));
    }
    OpenOptions::new().create(true).append(true).open(&current)
}

/// Sends every event to the log file. Falls back to the default stdout
/// sink when the folder cannot be written, so a run is never silent.
pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sdm_core=warn,sdm_desktop=info,librqbit=warn".into());
    let file = match directory().and_then(|dir| open(&dir)) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("the log file could not be opened: {error}");
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
            return;
        }
    };
    // A plain append-only file, not a background writer: the last lines
    // before a crash are the ones worth reading.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .try_init();
}

/// Writes a panic to the log before the process goes away. The default hook
/// still runs, so the system crash report keeps its own copy.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(directory) = directory()
            && let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join(CURRENT))
        {
            let location = info
                .location()
                .map(|place| format!("{}:{}", place.file(), place.line()))
                .unwrap_or_else(|| "an unknown place".into());
            let _ = writeln!(file, "panicked at {location}: {info}");
            let _ = file.flush();
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_a_large_log_and_keeps_writing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CURRENT),
            vec![b'x'; (MAX_BYTES + 1) as usize],
        )
        .unwrap();
        let mut file = open(dir.path()).unwrap();
        file.write_all(b"after the rotation\n").unwrap();
        file.flush().unwrap();

        assert!(dir.path().join(PREVIOUS).exists(), "the old log is kept");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(CURRENT)).unwrap(),
            "after the rotation\n"
        );
    }

    #[test]
    fn opens_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("logs");
        open(&nested).unwrap();
        assert!(nested.join(CURRENT).exists());
    }
}
