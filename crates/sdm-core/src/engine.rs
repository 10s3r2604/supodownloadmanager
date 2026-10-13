//! The download engine: probing the server, planning segments, running the
//! connections, saving progress and finishing the file.

use std::collections::{HashSet, VecDeque};
use std::io::{self, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Response, StatusCode, Url};
use serde::Serialize;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::completion::prepare_completion;
use crate::error::{Error, Result};
use crate::filename;
use crate::limiter::RateLimiter;
use crate::probe::{self, ContentRange, RemoteInfo};
use crate::segment::{Scheduler, Segment, lock};
use crate::state::{PartialFiles, SavedSegment, SavedState, sync_parent};

/// Data is collected in memory up to this size before it is written.
const WRITE_BUFFER: usize = 256 * 1024;
/// How often speed is measured.
const TICK: Duration = Duration::from_millis(250);
/// Changed progress is saved every `SAVE_EVERY` ticks (four seconds).
const SAVE_EVERY: u32 = 16;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(15);
const PROBE_ATTEMPTS: u32 = 3;
const MAX_REMOTE_RESTARTS: u32 = 1;

const RUNNING: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

/// Settings shared by all downloads of a [`Downloader`].
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// `User-Agent` sent with every request.
    pub user_agent: String,
    /// How long to wait for a connection to be established.
    pub connect_timeout: Duration,
    /// How long a connection may stay silent before it is considered dead.
    pub read_timeout: Duration,
    /// Which proxy to use.
    pub proxy: ProxyConfig,
    /// Total speed limit for all downloads in bytes per second.
    pub speed_limit: Option<u64>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("SupoDownloadManager/{}", env!("CARGO_PKG_VERSION")),
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
            proxy: ProxyConfig::System,
            speed_limit: None,
        }
    }
}

/// Proxy settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProxyConfig {
    /// Use the proxy configured in the environment (`HTTPS_PROXY`, ...).
    #[default]
    System,
    /// Connect directly.
    Disabled,
    /// Use this proxy (`http://`, `https://` or `socks5://`).
    Url(String),
}

/// Where a download should be saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Save into this directory using the name suggested by the server. When
    /// the name is taken, `name (1).ext`, `name (2).ext`, ... is used.
    Directory(PathBuf),
    /// Save to exactly this file.
    File(PathBuf),
}

/// Tuning knobs for a single download.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Maximum number of simultaneous connections.
    pub connections: usize,
    /// Segments are never made smaller than this many bytes.
    pub min_segment_size: u64,
    /// How often a connection retries after consecutive failures.
    pub max_retries: u32,
    /// Delay before the first retry; it doubles with every further attempt.
    pub retry_delay: Duration,
    /// Speed limit for this download in bytes per second.
    pub speed_limit: Option<u64>,
    /// Replace an existing file instead of choosing another name.
    pub overwrite: bool,
    /// Keep unfinished data and progress under this root. `None` preserves
    /// the CLI's sidecar layout. Each destination gets an isolated subfolder.
    pub temp_dir: Option<PathBuf>,
    /// Retain a verified publication journal until the application saves its
    /// Completed history and calls [`crate::acknowledge_completion`]. Use a
    /// stable, unique application row ID. `None` preserves CLI cleanup.
    pub completion_id: Option<String>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            connections: 8,
            min_segment_size: 512 * 1024,
            max_retries: 6,
            retry_delay: Duration::from_millis(500),
            speed_limit: None,
            overwrite: false,
            temp_dir: None,
            completion_id: None,
        }
    }
}

/// Everything needed to start a download.
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    /// The `http` or `https` URL to download.
    pub url: String,
    /// Where to save the file.
    pub destination: Destination,
    /// Extra request headers such as `Cookie`, `Referer` or `Authorization`.
    pub headers: Vec<(String, String)>,
    /// Tuning knobs.
    pub options: DownloadOptions,
}

impl DownloadRequest {
    /// Creates a request with default options.
    pub fn new(url: impl Into<String>, destination: Destination) -> Self {
        Self {
            url: url.into(),
            destination,
            headers: Vec::new(),
            options: DownloadOptions::default(),
        }
    }

    /// Adds a request header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Replaces the options.
    pub fn options(mut self, options: DownloadOptions) -> Self {
        self.options = options;
        self
    }
}

/// A small file read into memory by [`Downloader::fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub data: Vec<u8>,
    /// The `Content-Type` the server sent.
    pub content_type: Option<String>,
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The file is complete.
    Completed {
        /// Where the file was saved.
        path: PathBuf,
        /// Its size in bytes.
        size: u64,
    },
    /// The download was paused.
    Paused {
        /// Where the file will be saved, if already known.
        path: Option<PathBuf>,
        /// `true` when starting the download again continues where it
        /// stopped, `false` when the server forces it to start over.
        resumable: bool,
    },
    /// The download was cancelled and its partial data deleted.
    Cancelled,
}

/// What a download is doing right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Contacting the server.
    #[default]
    Connecting,
    /// Transferring data.
    Downloading,
    /// Moving the finished file into place.
    Finishing,
    /// Done.
    Completed,
    /// Stopped by [`DownloadControl::pause`].
    Paused,
    /// Stopped by [`DownloadControl::cancel`].
    Cancelled,
    /// Stopped by an error.
    Failed,
}

/// A snapshot of a running download.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// What the download is doing.
    pub phase: Phase,
    /// File name, once known.
    pub file_name: Option<String>,
    /// Final path, once known.
    pub path: Option<PathBuf>,
    /// Total size, if the server reported it.
    pub total: Option<u64>,
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Current speed in bytes per second.
    pub speed: u64,
    /// Connections currently transferring data.
    pub connections: usize,
    /// Whether the download can be paused and resumed.
    pub resumable: bool,
    /// Per-segment progress (empty when the size is unknown).
    pub segments: Vec<SegmentProgress>,
    /// Bytes that were already on disk when this session started (non-zero
    /// when the download was resumed).
    pub resumed_from: u64,
    /// Error message when the download failed.
    pub error: Option<String>,
}

