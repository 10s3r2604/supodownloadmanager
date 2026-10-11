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
            let line = redact_diagnostic(&line);
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

/// yt-dlp can echo signed URLs or proxy credentials in diagnostics.
fn redact_diagnostic(line: &str) -> String {
    line.split_whitespace()
        .map(|word| {
            let Some(separator) = word.find("://") else {
                return word.to_owned();
            };
            let start = word[..separator]
                .rfind(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.'))
                .map_or(0, |index| index + 1);
            if let Ok(mut url) = url::Url::parse(&word[start..]) {
                let _ = url.set_username("");
                let _ = url.set_password(None);
                url.set_query(None);
                url.set_fragment(None);
                return format!("{}{}", &word[..start], url);
            }
            // Malformed ports or punctuation must not cause a credential leak.
            let rest = &word[separator + 3..];
            let authority_end = rest.find('/').unwrap_or(rest.len());
            let rest = rest[..authority_end]
                .rfind('@')
                .map_or(rest, |index| &rest[index + 1..]);
            let end = rest.find(['?', '#']).unwrap_or(rest.len());
            format!("{}{}", &word[..separator + 3], &rest[..end])
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bounded discovery shared by native dialogs and browser job interests.
#[derive(Clone)]
pub struct Prober {
    inner: std::sync::Arc<ProbePool>,
}

struct ProbePool {
    pages: std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<Probe>>>,
    slots: std::sync::Arc<tokio::sync::Semaphore>,
    next_interest: std::sync::atomic::AtomicU64,
}

impl Default for Prober {
    fn default() -> Self {
        Self {
            inner: std::sync::Arc::new(ProbePool {
                pages: Default::default(),
                slots: std::sync::Arc::new(tokio::sync::Semaphore::new(PROBE_ACTIVE)),
                next_interest: std::sync::atomic::AtomicU64::new(1),
            }),
        }
    }
}

struct Probe {
    page: String,
    started: std::time::Instant,
    stage: std::sync::Mutex<String>,
    result: tokio::sync::watch::Sender<Option<Result<MediaInfo, String>>>,
    cancel: tokio::sync::watch::Sender<bool>,
    interests:
        std::sync::Mutex<std::collections::HashMap<u64, (bool, tokio::sync::watch::Sender<bool>)>>,
}

struct Interest {
    probe: std::sync::Arc<Probe>,
    id: u64,
}

impl Drop for Interest {
    fn drop(&mut self) {
        let mut interests = self
            .probe
            .interests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        interests.remove(&self.id);
        if interests.is_empty() && self.probe.result.borrow().is_none() {
            self.probe.cancel.send_replace(true);
        }
    }
}

const PROBE_ACTIVE: usize = 2;
/// Includes the two active jobs and eight waiting pages. Cache entries do not count.
const PROBE_PENDING: usize = 10;
const PROBE_CACHE: usize = 64;
const PROBE_KEEP: Duration = Duration::from_secs(20 * 60);

impl Prober {
    pub async fn probe(
        &self,
        tools: &std::sync::Arc<Tools>,
        source: &MediaSource,
        proxy: &str,
    ) -> Result<MediaInfo, String> {
        self.probe_with_interest(tools, source, proxy, true).await
    }

    /// Dropping this future releases just this browser's interest.
    pub async fn probe_browser(
        &self,
        tools: &std::sync::Arc<Tools>,
        source: &MediaSource,
        proxy: &str,
    ) -> Result<MediaInfo, String> {
        self.probe_with_interest(tools, source, proxy, false).await
    }

    async fn probe_with_interest(
        &self,
        tools: &std::sync::Arc<Tools>,
        source: &MediaSource,
        proxy: &str,
        native: bool,
    ) -> Result<MediaInfo, String> {
        use std::sync::atomic::Ordering;
        validate_page(&source.page_url)?;
        let key = source_key(source, proxy);
        let id = self.inner.next_interest.fetch_add(1, Ordering::Relaxed);
        let (interest_cancel, mut cancelled) = tokio::sync::watch::channel(false);
        let (entry, fresh) = {
            let mut pages = self
                .inner
                .pages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pages.retain(|_, p| {
                !*p.cancel.borrow()
                    && (p.result.borrow().is_none() || p.started.elapsed() < PROBE_KEEP)
            });
            if !pages.contains_key(&key)
                && pages
                    .values()
                    .filter(|p| p.result.borrow().is_none())
                    .count()
                    >= PROBE_PENDING
            {
                return Err("video discovery is busy; try again shortly".into());
            }
            if pages.len() >= PROBE_CACHE
                && let Some(oldest) = pages
                    .iter()
                    .filter(|(_, p)| p.result.borrow().is_some())
                    .min_by_key(|(_, p)| p.started)
                    .map(|(k, _)| *k)
            {
                pages.remove(&oldest);
            }
            let fresh = !pages.contains_key(&key);
            let entry = pages
                .entry(key)
                .or_insert_with(|| {
                    std::sync::Arc::new(Probe {
                        page: page_key(&source.page_url),
                        started: std::time::Instant::now(),
                        stage: std::sync::Mutex::new("waiting for video discovery".into()),
                        result: tokio::sync::watch::channel(None).0,
                        cancel: tokio::sync::watch::channel(false).0,
                        interests: Default::default(),
                    })
                })
                .clone();
            entry
                .interests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id, (native, interest_cancel));
            (entry, fresh)
        };
        let _interest = Interest {
            probe: entry.clone(),
            id,
        };
        if fresh {
            let pool = self.inner.clone();
            let entry = entry.clone();
            let tools = tools.clone();
            let source = source.clone();
            let proxy = proxy.to_owned();
            tokio::spawn(async move {
                let mut cancelled = entry.cancel.subscribe();
                let result = tokio::select! {
                    biased;
                    _ = wait_cancelled(&mut cancelled) => Err("video discovery cancelled".into()),
                    permit = tokio::time::timeout(PROBE_TIMEOUT, pool.slots.clone().acquire_owned()) => {
                        match permit {
                            Ok(Ok(_permit)) => read_page(&tools, &source, &proxy, &entry.stage, cancelled.clone()).await,
                            _ => Err("video discovery queue took too long; try again".into()),
                        }
                    }
                };
                let failed = result.is_err();
                entry.result.send_replace(Some(result));
                if failed {
                    let mut pages = pool
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
            });
        }
        let mut result = entry.result.subscribe();
        loop {
            if let Some(result) = result.borrow().clone() {
                return result;
            }
            tokio::select! {
                biased;
                _ = wait_cancelled(&mut cancelled) => {
                    // When this cancelled the shared worker, finish killing/reaping
                    // before completing the native invocation and releasing cookies.
                    if *entry.cancel.borrow() {
                        while result.borrow().is_none() {
                            if result.changed().await.is_err() { break; }
                        }
                    }
                    return Err("video discovery cancelled".into());
                },
                changed = result.changed() => if changed.is_err() { return Err("video discovery stopped".into()); }
            }
        }
    }

    /// Native dialog cancellation leaves browser interests attached.
    pub fn cancel(&self, page_url: &str) {
        let pages = self
            .inner
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for probe in pages.values().filter(|p| p.page == page_key(page_url)) {
            let mut interests = probe
                .interests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            interests.retain(|_, (native, cancel)| {
                if *native {
                    cancel.send_replace(true);
                    false
                } else {
                    true
                }
            });
            if interests.is_empty() && probe.result.borrow().is_none() {
                probe.cancel.send_replace(true);
            }
        }
    }

    pub fn stage(&self, page_url: &str) -> Option<String> {
        let pages = self
            .inner
            .pages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let probe = pages.values().find(|p| {
            p.page == page_key(page_url) && p.result.borrow().is_none() && !*p.cancel.borrow()
        })?;
        let stage = probe
            .stage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Some(stage).filter(|s| !s.is_empty())
    }
}

pub(crate) async fn wait_cancelled(cancelled: &mut tokio::sync::watch::Receiver<bool>) {
    while !*cancelled.borrow() {
        if cancelled.changed().await.is_err() {
            break;
        }
    }
}

/// Hash authentication context in memory; never persist or log the key/input.
pub(crate) fn source_key(source: &MediaSource, proxy: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    page_key(&source.page_url).hash(&mut hash);
    source.media_url.hash(&mut hash);
    source.user_agent.hash(&mut hash);
    source.referrer.hash(&mut hash);
    serde_json::to_string(&source.cookies)
        .unwrap_or_default()
        .hash(&mut hash);
    proxy.hash(&mut hash);
    hash.finish()
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
        let tools = std::sync::Arc::new(Tools::with_ytdlp(dir.path(), fixture.clone()));
        let prober = Prober::default();
        let source = MediaSource {
            page_url: "https://x.com/author/status/123".into(),
            ..MediaSource::default()
        };
        let cancel = async {
            while prober.stage(&source.page_url).as_deref() != Some("reading") {
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
    #[test]
    fn cache_separates_authentication_and_selected_clip() {
        let base = MediaSource {
            page_url: "https://example.org/clip".into(),
            ..Default::default()
        };
        let mut changed = base.clone();
        changed.page_url.push_str("#player");
        changed.title = Some("different tab title".into());
        assert_eq!(source_key(&base, ""), source_key(&changed, ""));
        changed.user_agent = Some("another browser".into());
        assert_ne!(source_key(&base, ""), source_key(&changed, ""));
        changed = base.clone();
        changed.referrer = Some("https://example.org/private".into());
        assert_ne!(source_key(&base, ""), source_key(&changed, ""));
        changed = base.clone();
        changed.media_url = Some("https://example.org/second-clip".into());
        assert_ne!(source_key(&base, ""), source_key(&changed, ""));
        changed = base.clone();
        changed.cookies = serde_json::from_str(r#"[{"name":"session","value":"private-value","domain":"example.org","path":"/","secure":true,"hostOnly":true,"expires":null}]"#).unwrap();
        assert_ne!(source_key(&base, ""), source_key(&changed, ""));
        assert_ne!(
            source_key(&base, ""),
            source_key(&base, "http://user:password@proxy.example")
        );
    }

    #[test]
    fn diagnostics_remove_url_credentials_and_tokens() {
        let line = redact_diagnostic(
            "ERROR: https://user:password@example.org/clip?token=secret#private failed",
        );
        assert_eq!(line, "ERROR: https://example.org/clip failed");
        for input in [
            "socks5://user:password@proxy.example:1080?token=secret",
            "socks5h://user:password@proxy.example:1080?token=secret",
            "SOCKS4://user:password@proxy.example:1080?token=secret",
            "HTTPS://user:password@example.org/clip?token=secret",
            "socks5://user:password@proxy.example:invalid?token=secret",
        ] {
            let line = redact_diagnostic(input);
            assert!(!line.contains("user"), "{line}");
            assert!(!line.contains("password"), "{line}");
            assert!(!line.contains("secret"), "{line}");
        }
    }

    #[cfg(unix)]
    fn stalled_tools(dir: &std::path::Path) -> std::sync::Arc<Tools> {
        use std::os::unix::fs::PermissionsExt;
        let fixture = dir.join("yt-dlp");
        std::fs::write(
            &fixture,
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/started-$$\"\necho PID $$ >&2\nexec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::sync::Arc::new(Tools::with_ytdlp(dir, fixture))
    }

    #[cfg(unix)]
    fn started_count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|p| p.file_name().to_string_lossy().starts_with("started-"))
            .count()
    }

    #[cfg(unix)]
    async fn await_started(dir: &std::path::Path, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while started_count(dir) < count {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_bounds_active_and_queued_pages_and_cancels_both() {
        let dir = tempfile::tempdir().unwrap();
        let tools = stalled_tools(dir.path());
        let prober = Prober::default();
        let mut tasks = Vec::new();
        for index in 0..PROBE_PENDING {
            let prober = prober.clone();
            let tools = tools.clone();
            tasks.push(tokio::spawn(async move {
                prober
                    .probe_browser(
                        &tools,
                        &MediaSource {
                            page_url: format!("https://example.org/{index}"),
                            ..Default::default()
                        },
                        "none",
                    )
                    .await
            }));
        }
        await_started(dir.path(), PROBE_ACTIVE).await;
        // Let all submitted tasks register; this does not rely on process sleeps.
        tokio::time::timeout(Duration::from_secs(3), async {
            while prober.inner.pages.lock().unwrap().len() < PROBE_PENDING {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(started_count(dir.path()), PROBE_ACTIVE);
        let error = prober
            .probe_browser(
                &tools,
                &MediaSource {
                    page_url: "https://example.org/overflow".into(),
                    ..Default::default()
                },
                "none",
            )
            .await
            .unwrap_err();
        assert!(error.contains("busy"));
        // Release every active and queued browser interest. Queued work must never spawn.
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            assert!(task.await.unwrap_err().is_cancelled());
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while !prober.inner.pages.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(started_count(dir.path()), PROBE_ACTIVE);
        assert_eq!(
            std::fs::read_dir(dir.path().join("media")).unwrap().count(),
            0
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_one_browser_or_native_interest_preserves_other_browser() {
        let dir = tempfile::tempdir().unwrap();
        let tools = stalled_tools(dir.path());
        let prober = Prober::default();
        let source = MediaSource {
            page_url: "https://example.org/shared".into(),
            ..Default::default()
        };
        let start = |native: bool| {
            let prober = prober.clone();
            let tools = tools.clone();
            let source = source.clone();
            tokio::spawn(async move {
                if native {
                    prober.probe(&tools, &source, "none").await
                } else {
                    prober.probe_browser(&tools, &source, "none").await
                }
            })
        };
        let first = start(false);
        let second = start(false);
        let native = start(true);
        await_started(dir.path(), 1).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let count = prober
                    .inner
                    .pages
                    .lock()
                    .unwrap()
                    .values()
                    .next()
                    .unwrap()
                    .interests
                    .lock()
                    .unwrap()
                    .len();
                if count == 3 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        first.abort();
        let _ = first.await;
        prober.cancel(&source.page_url);
        assert!(native.await.unwrap().unwrap_err().contains("cancelled"));
        assert!(!second.is_finished());
        assert_eq!(started_count(dir.path()), 1);
        let stage = prober.stage(&source.page_url).unwrap();
        let pid: i32 = stage.strip_prefix("PID ").unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        second.abort();
        let _ = second.await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while !prober.inner.pages.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
}
