//! yt-dlp, ffmpeg and Deno: shipped with the app, found on the computer, or
//! downloaded into the app's data folder.
//!
//! The installers ship all of them (ffmpeg only on Windows), so videos work
//! right away. Our copy of yt-dlp is replaced by a newer one when its
//! published checksum changes; newer copies go into the data folder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sdm_core::{Destination, DownloadOptions, DownloadRequest, Downloader, Outcome};
use serde::Serialize;
use tokio::sync::Mutex;

const YTDLP_RELEASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
/// The Windows build of ffmpeg used by yt-dlp's own installers. Only
/// `ffmpeg.exe` is taken out of the archive, with byte range requests.
const FFMPEG_ZIP: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
/// How often our own copy of yt-dlp updates itself; sites change often.
const UPDATE_EVERY: Duration = Duration::from_secs(2 * 24 * 3600);

/// Which tools are ready.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub ytdlp: bool,
    pub ffmpeg: bool,
    /// SDM can download ffmpeg by itself on this system.
    pub ffmpeg_installable: bool,
    /// About how many megabytes the missing tools are.
    pub ytdlp_size_mb: u32,
    pub ffmpeg_size_mb: u32,
}

/// Progress of a tool download.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgress {
    pub tool: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub struct Tools {
    dir: PathBuf,
    /// Tools shipped with the app, read only.
    bundled: Option<PathBuf>,
    work: PathBuf,
    /// One installation at a time.
    installing: Mutex<()>,
    /// Used instead of searching, in tests.
    ytdlp_override: Option<PathBuf>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The yt-dlp build for this system. On Windows it is a zipped folder: the
/// single-file build unpacks itself on every start, which takes seconds.
fn ytdlp_asset() -> &'static str {
    if cfg!(all(windows, target_arch = "aarch64")) {
        "yt-dlp_win_arm64.zip"
    } else if cfg!(windows) {
        "yt-dlp_win.zip"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    }
}

/// Folders searched besides `PATH`. Apps started from the macOS Finder do
/// not see the shell's `PATH`, so Homebrew is looked up directly.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    } else if cfg!(unix) {
        dirs.extend(["/usr/local/bin", "/usr/bin", "/snap/bin"].map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/bin"));
        }
    }
    dirs
}

/// `name` in `base`, either as `base/name/name.exe` (a zipped folder) or as
/// `base/name.exe`.
fn tool_in(base: &Path, name: &str) -> Option<PathBuf> {
    [base.join(name).join(exe(name)), base.join(exe(name))]
        .into_iter()
        .find(|path| path.is_file())
}

/// Copies a file or folder.
fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Unpacks a zip archive into a folder.
fn extract_zip(archive: &Path, into: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("broken archive: {e}"))?;
    zip.extract(into)
        .map_err(|e| format!("could not unpack: {e}"))
}

fn find_program(name: &str) -> Option<PathBuf> {
    let file = exe(name);
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra_dirs())
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
}

impl Tools {
    /// Tools live in `<data_dir>/tools`, unfinished media downloads in
    /// `<data_dir>/media`.
    pub fn new(data_dir: &Path, bundled: Option<PathBuf>) -> Self {
        Self {
            dir: data_dir.join("tools"),
            bundled: bundled.filter(|dir| dir.is_dir()),
            work: data_dir.join("media"),
            installing: Mutex::new(()),
            ytdlp_override: None,
        }
    }

    #[cfg(all(test, unix))]
    pub fn with_ytdlp(data_dir: &Path, ytdlp: PathBuf) -> Self {
        Self {
            ytdlp_override: Some(ytdlp),
            ..Self::new(data_dir, None)
        }
    }

    /// Where the unfinished files of a media download are kept.
    pub fn work_dir(&self, id: u64) -> PathBuf {
        self.work.join(id.to_string())
    }

