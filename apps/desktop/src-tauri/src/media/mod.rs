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
    cancelled: tokio::sync::watch::Receiver<bool>,
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
    let result = run_probe(command, stage, cancelled, PROBE_TIMEOUT).await;
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
    mut cancelled: tokio::sync::watch::Receiver<bool>,
    timeout: Duration,
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
        let (_, status, ()) = tokio::try_join!(
            async {
                stdout
                    .read_to_string(&mut out)
                    .await
                    .map_err(|e| e.to_string())
            },
            async { child.wait().await.map_err(|e| e.to_string()) },
            async {
                reader.await;
                Ok::<_, String>(())
            },
        )?;
        Ok::<_, String>((out, status))
    };
    // Dropping a pending read alone leaves yt-dlp running until kill_on_drop.
    // Kill and reap it before releasing the scratch directory/cookies.
    let finished = tokio::select! {
        biased;
        _ = async {
            if !*cancelled.borrow() { let _ = cancelled.changed().await; }
        } => {
            let _ = child.kill().await;
            return Err("video discovery cancelled".into());
        }
        result = tokio::time::timeout(timeout, work) => result,
    };
    if !matches!(finished, Ok(Ok(_))) {
        let _ = child.kill().await;
    }
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
    cancel: tokio::sync::watch::Sender<bool>,
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
        validate_page(&source.page_url)?;
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
                        cancel: tokio::sync::watch::channel(false).0,
                    })
                })
                .clone()
        };
        let result = entry
            .result
            .get_or_init(|| read_page(tools, source, proxy, &entry.stage, entry.cancel.subscribe()))
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
    pub fn cancel(&self, page_url: &str) {
        let probe = self
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&page_key(page_url));
        if let Some(probe) = probe {
            probe.cancel.send_replace(true);
        }
    }

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

/// X timelines do not identify the video under the pointer. Old extensions
/// sent /home; asking yt-dlp to extract that page cannot select the right post.
fn validate_page(page_url: &str) -> Result<(), String> {
    let url = url::Url::parse(page_url).map_err(|_| "invalid video URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https video links are supported".into());
    }
    if matches!(
        url.host_str(),
        Some("x.com" | "www.x.com" | "twitter.com" | "www.twitter.com" | "mobile.twitter.com")
    ) {
        let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if !parts.windows(2).any(|pair| {
            pair[0] == "status"
                && !pair[1].is_empty()
                && pair[1].bytes().all(|c| c.is_ascii_digit())
        }) {
            return Err("Open the X post containing the video and try again; the Home timeline is not a video link.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_requires_a_post_instead_of_a_timeline() {
        for page in [
            "https://x.com/home",
            "https://twitter.com/search?q=video",
            "https://x.com/person",
            "https://x.com/person/status/not-a-post",
        ] {
            assert!(validate_page(page).unwrap_err().contains("Open the X post"));
        }
        for page in [
            "https://x.com/person/status/123/video/1",
            "https://twitter.com/person/status/456?ref=home",
            "https://x.com/i/web/status/789",
            "https://example.org/video",
        ] {
            assert!(validate_page(page).is_ok());
        }
        assert!(validate_page("file:///tmp/video").is_err());
    }

    #[cfg(unix)]
    fn command(script: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    #[cfg(unix)]
    fn assert_reaped(stage: &std::sync::Mutex<String>) {
        let pid: i32 = stage
            .lock()
            .unwrap()
            .strip_prefix("PID ")
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: signal zero only checks whether the child is still alive.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_reaps_a_stalled_probe() {
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        let stage = std::sync::Mutex::new(String::new());
        let cancel_after_start = async {
            while stage.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            cancel.send_replace(true);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                run_probe(
                    command("echo PID $$ >&2; exec sleep 30"),
                    &stage,
                    receiver,
                    PROBE_TIMEOUT
                ),
                cancel_after_start
            )
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), "video discovery cancelled");
        assert_reaped(&stage);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_reaps_child_and_keeps_last_stage() {
        let (_cancel, receiver) = tokio::sync::watch::channel(false);
        let stage = std::sync::Mutex::new(String::new());
        let error = run_probe(
            command("echo PID $$ >&2; exec sleep 30"),
            &stage,
            receiver,
            Duration::from_millis(250),
        )
        .await
        .unwrap_err();
        assert!(error.contains("site took too long"));
        assert!(error.contains(&*stage.lock().unwrap()));
        assert_reaped(&stage);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_collects_both_pipes_and_reports_failure() {
        let (_cancel, receiver) = tokio::sync::watch::channel(false);
        let stage = std::sync::Mutex::new(String::new());
        let output = run_probe(
            command("echo reading >&2; printf '{\"title\":\"Clip\"}'"),
            &stage,
            receiver.clone(),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(output, r#"{"title":"Clip"}"#);
        let error = run_probe(
            command("echo 'ERROR: private video' >&2; exit 1"),
            &stage,
            receiver,
            Duration::from_secs(3),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "private video");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_shared_probe_cleans_up_and_can_retry() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("yt-dlp");
        std::fs::write(&fixture, "#!/bin/sh\necho reading >&2\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tools = Tools::with_ytdlp(dir.path(), fixture.clone());
        let prober = Prober::default();
        let source = MediaSource {
            page_url: "https://x.com/author/status/123".into(),
            ..MediaSource::default()
        };
        let cancel = async {
            while prober.stage(&source.page_url).is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            prober.cancel(&format!("{}#player", source.page_url));
        };
        let (first, second, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                prober.probe(&tools, &source, "none"),
                prober.probe(&tools, &source, "none"),
                cancel
            )
        })
        .await
        .unwrap();
        assert!(first.unwrap_err().contains("cancelled"));
        assert!(second.unwrap_err().contains("cancelled"));
        assert_eq!(prober.stage(&source.page_url), None);
        assert_eq!(
            std::fs::read_dir(dir.path().join("media")).unwrap().count(),
            0
        );
        std::fs::write(fixture, "#!/bin/sh\necho '{\"title\":\"Retry clip\",\"formats\":[{\"format_id\":\"0\",\"ext\":\"mp4\"}]}'\n").unwrap();
        let info = prober.probe(&tools, &source, "none").await.unwrap();
        assert_eq!(info.title, "Retry clip");
        assert!(!info.options.is_empty());
    }
}