impl Progress {
    /// Completed fraction between 0 and 1, if the size is known.
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(0) => Some(1.0),
            Some(total) => Some((self.downloaded as f64 / total as f64).min(1.0)),
            None => None,
        }
    }

    /// Estimated time left at the current speed.
    pub fn eta(&self) -> Option<Duration> {
        let left = self.total?.saturating_sub(self.downloaded);
        (self.speed > 0).then(|| Duration::from_secs(left.div_ceil(self.speed)))
    }
}

/// Progress of one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SegmentProgress {
    /// First byte of the segment.
    pub start: u64,
    /// One past the last byte of the segment.
    pub end: u64,
    /// Bytes of this segment downloaded so far.
    pub downloaded: u64,
}

/// Starts downloads. Cheap to clone; clones share connections and the
/// global speed limit.
#[derive(Debug, Clone)]
pub struct Downloader {
    client: Client,
    limiter: Arc<RateLimiter>,
}

impl Downloader {
    /// Creates a downloader.
    pub fn new(config: ClientConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(config.user_agent)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .redirect(reqwest::redirect::Policy::limited(10));
        builder = match config.proxy {
            ProxyConfig::System => builder,
            ProxyConfig::Disabled => builder.no_proxy(),
            ProxyConfig::Url(url) => builder.proxy(reqwest::Proxy::all(url)?),
        };
        Ok(Self {
            client: builder.build()?,
            limiter: Arc::new(RateLimiter::new(config.speed_limit)),
        })
    }

    /// Changes the total speed limit of all downloads.
    pub fn set_speed_limit(&self, limit: Option<u64>) {
        self.limiter.set_limit(limit);
    }

    /// The total speed limit of all downloads.
    pub fn speed_limit(&self) -> Option<u64> {
        self.limiter.limit()
    }

    /// Asks the server about a file without downloading it.
    pub async fn probe(&self, url: &str, headers: &[(String, String)]) -> Result<RemoteInfo> {
        let url = parse_url(url)?;
        let headers = build_headers(headers)?;
        let options = DownloadOptions::default();
        probe_with_retry(
            &self.client,
            &url,
            &headers,
            &options,
            &CancellationToken::new(),
        )
        .await
    }

    /// Downloads the bytes `range` of a file into memory, calling
    /// `progress` with the number of bytes received so far. Fails with
    /// [`Error::RangeIgnored`] when the server sends something else.
    ///
    /// Handy for taking one file out of a big archive on a server.
    pub async fn fetch_range(
        &self,
        url: &str,
        range: std::ops::Range<u64>,
        mut progress: impl FnMut(u64),
    ) -> Result<Vec<u8>> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let url = parse_url(url)?;
        let response = self
            .client
            .get(url)
            .header(
                header::RANGE,
                format!("bytes={}-{}", range.start, range.end - 1),
            )
            .send()
            .await?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {}
            StatusCode::OK => return Err(Error::RangeIgnored),
            status => return Err(Error::Status(status)),
        }
        let first = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(probe::parse_content_range)
            .and_then(|r| r.range)
            .map(|(first, _)| first);
        if first != Some(range.start) {
            return Err(Error::BadContentRange);
        }
        let wanted = range.end - range.start;
        let mut data = Vec::with_capacity(usize::try_from(wanted).unwrap_or(0));
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            self.limiter.acquire(chunk.len()).await;
            data.extend_from_slice(&chunk);
            progress(data.len() as u64);
        }
        if (data.len() as u64) < wanted {
            return Err(Error::UnexpectedEof);
        }
        data.truncate(usize::try_from(wanted).unwrap_or(usize::MAX));
        Ok(data)
    }

    /// Downloads a small file, such as a `.torrent` file, into memory.
    /// Fails with [`Error::TooLarge`] when it has more than `limit` bytes.
    pub async fn fetch(
        &self,
        url: &str,
        headers: &[(String, String)],
        limit: u64,
    ) -> Result<Fetched> {
        let url = parse_url(url)?;
        let response = self
            .client
            .get(url)
            .headers(build_headers(headers)?)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Status(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(Error::TooLarge(limit));
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut data = Vec::new();
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            if (data.len() + chunk.len()) as u64 > limit {
                return Err(Error::TooLarge(limit));
            }
            self.limiter.acquire(chunk.len()).await;
            data.extend_from_slice(&chunk);
        }
        Ok(Fetched { data, content_type })
    }

    /// Starts a download in the background. Must be called from within a
    /// Tokio runtime.
    pub fn start(&self, request: DownloadRequest) -> DownloadHandle {
        let shared = Arc::new(Shared {
            cancel: CancellationToken::new(),
            stop: AtomicU8::new(RUNNING),
            limiter: RateLimiter::new(request.options.speed_limit),
            speed: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            status: Mutex::new(Status::default()),
        });
        let task = tokio::spawn(run_task(
            self.client.clone(),
            self.limiter.clone(),
            shared.clone(),
            request,
        ));
        DownloadHandle {
            control: DownloadControl { shared },
            task,
        }
    }
}

/// A running download. Dropping the handle does not stop the download.
#[derive(Debug)]
pub struct DownloadHandle {
    control: DownloadControl,
    task: JoinHandle<Result<Outcome>>,
}

impl DownloadHandle {
    /// A cloneable remote control for this download.
    pub fn control(&self) -> DownloadControl {
        self.control.clone()
    }