    /// A new private folder for temporary files such as cookies.
    pub fn scratch_dir(&self) -> Result<PathBuf, String> {
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = self.work.join(format!("probe-{nanos}"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir)
    }

    /// A tool shipped with the app. Windows runs it where it is; elsewhere
    /// it is copied into the data folder first, because installers do not
    /// always keep files executable.
    fn shipped(&self, name: &str) -> Option<PathBuf> {
        let bundled = self.bundled.as_deref()?;
        let path = tool_in(bundled, name)?;
        if cfg!(windows) {
            return Some(path);
        }
        let from = if path.parent() == Some(bundled) {
            path.clone()
        } else {
            bundled.join(name)
        };
        let to = self.dir.join(from.file_name()?);
        std::fs::create_dir_all(&self.dir).ok()?;
        copy_all(&from, &to).ok()?;
        let record = format!("{name}.sha256");
        let _ = std::fs::copy(bundled.join(&record), self.dir.join(&record));
        let copied = tool_in(&self.dir, name)?;
        make_executable(&copied).ok()?;
        Some(copied)
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        tool_in(&self.dir, name)
            .or_else(|| self.shipped(name))
            .or_else(|| find_program(name))
    }

    pub fn ytdlp(&self) -> Option<PathBuf> {
        if let Some(path) = &self.ytdlp_override {
            return Some(path.clone());
        }
        self.find("yt-dlp")
    }

    pub fn ffmpeg(&self) -> Option<PathBuf> {
        self.find("ffmpeg")
    }

    /// The JavaScript runtime yt-dlp needs for all of YouTube's formats.
    pub fn deno(&self) -> Option<PathBuf> {
        if self.ytdlp_override.is_some() {
            return None;
        }
        self.find("deno")
    }

    pub fn status(&self) -> ToolStatus {
        ToolStatus {
            ytdlp: self.ytdlp().is_some(),
            ffmpeg: self.ffmpeg().is_some(),
            ffmpeg_installable: cfg!(all(windows, target_arch = "x86_64")),
            ytdlp_size_mb: if cfg!(windows) { 20 } else { 38 },
            ffmpeg_size_mb: 66,
        }
    }

    /// Downloads yt-dlp unless it is already there, checking it against the
    /// published SHA-256 sums.
    pub async fn install_ytdlp(
        &self,
        downloader: &Downloader,
        progress: impl Fn(ToolProgress),
    ) -> Result<PathBuf, String> {
        let _guard = self.installing.lock().await;
        if let Some(path) = self.ytdlp() {
            return Ok(path);
        }
        let expected = latest_ytdlp_sum(downloader).await?;
        self.download_ytdlp(downloader, &expected, progress).await
    }

    /// Downloads the yt-dlp release whose checksum is `expected` into the
    /// data folder.
    async fn download_ytdlp(
        &self,
        downloader: &Downloader,
        expected: &str,
        progress: impl Fn(ToolProgress),
    ) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let asset = ytdlp_asset();
        let partial = self.dir.join(format!("{asset}.download"));
        download_file(
            downloader,
            &format!("{YTDLP_RELEASE}/{asset}"),
            &partial,
            |downloaded, total| {
                progress(ToolProgress {
                    tool: "yt-dlp",
                    downloaded,
                    total,
                });
            },
        )
        .await?;
        let actual = sdm_core::sha256_file(&partial)
            .await
            .map_err(|e| e.to_string())?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&partial);
            return Err("the downloaded yt-dlp is damaged (checksum mismatch)".into());
        }
        let folder = self.dir.join("yt-dlp");
        let single = self.dir.join(exe("yt-dlp"));
        if asset.ends_with(".zip") {
            let fresh = self.dir.join("yt-dlp.new");
            let _ = std::fs::remove_dir_all(&fresh);
            let (archive, into) = (partial.clone(), fresh.clone());
            tokio::task::spawn_blocking(move || extract_zip(&archive, &into))
                .await
                .map_err(|e| e.to_string())??;
            let _ = std::fs::remove_file(&partial);
            let _ = std::fs::remove_dir_all(&folder);
            let _ = std::fs::remove_file(&single);
            std::fs::rename(&fresh, &folder).map_err(|e| e.to_string())?;
        } else {
            let _ = std::fs::remove_dir_all(&folder);
            make_executable(&partial)?;
            std::fs::rename(&partial, &single).map_err(|e| e.to_string())?;
        }
        std::fs::write(self.dir.join("yt-dlp.sha256"), expected).map_err(|e| e.to_string())?;
        touch(&self.dir.join("yt-dlp.checked"));
        tool_in(&self.dir, "yt-dlp").ok_or_else(|| "yt-dlp is missing from its archive".into())
    }

    /// Downloads ffmpeg on Windows: only `ffmpeg.exe` out of the 200 MB
    /// archive, checked with the archive's CRC-32.
    pub async fn install_ffmpeg(
        &self,
        downloader: &Downloader,
        progress: impl Fn(ToolProgress),
    ) -> Result<PathBuf, String> {
        let _guard = self.installing.lock().await;
        if let Some(path) = self.ffmpeg() {
            return Ok(path);
        }
        if !self.status().ffmpeg_installable {
            return Err("install ffmpeg with your package manager".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let data = fetch_zip_member(
            downloader,
            FFMPEG_ZIP,
            "/bin/ffmpeg.exe",
            |downloaded, total| {
                progress(ToolProgress {
                    tool: "ffmpeg",
                    downloaded,
                    total: Some(total),
                });
            },
        )
        .await?;
        let target = self.dir.join(exe("ffmpeg"));
        let partial = self.dir.join("ffmpeg.download");
        std::fs::write(&partial, data).map_err(|e| e.to_string())?;
        make_executable(&partial)?;
        std::fs::rename(&partial, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }

    /// Replaces our copy of yt-dlp every couple of days when a new release
    /// is out. A yt-dlp installed by the user is left alone.
    pub async fn update_ytdlp(&self, downloader: &Downloader) {
        let Some(current) = self.ytdlp() else {
            return;
        };
        if !current.starts_with(&self.dir) && find_program("yt-dlp").as_ref() == Some(&current) {
            return;
        }
        let stamp = self.dir.join("yt-dlp.checked");
        let fresh = std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() < UPDATE_EVERY);
        if fresh {
            return;
        }
        let _guard = self.installing.lock().await;
        let latest = match latest_ytdlp_sum(downloader).await {
            Ok(sum) => sum,
            Err(error) => {
                tracing::warn!(%error, "could not check for a new yt-dlp");
                return;
            }
        };
        let installed = std::fs::read_to_string(self.dir.join("yt-dlp.sha256"))
            .or_else(|_| {
                self.bundled
                    .as_ref()
                    .map_or(Err(std::io::ErrorKind::NotFound.into()), |b| {
                        std::fs::read_to_string(b.join("yt-dlp.sha256"))
                    })
            })
            .unwrap_or_default();
        if installed.trim().eq_ignore_ascii_case(&latest) {
            touch(&stamp);
            return;
        }
        match self.download_ytdlp(downloader, &latest, |_| {}).await {
            Ok(path) => tracing::info!(path = %path.display(), "yt-dlp updated"),
            Err(error) => tracing::warn!(%error, "yt-dlp update failed"),
        }
    }
}

fn touch(path: &Path) {
    let _ = std::fs::write(path, b"");
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let _ = path;
    Ok(())
}

/// The checksum of the latest yt-dlp build for this system.
async fn latest_ytdlp_sum(downloader: &Downloader) -> Result<String, String> {
    let sums = downloader
        .fetch(&format!("{YTDLP_RELEASE}/SHA2-256SUMS"), &[], 64 * 1024)
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    expected_sum(&String::from_utf8_lossy(&sums.data), ytdlp_asset())
        .ok_or_else(|| "the yt-dlp checksums do not list this system".into())
}

/// The checksum of `asset` in a `SHA2-256SUMS` file.
fn expected_sum(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, name) = line.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset).then(|| sum.to_owned())
    })
}

