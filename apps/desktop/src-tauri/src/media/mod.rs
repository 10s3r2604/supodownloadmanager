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

/// Asks yt-dlp which videos and qualities a page has.
pub async fn probe(tools: &Tools, source: &MediaSource, proxy: &str) -> Result<MediaInfo, String> {
    let ytdlp = tools.ytdlp().ok_or("yt-dlp is not installed")?;
    let work = tools.scratch_dir()?;
    let mut command = tokio::process::Command::new(ytdlp);
    command.args([
        "-J",
        "--no-playlist",
        "--no-warnings",
        "--encoding",
        "utf-8",
    ]);
    if !source.cookies.is_empty() {
        let file = work.join("cookies.txt");
        std::fs::write(&file, cookies::netscape(&source.cookies)).map_err(|e| e.to_string())?;
        command.arg("--cookies").arg(file);
    }
    if let Some(agent) = source.user_agent.as_deref().filter(|a| !a.is_empty()) {
        command.args(["--user-agent", agent]);
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
        .kill_on_drop(true);
    job::hide_window(&mut command);
    let output = tokio::time::timeout(Duration::from_secs(90), command.output()).await;
    let _ = std::fs::remove_dir_all(&work);
    let output = output
        .map_err(|_| "the site took too long to answer".to_owned())?
        .map_err(|e| format!("could not start yt-dlp: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("ERROR:"))
            .unwrap_or("no video found on this page");
        return Err(message.trim().to_owned());
    }
    let mut info = formats::parse(&String::from_utf8_lossy(&output.stdout))?;
    if info.url.is_empty() {
        info.url.clone_from(&source.page_url);
    }
    Ok(info)
}