    /// See [`DownloadControl::progress`].
    pub fn progress(&self) -> Progress {
        self.control.progress()
    }

    /// See [`DownloadControl::pause`].
    pub fn pause(&self) {
        self.control.pause();
    }

    /// See [`DownloadControl::cancel`].
    pub fn cancel(&self) {
        self.control.cancel();
    }

    /// Whether the download has ended.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Waits for the download to end.
    pub async fn wait(self) -> Result<Outcome> {
        match self.task.await {
            Ok(result) => result,
            Err(e) => Err(Error::Io(io::Error::other(format!(
                "download task failed: {e}"
            )))),
        }
    }
}

/// Controls a download from anywhere, for example from a user interface.
#[derive(Debug, Clone)]
pub struct DownloadControl {
    shared: Arc<Shared>,
}

impl DownloadControl {
    /// Stops the download and keeps the data, so it can be resumed later by
    /// starting the same download again.
    pub fn pause(&self) {
        self.shared.request_stop(PAUSE);
    }

    /// Stops the download and deletes the partial data.
    pub fn cancel(&self) {
        self.shared.request_stop(CANCEL);
    }

    /// Changes the speed limit of this download.
    pub fn set_speed_limit(&self, limit: Option<u64>) {
        self.shared.limiter.set_limit(limit);
    }