/// Downloads a whole file with the SDM engine.
async fn download_file(
    downloader: &Downloader,
    url: &str,
    path: &Path,
    progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    let request =
        DownloadRequest::new(url, Destination::File(path.to_path_buf())).options(DownloadOptions {
            overwrite: true,
            ..DownloadOptions::default()
        });
    let handle = downloader.start(request);
    let control = handle.control();
    let mut wait = std::pin::pin!(handle.wait());
    loop {
        tokio::select! {
            result = &mut wait => {
                return match result {
                    Ok(Outcome::Completed { .. }) => Ok(()),
                    Ok(_) => Err("the download stopped".into()),
                    Err(error) => Err(error.to_string()),
                };
            }
            () = tokio::time::sleep(Duration::from_millis(250)) => {
                let p = control.progress();
                progress(p.downloaded, p.total);
            }
        }
    }
}

/// Where a file is inside a zip archive.
#[derive(Debug, PartialEq, Eq)]
struct ZipEntry {
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    header_offset: u64,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Finds the central directory in the last bytes of an archive: its offset
/// and size.
fn central_directory(tail: &[u8]) -> Option<(u64, u64)> {
    let end = tail.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let size = u32_at(tail, end + 12)?;
    let offset = u32_at(tail, end + 16)?;
    Some((offset.into(), size.into()))
}

/// Finds the entry whose name ends with `suffix` in a central directory.
fn find_entry(directory: &[u8], suffix: &str) -> Option<ZipEntry> {
    let mut at = 0;
    while u32_at(directory, at)? == 0x0201_4b50 {
        let name_len = usize::from(u16_at(directory, at + 28)?);
        let extra_len = usize::from(u16_at(directory, at + 30)?);
        let comment_len = usize::from(u16_at(directory, at + 32)?);
        let name = directory.get(at + 46..at + 46 + name_len)?;
        if name.ends_with(suffix.as_bytes()) {
            return Some(ZipEntry {
                method: u16_at(directory, at + 10)?,
                crc: u32_at(directory, at + 16)?,
                compressed: u32_at(directory, at + 20)?.into(),
                size: u32_at(directory, at + 24)?.into(),
                header_offset: u32_at(directory, at + 42)?.into(),
            });
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    None
}

/// Unpacks an entry's data and checks its CRC-32.
fn unpack(entry: &ZipEntry, data: &[u8]) -> Result<Vec<u8>, String> {
    let out = match entry.method {
        0 => data.to_vec(),
        8 => {
            let mut out = Vec::with_capacity(usize::try_from(entry.size).unwrap_or(0));
            flate2::read::DeflateDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(|e| format!("could not unpack: {e}"))?;
            out
        }
        method => return Err(format!("unsupported compression method {method}")),
    };
    let mut crc = flate2::Crc::new();
    crc.update(&out);
    if out.len() as u64 != entry.size || crc.sum() != entry.crc {
        return Err("the downloaded file is damaged (CRC mismatch)".into());
    }
    Ok(out)
}

/// Takes one file out of a zip archive on a server without downloading the
/// rest of it.
async fn fetch_zip_member(
    downloader: &Downloader,
    url: &str,
    suffix: &str,
    progress: impl Fn(u64, u64),
) -> Result<Vec<u8>, String> {
    let info = downloader
        .probe(url, &[])
        .await
        .map_err(|e| e.to_string())?;
    let total = info
        .size
        .filter(|_| info.resumable)
        .ok_or("the server cannot send parts of the archive")?;
    let url = info.final_url;
    let fetch = |range: std::ops::Range<u64>| downloader.fetch_range(&url, range, |_| {});

    let tail_start = total.saturating_sub(64 * 1024);
    let tail = fetch(tail_start..total).await.map_err(|e| e.to_string())?;
    let (offset, size) = central_directory(&tail).ok_or("not a zip archive")?;
    let directory = fetch(offset..offset + size)
        .await
        .map_err(|e| e.to_string())?;
    let entry = find_entry(&directory, suffix).ok_or("ffmpeg is missing from the archive")?;

    // The local header has its own name and extra field lengths.
    let header = fetch(entry.header_offset..entry.header_offset + 30)
        .await
        .map_err(|e| e.to_string())?;
    if u32_at(&header, 0) != Some(0x0403_4b50) {
        return Err("broken zip archive".into());
    }
    let skip = 30
        + u64::from(u16_at(&header, 26).unwrap_or(0))
        + u64::from(u16_at(&header, 28).unwrap_or(0));
    let start = entry.header_offset + skip;
    let data = downloader
        .fetch_range(&url, start..start + entry.compressed, |n| {
            progress(n, entry.compressed)
        })
        .await
        .map_err(|e| e.to_string())?;
    unpack(&entry, &data)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A zip archive with two files, made by hand.
    fn archive() -> Vec<u8> {
        let files: [(&str, &[u8]); 2] = [
            ("pkg/doc/readme.txt", b"hello"),
            ("pkg/bin/ffmpeg.exe", &[7u8; 5000]),
        ];
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, content) in files {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(content).unwrap();
            let packed = encoder.finish().unwrap();
            let mut crc = flate2::Crc::new();
            crc.update(content);
            let offset = out.len() as u32;
            let common = |buf: &mut Vec<u8>| {
                buf.extend_from_slice(&8u16.to_le_bytes()); // method
                buf.extend_from_slice(&[0; 4]); // time and date
                buf.extend_from_slice(&crc.sum().to_le_bytes());
                buf.extend_from_slice(&(packed.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(content.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
            };
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[20, 0, 0, 0]); // version, flags
            common(&mut out);
            out.extend_from_slice(&3u16.to_le_bytes()); // extra length
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b"xyz");
            out.extend_from_slice(&packed);

            directory.extend_from_slice(b"PK\x01\x02");
            directory.extend_from_slice(&[20, 0, 20, 0, 0, 0]); // versions, flags
            common(&mut directory);
            directory.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&directory_offset.to_le_bytes());
        out.extend_from_slice(&[0; 2]);
        out
    }

    #[test]
    fn takes_one_file_out_of_a_zip() {
        let zip = archive();
        let (offset, size) = central_directory(&zip[zip.len() - 40..]).unwrap();
        let directory = &zip[offset as usize..(offset + size) as usize];
        let entry = find_entry(directory, "/bin/ffmpeg.exe").unwrap();
        assert_eq!(entry.size, 5000);
        assert!(find_entry(directory, "/bin/ffprobe.exe").is_none());

        let header = &zip[entry.header_offset as usize..];
        let skip = 30
            + usize::from(u16_at(header, 26).unwrap())
            + usize::from(u16_at(header, 28).unwrap());
        let start = entry.header_offset as usize + skip;
        let data = &zip[start..start + entry.compressed as usize];
        assert_eq!(unpack(&entry, data).unwrap(), vec![7u8; 5000]);

        let mut damaged = data.to_vec();
        damaged[2] ^= 0xff;
        assert!(unpack(&entry, &damaged).is_err());
    }

    /// Takes the real `ffmpeg.exe` out of the real archive. Needs the
    /// internet: `cargo test -p sdm-desktop -- --ignored ffmpeg`.
    #[tokio::test]
    #[ignore]
    async fn fetches_ffmpeg_from_github() {
        let downloader = Downloader::new(sdm_core::ClientConfig::default()).unwrap();
        let data = fetch_zip_member(&downloader, FFMPEG_ZIP, "/bin/ffmpeg.exe", |_, _| {})
            .await
            .unwrap();
        assert!(data.starts_with(b"MZ"), "not a Windows program");
        assert!(data.len() > 50_000_000);
    }

    #[test]
    fn reads_checksum_files() {
        let sums = "abc123  yt-dlp\ndef456  yt-dlp.exe\n0099 *yt-dlp_macos\n";
        assert_eq!(expected_sum(sums, "yt-dlp.exe").as_deref(), Some("def456"));
        assert_eq!(expected_sum(sums, "yt-dlp_macos").as_deref(), Some("0099"));
        assert_eq!(expected_sum(sums, "yt-dlp_linux"), None);
    }

    #[test]
    fn bundled_readme_is_not_a_tool() {
        let bundled = tempfile::tempdir().unwrap();
        std::fs::write(
            bundled.path().join("README.md"),
            "Local builds need no tools",
        )
        .unwrap();
        let data = tempfile::tempdir().unwrap();
        let tools = Tools::new(data.path(), Some(bundled.path().to_path_buf()));
        assert!(tools.shipped("yt-dlp").is_none());
        assert!(tools.shipped("deno").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn copies_bundled_tools_and_restores_execute_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let bundled = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let program = bundled.path().join("yt-dlp");
        std::fs::write(&program, "bundled tool").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(bundled.path().join("yt-dlp.sha256"), "checksum").unwrap();
        let tools = Tools::new(data.path(), Some(bundled.path().to_path_buf()));
        let found = tools.ytdlp().unwrap();
        assert_eq!(found, data.path().join("tools/yt-dlp"));
        assert_eq!(std::fs::read_to_string(&found).unwrap(), "bundled tool");
        assert_eq!(
            std::fs::metadata(found).unwrap().permissions().mode() & 0o111,
            0o111
        );
        assert_eq!(
            std::fs::read_to_string(data.path().join("tools/yt-dlp.sha256")).unwrap(),
            "checksum"
        );

        // A downloaded update takes precedence over the bundled copy.
        std::fs::write(data.path().join("tools/yt-dlp"), "newer tool").unwrap();
        assert_eq!(
            std::fs::read_to_string(tools.ytdlp().unwrap()).unwrap(),
            "newer tool"
        );
    }
}
