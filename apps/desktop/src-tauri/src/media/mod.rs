//! Video and audio from web pages (YouTube, Facebook, X, TikTok and
//! about 1800 other sites), downloaded with yt-dlp.

mod cookies;
mod formats;
pub mod job;
mod tools;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use cookies::Cookie;
#[cfg(all(test, unix))]
pub use formats::MediaKind;
pub use formats::{MediaInfo, MediaOption};
pub use job::{JobConfig, MediaControl};
pub use tools::{ToolStatus, Tools};

/// A page with a video, from the browser extension or typed by the user.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSource {
    pub page_url: String,
    /// The address of the `<video>` itself, when the browser knows it.
    #[serde(default)]
    pub media_url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub referrer: Option<String>,
}

/// How a media download is made. Stored with the download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSpec {
    /// yt-dlp format selector, e.g. `137+140/bv*[height<=1080]+ba/b`.
    pub format: String,
    /// Container for merged video and audio, e.g. `mp4`.
    #[serde(default)]
    pub merge: Option<String>,
    /// Convert the audio to this format, e.g. `mp3`.
    #[serde(default)]
    pub audio: Option<String>,
    /// Estimated size in bytes.
    #[serde(default)]
    pub estimate: Option<u64>,
    /// Cookies of the page. Cleared once the download is complete because
    /// they may contain login data.
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub referrer: Option<String>,
}

/// A request to download a video in the chosen quality.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMediaRequest {
    pub source: MediaSource,
    pub option: MediaOption,
    /// File name without extension.
    pub title: String,
    #[serde(default)]
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub start: bool,
}

impl AddMediaRequest {
    pub fn spec(&self) -> MediaSpec {
        MediaSpec {
            format: self.option.format.clone(),
            merge: self.option.merge.clone(),
            audio: self.option.audio.clone(),
            estimate: self.option.size,
            cookies: self.source.cookies.clone(),
            user_agent: self.source.user_agent.clone(),
            referrer: self.source.referrer.clone(),
        }
    }
}

/// Asks yt-dlp which videos and qualities a page has, writing what it is
/// doing into `stage` meanwhile.
async fn read_page(
    tools: &Tools,
    source: &MediaSource,
    proxy: &str,
    stage: &std::sync::Mutex<String>,
) -> Result<MediaInfo, String> {
    let ytdlp = tools.ytdlp().ok_or("yt-dlp is not installed")?;
    let work = tools.scratch_dir()?;
    let mut command = tokio::process::Command::new(ytdlp);
    command.args(["-J", "--no-playlist", "--encoding", "utf-8"]);
    if !source.cookies.is_empty() {
        let file = work.join("cookies.txt");
        std::fs::write(&file, cookies::netscape(&source.cookies)).map_err(|e| e.to_string())?;
        command.arg("--cookies").arg(file);
    }
    if let Some(agent) = source.user_agent.as_deref().filter(|a| !a.is_empty()) {
        command.args(["--user-agent", agent]);
    }
    if let Some(deno) = tools.deno() {
        let mut runtime = std::ffi::OsString::from("deno:");
        runtime.push(deno.as_os_str());
        command.arg("--js-runtimes").arg(runtime);
    }
    if let Some(referrer) = source.referrer.as_deref().filter(|r| !r.is_empty()) {
        command.args(["--referer", referrer]);
    }
    match proxy {
        "" => {}
        "none" => {
            command.args(["--proxy", ""]);
        }
        proxy => {
            command.args(["--proxy", proxy]);
        }
    }
    command
        .args(["--", &source.page_url])
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    job::hide_window(&mut command);
    let result = run_probe(command, stage).await;
    let _ = std::fs::remove_dir_all(&work);
    let stdout = result?;
    let mut info = formats::parse(&stdout)?;
    if info.url.is_empty() {
        info.url.clone_from(&source.page_url);
    }
    Ok(info)
}