    /// Takes a snapshot of the progress.
    pub fn progress(&self) -> Progress {
        let status = lock(&self.shared.status);
        let segments = status
            .scheduler
            .as_ref()
            .map(|s| s.snapshot())
            .unwrap_or_default();
        let total = status.info.as_ref().and_then(|info| info.size);
        Progress {
            phase: status.phase,
            file_name: status.info.as_ref().map(|info| info.file_name.clone()),
            path: status.path.clone(),
            total,
            downloaded: segments.iter().map(|s| s.claimed - s.start).sum(),
            speed: self.shared.speed.load(Ordering::Relaxed),
            connections: self.shared.connections.load(Ordering::Relaxed),
            resumable: status.info.as_ref().is_some_and(|info| info.resumable),
            segments: if total.is_some() {
                segments
                    .iter()
                    .map(|s| SegmentProgress {
                        start: s.start,
                        end: s.end,
                        downloaded: s.claimed - s.start,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            resumed_from: status.resumed_from,
            error: status.error.clone(),
        }
    }
}

/// State shared between a download task and its controls.
#[derive(Debug)]
struct Shared {
    cancel: CancellationToken,
    stop: AtomicU8,
    limiter: RateLimiter,
    speed: AtomicU64,
    connections: AtomicUsize,
    status: Mutex<Status>,
}

#[derive(Debug, Default)]
struct Status {
    phase: Phase,
    info: Option<RemoteInfo>,
    path: Option<PathBuf>,
    error: Option<String>,
    scheduler: Option<Arc<Scheduler>>,
    resumed_from: u64,
}

impl Shared {
    fn request_stop(&self, reason: u8) {
        // A cancel overrides a pause, never the other way round.
        self.stop.fetch_max(reason, Ordering::SeqCst);
        self.cancel.cancel();
    }

    fn stop_reason(&self) -> u8 {
        self.stop.load(Ordering::SeqCst)
    }

    fn stopped_outcome(&self, path: Option<PathBuf>) -> Outcome {
        match self.stop_reason() {
            CANCEL => Outcome::Cancelled,
            _ => Outcome::Paused {
                path,
                resumable: true,
            },
        }
    }
}

/// Paths that are being downloaded by this process right now.
static ACTIVE_PATHS: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);

/// Keeps a path reserved in [`ACTIVE_PATHS`] while it is alive.
pub(crate) struct PathGuard(PathBuf);

impl PathGuard {
    pub(crate) fn try_new(path: &Path) -> Option<Self> {
        lock(&ACTIVE_PATHS)
            .insert(path.to_owned())
            .then(|| Self(path.to_owned()))
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        lock(&ACTIVE_PATHS).remove(&self.0);
    }
}

/// Cleans this download's workspace only when no transfer owns its target.
pub async fn cleanup_partial(path: &Path, temp_dir: Option<&Path>) -> Result<()> {
    let _guard = PathGuard::try_new(path).ok_or_else(|| Error::InUse(path.to_owned()))?;
    PartialFiles::new(path, temp_dir).cleanup().await?;
    let root = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".sdm-finalizing");
    PartialFiles::new(path, Some(&root)).cleanup().await?;
    let _ = fs::remove_dir(root).await;
    Ok(())
}

/// Moves legacy sidecars without contacting the server or starting a transfer.
pub async fn migrate_partial(path: &Path, temp_dir: &Path) -> Result<()> {
    let _guard = PathGuard::try_new(path).ok_or_else(|| Error::InUse(path.to_owned()))?;
    PartialFiles::new(path, Some(temp_dir)).prepare(path).await
}

async fn run_task(
    client: Client,
    global: Arc<RateLimiter>,
    shared: Arc<Shared>,
    request: DownloadRequest,
) -> Result<Outcome> {
    let result = run(client, global, &shared, request).await;
    shared.speed.store(0, Ordering::Relaxed);
    shared.connections.store(0, Ordering::Relaxed);
    let mut status = lock(&shared.status);
    status.phase = match &result {
        Ok(Outcome::Completed { .. }) => Phase::Completed,
        Ok(Outcome::Paused { .. }) => Phase::Paused,
        Ok(Outcome::Cancelled) => Phase::Cancelled,
        Err(_) => Phase::Failed,
    };
    if let Err(error) = &result {
        status.error = Some(error.to_string());
    }
    result
}

async fn run(
    client: Client,
    global: Arc<RateLimiter>,
    shared: &Arc<Shared>,
    request: DownloadRequest,
) -> Result<Outcome> {
    let DownloadRequest {
        url,
        destination,
        headers,
        options,
    } = request;
    let url = parse_url(&url)?;
    let headers = build_headers(&headers)?;

    let mut info = match probe_with_retry(&client, &url, &headers, &options, &shared.cancel).await {
        Err(Error::Cancelled) => return Ok(shared.stopped_outcome(None)),
        result => result?,
    };
    let (path, mut saved, _guard, files) = resolve_destination(
        &destination,
        &info,
        options.overwrite,
        options.temp_dir.as_deref(),
    )
    .await?;
    {
        let mut status = lock(&shared.status);
        status.info = Some(info.clone());
        status.path = Some(path.clone());
        status.phase = Phase::Downloading;
    }
    if let Some(saved) = &saved {
        tracing::info!(path = %path.display(), segments = saved.segments.len(), "resuming download");
    }

    let mut ranged = info.resumable && info.size.is_some_and(|size| size > 0);
    let mut remote_restarts = 0;
    loop {
        let ctx = Arc::new(Ctx {
            client: client.clone(),
            url: url.clone(),
            headers: headers.clone(),
            part: files.data.clone(),
            state: files.state.clone(),
            info: info.clone(),
            ranged,
            options: options.clone(),
            shared: shared.clone(),
            global: global.clone(),
            cancel: shared.cancel.child_token(),
        });
        match transfer(ctx, &path, saved.take()).await {
            Err(Error::RemoteChanged) => {
                // No byte from the previous representation may survive. Bound
                // the automatic restart so unstable servers cannot loop.
                files.cleanup().await?;
                {
                    let mut status = lock(&shared.status);
                    status.scheduler = None;
                    status.resumed_from = 0;
                    status.phase = Phase::Connecting;
                }
                if remote_restarts >= MAX_REMOTE_RESTARTS {
                    return Err(Error::RemoteChanged);
                }
                remote_restarts += 1;
                info = match probe_with_retry(&client, &url, &headers, &options, &shared.cancel)
                    .await
                {
                    Err(Error::Cancelled) => return Ok(shared.stopped_outcome(Some(path))),
                    result => result?,
                };
                files.prepare(&path).await?;
                ranged = info.resumable && info.size.is_some_and(|size| size > 0);
                let mut status = lock(&shared.status);
                status.info = Some(info.clone());
                status.phase = Phase::Downloading;
            }
            Err(Error::RangeIgnored) if ranged => {
                tracing::warn!("server ignores byte ranges; continuing with a single connection");
                ranged = false;
                remove_if_exists(&files.state).await?;
            }
            result => {
                if matches!(
                    result,
                    Ok(Outcome::Completed { .. }
                        | Outcome::Cancelled
                        | Outcome::Paused {
                            resumable: false,
                            ..
                        })
                ) {
                    let cleanup = if options.completion_id.is_some()
                        && matches!(result, Ok(Outcome::Completed { .. }))
                    {
                        files.cleanup_transfer().await
                    } else {
                        files.cleanup().await
                    };
                    if let Err(error) = cleanup {
                        tracing::warn!(%error, "could not clean download temporary files");
                    }
                    if options.temp_dir.is_some()
                        && let Err(error) = PartialFiles::new(&path, None).cleanup().await
                    {
                        tracing::warn!(%error, "could not clean migrated download files");
                    }
                }
                return result;
            }
        }
    }
}

async fn probe_with_retry(
    client: &Client,
    url: &Url,
    headers: &HeaderMap,
    options: &DownloadOptions,
    cancel: &CancellationToken,
) -> Result<RemoteInfo> {
    let mut attempt = 0;
    loop {
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            result = probe::probe(client, url, headers) => result,
        };
        match result {
            Err(error) if error.is_retryable() && attempt + 1 < PROBE_ATTEMPTS => {
                attempt += 1;
                let delay = retry_delay(options.retry_delay, attempt);
                tracing::warn!(%error, "could not reach server; retrying in {delay:?}");
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(Error::Cancelled),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            result => return result,
        }
    }
}

/// Picks the final path and finds saved progress to resume from.
async fn resolve_destination(
    destination: &Destination,
    info: &RemoteInfo,
    overwrite: bool,
    temp_dir: Option<&Path>,
) -> Result<(PathBuf, Option<SavedState>, PathGuard, PartialFiles)> {
    match destination {
        Destination::File(path) => {
            let guard = PathGuard::try_new(path).ok_or_else(|| Error::InUse(path.clone()))?;
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent).await?;
            }
            let files = PartialFiles::new(path, temp_dir);
            files.prepare(path).await?;
            if let Some(saved) = saved_progress(&files, info).await? {
                return Ok((path.clone(), Some(saved), guard, files));
            }
            if !overwrite && fs::try_exists(path).await? {
                if let Some(directory) = files.directory() {
                    let _ = fs::remove_dir(directory).await;
                }
                return Err(Error::FileExists(path.clone()));
            }
            Ok((path.clone(), None, guard, files))
        }
        Destination::Directory(dir) => {
            fs::create_dir_all(dir).await?;
            for candidate in filename::candidates(dir, &info.file_name) {
                let Some(guard) = PathGuard::try_new(&candidate) else {
                    continue;
                };
                let files = PartialFiles::new(&candidate, temp_dir);
                files.prepare(&candidate).await?;
                if let Some(saved) = saved_progress(&files, info).await? {
                    return Ok((candidate, Some(saved), guard, files));
                }
                if !overwrite && fs::try_exists(&candidate).await? {
                    if let Some(directory) = files.directory() {
                        let _ = fs::remove_dir(directory).await;
                    }
                    continue;
                }
                // Creating the part file reserves the name, so two programs
                // downloading the same file do not write into each other.
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&files.data)
                    .await
                {
                    Ok(_) => return Ok((candidate, None, guard, files)),
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            Err(Error::FileExists(dir.join(&info.file_name)))
        }
    }
}

