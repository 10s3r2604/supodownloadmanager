//! One media download: a yt-dlp process and its progress.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sdm_core::{Outcome, Phase, Progress};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use super::MediaSpec;
use super::cookies;

/// Fragments downloaded at the same time for streams split into many parts.
const FRAGMENTS: &str = "4";
const PREFIX_PROGRESS: &str = "SDMP ";
const PREFIX_FILE: &str = "SDMF ";

/// Everything a media download needs.
#[derive(Debug, Clone)]
pub struct JobConfig {
    pub ytdlp: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    /// The page of the video.
    pub url: String,
    pub spec: MediaSpec,
    /// Final folder, and the file name without its extension.
    pub directory: PathBuf,
    pub stem: String,
    /// Unfinished files, kept between pauses.
    pub work_dir: PathBuf,
    pub speed_limit: Option<u64>,
    /// Empty for the system proxy, `none` for no proxy, or a proxy URL.
    pub proxy: String,
}

const RUNNING: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

struct Shared {
    progress: Mutex<Progress>,
    stop: AtomicU8,
    wake: Notify,
}

/// A remote control for a media download, like `sdm_core::DownloadControl`.
#[derive(Clone)]
pub struct MediaControl(Arc<Shared>);

impl MediaControl {
    pub fn progress(&self) -> Progress {
        self.0
            .progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn pause(&self) {
        self.stop(PAUSE);
    }

    pub fn cancel(&self) {
        self.stop(CANCEL);
    }

    fn stop(&self, how: u8) {
        let _ = self
            .0
            .stop
            .compare_exchange(RUNNING, how, Ordering::SeqCst, Ordering::SeqCst);
        if how == CANCEL {
            self.0.stop.store(CANCEL, Ordering::SeqCst);
        }
        self.0.wake.notify_one();
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        change(
            &mut self
                .0
                .progress
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

pub struct MediaJob {
    control: MediaControl,
    task: JoinHandle<Result<Outcome, String>>,
}

impl MediaJob {
    pub fn control(&self) -> MediaControl {
        self.control.clone()
    }

    pub async fn wait(self) -> Result<Outcome, String> {
        self.task
            .await
            .unwrap_or_else(|e| Err(format!("the download crashed: {e}")))
    }
}

/// Starts yt-dlp in the background. Must be called inside a Tokio runtime.
pub fn start(config: JobConfig) -> MediaJob {
    let control = MediaControl(Arc::new(Shared {
        progress: Mutex::new(Progress {
            phase: Phase::Connecting,
            total: config.spec.estimate,
            resumable: true,
            ..Progress::default()
        }),
        stop: AtomicU8::new(RUNNING),
        wake: Notify::new(),
    }));
    let task = tokio::spawn(run(config, control.clone()));
    MediaJob { control, task }
}

/// The yt-dlp command line.
pub fn args(config: &JobConfig, cookies_file: Option<&Path>) -> Vec<OsString> {
    let spec = &config.spec;
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |values: &[&str]| args.extend(values.iter().map(OsString::from));
    push(&[
        "--newline",
        "--progress",
        "--no-warnings",
        "--no-playlist",
        "--no-mtime",
        "--encoding",
        "utf-8",
        "--progress-template",
        "download:SDMP %(progress.status)s %(progress.downloaded_bytes)s %(progress.total_bytes)s \
         %(progress.total_bytes_estimate)s %(progress.speed)s %(info.format_id)s",
        "--print",
        "after_move:SDMF %(filepath)s",
        "-f",
        &spec.format,
        "-N",
        FRAGMENTS,
    ]);
    if let Some(merge) = &spec.merge {
        push(&["--merge-output-format", merge]);
    }
    if let Some(audio) = &spec.audio {
        push(&["-x", "--audio-format", audio, "--audio-quality", "2"]);
    }
    if let Some(agent) = spec.user_agent.as_deref().filter(|a| !a.is_empty()) {
        push(&["--user-agent", agent]);
    }
    if let Some(referrer) = spec.referrer.as_deref().filter(|r| !r.is_empty()) {
        push(&["--referer", referrer]);
    }
    if let Some(limit) = config.speed_limit.filter(|&l| l > 0) {
        push(&["--limit-rate", &limit.to_string()]);
    }
    match config.proxy.as_str() {
        "" => {}
        "none" => push(&["--proxy", ""]),
        proxy => push(&["--proxy", proxy]),
    }
    // A template, so "%" in the title has to be doubled.
    let template = format!("{}.%(ext)s", config.stem.replace('%', "%%"));
    push(&["-o", &template]);

    let mut path_arg = |flag: &str, prefix: &str, path: &Path| {
        args.push(flag.into());
        let mut value = OsString::from(prefix);
        value.push(path.as_os_str());
        args.push(value);
    };
    path_arg("-P", "home:", &config.directory);
    path_arg("-P", "temp:", &config.work_dir);
    if let Some(ffmpeg) = &config.ffmpeg {
        path_arg("--ffmpeg-location", "", ffmpeg);
    }
    if let Some(file) = cookies_file {
        path_arg("--cookies", "", file);
    }
    args.push("--".into());
    args.push(config.url.clone().into());
    args
}

/// Keeps no console window from flashing up on Windows.
pub fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let _ = command;
}

async fn run(config: JobConfig, control: MediaControl) -> Result<Outcome, String> {
    std::fs::create_dir_all(&config.work_dir).map_err(|e| e.to_string())?;
    let cookies_file = if config.spec.cookies.is_empty() {
        None
    } else {
        let path = config.work_dir.join("cookies.txt");
        std::fs::write(&path, cookies::netscape(&config.spec.cookies))
            .map_err(|e| e.to_string())?;
        Some(path)
    };

    let mut command = Command::new(&config.ytdlp);
    command
        .args(args(&config, cookies_file.as_deref()))
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    hide_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start yt-dlp: {e}"))?;

    // Both outputs, line by line.
    let (sender, mut lines) = mpsc::unbounded_channel::<String>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
    ]
    .into_iter()
    .flatten()
    {
        let sender = sender.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(sender);

    let mut parser = ProgressParser::new(config.spec.estimate);
    let mut final_path = None;
    let mut errors: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            line = lines.recv() => {
                let Some(line) = line else { break };
                if let Some(path) = line.strip_prefix(PREFIX_FILE) {
                    final_path = Some(PathBuf::from(path.trim()));
                } else if let Some(update) = parser.feed(&line) {
                    control.update(|p| {
                        p.phase = if update.finished { Phase::Finishing } else { Phase::Downloading };
                        p.downloaded = update.downloaded;
                        p.total = update.total;
                        p.speed = update.speed;
                        p.connections = usize::from(!update.finished);
                    });
                } else if let Some(error) = line.strip_prefix("ERROR:") {
                    errors.push(error.trim().to_owned());
                }
            }
            () = control.0.wake.notified() => {
                if control.0.stop.load(Ordering::SeqCst) != RUNNING {
                    stop(&mut child).await;
                    break;
                }
            }
        }
    }
    let status = child.wait().await;
    control.update(|p| {
        p.speed = 0;
        p.connections = 0;
    });

    if status.as_ref().is_ok_and(|s| s.success())
        && let Some(path) = final_path.filter(|p| p.is_file())
    {
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or_default();
        let _ = std::fs::remove_dir_all(&config.work_dir);
        return Ok(Outcome::Completed { path, size });
    }
    match control.0.stop.load(Ordering::SeqCst) {
        PAUSE => Ok(Outcome::Paused {
            path: None,
            resumable: true,
        }),
        CANCEL => {
            let _ = std::fs::remove_dir_all(&config.work_dir);
            Ok(Outcome::Cancelled)
        }
        _ => Err(errors.pop().unwrap_or_else(|| match status {
            Ok(status) => format!("yt-dlp stopped ({status})"),
            Err(error) => error.to_string(),
        })),
    }
}

/// Stops yt-dlp and whatever it started (ffmpeg).
async fn stop(child: &mut Child) {
    let Some(pid) = child.id() else { return };
    #[cfg(unix)]
    {
        // yt-dlp runs in its own process group, together with the ffmpeg it
        // starts and, for the standalone builds, the unpacked Python.
        signal_group(pid, libc::SIGTERM);
        if tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .is_err()
        {
            signal_group(pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T", "/F"]);
        hide_window(&mut command);
        let _ = command.status().await;
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
}

#[cfg(unix)]
fn signal_group(group: u32, signal: libc::c_int) {
    let Ok(group) = libc::pid_t::try_from(group) else {
        return;
    };
    // SAFETY: killpg only sends a signal; `group` is the process group that
    // was created for yt-dlp with `process_group(0)`.
    unsafe {
        libc::killpg(group, signal);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Update {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: u64,
    /// The last part finished; yt-dlp may be merging it now.
    pub finished: bool,
}

/// Reads yt-dlp's progress lines. Video and audio are downloaded one after
/// the other, so the parts are added up.
#[derive(Debug, Default)]
pub struct ProgressParser {
    estimate: Option<u64>,
    done: u64,
    part: Option<String>,
    part_total: u64,
    part_downloaded: u64,
}

fn number(value: &str) -> Option<u64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v as u64)
}

impl ProgressParser {
    pub fn new(estimate: Option<u64>) -> Self {
        Self {
            estimate,
            ..Self::default()
        }
    }

    pub fn feed(&mut self, line: &str) -> Option<Update> {
        let mut fields = line.strip_prefix(PREFIX_PROGRESS)?.split_whitespace();
        let status = fields.next()?;
        let downloaded = number(fields.next()?).unwrap_or(0);
        let total = fields.next().and_then(number);
        let estimate = fields.next().and_then(number);
        let speed = fields.next().and_then(number).unwrap_or(0);
        let part = fields.next().unwrap_or("").to_owned();

        if self.part.as_ref().is_some_and(|p| *p != part) {
            self.done += self.part_total.max(self.part_downloaded);
            self.part_total = 0;
        }
        self.part = Some(part);
        self.part_downloaded = downloaded;
        if let Some(size) = total.or(estimate) {
            self.part_total = size;
        }
        let downloaded = self.done + downloaded;
        let known = (self.part_total > 0).then_some(self.done + self.part_total);
        let total = match (self.estimate, known) {
            (Some(estimate), Some(known)) => Some(estimate.max(known)),
            (estimate, known) => estimate.or(known),
        }
        .map(|t| t.max(downloaded));
        Some(Update {
            downloaded,
            total,
            speed,
            finished: status == "finished",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_up_video_and_audio() {
        let mut parser = ProgressParser::new(Some(1000));
        let up = parser.feed("SDMP downloading 100 800 NA 50.5 137").unwrap();
        assert_eq!(
            (up.downloaded, up.total, up.speed, up.finished),
            (100, Some(1000), 50, false)
        );
        let up = parser.feed("SDMP finished 800 800 NA NA 137").unwrap();
        assert!(up.finished);
        // The audio starts: the video counts as done.
        let up = parser.feed("SDMP downloading 50 250 NA 10 140").unwrap();
        assert_eq!(
            (up.downloaded, up.total, up.finished),
            (850, Some(1050), false)
        );
        assert!(parser.feed("[download] something else").is_none());
    }

    #[test]
    fn copes_with_unknown_sizes() {
        let mut parser = ProgressParser::new(None);
        let up = parser.feed("SDMP downloading 834 NA NA NA hls").unwrap();
        assert_eq!((up.downloaded, up.total, up.speed), (834, None, 0));
        let up = parser
            .feed("SDMP downloading 2000 NA 9000.0 100 hls")
            .unwrap();
        assert_eq!(up.total, Some(9000));
    }

    #[test]
    fn builds_the_command_line() {
        let config = JobConfig {
            ytdlp: "yt-dlp".into(),
            ffmpeg: Some("/tools/ffmpeg".into()),
            url: "https://www.youtube.com/watch?v=x".into(),
            spec: MediaSpec {
                format: "137+140/b".into(),
                merge: Some("mp4".into()),
                audio: None,
                estimate: None,
                cookies: Vec::new(),
                user_agent: Some("Mozilla/5.0".into()),
                referrer: None,
            },
            directory: "/home/a/Downloads/Video".into(),
            stem: "50% off".into(),
            work_dir: "/data/media/7".into(),
            speed_limit: Some(1000),
            proxy: "none".into(),
        };
        let args: Vec<String> = args(&config, Some(Path::new("/data/media/7/cookies.txt")))
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w == pair);
        assert!(has(["-f", "137+140/b"]));
        assert!(has(["--merge-output-format", "mp4"]));
        assert!(has(["-o", "50%% off.%(ext)s"]));
        assert!(has(["-P", "home:/home/a/Downloads/Video"]));
        assert!(has(["-P", "temp:/data/media/7"]));
        assert!(has(["--ffmpeg-location", "/tools/ffmpeg"]));
        assert!(has(["--cookies", "/data/media/7/cookies.txt"]));
        assert!(has(["--limit-rate", "1000"]));
        assert!(has(["--proxy", ""]));
        assert!(has(["--user-agent", "Mozilla/5.0"]));
        assert_eq!(
            &args[args.len() - 2..],
            ["--", "https://www.youtube.com/watch?v=x"]
        );
    }
}