/// How long reading a page may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs `yt-dlp -J` and returns its output. Errors name the step yt-dlp
/// was at, so slow or blocked sites can be told apart.
async fn run_probe(
    mut command: tokio::process::Command,
    stage: &std::sync::Mutex<String>,
) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start yt-dlp: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no output from yt-dlp")?;
    let stderr = child.stderr.take().ok_or("no output from yt-dlp")?;
    let last = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = last.clone();
    let mut lines = BufReader::new(stderr).lines();
    let reader = async move {
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "sdm_desktop::ytdlp", "{line}");
            *stage
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = line.clone();
            let mut kept = log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            kept.push(line);
            let excess = kept.len().saturating_sub(20);
            kept.drain(..excess);
        }
    };
    let work = async {
        let mut out = String::new();
        stdout
            .read_to_string(&mut out)
            .await
            .map_err(|e| e.to_string())?;
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((out, status))
    };
    let (finished, ()) = tokio::join!(tokio::time::timeout(PROBE_TIMEOUT, work), async {
        let _ = tokio::time::timeout(PROBE_TIMEOUT + Duration::from_secs(2), reader).await;
    });
    let lines = last
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    match finished {
        Err(_) => {
            let step = lines.last().map(|l| l.trim()).unwrap_or("starting");
            Err(format!(
                "the site took too long to answer (yt-dlp was at: {step})"
            ))
        }
        Ok(Err(error)) => Err(error),
        Ok(Ok((_, status))) if !status.success() => Err(lines
            .iter()
            .rev()
            .find_map(|l| l.strip_prefix("ERROR:"))
            .map_or_else(
                || "no video found on this page".to_owned(),
                |m| m.trim().to_owned(),
            )),
        Ok(Ok((out, _))) => Ok(out),
    }
}

/// Remembers what yt-dlp found on pages for a while, so the quality menu
/// opens at once when the page was read in advance (when the pointer
/// reached the video).
#[derive(Default)]
pub struct Prober {
    pages: std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Probe>>>,
}

struct Probe {
    started: std::time::Instant,
    /// The last thing yt-dlp said, while it works.
    stage: std::sync::Mutex<String>,
    result: tokio::sync::OnceCell<Result<MediaInfo, String>>,
}

/// How long a page's formats stay valid; their addresses expire.
const PROBE_KEEP: Duration = Duration::from_secs(20 * 60);

impl Prober {
    /// What yt-dlp finds on a page, read once and shared by everyone who
    /// asks while it is being read.
    pub async fn probe(
        &self,
        tools: &Tools,
        source: &MediaSource,
        proxy: &str,
    ) -> Result<MediaInfo, String> {
        let key = page_key(&source.page_url);
        let entry = {
            let mut pages = self
                .pages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pages.retain(|_, p| p.started.elapsed() < PROBE_KEEP);
            pages
                .entry(key.clone())
                .or_insert_with(|| {
                    std::sync::Arc::new(Probe {
                        started: std::time::Instant::now(),
                        stage: std::sync::Mutex::new(String::new()),
                        result: tokio::sync::OnceCell::new(),
                    })
                })
                .clone()
        };
        let result = entry
            .result
            .get_or_init(|| read_page(tools, source, proxy, &entry.stage))
            .await
            .clone();
        if result.is_err() {
            // Let the next attempt try again.
            let mut pages = self
                .pages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pages
                .get(&key)
                .is_some_and(|p| std::sync::Arc::ptr_eq(p, &entry))
            {
                pages.remove(&key);
            }
        }
        result
    }
}

impl Prober {
    /// What yt-dlp is doing for a page being read, for the user to see.
    pub fn stage(&self, page_url: &str) -> Option<String> {
        let pages = self
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let probe = pages.get(&page_key(page_url))?;
        if probe.result.initialized() {
            return None;
        }
        let stage = probe
            .stage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(stage.clone()).filter(|s| !s.is_empty())
    }
}

/// Pages that differ only in their fragment are the same video.
fn page_key(url: &str) -> String {
    url.split('#').next().unwrap_or(url).to_owned()
}