/// Loads usable saved progress for `path`. Progress that does not match the
/// file on the server any more is deleted.
async fn saved_progress(files: &PartialFiles, info: &RemoteInfo) -> Result<Option<SavedState>> {
    let state_path = &files.state;
    let part_path = &files.data;
    let saved = match SavedState::load(state_path).await {
        Ok(None) => return Ok(None),
        Ok(Some(saved)) => Some(saved),
        Err(Error::State(error)) => {
            tracing::warn!(%error, path = %state_path.display(), "ignoring unreadable progress file");
            None
        }
        Err(error) => return Err(error),
    };
    if let Some(saved) = saved
        && saved.is_valid()
        && saved.has_valid_data(part_path).await?
    {
        if saved.matches(info) {
            return Ok(Some(saved));
        }
        if saved.url != info.url {
            // A new link must be validated by retarget_partial before
            // replacing a different URL's usable saved data.
            return Err(Error::NotSameFile(
                "this partial download belongs to another link; refresh its link or restart it"
                    .into(),
            ));
        }
    }
    tracing::warn!(path = %part_path.display(), "the file changed on the server; starting over");
    remove_if_exists(state_path).await?;
    remove_if_exists(part_path).await?;
    Ok(None)
}

/// Everything a connection needs to know.
struct Ctx {
    client: Client,
    url: Url,
    headers: HeaderMap,
    part: PathBuf,
    state: PathBuf,
    info: RemoteInfo,
    /// Whether we use byte ranges (several connections, resumable).
    ranged: bool,
    options: DownloadOptions,
    shared: Arc<Shared>,
    global: Arc<RateLimiter>,
    cancel: CancellationToken,
}

impl Ctx {
    async fn throttle(&self, bytes: usize) {
        self.global.acquire(bytes).await;
        self.shared.limiter.acquire(bytes).await;
    }

    async fn save_state(&self, scheduler: &Scheduler) -> Result<()> {
        match self.info.size {
            Some(size) if self.ranged => {
                // Snapshot first, then sync the data backing those positions.
                // Taking the snapshot after syncing could claim newer writes
                // which have not yet reached stable storage.
                let saved = SavedState::new(&self.info, size, scheduler.saved_segments());
                OpenOptions::new()
                    .write(true)
                    .open(&self.part)
                    .await?
                    .sync_all()
                    .await?;
                saved.save(&self.state).await
            }
            _ => Ok(()),
        }
    }
}

/// Runs the connections until the file is complete, the download is
/// stopped or an error occurs.
async fn transfer(ctx: Arc<Ctx>, path: &Path, saved: Option<SavedState>) -> Result<Outcome> {
    let shared = &ctx.shared;
    let resuming = saved.is_some();
    let plan = match saved {
        Some(saved) => saved.segments,
        None => initial_plan(ctx.info.size, ctx.ranged, &ctx.options),
    };

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resuming)
        .open(&ctx.part)
        .await?;
    if let Some(size) = ctx.info.size {
        file.set_len(size).await?;
    }
    drop(file);

    let workers = if ctx.ranged {
        ctx.options.connections.max(1)
    } else {
        1
    };
    let scheduler = Arc::new(Scheduler::new(
        &plan,
        ctx.ranged,
        ctx.options.min_segment_size,
        workers,
    ));
    {
        let mut status = lock(&shared.status);
        status.resumed_from = scheduler.downloaded();
        status.scheduler = Some(scheduler.clone());
    }

    // Keep a fresh range plan recoverable from the first connection onward.
    ctx.save_state(&scheduler).await?;

    let mut last_saved = scheduler.saved_segments();

    let mut tasks = JoinSet::new();
    for _ in 0..workers {
        tasks.spawn(worker(ctx.clone(), scheduler.clone()));
    }

    let mut ticker = tokio::time::interval(TICK);
    let mut meter = SpeedMeter::default();
    let mut ticks = 0u32;
    let mut failure = None;
    loop {
        tokio::select! {
            joined = tasks.join_next() => match joined {
                None => break,
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(error))) => {
                    failure.get_or_insert(error);
                    ctx.cancel.cancel();
                }
                Some(Err(error)) => {
                    failure.get_or_insert(Error::Io(io::Error::other(error)));
                    ctx.cancel.cancel();
                }
            },
            _ = ticker.tick() => {
                shared.speed.store(meter.update(scheduler.downloaded()), Ordering::Relaxed);
                ticks += 1;
                if ticks.is_multiple_of(SAVE_EVERY) && ctx.ranged {
                    let segments = scheduler.saved_segments();
                    if segments != last_saved {
                        match ctx.save_state(&scheduler).await {
                            // Keep the pre-flush snapshot: workers may advance
                            // while syncing, and those bytes need a later save.
                            Ok(()) => last_saved = segments,
                            Err(error) => tracing::warn!(%error, "could not save progress"),
                        }
                    }
                }
            }
        }
    }
    shared.speed.store(0, Ordering::Relaxed);

    // A simultaneous pause must never persist positions from a representation
    // which one of the workers has already rejected.
    if matches!(&failure, Some(Error::RemoteChanged)) && shared.stop_reason() != RUNNING {
        PartialFiles::new(path, ctx.options.temp_dir.as_deref())
            .cleanup()
            .await?;
        return Ok(if shared.stop_reason() == CANCEL {
            Outcome::Cancelled
        } else {
            Outcome::Paused {
                path: Some(path.to_owned()),
                resumable: false,
            }
        });
    }

    match shared.stop_reason() {
        CANCEL => {
            remove_if_exists(&ctx.part).await?;
            remove_if_exists(&ctx.state).await?;
            return Ok(Outcome::Cancelled);
        }
        PAUSE => {
            let path = Some(path.to_owned());
            return if ctx.ranged {
                ctx.save_state(&scheduler).await?;
                Ok(Outcome::Paused {
                    path,
                    resumable: true,
                })
            } else {
                remove_if_exists(&ctx.part).await?;
                Ok(Outcome::Paused {
                    path,
                    resumable: false,
                })
            };
        }
        _ => {}
    }

    if let Some(error) = failure {
        if matches!(error, Error::RangeIgnored | Error::RemoteChanged) {
            return Err(error);
        }
        if ctx.ranged {
            if let Err(e) = ctx.save_state(&scheduler).await {
                tracing::warn!(error = %e, "could not save progress");
            }
        } else {
            remove_if_exists(&ctx.part).await?;
        }
        return Err(error);
    }

    if !scheduler.is_complete() {
        return Err(Error::UnexpectedEof);
    }

    lock(&shared.status).phase = Phase::Finishing;
    let size = scheduler.downloaded();
    if !ctx.ranged {
        // The size may have been unknown, or an earlier attempt wrote more.
        OpenOptions::new()
            .write(true)
            .open(&ctx.part)
            .await?
            .set_len(size)
            .await?;
    }
    // Save complete progress before committing, so disk/copy failures can be
    // retried without downloading the bytes again.
    ctx.save_state(&scheduler).await?;
    // Non-ranged downloads have no state snapshot but their data must also
    // reach stable storage before publishing the final name.
    OpenOptions::new()
        .write(true)
        .open(&ctx.part)
        .await?
        .sync_all()
        .await?;
    // Without ranges the data cannot be kept for later, so a pause (or
    // quitting) must not stop the last steps; only deleting it does.
    let never = CancellationToken::new();
    let finishing_cancel = if ctx.ranged { &ctx.cancel } else { &never };
    if let Some(completion_id) = &ctx.options.completion_id {
        match prepare_completion(
            path,
            ctx.options.temp_dir.as_deref(),
            completion_id,
            size,
            finishing_cancel,
        )
        .await
        {
            Err(Error::Cancelled) => return stopped_before_publication(&ctx, path).await,
            result => result?,
        }
    }
    if let Err(error) = publish_file(&ctx, path).await {
        if matches!(error, Error::Cancelled) {
            return match shared.stop_reason() {
                CANCEL => Ok(Outcome::Cancelled),
                _ => Ok(Outcome::Paused {
                    path: Some(path.to_owned()),
                    resumable: ctx.ranged,
                }),
            };
        }
        return Err(error);
    }
    Ok(Outcome::Completed {
        path: path.to_owned(),
        size,
    })
}

async fn stopped_before_publication(ctx: &Ctx, path: &Path) -> Result<Outcome> {
    if ctx.shared.stop_reason() == CANCEL {
        PartialFiles::new(path, ctx.options.temp_dir.as_deref())
            .cleanup()
            .await?;
        Ok(Outcome::Cancelled)
    } else {
        Ok(Outcome::Paused {
            path: Some(path.to_owned()),
            resumable: ctx.ranged,
        })
    }
}

/// Rename on one volume; copy to a private staging folder on the destination
/// volume otherwise. The final filename appears only after the copy succeeds.
async fn publish_file(ctx: &Ctx, path: &Path) -> Result<()> {
    // Paused downloads without ranges finish anyway: see `finish`.
    let stopped =
        || ctx.shared.cancel.is_cancelled() && (ctx.ranged || ctx.shared.stop_reason() == CANCEL);
    if stopped() {
        return Err(Error::Cancelled);
    }
    match fs::rename(&ctx.part, path).await {
        Ok(()) => {
            sync_parent(path).await?;
            sync_parent(&ctx.part).await?;
            return Ok(());
        }
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
        Err(error) => return Err(error.into()),
    }
    let root = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".sdm-finalizing");
    let staging_files = PartialFiles::new(path, Some(&root));
    let directory = staging_files.directory().unwrap();
    fs::create_dir_all(directory).await?;
    let result = async {
        let mut source = File::open(&ctx.part).await?;
        let mut target = File::create(&staging_files.data).await?;
        let mut buffer = vec![0; WRITE_BUFFER];
        loop {
            if stopped() {
                return Err(Error::Cancelled);
            }
            let count = source.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            target.write_all(&buffer[..count]).await?;
        }
        target.sync_all().await?;
        drop(target);
        if stopped() {
            return Err(Error::Cancelled);
        }
        fs::rename(&staging_files.data, path).await?;
        sync_parent(path).await?;
        Ok(())
    }
    .await;
    if let Err(error) = staging_files.cleanup().await {
        tracing::warn!(%error, "could not clean finalization staging files");
    }
    let _ = fs::remove_dir(root).await; // Only remove an empty shared folder.
    result
}

/// Splits a fresh download into segments of roughly equal size.
fn initial_plan(size: Option<u64>, ranged: bool, options: &DownloadOptions) -> Vec<SavedSegment> {
    let whole = |end| {
        vec![SavedSegment {
            start: 0,
            pos: 0,
            end,
        }]
    };
    match size {
        Some(size) if ranged => {
            let count = (size / options.min_segment_size.max(1))
                .clamp(1, options.connections.max(1) as u64);
            let chunk = size / count;
            (0..count)
                .map(|i| {
                    let start = i * chunk;
                    let end = if i + 1 == count { size } else { start + chunk };
                    SavedSegment {
                        start,
                        pos: start,
                        end,
                    }
                })
                .collect()
        }
        Some(size) => whole(size),
        None => whole(u64::MAX),
    }
}

enum SegmentEnd {
    Done,
    Retired,
    Stopped,
}

/// One connection: keeps taking segments until there is no work left.
async fn worker(ctx: Arc<Ctx>, scheduler: Arc<Scheduler>) -> Result<()> {
    while let Some(segment) = scheduler.next() {
        match download_segment(&ctx, &scheduler, &segment).await {
            Ok(SegmentEnd::Done) => {}
            Ok(SegmentEnd::Retired) => return Ok(()),
            Ok(SegmentEnd::Stopped) => {
                scheduler.leave();
                return Ok(());
            }
            Err(error) => {
                scheduler.leave();
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Downloads one segment, retrying on network errors.
async fn download_segment(
    ctx: &Ctx,
    scheduler: &Scheduler,
    segment: &Segment,
) -> Result<SegmentEnd> {
    let mut failures = 0;
    loop {
        if ctx.cancel.is_cancelled() {
            return Ok(SegmentEnd::Stopped);
        }
        let before = segment.written();
        let error = match fetch(ctx, segment).await {
            Ok(()) => return Ok(SegmentEnd::Done),
            Err(_) if ctx.cancel.is_cancelled() => return Ok(SegmentEnd::Stopped),
            Err(error) => error,
        };
        if !error.is_retryable() {
            return Err(error);
        }
        if segment.written() > before {
            failures = 0;
        }
        failures += 1;
        if failures > ctx.options.max_retries {
            return Err(error);
        }
        if !ctx.ranged {
            segment.restart();
        }
        // Servers often limit connections per client; if this connection
        // keeps failing, let the others finish its work.
        if failures >= 2 && scheduler.try_retire(segment) {
            tracing::debug!(%error, "closing a connection that keeps failing");
            return Ok(SegmentEnd::Retired);
        }
        let delay = retry_delay(ctx.options.retry_delay, failures);
        tracing::warn!(%error, attempt = failures, "connection failed; retrying in {delay:?}");
        tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Ok(SegmentEnd::Stopped),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// Makes one request for the rest of a segment and writes what arrives.
async fn fetch(ctx: &Ctx, segment: &Segment) -> Result<()> {
    let Some((start, end)) = segment.begin() else {
        return Ok(());
    };
    let mut request = ctx.client.get(ctx.url.clone()).headers(ctx.headers.clone());
    if ctx.ranged {
        request = request.header(header::RANGE, format!("bytes={start}-{}", end - 1));
        if let Some(validator) = if_range_validator(&ctx.info) {
            request = request.header(header::IF_RANGE, validator);
        }
    }
    let response = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => return Err(Error::Cancelled),
        response = request.send() => response?,
    };
    check_response(ctx, &response, start, end)?;
    let _active = ActiveConnection::new(&ctx.shared.connections);

    let mut file = OpenOptions::new().write(true).open(&ctx.part).await?;
    file.seek(SeekFrom::Start(start)).await?;
    let mut writer = SegmentWriter {
        file,
        buffer: Vec::with_capacity(WRITE_BUFFER),
        segment,
    };
    let streamed = stream_body(ctx, response, segment, &mut writer).await;
    // Keep whatever arrived, even when the connection broke.
    let flushed = writer.flush().await;
    streamed.and(flushed)
}

/// Weak entity tags are forbidden in If-Range; a date is the fallback.
fn if_range_validator(info: &RemoteInfo) -> Option<&str> {
    info.etag
        .as_deref()
        .filter(|etag| etag.starts_with('"') && etag.ends_with('"'))
        .or(info.last_modified.as_deref())
}

fn check_response(ctx: &Ctx, response: &Response, start: u64, end: u64) -> Result<()> {
    let status = response.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE
        && let Some(range) = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(probe::parse_content_range)
        && range.total.is_some()
        && range.total != ctx.info.size
    {
        return Err(Error::RemoteChanged);
    }
    // A different entity tag means another version of the file. A missing
    // one proves nothing (If-Range already guards ranges), and Last-Modified
    // is left out: many servers send the current time with every response.
    if status.is_success()
        && let Some(expected) = &ctx.info.etag
        && let Some(actual) = response
            .headers()
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
        && actual != expected.as_str()
    {
        return Err(Error::RemoteChanged);
    }
    if !ctx.ranged {
        return if status.is_success() {
            Ok(())
        } else {
            Err(Error::Status(status))
        };
    }
    match status {
        StatusCode::PARTIAL_CONTENT => {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(probe::parse_content_range);
            match range {
                Some(ContentRange {
                    range: Some((first, last)),
                    total,
                }) if first == start && last < end => {
                    if total.is_some() && total != ctx.info.size {
                        return Err(Error::RemoteChanged);
                    }
                    Ok(())
                }
                _ => Err(Error::BadContentRange),
            }
        }
        // A full response must restart the entire file with one connection,
        // even at offset zero; otherwise its bytes can mix with other ranges.
        StatusCode::OK => Err(Error::RangeIgnored),
        status => Err(Error::Status(status)),
    }
}

async fn stream_body(
    ctx: &Ctx,
    response: Response,
    segment: &Segment,
    writer: &mut SegmentWriter<'_>,
) -> Result<()> {
    let mut body = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            chunk = body.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }
        tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            _ = ctx.throttle(chunk.len()) => {}
        }
        let take = segment.claim(chunk.len() as u64) as usize;
        writer.write(&chunk[..take]).await?;
        if take < chunk.len() || segment.is_claimed() {
            // Our part is done; another connection handles the rest.
            return Ok(());
        }
    }
    if segment.is_unbounded() {
        segment.close();
        return Ok(());
    }
    if segment.is_claimed() {
        Ok(())
    } else {
        Err(Error::UnexpectedEof)
    }
}

/// Buffers received data and writes it to the right place in the file.
struct SegmentWriter<'a> {
    file: File,
    buffer: Vec<u8>,
    segment: &'a Segment,
}

impl SegmentWriter<'_> {
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(data);
        if self.buffer.len() >= WRITE_BUFFER {
            self.flush().await?;
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let result = async {
            self.file.write_all(&self.buffer).await?;
            self.file.flush().await
        }
        .await;
        let len = self.buffer.len() as u64;
        // After a failed write the file position is unknown, so the data is
        // dropped and downloaded again by the next attempt.
        self.buffer.clear();
        result?;
        self.segment.commit(len);
        Ok(())
    }
}

/// Counts a connection as active while it is alive.
struct ActiveConnection<'a>(&'a AtomicUsize);

impl<'a> ActiveConnection<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ActiveConnection<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Average speed over the last few seconds.
#[derive(Default)]
struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    const WINDOW: Duration = Duration::from_secs(3);

    fn update(&mut self, downloaded: u64) -> u64 {
        let now = Instant::now();
        if self
            .samples
            .back()
            .is_some_and(|&(_, last)| downloaded < last)
        {
            self.samples.clear();
        }
        self.samples.push_back((now, downloaded));
        while self.samples.len() > 2 && now.duration_since(self.samples[0].0) > Self::WINDOW {
            self.samples.pop_front();
        }
        let (since, base) = self.samples[0];
        let seconds = now.duration_since(since).as_secs_f64();
        if seconds > 0.0 {
            ((downloaded - base) as f64 / seconds) as u64
        } else {
            0
        }
    }
}

fn retry_delay(base: Duration, attempt: u32) -> Duration {
    let factor = 1u32 << attempt.saturating_sub(1).min(16);
    base.saturating_mul(factor).min(MAX_RETRY_DELAY)
}

fn parse_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url.trim()).map_err(|_| Error::InvalidUrl(url.to_owned()))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        _ => Err(Error::InvalidUrl(url.to_owned())),
    }
}

/// Builds request headers. Headers that would break segmented downloads
/// (`Range`, compression) are ignored.
fn build_headers(pairs: &[(String, String)]) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let invalid = || Error::InvalidHeader(format!("{name}: {value}"));
        let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|_| invalid())?;
        if [
            header::RANGE,
            header::IF_RANGE,
            header::ACCEPT_ENCODING,
            header::CONTENT_LENGTH,
            header::HOST,
        ]
        .contains(&name)
        {
            continue;
        }
        let value = HeaderValue::from_str(value.trim()).map_err(|_| invalid())?;
        headers.append(name, value);
    }
    // Compressed responses would make byte offsets meaningless.
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    Ok(headers)
}

async fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_equal_segments() {
        let options = DownloadOptions {
            connections: 4,
            min_segment_size: 10,
            ..Default::default()
        };
        let plan = initial_plan(Some(103), true, &options);
        assert_eq!(plan.len(), 4);
        assert_eq!(
            plan[0],
            SavedSegment {
                start: 0,
                pos: 0,
                end: 25
            }
        );
        assert_eq!(
            plan[3],
            SavedSegment {
                start: 75,
                pos: 75,
                end: 103
            }
        );

        // Small files get fewer segments.
        assert_eq!(initial_plan(Some(25), true, &options).len(), 2);
        assert_eq!(initial_plan(Some(5), true, &options).len(), 1);
        // Without ranges there is one segment.
        assert_eq!(initial_plan(Some(1000), false, &options).len(), 1);
        assert_eq!(initial_plan(None, false, &options)[0].end, u64::MAX);
    }

    #[test]
    fn retry_delay_grows_and_is_capped() {
        let base = Duration::from_millis(500);
        assert_eq!(retry_delay(base, 1), Duration::from_millis(500));
        assert_eq!(retry_delay(base, 2), Duration::from_secs(1));
        assert_eq!(retry_delay(base, 3), Duration::from_secs(2));
        assert_eq!(retry_delay(base, 30), MAX_RETRY_DELAY);
    }

    #[test]
    fn rejects_unsupported_urls() {
        assert!(parse_url("https://example.com/a").is_ok());
        assert!(matches!(
            parse_url("ftp://example.com/a"),
            Err(Error::InvalidUrl(_))
        ));
        assert!(matches!(parse_url("not a url"), Err(Error::InvalidUrl(_))));
    }

    #[test]
    fn filters_headers() {
        let headers = build_headers(&[
            ("Cookie".into(), "a=1".into()),
            ("Range".into(), "bytes=5-".into()),
            ("Accept-Encoding".into(), "gzip".into()),
        ])
        .unwrap();
        assert_eq!(headers.get(header::COOKIE).unwrap(), "a=1");
        assert!(headers.get(header::RANGE).is_none());
        assert_eq!(headers.get(header::ACCEPT_ENCODING).unwrap(), "identity");
        assert!(build_headers(&[("Bad Name".into(), "x".into())]).is_err());
    }

    #[test]
    fn progress_math() {
        let progress = Progress {
            phase: Phase::Downloading,
            file_name: None,
            path: None,
            total: Some(1000),
            downloaded: 250,
            speed: 100,
            connections: 1,
            resumable: true,
            segments: Vec::new(),
            resumed_from: 0,
            error: None,
        };
        assert_eq!(progress.fraction(), Some(0.25));
        assert_eq!(progress.eta(), Some(Duration::from_secs(8)));
    }
}
