//! The download list: a queue of downloads that survives restarts.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;
use sdm_core::{
    Destination, DownloadControl, DownloadOptions, DownloadRequest, Downloader, Outcome, Phase,
    Progress, SegmentProgress,
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::category::Category;
use crate::media::{self, AddMediaRequest, MediaControl, MediaSpec, Tools};
use crate::queue::{self, MAIN_QUEUE, PowerAction, Queue, ScheduleEvent};
use crate::settings::Settings;
use crate::store;
use crate::torrent::{
    self, AddTorrentRequest, TorrentControl, TorrentEngine, TorrentInfo, TorrentLive,
    TorrentSource, TorrentSpec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

/// A download as stored in `downloads.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: u64,
    pub url: String,
    /// Request headers such as cookies and referer. Cleared once the file
    /// is complete because they may contain login data.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub file_name: String,
    pub path: PathBuf,
    /// Root used for this download's temporary files. Missing in old lists;
    /// legacy HTTP sidecars migrate at startup, legacy media keeps its workdir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_dir: Option<PathBuf>,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub resumable: bool,
    pub category: Category,
    pub state: State,
    pub error: Option<String>,
    pub added_at: u64,
    pub completed_at: Option<u64>,
    /// Time spent transferring data, over all sessions.
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Set for videos from web pages, which yt-dlp downloads; `url` is the
    /// page then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaSpec>,
    /// Set for Torrent downloads; `url` is the magnet link or `.torrent`
    /// address then, and `path` the file or folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentSpec>,
    /// The queue the download waits in.
    #[serde(default = "main_queue")]
    pub queue: String,
}

fn main_queue() -> String {
    MAIN_QUEUE.into()
}

/// What the user interface shows for a download.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadView {
    pub id: u64,
    pub url: String,
    pub file_name: String,
    pub path: PathBuf,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub resumable: bool,
    pub category: Category,
    pub state: State,
    pub error: Option<String>,
    pub added_at: u64,
    pub completed_at: Option<u64>,
    /// Milliseconds spent transferring data so far.
    pub elapsed: u64,
    /// Bytes per second.
    pub speed: u64,
    /// Seconds left.
    pub eta: Option<u64>,
    pub connections: usize,
    pub segments: Vec<SegmentProgress>,
    /// A pause or delete was requested and is being carried out.
    pub stopping: bool,
    /// A video from a web page.
    pub media: bool,
    /// The data is there and is being put together (video and audio merged).
    pub processing: bool,
    /// A Torrent download; `connections` counts peers then.
    pub torrent: bool,
    /// Sharing a finished torrent with others.
    pub seeding: bool,
    /// Checking torrent data that is already on disk.
    pub checking: bool,
    /// Bytes per second sent to other peers.
    pub upload_speed: u64,
    /// Bytes sent to other peers.
    pub uploaded: u64,
    pub queue: String,
}

/// A request to add a download.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddRequest {
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// File name chosen by the user; asked from the server when missing.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Folder chosen by the user; the category folder when missing.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    /// `true` to queue the download now, `false` to add it paused.
    #[serde(default)]
    pub start: bool,
    /// The queue to put it in; the main queue when missing.
    #[serde(default)]
    pub queue: Option<String>,
}

/// Many links added at once (a batch pattern, or the links of a page).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRequest {
    pub links: Vec<BatchLink>,
    /// Cookies and referer for all links.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// One folder for all; the category folders when missing.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub queue: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchLink {
    pub url: String,
    /// The name to save it as; taken from the link when missing.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Extra headers for this link (cookies of its site, say).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// The link's text on the page, to help pick links.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Something worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// A download began transferring data (also after resuming).
    Started {
        id: u64,
    },
    Completed {
        id: u64,
        file_name: String,
        path: PathBuf,
    },
    Failed {
        file_name: String,
        error: String,
    },
    /// A queue (or, with `queue: None`, the whole list) has nothing left
    /// to download and asks for `action`.
    Done {
        queue: Option<Queue>,
        action: PowerAction,
    },
}

pub type Notify = Box<dyn Fn(Notice) + Send + Sync>;

#[derive(Debug)]
pub enum UpdatePreparationError {
    Busy,
    Persistence(std::io::Error),
}

pub struct Manager {
    inner: Mutex<Inner>,
    store: PathBuf,
    queue_store: PathBuf,
    tools: Arc<Tools>,
    torrents: Arc<TorrentEngine>,
    runtime: Handle,
    notify: Notify,
    recovery_notice: Option<String>,
}

struct Inner {
    records: Vec<Record>,
    active: HashMap<u64, Active>,
    /// Legacy file moves in progress; removal waits for the move to finish.
    migrating: HashMap<u64, Option<bool>>,
    cleanup_pending: HashSet<u64>,
    cleanup_running: HashSet<u64>,
    cleanup_remove: HashMap<u64, bool>,
    downloader: Downloader,
    settings: Settings,
    next_id: u64,
    dirty: bool,
    closing: bool,
    /// An updater holds scheduling stopped after a successful durable save.
    update_prepared: bool,
    queues: Vec<Queue>,
    /// What to do once every download is done; not remembered.
    after_all: PowerAction,
    /// When the queue schedules were last looked at.
    checked_at: Option<chrono::NaiveDateTime>,
}

/// A running download of any kind.
enum Job {
    File(DownloadControl),
    Media(MediaControl),
    Torrent(TorrentControl),
}

impl Job {
    fn progress(&self) -> Progress {
        match self {
            Job::File(control) => control.progress(),
            Job::Media(control) => control.progress(),
            Job::Torrent(control) => control.progress(),
        }
    }

    fn torrent(&self) -> Option<TorrentLive> {
        match self {
            Job::Torrent(control) => Some(control.live()),
            _ => None,
        }
    }

    fn pause(&self) {
        match self {
            Job::File(control) => control.pause(),
            Job::Media(control) => control.pause(),
            Job::Torrent(control) => control.pause(),
        }
    }
}

struct Active {
    control: Job,
    since: Instant,
    stopping: bool,
    /// `Some(delete_file)` when the download should disappear from the list.
    remove: Option<bool>,
}

impl Manager {
    /// Loads the download list from `store`. Downloads that were running
    /// when the app closed come back paused.
    pub fn new(
        store: PathBuf,
        mut settings: Settings,
        tools: Arc<Tools>,
        torrents: Arc<TorrentEngine>,
        runtime: Handle,
        notify: Notify,
    ) -> anyhow::Result<Arc<Self>> {
        if settings.temp_dir.as_os_str().is_empty() {
            settings.temp_dir = store
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("temp");
        }
        let downloader = Downloader::new(settings.client_config())?;
        let loaded =
            store::load_json_validated::<Vec<Record>>(&store, |records| validate_records(records))?;
        let mut recovery_notice = loaded.recovery_notice("the download list");
        let mut records = loaded.value.unwrap_or_default();
        for record in &mut records {
            if matches!(record.state, State::Queued | State::Downloading) {
                record.state = State::Paused;
            }
        }
        let next_id = records
            .iter()
            .map(|r| r.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("The download list has exhausted its record IDs"))?;
        let queue_store = store.with_file_name("queues.json");
        let loaded = store::load_json_validated(&queue_store, |_| Ok(()))?;
        if let Some(message) = loaded.recovery_notice("queues") {
            match &mut recovery_notice {
                Some(notice) => {
                    notice.push('\n');
                    notice.push_str(&message);
                }
                None => recovery_notice = Some(message),
            }
        }
        let queues = queue::sanitize_all(loaded.value.unwrap_or_default());
        for record in &mut records {
            if !queues.iter().any(|q| q.id == record.queue) {
                record.queue = main_queue();
            }
        }
        let mut migrations = Vec::new();
        for record in &mut records {
            if record.state != State::Completed
                && record.media.is_none()
                && record.torrent.is_none()
                && (sdm_core::partial_data_path(&record.path).exists()
                    || sdm_core::partial_state_path(&record.path).exists()
                    || with_suffix(&sdm_core::partial_state_path(&record.path), ".tmp").exists())
            {
                record
                    .temp_dir
                    .get_or_insert_with(|| settings.temp_dir.clone());
                migrations.push(record.clone());
            }
        }
        // Remember the destination before moving any bytes. A restart or a
        // settings change must never strand an interrupted migration.
        if !migrations.is_empty() {
            store::save_json(&store, &records)?;
        }
        let completed: HashSet<u64> = records
            .iter()
            .filter(|r| r.state == State::Completed && r.torrent.is_none())
            .map(|r| r.id)
            .collect();
        let recovering: Vec<Record> = records
            .iter()
            .filter(|r| r.state != State::Completed && r.media.is_none() && r.torrent.is_none())
            .cloned()
            .collect();
        let manager = Arc::new(Self {
            inner: Mutex::new(Inner {
                records,
                active: HashMap::new(),
                migrating: recovering.iter().map(|r| (r.id, None)).collect(),
                cleanup_pending: completed,
                cleanup_running: HashSet::new(),
                cleanup_remove: HashMap::new(),
                downloader,
                settings,
                next_id,
                dirty: false,
                closing: false,
                update_prepared: false,
                queues,
                after_all: PowerAction::Nothing,
                checked_at: None,
            }),
            store,
            queue_store,
            tools,
            torrents,
            runtime,
            notify,
            recovery_notice,
        });
        // Retry cleanup left behind by a crash after the final file commit.
        // Never sweep untracked files or paused/failed downloads.
        manager.retry_cleanup();
        let this = manager.clone();
        manager.runtime.spawn(async move {
            for record in recovering {
                match sdm_core::reconcile_completion(
                    &record.path,
                    record.temp_dir.as_deref(),
                    &record.id.to_string(),
                )
                .await
                {
                    Ok(Some(receipt)) => {
                        this.recovered_completion(record.id, receipt).await;
                        continue;
                    }
                    Err(error) => {
                        {
                            let mut inner = this.lock();
                            if let Some(record) =
                                inner.records.iter_mut().find(|r| r.id == record.id)
                            {
                                record.state = State::Paused;
                                record.error =
                                    Some(format!("Could not verify completion: {error}"));
                                inner.dirty = true;
                            }
                        }
                        this.migration_finished(record.id, Ok(()));
                        continue;
                    }
                    Ok(None) => {}
                }
                let result = if migrations.iter().any(|r| r.id == record.id) {
                    sdm_core::migrate_partial(&record.path, record.temp_dir.as_deref().unwrap())
                        .await
                } else {
                    Ok(())
                };
                this.migration_finished(record.id, result);
            }
        });
        Ok(manager)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn recovery_notice(&self) -> Option<&str> {
        self.recovery_notice.as_deref()
    }

    pub fn downloader(&self) -> Downloader {
        self.lock().downloader.clone()
    }

    /// Applies new settings to running and future downloads.
    #[cfg(test)]
    pub fn update_settings(self: &Arc<Self>, mut settings: Settings) -> anyhow::Result<()> {
        {
            let mut inner = self.lock();
            if settings.temp_dir.as_os_str().is_empty() {
                settings.temp_dir.clone_from(&inner.settings.temp_dir);
            }
            if settings.proxy != inner.settings.proxy {
                inner.downloader = Downloader::new(settings.client_config())?;
            }
            inner.downloader.set_speed_limit(settings.speed_limit());
            self.torrents.set_config(settings.torrent_config());
            inner.settings = settings;
        }
        self.schedule();
        Ok(())
    }

    /// Validate and persist before changing any running configuration. Keeping
    /// the lock across the transaction also serializes concurrent UI saves.
    pub fn save_settings(
        self: &Arc<Self>,
        path: &Path,
        mut settings: Settings,
    ) -> anyhow::Result<()> {
        {
            let mut inner = self.lock();
            if settings.temp_dir.as_os_str().is_empty() {
                settings.temp_dir.clone_from(&inner.settings.temp_dir);
            }
            let downloader = Downloader::new(settings.client_config())?;
            settings.save(path)?;
            if settings.proxy != inner.settings.proxy {
                inner.downloader = downloader;
            }
            inner.downloader.set_speed_limit(settings.speed_limit());
            self.torrents.set_config(settings.torrent_config());
            inner.settings = settings;
        }
        self.schedule();
        Ok(())
    }

    /// All downloads, oldest first, with live progress.
    pub fn list(&self) -> Vec<DownloadView> {
        let mut inner = self.lock();
        let progress = inner.sync_progress();
        inner
            .records
            .iter()
            .map(|record| {
                let active = inner.active.get(&record.id);
                let torrent = match (active, &record.torrent) {
                    (Some(active), _) => active.control.torrent(),
                    (None, Some(spec)) if record.state == State::Completed => {
                        self.torrents.seeding(&spec.info_hash)
                    }
                    _ => None,
                };
                let live = Live {
                    progress: progress.get(&record.id),
                    stopping: active.is_some_and(|a| a.stopping)
                        || inner.migrating.get(&record.id).is_some_and(Option::is_some)
                        || inner.cleanup_remove.contains_key(&record.id),
                    running_for: active.map_or(Duration::ZERO, |a| a.since.elapsed()),
                    torrent,
                };
                view(record, &live)
            })
            .collect()
    }

    /// One download with live progress.
    pub fn get(&self, id: u64) -> Option<DownloadView> {
        self.list().into_iter().find(|d| d.id == id)
    }

    /// Where a download is (or will be) saved.
    pub fn path_of(&self, id: u64) -> Option<(PathBuf, State)> {
        self.lock()
            .records
            .iter()
            .find(|r| r.id == id)
            .map(|r| (r.path.clone(), r.state))
    }

    /// Adds a download and returns its id.
    pub async fn add(self: &Arc<Self>, request: AddRequest) -> Result<u64, String> {
        let url = request.url.trim().to_owned();
        let parsed = url::Url::parse(&url).map_err(|_| format!("invalid URL: {url}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("only http and https links are supported: {url}"));
        }

        let (downloader, settings) = {
            let inner = self.lock();
            (inner.downloader.clone(), inner.settings.clone())
        };
        let file_name = match request.file_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => sdm_core::sanitize_file_name(name),
            _ => match downloader.probe(&url, &request.headers).await {
                Ok(info) => info.file_name,
                Err(error) => {
                    tracing::warn!(%error, %url, "could not ask the server for a file name");
                    name_from_url(&parsed)
                }
            },
        };
        let category = Category::from_file_name(&file_name);
        let directory = request
            .directory
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| settings.directory_for(category));

        let id = {
            let mut inner = self.lock();
            let rollback = (inner.records.len(), inner.next_id, inner.dirty);
            let queue = inner.queue_id(request.queue.as_deref());
            let path = inner.free_path(&directory, &file_name);
            let id = inner.allocate_id()?;
            let temp_dir = Some(inner.settings.temp_dir.clone());
            inner.records.push(Record {
                id,
                url,
                headers: request.headers,
                file_name: file_name_of(&path),
                temp_dir,
                path,
                size: None,
                downloaded: 0,
                resumable: false,
                category,
                state: if request.start {
                    State::Queued
                } else {
                    State::Paused
                },
                error: None,
                added_at: now_ms(),
                completed_at: None,
                elapsed_ms: 0,
                media: None,
                torrent: None,
                queue,
            });
            inner.dirty = true;
            self.save_added(&mut inner, rollback)?;
            id
        };
        self.schedule();
        Ok(id)
    }

    /// Adds many links without asking each server first, and returns their
    /// ids. Links that are not http(s) are skipped.
    pub fn add_batch(self: &Arc<Self>, request: BatchRequest) -> Result<Vec<u64>, String> {
        let mut ids = Vec::new();
        {
            let mut inner = self.lock();
            let rollback = (inner.records.len(), inner.next_id, inner.dirty);
            let queue = inner.queue_id(request.queue.as_deref());
            for link in request.links {
                let url = link.url.trim().to_owned();
                let Ok(parsed) = url::Url::parse(&url) else {
                    continue;
                };
                if !matches!(parsed.scheme(), "http" | "https") {
                    continue;
                }
                let file_name = match link.file_name.as_deref().map(sdm_core::sanitize_file_name) {
                    Some(name) if !name.is_empty() => name,
                    _ => match name_from_url(&parsed) {
                        name if name.is_empty() => "download".to_owned(),
                        name => name,
                    },
                };
                let category = Category::from_file_name(&file_name);
                let directory = request
                    .directory
                    .clone()
                    .filter(|d| !d.as_os_str().is_empty())
                    .unwrap_or_else(|| inner.settings.directory_for(category));
                let path = inner.free_path(&directory, &file_name);
                let id = match inner.allocate_id() {
                    Ok(id) => id,
                    Err(error) => {
                        inner.records.truncate(rollback.0);
                        inner.next_id = rollback.1;
                        inner.dirty = rollback.2;
                        return Err(error);
                    }
                };
                let mut headers = request.headers.clone();
                for (name, value) in link.headers {
                    headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
                    headers.push((name, value));
                }
                let temp_dir = Some(inner.settings.temp_dir.clone());
                inner.records.push(Record {
                    id,
                    url,
                    headers,
                    file_name: file_name_of(&path),
                    temp_dir,
                    path,
                    size: None,
                    downloaded: 0,
                    resumable: false,
                    category,
                    state: if request.start {
                        State::Queued
                    } else {
                        State::Paused
                    },
                    error: None,
                    added_at: now_ms(),
                    completed_at: None,
                    elapsed_ms: 0,
                    media: None,
                    torrent: None,
                    queue: queue.clone(),
                });
                ids.push(id);
            }
            if ids.is_empty() {
                return Err("none of these are http or https links".into());
            }
            inner.dirty = true;
            self.save_added(&mut inner, rollback)?;
        }
        self.schedule();
        Ok(ids)
    }

    /// Gives an unfinished download a new address for the same file, such
    /// as a fresh link after the old one expired. What was downloaded so
    /// far is kept.
    pub async fn change_address(
        self: &Arc<Self>,
        id: u64,
        url: String,
        headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        let url = url.trim().to_owned();
        let parsed = url::Url::parse(&url).map_err(|_| format!("invalid URL: {url}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("only http and https links are supported: {url}"));
        }
        let (path, temp_dir, size, old_headers, downloader) = {
            let inner = self.lock();
            if inner.migrating.contains_key(&id) {
                return Err("temporary files are still being moved; try again shortly".into());
            }
            let record = inner
                .records
                .iter()
                .find(|r| r.id == id)
                .ok_or("this download is no longer in the list")?;
            if record.media.is_some() || record.torrent.is_some() {
                return Err("only normal downloads can get a new address".into());
            }
            if record.state == State::Completed {
                return Err("this download is already complete".into());
            }
            (
                record.path.clone(),
                record.temp_dir.clone(),
                record.size,
                record.headers.clone(),
                inner.downloader.clone(),
            )
        };
        // Stop it first; it starts again with the new address.
        let was_running = self.lock().active.contains_key(&id);
        if was_running {
            self.pause(&[id]);
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.lock().active.contains_key(&id) {
                if Instant::now() > deadline {
                    return Err("the download did not stop; try again".into());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        let headers = headers.unwrap_or(old_headers);
        let info = downloader
            .probe(&url, &headers)
            .await
            .map_err(|error| format!("could not open the new link: {error}"))?;
        if let (Some(size), Some(new)) = (size, info.size)
            && size != new
        {
            return Err(format!(
                "the new link is for a different file ({new} bytes instead of {size})"
            ));
        }
        sdm_core::retarget_partial_in(&path, temp_dir.as_deref(), &info)
            .await
            .map_err(|error| error.to_string())?;
        {
            let mut inner = self.lock();
            let Some(record) = inner.records.iter_mut().find(|r| r.id == id) else {
                return Err("this download is no longer in the list".into());
            };
            record.url = url;
            record.headers = headers;
            record.error = None;
            if record.state == State::Failed {
                record.state = State::Paused;
            }
            inner.dirty = true;
            self.save(&mut inner);
        }
        if was_running {
            self.resume(&[id]);
        }
        Ok(())
    }

    /// Adds a video from a web page and returns its id.
    pub fn add_media(self: &Arc<Self>, request: AddMediaRequest) -> Result<u64, String> {
        let page = url::Url::parse(request.source.page_url.trim())
            .map_err(|_| format!("invalid URL: {}", request.source.page_url))?;
        if !matches!(page.scheme(), "http" | "https") {
            return Err(format!("only http and https pages are supported: {page}"));
        }
        let settings = self.settings();
        let title = sdm_core::sanitize_file_name(request.title.trim());
        let title = if title.is_empty() {
            "video".to_owned()
        } else {
            title
        };
        let file_name = format!("{title}.{}", request.option.ext);
        let category = Category::from_file_name(&file_name);
        let directory = request
            .directory
            .clone()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| settings.directory_for(category));

        let id = {
            let mut inner = self.lock();
            let rollback = (inner.records.len(), inner.next_id, inner.dirty);
            let path = inner.free_path(&directory, &file_name);
            let id = inner.allocate_id()?;
            let temp_dir = Some(inner.settings.temp_dir.clone());
            inner.records.push(Record {
                id,
                url: page.to_string(),
                headers: Vec::new(),
                file_name: file_name_of(&path),
                temp_dir,
                path,
                size: request.option.size,
                downloaded: 0,
                resumable: true,
                category,
                state: if request.start {
                    State::Queued
                } else {
                    State::Paused
                },
                error: None,
                added_at: now_ms(),
                completed_at: None,
                elapsed_ms: 0,
                media: Some(request.spec()),
                torrent: None,
                queue: main_queue(),
            });
            inner.dirty = true;
            self.save_added(&mut inner, rollback)?;
            id
        };
        self.schedule();
        Ok(id)
    }

    /// Reads a magnet link or `.torrent` file for the "add torrent" dialog.
    pub async fn inspect_torrent(&self, source: TorrentSource) -> Result<TorrentInfo, String> {
        let downloader = self.downloader();
        let mut info = self.torrents.inspect(&source, &downloader).await?;
        let inner = self.lock();
        info.existing = inner.torrent_download(&info.info_hash);
        info.directory = inner
            .settings
            .directory_for(Category::from_file_name(info.main_file()));
        Ok(info)
    }

    /// The files a torrent download gets, in torrent order.
    pub fn torrent_files(&self, id: u64) -> Option<Vec<torrent::TorrentFile>> {
        let spec = self
            .lock()
            .records
            .iter()
            .find(|r| r.id == id)?
            .torrent
            .clone()?;
        let (info, _) = self.torrents.known(&spec.info_hash)?;
        Some(
            info.files
                .into_iter()
                .filter(|f| spec.files.as_ref().is_none_or(|s| s.contains(&f.index)))
                .collect(),
        )
    }

    /// Stops reading magnet links.
    pub fn cancel_torrent_inspections(&self) {
        self.torrents.cancel_inspections();
    }

    /// Adds (some files of) a torrent read with [`Self::inspect_torrent`].
    pub fn add_torrent(self: &Arc<Self>, request: AddTorrentRequest) -> Result<u64, String> {
        let (info, link) = self
            .torrents
            .known(&request.info_hash)
            .ok_or("this torrent is no longer known; please add it again")?;
        let files = info.selection(request.files.as_deref())?;
        let size = info.selected_size(files.as_deref());
        let category = Category::from_file_name(info.main_file());
        let name = match sdm_core::sanitize_file_name(&info.name) {
            name if name.is_empty() => info.info_hash.clone(),
            name => name,
        };

        let id = {
            let mut inner = self.lock();
            let rollback = (inner.records.len(), inner.next_id, inner.dirty);
            if inner.torrent_download(&info.info_hash).is_some() {
                return Err("this torrent is already in the download list".into());
            }
            let directory = request
                .directory
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or_else(|| inner.settings.directory_for(category));
            // librqbit writes the files under their names in the torrent.
            let (folder, path) = if info.single_file() {
                let file = directory.join(&info.files[0].path);
                if file.exists() || inner.records.iter().any(|r| r.path == file) {
                    // Keep the existing file; this one gets its own folder.
                    let stem = Path::new(&name)
                        .file_stem()
                        .map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
                    let folder = inner.free_path(&directory, &stem);
                    let file = folder.join(&info.files[0].path);
                    (folder, file)
                } else {
                    (directory, file)
                }
            } else {
                let folder = inner.free_path(&directory, &name);
                (folder.clone(), folder)
            };
            let id = inner.allocate_id()?;
            inner.records.push(Record {
                id,
                url: link,
                headers: Vec::new(),
                file_name: file_name_of(&path),
                temp_dir: None,
                path,
                size: Some(size),
                downloaded: 0,
                resumable: true,
                category,
                state: if request.start {
                    State::Queued
                } else {
                    State::Paused
                },
                error: None,
                added_at: now_ms(),
                completed_at: None,
                elapsed_ms: 0,
                media: None,
                torrent: Some(TorrentSpec {
                    info_hash: info.info_hash.clone(),
                    files,
                    folder,
                }),
                queue: main_queue(),
            });
            inner.dirty = true;
            self.save_added(&mut inner, rollback)?;
            id
        };
        self.schedule();
        Ok(id)
    }

    pub fn pause(&self, ids: &[u64]) {
        let mut inner = self.lock();
        for &id in ids {
            inner.pause_one(id);
        }
        self.stop_seeding(&inner, ids);
        self.save(&mut inner);
    }

    pub fn pause_all(&self) {
        let mut inner = self.lock();
        let ids: Vec<u64> = inner.records.iter().map(|r| r.id).collect();
        for &id in &ids {
            inner.pause_one(id);
        }
        self.stop_seeding(&inner, &ids);
        self.save(&mut inner);
    }

    /// Pausing a finished torrent stops sharing it.
    fn stop_seeding(&self, inner: &Inner, ids: &[u64]) {
        for record in inner.records.iter().filter(|r| ids.contains(&r.id)) {
            if let (State::Completed, Some(spec)) = (record.state, &record.torrent) {
                let torrents = self.torrents.clone();
                let hash = spec.info_hash.clone();
                self.runtime
                    .spawn(async move { torrents.stop_seeding(&hash).await });
            }
        }
    }

    pub fn resume(self: &Arc<Self>, ids: &[u64]) {
        {
            let mut inner = self.lock();
            for record in inner.records.iter_mut().filter(|r| ids.contains(&r.id)) {
                if matches!(record.state, State::Paused | State::Failed) {
                    record.state = State::Queued;
                    record.error = None;
                }
            }
            inner.dirty = true;
            self.save(&mut inner);
        }
        self.schedule();
    }

    pub fn resume_all(self: &Arc<Self>) {
        let ids: Vec<u64> = self.lock().records.iter().map(|r| r.id).collect();
        self.resume(&ids);
    }

    /// The queues, the main queue first.
    pub fn queues(&self) -> Vec<Queue> {
        self.lock().queues.clone()
    }

    /// Adds a queue (when its id is empty) or changes one. Returns it.
    pub fn save_queue(self: &Arc<Self>, queue: Queue) -> Result<Queue, String> {
        let saved = {
            let mut inner = self.lock();
            let mut queue = queue;
            if queue.id.is_empty() {
                let mut n = inner.queues.len();
                queue.id = loop {
                    let id = format!("q{n}");
                    if !inner.queues.iter().any(|q| q.id == id) {
                        break id;
                    }
                    n += 1;
                };
                inner.queues.push(queue.clone().sanitized());
            } else {
                let slot = inner
                    .queues
                    .iter_mut()
                    .find(|q| q.id == queue.id)
                    .ok_or("this queue no longer exists")?;
                *slot = queue.clone().sanitized();
            }
            let saved = inner
                .queues
                .iter()
                .find(|q| q.id == queue.id)
                .cloned()
                .unwrap_or(queue);
            self.save_queues(&inner);
            saved
        };
        self.schedule();
        Ok(saved)
    }

    /// Deletes a queue; its downloads move to the main queue.
    pub fn delete_queue(self: &Arc<Self>, id: &str) {
        if id == MAIN_QUEUE {
            return;
        }
        {
            let mut inner = self.lock();
            inner.queues.retain(|q| q.id != id);
            for record in inner.records.iter_mut().filter(|r| r.queue == id) {
                record.queue = main_queue();
            }
            inner.dirty = true;
            self.save(&mut inner);
            self.save_queues(&inner);
        }
        self.schedule();
    }

    /// Puts downloads into another queue.
    pub fn move_to_queue(self: &Arc<Self>, ids: &[u64], queue: &str) {
        {
            let mut inner = self.lock();
            let queue = inner.queue_id(Some(queue));
            for record in inner.records.iter_mut().filter(|r| ids.contains(&r.id)) {
                record.queue.clone_from(&queue);
            }
            inner.dirty = true;
            self.save(&mut inner);
        }
        self.schedule();
    }

    /// Resumes every unfinished download of a queue.
    pub fn start_queue(self: &Arc<Self>, id: &str) {
        let ids = self.queue_downloads(id);
        self.resume(&ids);
    }

    /// Pauses every download of a queue.
    pub fn stop_queue(&self, id: &str) {
        let ids = self.queue_downloads(id);
        let mut inner = self.lock();
        for &id in &ids {
            inner.pause_one(id);
        }
        self.save(&mut inner);
    }

    fn queue_downloads(&self, queue: &str) -> Vec<u64> {
        self.lock()
            .records
            .iter()
            .filter(|r| r.queue == queue && r.state != State::Completed)
            .map(|r| r.id)
            .collect()
    }

    pub fn after_all(&self) -> PowerAction {
        self.lock().after_all
    }

    /// What to do once every download is done.
    pub fn set_after_all(&self, action: PowerAction) {
        self.lock().after_all = action;
    }

    /// Starts and stops queues whose time has come. Call this regularly
    /// with the local time.
    pub fn check_schedules(self: &Arc<Self>, now: chrono::NaiveDateTime) {
        let due: Vec<(String, ScheduleEvent)> = {
            let mut inner = self.lock();
            let Some(from) = inner.checked_at.replace(now) else {
                return;
            };
            let mut due = Vec::new();
            let mut changed = false;
            for queue in &mut inner.queues {
                for event in queue.schedule.due(from, now) {
                    due.push((queue.id.clone(), event));
                    if queue.schedule.finished_by(event) {
                        queue.schedule.enabled = false;
                        changed = true;
                    }
                }
            }
            if changed {
                self.save_queues(&inner);
            }
            due
        };
        for (queue, event) in due {
            tracing::info!(%queue, ?event, "scheduled queue event");
            match event {
                ScheduleEvent::Start => self.start_queue(&queue),
                ScheduleEvent::Stop => self.stop_queue(&queue),
            }
        }
    }

    fn save_queues(&self, inner: &Inner) {
        if let Err(error) = store::save_json(&self.queue_store, &inner.queues) {
            tracing::warn!(%error, "could not save the queues");
        }
    }

    /// Removes downloads from the list. Unfinished data is always deleted;
    /// finished files are moved to the trash only when `delete_file` is set.
    pub fn remove(&self, ids: &[u64], delete_file: bool) {
        let mut inner = self.lock();
        for &id in ids {
            if inner.cleanup_running.contains(&id) {
                inner.cleanup_remove.insert(id, delete_file);
                continue;
            }
            if let Some(remove) = inner.migrating.get_mut(&id) {
                *remove = Some(delete_file);
                continue;
            }
            if let Some(active) = inner.active.get_mut(&id) {
                active.remove = Some(delete_file);
                active.stopping = true;
                // Preserve partials until the row removal is durably saved.
                // Cancelling a core job would delete them before that write.
                active.control.pause();
                continue;
            }
            if let Some(index) = inner.records.iter().position(|r| r.id == id) {
                self.remove_record(&mut inner, index, delete_file);
            }
        }
        self.save(&mut inner);
    }

    /// Copies live progress into the stored records and saves them now and
    /// then. Call this regularly.
    pub fn tick(self: &Arc<Self>, save: bool) {
        {
            let mut inner = self.lock();
            inner.sync_progress();
            if save {
                self.save(&mut inner);
            }
        }
        if save {
            self.retry_cleanup();
            self.schedule();
        }
    }

    /// Pauses everything and waits (up to a few seconds) until all progress
    /// is safely on disk. Used when the app quits.
    pub async fn shutdown(&self) {
        {
            let mut inner = self.lock();
            inner.closing = true;
            inner.update_prepared = false;
            for active in inner.active.values_mut() {
                active.stopping = true;
                active.control.pause();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while {
            let inner = self.lock();
            !inner.active.is_empty()
                || !inner.migrating.is_empty()
                || !inner.cleanup_running.is_empty()
        } && Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        {
            let mut inner = self.lock();
            inner.sync_progress();
            inner.dirty = true;
            self.save(&mut inner);
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), self.torrents.shutdown()).await;
    }

    /// Reserve an idle manager for installing an update. Unlike normal quit,
    /// this never pauses transfers or sharing, and a failed durable save must
    /// keep the current app running. The scheduling gate and readiness check
    /// share one lock so an incoming link cannot start between them.
    pub fn prepare_update(&self) -> Result<(), UpdatePreparationError> {
        let mut inner = self.lock();
        if inner.closing
            || !inner.active.is_empty()
            || !inner.migrating.is_empty()
            || !inner.cleanup_pending.is_empty()
            || !inner.cleanup_running.is_empty()
            || !inner.cleanup_remove.is_empty()
            || inner.busy(None)
            || inner.records.iter().any(|record| {
                record
                    .torrent
                    .as_ref()
                    .is_some_and(|spec| self.torrents.seeding(&spec.info_hash).is_some())
            })
        {
            return Err(UpdatePreparationError::Busy);
        }
        inner.closing = true;
        inner.sync_progress();
        let save = store::save_json(&self.store, &inner.records)
            .and_then(|()| store::save_json(&self.queue_store, &inner.queues));
        if let Err(error) = save {
            inner.closing = false;
            inner.dirty = true;
            return Err(UpdatePreparationError::Persistence(error));
        }
        inner.dirty = false;
        inner.update_prepared = true;
        Ok(())
    }

    /// An installer could not start: release its reservation so downloads can
    /// be added and scheduled normally. A simultaneous ordinary quit keeps
    /// its own closing gate intact.
    pub fn cancel_update(self: &Arc<Self>) {
        {
            let mut inner = self.lock();
            if !inner.update_prepared {
                return;
            }
            inner.update_prepared = false;
            inner.closing = false;
        }
        self.schedule();
    }

    /// Starts queued downloads while there are free slots.
    fn schedule(self: &Arc<Self>) {
        let started = {
            let mut inner = self.lock();
            if inner.closing {
                return;
            }
            // Free places in every queue.
            let mut free: HashMap<&str, usize> = inner
                .queues
                .iter()
                .map(|q| {
                    let limit = if q.is_main() {
                        inner.settings.max_parallel
                    } else {
                        q.max_parallel
                    };
                    let running = inner
                        .records
                        .iter()
                        .filter(|r| r.queue == q.id && inner.active.contains_key(&r.id))
                        .count();
                    (q.id.as_str(), usize::from(limit).saturating_sub(running))
                })
                .collect();
            let mut next = Vec::new();
            for record in &inner.records {
                if record.state != State::Queued
                    || inner.active.contains_key(&record.id)
                    || inner.migrating.contains_key(&record.id)
                {
                    continue;
                }
                if let Some(places) = free.get_mut(record.queue.as_str())
                    && *places > 0
                {
                    *places -= 1;
                    next.push(record.id);
                }
            }
            if !next.is_empty() {
                let previous = inner.records.clone();
                let was_dirty = inner.dirty;
                let temp_dir = inner.settings.temp_dir.clone();
                for record in &mut inner.records {
                    if next.contains(&record.id) {
                        record.state = State::Downloading;
                        record.error = None;
                        if record.temp_dir.is_none()
                            && record.media.is_none()
                            && record.torrent.is_none()
                        {
                            record.temp_dir = Some(temp_dir.clone());
                        }
                    }
                }
                if let Err(error) = store::save_json(&self.store, &inner.records) {
                    inner.records = previous;
                    inner.dirty = was_dirty;
                    tracing::warn!(%error, "could not save scheduled downloads; no new transfers started");
                    return;
                }
                inner.dirty = false;
            }
            for &id in &next {
                self.start(&mut inner, id);
            }
            self.save(&mut inner);
            next
        };
        // Outside the lock: the receiver may ask the manager for details.
        for id in started {
            (self.notify)(Notice::Started { id });
        }
    }

    fn start(self: &Arc<Self>, inner: &mut Inner, id: u64) {
        let Some(record) = inner.records.iter_mut().find(|r| r.id == id) else {
            return;
        };
        record.state = State::Downloading;
        record.error = None;
        if record.temp_dir.is_none() && record.media.is_none() && record.torrent.is_none() {
            record.temp_dir = Some(inner.settings.temp_dir.clone());
        }
        let (url, path, headers, media, torrent, temp_dir) = (
            record.url.clone(),
            record.path.clone(),
            record.headers.clone(),
            record.media.clone(),
            record.torrent.clone(),
            record.temp_dir.clone(),
        );
        inner.dirty = true;
        let this = Arc::clone(self);
        let _runtime = self.runtime.enter();

        let control = if let Some(spec) = torrent {
            let Some((info, _)) = self.torrents.known(&spec.info_hash) else {
                if let Some(record) = inner.records.iter_mut().find(|r| r.id == id) {
                    record.state = State::Failed;
                    record.error = Some("the .torrent data of this download is missing".into());
                }
                return;
            };
            let sizes = info
                .files
                .iter()
                .filter(|f| spec.files.as_ref().is_none_or(|s| s.contains(&f.index)))
                .map(|f| (f.index, f.size))
                .collect();
            let job = self.torrents.start(torrent::JobConfig {
                info_hash: spec.info_hash,
                files: spec.files,
                sizes,
                folder: spec.folder,
                path,
            });
            let control = job.control();
            self.runtime.spawn(async move {
                let result = job.wait().await;
                this.finished(id, result).await;
            });
            Job::Torrent(control)
        } else if let Some(spec) = media {
            let Some(ytdlp) = self.tools.ytdlp() else {
                record.state = State::Failed;
                record.error = Some("yt-dlp is not installed".into());
                return;
            };
            let config = media::JobConfig {
                ytdlp,
                ffmpeg: self.tools.ffmpeg(),
                deno: self.tools.deno(),
                url,
                spec,
                directory: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                stem: path
                    .file_stem()
                    .map_or_else(|| "video".into(), |s| s.to_string_lossy().into_owned()),
                work_dir: temp_dir.as_deref().map_or_else(
                    || self.tools.work_dir(id),
                    |root| {
                        sdm_core::PartialFiles::new(&path, Some(root))
                            .directory()
                            .unwrap()
                            .to_owned()
                    },
                ),
                speed_limit: inner.downloader.speed_limit(),
                proxy: inner.settings.proxy.clone(),
            };
            let job = media::job::start(config);
            let control = job.control();
            self.runtime.spawn(async move {
                let result = job.wait().await;
                this.finished(id, result).await;
            });
            Job::Media(control)
        } else {
            let request = DownloadRequest {
                url,
                destination: Destination::File(path),
                headers,
                options: DownloadOptions {
                    connections: inner.settings.connections.into(),
                    temp_dir,
                    completion_id: Some(id.to_string()),
                    ..DownloadOptions::default()
                },
            };
            let handle = inner.downloader.start(request);
            let control = handle.control();
            self.runtime.spawn(async move {
                let result = handle.wait().await.map_err(|e| e.to_string());
                this.finished(id, result).await;
            });
            Job::File(control)
        };
        inner.active.insert(
            id,
            Active {
                control,
                since: Instant::now(),
                stopping: false,
                remove: None,
            },
        );
    }

    async fn finished(self: &Arc<Self>, id: u64, result: Result<Outcome, String>) {
        // Only downloads that end by themselves, not paused ones, can make
        // a queue done.
        let ended = matches!(result, Ok(Outcome::Completed { .. }) | Err(_));
        let (notice, done) = {
            let mut inner = self.lock();
            let Some(active) = inner.active.remove(&id) else {
                return;
            };
            let progress = active.control.progress();
            let index = inner.records.iter().position(|r| r.id == id);
            let notice = match (index, active.remove) {
                (Some(index), Some(delete_file)) => {
                    let record = &mut inner.records[index];
                    record.elapsed_ms += millis(active.since.elapsed());
                    record.apply(&progress);
                    record.finish(result);
                    self.remove_record(&mut inner, index, delete_file);
                    None
                }
                (Some(index), None) => {
                    let record = &mut inner.records[index];
                    record.elapsed_ms += millis(active.since.elapsed());
                    record.apply(&progress);
                    record.finish(result)
                }
                (None, _) => None,
            };
            if inner
                .records
                .iter()
                .any(|r| r.id == id && r.state == State::Completed && r.torrent.is_none())
            {
                inner.cleanup_pending.insert(id);
            }
            inner.dirty = true;
            self.save(&mut inner);
            let done = if ended && notice.is_some() {
                inner.done_after(id)
            } else {
                None
            };
            (notice, done)
        };
        if let Some(notice) = notice {
            (self.notify)(notice);
        }
        self.retry_cleanup();
        self.schedule();
        if let Some(done) = done {
            (self.notify)(done);
        }
    }

    fn migration_finished(self: &Arc<Self>, id: u64, result: sdm_core::Result<()>) {
        {
            let mut inner = self.lock();
            let remove = inner.migrating.remove(&id).flatten();
            if let Some(index) = inner.records.iter().position(|r| r.id == id) {
                if let Some(delete_file) = remove {
                    self.remove_record(&mut inner, index, delete_file);
                } else if let Err(error) = result {
                    tracing::warn!(%error, id, "could not move legacy download temporary files");
                    inner.records[index].state = State::Failed;
                    inner.records[index].error = Some(error.to_string());
                }
                inner.dirty = true;
                self.save(&mut inner);
            }
        }
        self.schedule();
    }

    async fn recovered_completion(self: &Arc<Self>, id: u64, receipt: sdm_core::CompletionReceipt) {
        {
            let mut inner = self.lock();
            let remove = inner.migrating.remove(&id).flatten();
            if let Some(index) = inner.records.iter().position(|r| r.id == id) {
                if let Some(delete_file) = remove {
                    let record = &mut inner.records[index];
                    record.finish(Ok(Outcome::Completed {
                        path: receipt.path,
                        size: receipt.size,
                    }));
                    self.remove_record(&mut inner, index, delete_file);
                } else {
                    inner.records[index].finish(Ok(Outcome::Completed {
                        path: receipt.path,
                        size: receipt.size,
                    }));
                    inner.records[index].error = None;
                }
                if inner
                    .records
                    .iter()
                    .any(|r| r.id == id && r.state == State::Completed)
                {
                    inner.cleanup_pending.insert(id);
                }
                inner.dirty = true;
                self.save(&mut inner);
            }
        }
        self.retry_cleanup();
        self.schedule();
    }

    /// Receipt acknowledgement and cleanup happen only after the corresponding
    /// Completed record has been saved. Failed saves and cleanup retry on ticks.
    fn retry_cleanup(self: &Arc<Self>) {
        let records = {
            let mut inner = self.lock();
            if inner.dirty {
                return;
            }
            let records: Vec<Record> = inner
                .records
                .iter()
                .filter(|r| {
                    r.state == State::Completed
                        && inner.cleanup_pending.contains(&r.id)
                        && !inner.cleanup_running.contains(&r.id)
                })
                .cloned()
                .collect();
            for record in &records {
                inner.cleanup_running.insert(record.id);
            }
            records
        };
        for record in records {
            let this = self.clone();
            self.runtime.spawn(async move {
                let cleaned = cleanup_completed(&record, &this.tools).await;
                let mut inner = this.lock();
                inner.cleanup_running.remove(&record.id);
                if cleaned {
                    inner.cleanup_pending.remove(&record.id);
                }
                if let Some(delete_file) = inner.cleanup_remove.remove(&record.id)
                    && let Some(index) = inner.records.iter().position(|r| r.id == record.id)
                {
                    this.remove_record(&mut inner, index, delete_file);
                }
            });
        }
    }

    /// Deletes what is left of a download that leaves the list.
    fn remove_record(&self, inner: &mut Inner, index: usize, delete_file: bool) -> bool {
        let record = inner.records.remove(index);
        match store::save_json(&self.store, &inner.records) {
            Ok(()) => {
                inner.dirty = false;
                inner.cleanup_pending.remove(&record.id);
                self.discard(&record, delete_file);
                true
            }
            Err(error) => {
                tracing::warn!(%error, id = record.id, "could not save removal; retaining the download and its files");
                let mut record = record;
                record.error = Some(format!(
                    "Could not remove the download; its files were kept: {error}"
                ));
                inner.records.insert(index, record);
                inner.dirty = true;
                false
            }
        }
    }

    fn discard(&self, record: &Record, delete_file: bool) {
        let Some(spec) = &record.torrent else {
            discard_files(record, delete_file, &self.tools);
            return;
        };
        let torrents = self.torrents.clone();
        let hash = spec.info_hash.clone();
        let finished = record.state == State::Completed;
        let path = record.path.clone();
        self.runtime.spawn(async move {
            // Unfinished data goes for good, finished files to the trash.
            torrents.forget(&hash, !finished).await;
            if finished
                && delete_file
                && path.exists()
                && let Err(error) = trash::delete(&path)
            {
                tracing::warn!(%error, path = %path.display(), "could not move the files to the trash");
            }
        });
    }

    fn save_added(&self, inner: &mut Inner, rollback: (usize, u64, bool)) -> Result<(), String> {
        match store::save_json(&self.store, &inner.records) {
            Ok(()) => {
                inner.dirty = false;
                Ok(())
            }
            Err(error) => {
                inner.records.truncate(rollback.0);
                inner.next_id = rollback.1;
                inner.dirty = rollback.2;
                Err(format!(
                    "Could not save the new downloads; nothing was started: {error}"
                ))
            }
        }
    }

    fn save(&self, inner: &mut Inner) {
        if !inner.dirty {
            return;
        }
        match store::save_json(&self.store, &inner.records) {
            Ok(()) => inner.dirty = false,
            Err(error) => tracing::warn!(%error, "could not save the download list"),
        }
    }
}

impl Inner {
    fn allocate_id(&mut self) -> Result<u64, String> {
        if self.update_prepared {
            return Err("An update is being installed; please try again after SDM restarts".into());
        }
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("The download list has exhausted its record IDs; no downloads were added")?;
        let id = self.next_id;
        self.next_id = next;
        Ok(id)
    }

    /// An existing queue id, or the main queue.
    fn queue_id(&self, wanted: Option<&str>) -> String {
        wanted
            .filter(|id| self.queues.iter().any(|q| q.id == *id))
            .map_or_else(main_queue, str::to_owned)
    }

    /// Whether a queue (or, with `None`, any queue) has downloads that are
    /// running or waiting to run.
    fn busy(&self, queue: Option<&str>) -> bool {
        self.records.iter().any(|r| {
            matches!(r.state, State::Queued | State::Downloading)
                && queue.is_none_or(|q| r.queue == q)
        })
    }

    /// After download `id` ended: what to do because its queue, or the
    /// whole list, has nothing left.
    fn done_after(&mut self, id: u64) -> Option<Notice> {
        let queue = self.records.iter().find(|r| r.id == id)?.queue.clone();
        if self.after_all != PowerAction::Nothing && !self.busy(None) {
            let action = std::mem::take(&mut self.after_all);
            return Some(Notice::Done {
                queue: None,
                action,
            });
        }
        let queue = self.queues.iter().find(|q| q.id == queue)?;
        (queue.when_done != PowerAction::Nothing && !self.busy(Some(&queue.id))).then(|| {
            Notice::Done {
                queue: Some(queue.clone()),
                action: queue.when_done,
            }
        })
    }

    /// The download of a torrent, if one has it.
    fn torrent_download(&self, info_hash: &str) -> Option<u64> {
        self.records
            .iter()
            .find(|r| r.torrent.as_ref().is_some_and(|t| t.info_hash == info_hash))
            .map(|r| r.id)
    }

    /// Copies live progress into the records and returns it.
    fn sync_progress(&mut self) -> HashMap<u64, Progress> {
        let progress: HashMap<u64, Progress> = self
            .active
            .iter()
            .map(|(id, active)| (*id, active.control.progress()))
            .collect();
        for record in &mut self.records {
            if let Some(p) = progress.get(&record.id)
                && record.apply(p)
            {
                self.dirty = true;
            }
        }
        progress
    }

    fn pause_one(&mut self, id: u64) {
        if let Some(active) = self.active.get_mut(&id) {
            active.stopping = true;
            active.control.pause();
        } else if let Some(record) = self.records.iter_mut().find(|r| r.id == id)
            && record.state == State::Queued
        {
            record.state = State::Paused;
            self.dirty = true;
        }
    }

    /// A path in `directory` that no file, partial download or other list
    /// entry uses yet.
    fn free_path(&self, directory: &Path, file_name: &str) -> PathBuf {
        sdm_core::path_candidates(directory, file_name)
            .find(|path| {
                !path.exists()
                    && !sdm_core::partial_data_path(path).exists()
                    && !sdm_core::PartialFiles::new(path, Some(&self.settings.temp_dir))
                        .data
                        .exists()
                    && !self.records.iter().any(|r| r.path == *path)
            })
            .unwrap_or_else(|| directory.join(file_name))
    }
}

impl Record {
    /// Takes over live progress. Returns whether anything changed.
    fn apply(&mut self, progress: &Progress) -> bool {
        if progress.phase == Phase::Connecting {
            return false;
        }
        let before = (self.size, self.downloaded, self.resumable);
        if progress.total.is_some() {
            self.size = progress.total;
        }
        self.downloaded = progress.downloaded;
        self.resumable = progress.resumable;
        before != (self.size, self.downloaded, self.resumable)
    }

    fn finish(&mut self, result: Result<Outcome, String>) -> Option<Notice> {
        match result {
            Ok(Outcome::Completed { path, size }) => {
                self.state = State::Completed;
                self.file_name = file_name_of(&path);
                self.path = path.clone();
                self.size = Some(size);
                self.downloaded = size;
                self.completed_at = Some(now_ms());
                self.headers.clear();
                if let Some(media) = &mut self.media {
                    media.cookies.clear();
                }
                Some(Notice::Completed {
                    id: self.id,
                    file_name: self.file_name.clone(),
                    path,
                })
            }
            Ok(Outcome::Paused { .. } | Outcome::Cancelled) => {
                self.state = State::Paused;
                None
            }
            Err(error) => {
                self.state = State::Failed;
                self.error = Some(error.clone());
                Some(Notice::Failed {
                    file_name: self.file_name.clone(),
                    error,
                })
            }
        }
    }
}

fn validate_records(records: &[Record]) -> std::io::Result<()> {
    let mut ids = HashSet::new();
    for record in records {
        if record.id == 0 || record.id == u64::MAX || !ids.insert(record.id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "The download list contains an invalid or duplicate record ID: {}",
                    record.id
                ),
            ));
        }
    }
    Ok(())
}

/// What is going on with a running download right now.
struct Live<'a> {
    progress: Option<&'a Progress>,
    stopping: bool,
    running_for: Duration,
    torrent: Option<TorrentLive>,
}

fn view(record: &Record, live: &Live) -> DownloadView {
    let progress = live.progress;
    DownloadView {
        id: record.id,
        url: record.url.clone(),
        file_name: record.file_name.clone(),
        path: record.path.clone(),
        size: record.size,
        downloaded: record.downloaded,
        resumable: record.resumable,
        category: record.category,
        state: record.state,
        error: record.error.clone(),
        added_at: record.added_at,
        completed_at: record.completed_at,
        elapsed: record.elapsed_ms + millis(live.running_for),
        speed: progress.map_or(0, |p| p.speed),
        eta: progress.and_then(Progress::eta).map(|eta| eta.as_secs()),
        connections: progress.map_or(0, |p| p.connections),
        segments: progress.map(|p| p.segments.clone()).unwrap_or_default(),
        stopping: live.stopping,
        media: record.media.is_some(),
        processing: progress.is_some_and(|p| p.phase == Phase::Finishing),
        torrent: record.torrent.is_some(),
        seeding: live.torrent.is_some_and(|t| t.seeding),
        checking: live.torrent.is_some_and(|t| t.checking),
        upload_speed: live.torrent.map_or(0, |t| t.upload_speed),
        uploaded: live.torrent.map_or(0, |t| t.uploaded),
        queue: record.queue.clone(),
    }
}

/// Deletes what is left of a download that leaves the list.
fn discard_files(record: &Record, delete_file: bool, tools: &Tools) {
    let files = sdm_core::PartialFiles::new(&record.path, record.temp_dir.as_deref());
    if record.media.is_some() {
        let directory = files
            .directory()
            .map_or_else(|| tools.work_dir(record.id), Path::to_path_buf);
        let _ = std::fs::remove_dir_all(directory);
    }
    for path in [
        sdm_core::partial_data_path(&record.path),
        sdm_core::partial_state_path(&record.path),
        files.data.clone(),
        files.state.clone(),
        files.completion.clone(),
        with_suffix(&files.state, ".tmp"),
        with_suffix(&files.data, ".migrating"),
        with_suffix(&files.state, ".migrating"),
    ] {
        let _ = std::fs::remove_file(path);
    }
    if let Some(directory) = files.directory() {
        let _ = std::fs::remove_dir(directory);
    }
    let root = record
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".sdm-finalizing");
    let staging = sdm_core::PartialFiles::new(&record.path, Some(&root));
    let _ = std::fs::remove_file(&staging.data);
    if let Some(directory) = staging.directory() {
        let _ = std::fs::remove_dir(directory);
    }
    let _ = std::fs::remove_dir(root);
    if delete_file
        && record.state == State::Completed
        && record.path.exists()
        && let Err(error) = trash::delete(&record.path)
    {
        tracing::warn!(%error, path = %record.path.display(), "could not move the file to the trash");
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

async fn cleanup_completed(record: &Record, tools: &Tools) -> bool {
    if record.torrent.is_some() {
        return true;
    }
    let mut cleaned = true;
    let files = sdm_core::PartialFiles::new(&record.path, record.temp_dir.as_deref());
    if record.media.is_some() {
        let directory = files
            .directory()
            .map_or_else(|| tools.work_dir(record.id), Path::to_path_buf);
        if let Err(error) = tokio::fs::remove_dir_all(&directory).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "could not clean completed media temporary files");
            cleaned = false;
        }
    } else {
        if let Err(error) = sdm_core::acknowledge_completion(
            &record.path,
            record.temp_dir.as_deref(),
            &record.id.to_string(),
        )
        .await
        {
            tracing::warn!(%error, id = record.id, "could not acknowledge the saved completion");
            return false;
        }
        for root in [record.temp_dir.as_deref(), None] {
            if let Err(error) = sdm_core::cleanup_partial(&record.path, root).await {
                tracing::warn!(%error, "could not clean completed download temporary files");
                cleaned = false;
            }
        }
    }
    cleaned
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn name_from_url(url: &url::Url) -> String {
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
        .unwrap_or_default();
    sdm_core::sanitize_file_name(&percent_decode_str(segment).decode_utf8_lossy())
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use axum::Router;
    use axum::routing::get;

    use super::*;

    const DATA: &[u8] = b"pretend this is a video file";

    /// Serves `DATA` for `/<name>` and 404 for everything else.
    async fn server() -> String {
        let app = Router::new().route("/{name}", get(|| async { DATA }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    struct Fixture {
        dir: tempfile::TempDir,
        notices: Arc<StdMutex<Vec<Notice>>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                notices: Arc::default(),
            }
        }

        fn downloads(&self) -> PathBuf {
            self.dir.path().join("Downloads")
        }

        fn manager(&self) -> Arc<Manager> {
            self.manager_with(Tools::new(self.dir.path(), None))
        }

        fn manager_with(&self, tools: Tools) -> Arc<Manager> {
            self.try_manager_with(tools).unwrap()
        }

        fn try_manager_with(&self, tools: Tools) -> anyhow::Result<Arc<Manager>> {
            let settings = Settings {
                download_dir: self.downloads(),
                proxy: "none".into(),
                ..Settings::default()
            };
            let notices = self.notices.clone();
            let torrents = TorrentEngine::new(
                self.dir.path().join("torrent"),
                torrent::EngineConfig {
                    local_only: true,
                    ..torrent::EngineConfig::default()
                },
            );
            Manager::new(
                self.dir.path().join("downloads.json"),
                settings,
                Arc::new(tools),
                torrents,
                Handle::current(),
                Box::new(move |notice| notices.lock().unwrap().push(notice)),
            )
        }

        /// A stand-in for yt-dlp: a shell script that reports progress and
        /// writes the file, or fails when the page contains "broken", or
        /// hangs when it contains "slow".
        #[cfg(unix)]
        fn fake_ytdlp(&self) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let script = self.dir.path().join("yt-dlp");
            std::fs::write(
                &script,
                r#"#!/bin/sh
home=""; out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -P) case "$2" in home:*) home="${2#home:}";; esac; shift 2;;
    -o) out="$2"; shift 2;;
    --) url="$2"; shift 2;;
    *) shift;;
  esac
done
case "$url" in *broken*) echo "ERROR: [generic] Unsupported URL: $url" >&2; exit 1;; esac
case "$url" in *slow*) echo "SDMP downloading 10 100 NA 5 137"; sleep 30; exit 1;; esac
name=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/; s/%%/%/g')
echo "SDMP downloading 40 100 NA 1000 137"
echo "SDMP finished 100 100 NA 1000 137"
echo "SDMP downloading 10 20 NA 1000 140"
echo "SDMP finished 20 20 NA 1000 140"
mkdir -p "$home"
printf 'video+audio' > "$home/$name"
echo "SDMF $home/$name"
"#,
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }
    }

    #[cfg(unix)]
    fn media_request(page: &str, title: &str) -> AddMediaRequest {
        AddMediaRequest {
            source: media::MediaSource {
                page_url: page.into(),
                cookies: vec![media::Cookie {
                    name: "SID".into(),
                    value: "secret".into(),
                    domain: ".example.com".into(),
                    path: "/".into(),
                    secure: true,
                    host_only: false,
                    expires: None,
                }],
                ..media::MediaSource::default()
            },
            option: media::MediaOption {
                id: "v1080".into(),
                kind: media::MediaKind::Video,
                height: Some(1080),
                fps: Some(30),
                ext: "mp4".into(),
                vcodec: Some("H.264".into()),
                acodec: Some("AAC".into()),
                size: Some(120),
                needs_ffmpeg: true,
                format: "137+140".into(),
                merge: Some("mp4".into()),
                audio: None,
            },
            title: title.into(),
            directory: None,
            start: true,
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_videos_with_ytdlp() {
        let fixture = Fixture::new();
        let manager =
            fixture.manager_with(Tools::with_ytdlp(fixture.dir.path(), fixture.fake_ytdlp()));

        let id = manager
            .add_media(media_request(
                "https://example.com/watch?v=1",
                "Clip: 100% <live>",
            ))
            .unwrap();
        let done = wait_for(&manager, id, State::Completed).await;
        let expected = fixture
            .downloads()
            .join("Video")
            .join("Clip_ 100% _live_.mp4");
        assert_eq!(done.path, expected);
        assert!(done.media);
        assert_eq!(std::fs::read(&expected).unwrap(), b"video+audio");
        assert_eq!(done.size, Some(11));
        // Login cookies and the unfinished files are gone.
        let stored = std::fs::read_to_string(fixture.dir.path().join("downloads.json")).unwrap();
        assert!(!stored.contains("secret"));
        assert!(
            !fixture
                .dir
                .path()
                .join("media")
                .join(id.to_string())
                .exists()
        );

        let id = manager
            .add_media(media_request("https://example.com/broken", "Broken"))
            .unwrap();
        let failed = wait_for(&manager, id, State::Failed).await;
        assert_eq!(
            failed.error.as_deref(),
            Some("[generic] Unsupported URL: https://example.com/broken")
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn pauses_videos_by_stopping_ytdlp() {
        let fixture = Fixture::new();
        let manager =
            fixture.manager_with(Tools::with_ytdlp(fixture.dir.path(), fixture.fake_ytdlp()));
        let id = manager
            .add_media(media_request("https://example.com/slow", "Slow"))
            .unwrap();
        for _ in 0..500 {
            if manager.get(id).unwrap().downloaded > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(manager.get(id).unwrap().downloaded, 10);
        let started = Instant::now();
        manager.pause(&[id]);
        wait_for(&manager, id, State::Paused).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "yt-dlp was not stopped"
        );
    }

    /// A torrent from a .torrent file: read, one file chosen, downloaded
    /// from a seeder on this computer, then removed from the list.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn downloads_torrents() {
        use crate::torrent::testing::{data, make_torrent, tracker};

        let fixture = Fixture::new();
        let shared = fixture.dir.path().join("shared/Holiday");
        std::fs::create_dir_all(&shared).unwrap();
        let video = data(900_000, 3);
        std::fs::write(shared.join("beach.mp4"), &video).unwrap();
        std::fs::write(shared.join("notes.txt"), b"sunny").unwrap();
        let file = fixture.dir.path().join("Holiday.torrent");
        std::fs::write(&file, make_torrent(&shared, &tracker().await).await).unwrap();
        let source = TorrentSource {
            path: Some(file),
            ..TorrentSource::default()
        };

        let seeder = TorrentEngine::new(
            fixture.dir.path().join("seeder"),
            torrent::EngineConfig {
                local_only: true,
                seed_ratio: torrent::SEED_FOREVER,
                ..torrent::EngineConfig::default()
            },
        );
        let downloader = Downloader::new(sdm_core::ClientConfig::default()).unwrap();
        let info = seeder.inspect(&source, &downloader).await.unwrap();
        let shares = seeder.start(torrent::JobConfig {
            info_hash: info.info_hash.clone(),
            files: None,
            sizes: info.files.iter().map(|f| (f.index, f.size)).collect(),
            folder: shared.clone(),
            path: shared.clone(),
        });
        assert!(matches!(shares.wait().await, Ok(Outcome::Completed { .. })));

        let manager = fixture.manager();
        manager.torrents.set_config(torrent::EngineConfig {
            local_only: true,
            seed_ratio: torrent::SEED_FOREVER,
            ..torrent::EngineConfig::default()
        });
        let info = manager.inspect_torrent(source.clone()).await.unwrap();
        assert_eq!(info.directory, fixture.downloads().join("Video"));
        assert_eq!(info.existing, None);
        let beach = info.files.iter().find(|f| f.path == "beach.mp4").unwrap();
        let id = manager
            .add_torrent(AddTorrentRequest {
                info_hash: info.info_hash.clone(),
                files: Some(vec![beach.index]),
                start: true,
                ..AddTorrentRequest::default()
            })
            .unwrap();
        let again = AddTorrentRequest {
            info_hash: info.info_hash.clone(),
            start: true,
            ..AddTorrentRequest::default()
        };
        assert!(manager.add_torrent(again).is_err());
        assert_eq!(
            manager.inspect_torrent(source).await.unwrap().existing,
            Some(id)
        );

        let done = wait_for(&manager, id, State::Completed).await;
        let folder = fixture.downloads().join("Video").join("Holiday");
        assert_eq!(done.path, folder);
        assert_eq!(done.file_name, "Holiday");
        assert!(done.torrent);
        assert!(done.seeding);
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Busy)
        ));
        assert!(
            manager.get(id).unwrap().seeding,
            "updates never stop sharing"
        );
        assert_eq!(done.size, Some(900_000));
        assert!(done.url.starts_with("magnet:?xt=urn:btih:"));
        assert_eq!(std::fs::read(folder.join("beach.mp4")).unwrap(), video);
        assert!(
            fixture
                .notices
                .lock()
                .unwrap()
                .iter()
                .any(|n| matches!(n, Notice::Completed { id: done, .. } if *done == id))
        );

        // Removing keeps the finished files but forgets the torrent.
        let torrent_file = fixture
            .dir
            .path()
            .join("torrent/torrents")
            .join(format!("{}.torrent", info.info_hash));
        assert!(torrent_file.exists());
        manager.remove(&[id], false);
        for _ in 0..200 {
            if !torrent_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!torrent_file.exists());
        assert!(folder.join("beach.mp4").exists());
        manager.shutdown().await;
        seeder.shutdown().await;
    }

    fn request(url: String, start: bool) -> AddRequest {
        AddRequest {
            url,
            start,
            ..AddRequest::default()
        }
    }

    async fn wait_for(manager: &Manager, id: u64, state: State) -> DownloadView {
        for _ in 0..1000 {
            let view = manager.list().into_iter().find(|d| d.id == id).unwrap();
            if view.state == state {
                return view;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("download {id} never reached {state:?}");
    }

    async fn wait_for_cleanup(manager: &Manager, id: u64) {
        for _ in 0..500 {
            {
                let inner = manager.lock();
                if !inner.cleanup_pending.contains(&id) && !inner.cleanup_running.contains(&id) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("download {id} did not finish cleaning its temporary files");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_into_category_folder_and_notifies() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();

        let id = manager
            .add(request(format!("{base}/clip.mp4"), true))
            .await
            .unwrap();
        let done = wait_for(&manager, id, State::Completed).await;

        let expected = fixture.downloads().join("Video").join("clip.mp4");
        assert_eq!(done.path, expected);
        assert_eq!(done.category, Category::Video);
        assert_eq!(done.size, Some(DATA.len() as u64));
        assert_eq!(std::fs::read(&expected).unwrap(), DATA);
        assert_eq!(
            fixture.notices.lock().unwrap().as_slice(),
            [
                Notice::Started { id },
                Notice::Completed {
                    id,
                    file_name: "clip.mp4".into(),
                    path: expected
                }
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_later_then_resume() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();

        let id = manager
            .add(request(format!("{base}/notes.txt"), false))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(manager.list()[0].state, State::Paused);

        manager.resume(&[id]);
        let done = wait_for(&manager, id, State::Completed).await;
        assert_eq!(std::fs::read(done.path).unwrap(), DATA);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gives_duplicates_their_own_names() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();

        let url = format!("{base}/setup.zip");
        manager.add(request(url.clone(), false)).await.unwrap();
        manager.add(request(url, false)).await.unwrap();
        let names: Vec<String> = manager.list().into_iter().map(|d| d.file_name).collect();
        assert_eq!(names, ["setup.zip", "setup (1).zip"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uses_the_chosen_name_and_folder() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();
        let folder = fixture.dir.path().join("Chosen");

        let id = manager
            .add(AddRequest {
                url: format!("{base}/x.bin"),
                file_name: Some("my: file.bin".into()),
                directory: Some(folder.clone()),
                start: true,
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let done = wait_for(&manager, id, State::Completed).await;
        assert_eq!(done.path, folder.join("my_ file.bin"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_failures() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();

        let id = manager
            .add(request(format!("{base}/missing/file.zip"), true))
            .await
            .unwrap();
        let failed = wait_for(&manager, id, State::Failed).await;
        assert_eq!(failed.file_name, "file.zip");
        assert!(failed.error.unwrap().contains("404"));
        assert!(matches!(
            fixture.notices.lock().unwrap().as_slice(),
            [Notice::Started { .. }, Notice::Failed { .. }]
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_unsupported_links() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        assert!(
            manager
                .add(request("ftp://example.com/a.zip".into(), true))
                .await
                .is_err()
        );
        assert!(
            manager
                .add(request("not a link".into(), true))
                .await
                .is_err()
        );
        assert!(manager.list().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remembers_downloads_across_restarts() {
        let fixture = Fixture::new();
        let base = server().await;
        let id = {
            let manager = fixture.manager();
            manager
                .add(request(format!("{base}/movie.mkv"), false))
                .await
                .unwrap()
        };

        // Pretend the app was closed while the download was running.
        let store = fixture.dir.path().join("downloads.json");
        let mut records: Vec<Record> = store::load_json(&store).unwrap().unwrap();
        records[0].state = State::Downloading;
        records[0].headers = vec![("Cookie".into(), "s=1".into())];
        store::save_json(&store, &records).unwrap();

        let manager = fixture.manager();
        let list = manager.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].state, State::Paused);

        // New downloads continue the numbering.
        let next = manager
            .add(request(format!("{base}/other.mkv"), false))
            .await
            .unwrap();
        assert_eq!(next, id + 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn removing_deletes_unfinished_data_but_keeps_files() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();

        let paused = manager
            .add(request(format!("{base}/big.iso"), false))
            .await
            .unwrap();
        let path = manager.path_of(paused).unwrap().0;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(sdm_core::partial_data_path(&path), b"partial").unwrap();
        std::fs::write(sdm_core::partial_state_path(&path), b"{}").unwrap();

        let finished = manager
            .add(request(format!("{base}/done.txt"), true))
            .await
            .unwrap();
        let done = wait_for(&manager, finished, State::Completed).await;

        manager.remove(&[paused, finished], false);
        // Completion is saved before cleanup. Removal retains the row and
        // its path reservation until an in-flight cleanup finishes.
        for _ in 0..500 {
            if manager.get(paused).is_none() && manager.get(finished).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(manager.list().is_empty());
        assert!(!sdm_core::partial_data_path(&path).exists());
        assert!(!sdm_core::partial_state_path(&path).exists());
        let files = sdm_core::PartialFiles::new(&done.path, Some(&manager.settings().temp_dir));
        assert!(!files.directory().unwrap().exists());
        assert!(!files.completion.exists());
        assert_eq!(
            std::fs::read(done.path).unwrap(),
            DATA,
            "finished files stay unless asked"
        );
        let saved: Vec<Record> = store::load_json(&fixture.dir.path().join("downloads.json"))
            .unwrap()
            .unwrap();
        assert!(saved.is_empty(), "both removals must be durably recorded");
        manager.shutdown().await;
    }

    /// Serves 2 MiB with byte range support, so downloads can be resumed.
    async fn big_file_server() -> (String, Vec<u8>) {
        use axum::http::{HeaderMap, StatusCode, header};
        let data: Vec<u8> = (0..2 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        let body = data.clone();
        let app = Router::new().route(
            "/{name}",
            get(move |headers: HeaderMap| {
                let data = body.clone();
                async move {
                    let start: usize = headers
                        .get(header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("bytes="))
                        .and_then(|v| v.split('-').next())
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let end = headers
                        .get(header::RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.split('-').nth(1))
                        .and_then(|v| v.parse::<usize>().ok())
                        .map_or(data.len(), |e| e + 1);
                    let range = format!("bytes {start}-{}/{}", end - 1, data.len());
                    (
                        StatusCode::PARTIAL_CONTENT,
                        [(header::CONTENT_RANGE, range)],
                        data[start..end].to_vec(),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), data)
    }

    fn local(hour: u32, minute: u32) -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 12)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queues_run_their_downloads_on_schedule() {
        let fixture = Fixture::new();
        let (base, _) = big_file_server().await;
        let manager = fixture.manager();
        let mut slow = manager.settings();
        slow.speed_limit = 256 * 1024;
        manager.update_settings(slow).unwrap();

        let night = manager
            .save_queue(Queue {
                id: String::new(),
                name: "Night".into(),
                max_parallel: 1,
                schedule: queue::Schedule {
                    enabled: true,
                    start: Some("01:00".into()),
                    stop: Some("05:00".into()),
                    days: Vec::new(),
                },
                when_done: PowerAction::Nothing,
            })
            .unwrap();
        assert_eq!(night.id, "q1");
        let mut ids = Vec::new();
        for name in ["a.bin", "b.bin"] {
            let request = AddRequest {
                url: format!("{base}/{name}"),
                queue: Some(night.id.clone()),
                ..AddRequest::default()
            };
            ids.push(manager.add(request).await.unwrap());
        }
        // An unknown queue means the main queue.
        let other = manager
            .add(AddRequest {
                url: format!("{base}/c.bin"),
                queue: Some("gone".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();

        // Nothing runs until the schedule starts the queue.
        manager.check_schedules(local(0, 59));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let list = manager.list();
        assert!(list.iter().all(|d| d.state == State::Paused));
        assert_eq!(
            list.iter().map(|d| d.queue.as_str()).collect::<Vec<_>>(),
            ["q1", "q1", "main"]
        );

        // One at a time, as the queue says.
        manager.check_schedules(local(1, 0));
        wait_for(&manager, ids[0], State::Downloading).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(manager.get(ids[1]).unwrap().state, State::Queued);
        assert_eq!(manager.get(other).unwrap().state, State::Paused);

        manager.check_schedules(local(5, 0));
        wait_for(&manager, ids[0], State::Paused).await;
        assert_eq!(manager.get(ids[1]).unwrap().state, State::Paused);
        // A schedule without days runs once.
        assert!(!manager.queues()[1].schedule.enabled);

        // Queues are remembered; deleting one moves its downloads.
        manager.shutdown().await;
        drop(manager);
        let manager = fixture.manager();
        assert_eq!(manager.queues()[1].name, "Night");
        assert_eq!(manager.get(ids[1]).unwrap().queue, "q1");
        manager.delete_queue("q1");
        manager.delete_queue(MAIN_QUEUE);
        assert_eq!(manager.queues().len(), 1);
        assert!(manager.list().iter().all(|d| d.queue == MAIN_QUEUE));
        manager.move_to_queue(&ids, "nowhere");
        assert_eq!(manager.get(ids[0]).unwrap().queue, MAIN_QUEUE);
        manager.shutdown().await;
    }

    async fn wait_for_notice(fixture: &Fixture, wanted: &Notice) {
        for _ in 0..200 {
            if fixture.notices.lock().unwrap().contains(wanted) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no {wanted:?} in {:?}", fixture.notices.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn says_what_to_do_when_downloads_are_done() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();
        let mut main = manager.queues()[0].clone();
        main.when_done = PowerAction::Shutdown;
        let main = manager.save_queue(main).unwrap();

        let id = manager
            .add(request(format!("{base}/one.zip"), true))
            .await
            .unwrap();
        wait_for(&manager, id, State::Completed).await;
        wait_for_notice(
            &fixture,
            &Notice::Done {
                queue: Some(main),
                action: PowerAction::Shutdown,
            },
        )
        .await;

        // "When all downloads are done" wins, and only counts once.
        manager.set_after_all(PowerAction::Quit);
        let id = manager
            .add(request(format!("{base}/two.zip"), true))
            .await
            .unwrap();
        wait_for(&manager, id, State::Completed).await;
        wait_for_notice(
            &fixture,
            &Notice::Done {
                queue: None,
                action: PowerAction::Quit,
            },
        )
        .await;
        assert_eq!(manager.after_all(), PowerAction::Nothing);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pausing_does_not_count_as_done() {
        let fixture = Fixture::new();
        let (base, _) = big_file_server().await;
        let manager = fixture.manager();
        let mut slow = manager.settings();
        slow.speed_limit = 64 * 1024;
        manager.update_settings(slow).unwrap();
        manager.set_after_all(PowerAction::Shutdown);

        let id = manager
            .add(request(format!("{base}/big.bin"), true))
            .await
            .unwrap();
        wait_for(&manager, id, State::Downloading).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        manager.pause(&[id]);
        wait_for(&manager, id, State::Paused).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !fixture
                .notices
                .lock()
                .unwrap()
                .iter()
                .any(|n| matches!(n, Notice::Done { .. }))
        );
        assert_eq!(manager.after_all(), PowerAction::Shutdown);
        manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adds_batches_without_asking_each_server() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();
        let links = crate::batch::expand(&format!("{base}/photo[1-3].jpg"))
            .unwrap()
            .into_iter()
            .map(|url| BatchLink {
                url,
                ..BatchLink::default()
            })
            .chain([
                BatchLink {
                    url: "mailto:someone@example.com".into(),
                    ..BatchLink::default()
                },
                BatchLink {
                    url: format!("{base}/x"),
                    file_name: Some("named.txt".into()),
                    headers: vec![("referer".into(), "https://page.example/b".into())],
                    text: None,
                },
            ])
            .collect();
        let ids = manager
            .add_batch(BatchRequest {
                links,
                headers: vec![("Referer".into(), "https://page.example/a".into())],
                start: true,
                ..BatchRequest::default()
            })
            .unwrap();
        assert_eq!(ids.len(), 4);
        for &id in &ids {
            wait_for(&manager, id, State::Completed).await;
        }
        let list = manager.list();
        assert_eq!(
            list.iter()
                .map(|d| d.file_name.as_str())
                .collect::<Vec<_>>(),
            ["photo1.jpg", "photo2.jpg", "photo3.jpg", "named.txt"]
        );
        assert_eq!(list[0].category, Category::Image);
        let record = manager.lock().records[3].clone();
        // Headers of a link replace those of the batch; cleared once done.
        assert!(record.headers.is_empty());
        assert!(
            manager
                .add_batch(BatchRequest {
                    links: vec![BatchLink {
                        url: "ftp://x/y".into(),
                        ..BatchLink::default()
                    }],
                    ..BatchRequest::default()
                })
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_link_continues_the_download() {
        let fixture = Fixture::new();
        let (base, data) = big_file_server().await;
        let small = server().await;
        let manager = fixture.manager();
        let mut slow = manager.settings();
        slow.speed_limit = 256 * 1024;
        manager.update_settings(slow).unwrap();

        let id = manager
            .add(request(format!("{base}/old-link.bin"), true))
            .await
            .unwrap();
        wait_for(&manager, id, State::Downloading).await;
        while manager.get(id).unwrap().downloaded < 300 * 1024 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // A link to another file is refused, also while it runs.
        let error = manager
            .change_address(id, format!("{small}/other.bin"), None)
            .await
            .unwrap_err();
        assert!(error.contains("different file"), "{error}");
        assert!(
            manager
                .change_address(id, "nope".into(), None)
                .await
                .is_err()
        );
        let kept = manager.get(id).unwrap();
        assert_eq!(kept.state, State::Paused);
        assert!(kept.downloaded >= 300 * 1024);

        let fresh = format!("{base}/fresh-link.bin?token=2");
        manager
            .change_address(id, fresh.clone(), None)
            .await
            .unwrap();
        let view = manager.get(id).unwrap();
        assert_eq!(view.url, fresh);
        assert_eq!(view.downloaded, kept.downloaded);
        let state = std::fs::read_to_string(
            sdm_core::PartialFiles::new(&view.path, Some(&manager.settings().temp_dir)).state,
        )
        .unwrap();
        assert!(state.contains("fresh-link.bin"), "{state}");

        let mut fast = manager.settings();
        fast.speed_limit = 0;
        manager.update_settings(fast).unwrap();
        manager.resume(&[id]);
        let done = wait_for(&manager, id, State::Completed).await;
        assert_eq!(std::fs::read(done.path).unwrap(), data);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_keeps_progress_for_the_next_start() {
        let fixture = Fixture::new();
        let (base, data) = big_file_server().await;
        let manager = fixture.manager();
        let mut slow = manager.settings();
        slow.speed_limit = 256 * 1024;
        manager.update_settings(slow).unwrap();

        let id = manager
            .add(request(format!("{base}/backup.iso"), true))
            .await
            .unwrap();
        for _ in 0..500 {
            if manager.list()[0].downloaded > 100 * 1024 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        manager.shutdown().await;

        let path = manager.path_of(id).unwrap().0;
        assert!(
            sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir))
                .state
                .exists(),
            "progress was saved in the configured temp folder"
        );
        let saved = manager.list()[0].downloaded;
        assert!(saved > 0);
        drop(manager);

        // The next start shows the download paused and finishes it.
        let manager = fixture.manager();
        let view = &manager.list()[0];
        assert_eq!(view.state, State::Paused);
        assert_eq!(view.downloaded, saved);
        manager.resume(&[id]);
        let done = wait_for(&manager, id, State::Completed).await;
        assert_eq!(std::fs::read(done.path).unwrap(), data);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn temp_folder_change_preserves_paused_downloads_across_restart() {
        let fixture = Fixture::new();
        let (base, data) = big_file_server().await;
        let manager = fixture.manager();
        let old_root = manager.settings().temp_dir;
        let mut slow = manager.settings();
        slow.speed_limit = 256 * 1024;
        manager.update_settings(slow).unwrap();
        let id = manager
            .add(request(format!("{base}/old.bin"), true))
            .await
            .unwrap();
        for _ in 0..500 {
            if manager.get(id).unwrap().downloaded > 100 * 1024 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        manager.pause(&[id]);
        let paused = wait_for(&manager, id, State::Paused).await;
        assert!(paused.downloaded > 0);
        let old_files = sdm_core::PartialFiles::new(&paused.path, Some(&old_root));
        assert!(old_files.state.exists());
        assert!(!sdm_core::partial_data_path(&paused.path).exists());
        let new_root = fixture.dir.path().join("custom-temp");
        let mut changed = manager.settings();
        changed.temp_dir = new_root.clone();
        changed.speed_limit = 0;
        manager.update_settings(changed).unwrap();
        let next = manager
            .add(request(format!("{base}/new.bin"), false))
            .await
            .unwrap();
        assert_eq!(
            manager
                .lock()
                .records
                .iter()
                .find(|r| r.id == next)
                .unwrap()
                .temp_dir
                .as_ref(),
            Some(&new_root)
        );
        manager.resume(&[next]);
        let done = wait_for(&manager, next, State::Completed).await;
        wait_for_cleanup(&manager, next).await;
        assert_eq!(std::fs::read(&done.path).unwrap(), data);
        assert_eq!(std::fs::read_dir(&new_root).unwrap().count(), 0);
        manager.shutdown().await;
        drop(manager);
        // Even when global settings change, the stored per-download root wins.
        let manager = fixture.manager();
        assert!(old_files.state.exists());
        manager.resume(&[id]);
        let done = wait_for(&manager, id, State::Completed).await;
        wait_for_cleanup(&manager, id).await;
        assert_eq!(std::fs::read(done.path).unwrap(), data);
        assert!(!old_files.directory().unwrap().exists());
        manager.shutdown().await;
    }

    async fn legacy_fixture(fixture: &Fixture) -> (u64, PathBuf) {
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/offline.zip".into(),
                file_name: Some("offline.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let path = manager.path_of(id).unwrap().0;
        {
            let mut inner = manager.lock();
            inner.records[0].temp_dir = None;
            inner.records[0].downloaded = 7;
            inner.dirty = true;
            manager.save(&mut inner);
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(sdm_core::partial_data_path(&path), b"partial").unwrap();
        std::fs::write(sdm_core::partial_state_path(&path), b"legacy progress").unwrap();
        std::fs::write(
            with_suffix(&sdm_core::partial_state_path(&path), ".tmp"),
            b"scratch",
        )
        .unwrap();
        manager.shutdown().await;
        (id, path)
    }

    async fn wait_for_migrations(manager: &Manager) {
        for _ in 0..500 {
            if manager.lock().migrating.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("legacy migration did not finish");
    }

    #[tokio::test]
    async fn startup_moves_known_legacy_sidecars_while_offline() {
        let fixture = Fixture::new();
        let (id, path) = legacy_fixture(&fixture).await;
        let unknown = path.with_file_name("untracked.exe.sdm-part");
        std::fs::write(&unknown, b"keep").unwrap();
        let manager = fixture.manager();
        wait_for_migrations(&manager).await;
        assert_eq!(manager.get(id).unwrap().state, State::Paused);
        assert_eq!(manager.get(id).unwrap().downloaded, 7);
        let files = sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir));
        assert_eq!(std::fs::read(&files.data).unwrap(), b"partial");
        assert_eq!(std::fs::read(&files.state).unwrap(), b"legacy progress");
        assert_eq!(
            std::fs::read(with_suffix(&files.state, ".tmp")).unwrap(),
            b"scratch"
        );
        assert!(!with_suffix(&sdm_core::partial_state_path(&path), ".tmp").exists());
        assert!(!sdm_core::partial_data_path(&path).exists());
        assert!(!sdm_core::partial_state_path(&path).exists());
        assert_eq!(std::fs::read(unknown).unwrap(), b"keep");
        let persisted: Vec<Record> = store::load_json(&fixture.dir.path().join("downloads.json"))
            .unwrap()
            .unwrap();
        assert_eq!(persisted[0].temp_dir, Some(manager.settings().temp_dir));
        manager.remove(&[id], false);
        assert!(!files.directory().unwrap().exists());
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn removing_during_legacy_move_cleans_both_locations() {
        let fixture = Fixture::new();
        let (id, path) = legacy_fixture(&fixture).await;
        let manager = fixture.manager();
        // Current-thread runtime: the background task has not run yet.
        assert!(manager.lock().migrating.contains_key(&id));
        manager.remove(&[id], false);
        assert!(manager.get(id).unwrap().stopping);
        wait_for_migrations(&manager).await;
        assert!(manager.get(id).is_none());
        assert!(!sdm_core::partial_data_path(&path).exists());
        assert!(!sdm_core::partial_state_path(&path).exists());
        assert!(
            !sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir))
                .directory()
                .unwrap()
                .exists()
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn startup_cleans_completed_leftovers_without_deleting_final_file() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();
        let id = manager
            .add(request(format!("{base}/done.zip"), true))
            .await
            .unwrap();
        let done = wait_for(&manager, id, State::Completed).await;
        let root = manager.settings().temp_dir;
        let files = sdm_core::PartialFiles::new(&done.path, Some(&root));
        manager.shutdown().await;
        drop(manager);
        std::fs::create_dir_all(files.directory().unwrap()).unwrap();
        std::fs::write(&files.data, b"leftover").unwrap();
        std::fs::write(with_suffix(&files.state, ".tmp"), b"leftover").unwrap();
        std::fs::write(sdm_core::partial_state_path(&done.path), b"legacy leftover").unwrap();
        let staging_root = done.path.parent().unwrap().join(".sdm-finalizing");
        let staging = sdm_core::PartialFiles::new(&done.path, Some(&staging_root));
        std::fs::create_dir_all(staging.directory().unwrap()).unwrap();
        std::fs::write(staging.data, b"interrupted copy").unwrap();
        let unrelated = root.join("keep.txt");
        std::fs::write(&unrelated, b"keep").unwrap();
        let manager = fixture.manager();
        for _ in 0..500 {
            if !files.directory().unwrap().exists()
                && !staging_root.exists()
                && !sdm_core::partial_state_path(&done.path).exists()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!files.directory().unwrap().exists());
        assert!(!staging_root.exists());
        assert!(!sdm_core::partial_state_path(&done.path).exists());
        assert_eq!(std::fs::read(done.path).unwrap(), DATA);
        assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
        assert_eq!(manager.get(id).unwrap().state, State::Completed);
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn rejected_settings_leave_disk_and_runtime_unchanged() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let path = fixture.dir.path().join("settings.json");
        let previous = manager.settings();
        previous.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut invalid = previous.clone();
        invalid.proxy = "http://[".into();
        invalid.speed_limit = 12345;
        assert!(manager.save_settings(&path, invalid).is_err());
        assert_eq!(manager.settings(), previous);
        assert_eq!(manager.downloader().speed_limit(), previous.speed_limit());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let backup = with_suffix(&path, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        let mut candidate = previous.clone();
        candidate.language = "vi".into();
        candidate.speed_limit = 54321;
        assert!(manager.save_settings(&path, candidate).is_err());
        assert_eq!(manager.settings(), previous);
        assert_eq!(manager.downloader().speed_limit(), previous.speed_limit());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_reservation_saves_history_and_releases_downloads_after_installer_failure() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let base = server().await;
        let id = manager
            .add(request(format!("{base}/kept.zip"), false))
            .await
            .unwrap();
        assert!(manager.prepare_update().is_ok());
        let saved: Vec<Record> = store::load_json(&fixture.dir.path().join("downloads.json"))
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].id, id);
        assert_eq!(saved[0].state, State::Paused);
        let rejected = AddRequest {
            file_name: Some("blocked.zip".into()),
            ..request(format!("{base}/blocked.zip"), true)
        };
        assert!(manager.add(rejected).await.is_err());
        assert_eq!(
            manager.list().len(),
            1,
            "no incoming transfer races installer launch"
        );
        manager.resume(&[id]);
        assert!(
            manager.lock().active.is_empty(),
            "scheduling is reserved for the installer"
        );
        manager.cancel_update();
        let done = wait_for(&manager, id, State::Completed).await;
        assert_eq!(std::fs::read(done.path).unwrap(), DATA);
        wait_for_cleanup(&manager, id).await;
        assert!(
            manager.prepare_update().is_ok(),
            "a failed installer can be retried after cleanup"
        );
        manager.cancel_update();
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn update_reservation_rejects_history_failure_without_closing_or_losing_data() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let added = AddRequest {
            file_name: Some("kept.zip".into()),
            ..request("http://127.0.0.1:1/kept.zip".into(), false)
        };
        let id = manager.add(added).await.unwrap();
        let path = fixture.dir.path().join("downloads.json");
        let bytes = std::fs::read(&path).unwrap();
        let backup = with_suffix(&path, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Persistence(_))
        ));
        assert!(
            !manager.lock().closing,
            "failed persistence leaves the running app usable"
        );
        assert!(!manager.lock().update_prepared);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(manager.get(id).unwrap().state, State::Paused);
        std::fs::remove_dir(&backup).unwrap();
        assert!(
            manager.prepare_update().is_ok(),
            "retry succeeds after fixing storage"
        );
        manager.cancel_update();
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn update_reservation_rejects_queue_failure_and_pending_recovery_cleanup() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        std::fs::create_dir(&manager.queue_store).unwrap();
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Persistence(_))
        ));
        assert!(!manager.lock().closing);
        std::fs::remove_dir(&manager.queue_store).unwrap();
        {
            let mut inner = manager.lock();
            inner.migrating.insert(1, None);
        }
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Busy)
        ));
        {
            let mut inner = manager.lock();
            inner.migrating.clear();
            inner.cleanup_pending.insert(1);
        }
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Busy)
        ));
        {
            let mut inner = manager.lock();
            inner.cleanup_pending.clear();
            inner.cleanup_running.insert(1);
        }
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Busy)
        ));
        manager.lock().cleanup_running.clear();
        assert!(manager.prepare_update().is_ok());
        manager.cancel_update();
        manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_reservation_never_pauses_an_active_transfer() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let mut settings = manager.settings();
        settings.speed_limit = 16 * 1024;
        manager.update_settings(settings).unwrap();
        let (base, _) = big_file_server().await;
        let id = manager
            .add(request(format!("{base}/large.zip"), true))
            .await
            .unwrap();
        wait_for(&manager, id, State::Downloading).await;
        assert!(matches!(
            manager.prepare_update(),
            Err(UpdatePreparationError::Busy)
        ));
        let live = manager.get(id).unwrap();
        assert_eq!(live.state, State::Downloading);
        assert!(!live.stopping, "an updater must not pause a live transfer");
        assert!(!manager.lock().closing);
        manager.pause(&[id]);
        wait_for(&manager, id, State::Paused).await;
        assert!(manager.prepare_update().is_ok());
        manager.cancel_update();
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn failed_adds_rollback_records_and_never_start_transfers() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let request = |name: &str, start| AddRequest {
            url: format!("http://127.0.0.1:1/{name}"),
            file_name: Some(name.into()),
            start,
            ..AddRequest::default()
        };
        let kept = manager.add(request("kept.zip", false)).await.unwrap();
        let (previous, next_id) = {
            let inner = manager.lock();
            (inner.records.clone(), inner.next_id)
        };
        let path = fixture.dir.path().join("downloads.json");
        let bytes = std::fs::read(&path).unwrap();
        let backup = with_suffix(&path, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        assert!(
            manager
                .add(request("unsaved.zip", true))
                .await
                .unwrap_err()
                .contains("nothing was started")
        );
        assert!(
            manager
                .add_batch(BatchRequest {
                    links: vec![BatchLink {
                        url: "http://127.0.0.1:1/batch.zip".into(),
                        ..BatchLink::default()
                    }],
                    start: true,
                    ..BatchRequest::default()
                })
                .is_err()
        );
        #[cfg(unix)]
        assert!(
            manager
                .add_media(media_request("http://example.com/video", "unsaved-media"))
                .is_err()
        );
        {
            let inner = manager.lock();
            assert_eq!(inner.records, previous);
            assert_eq!(inner.next_id, next_id);
            assert!(inner.active.is_empty());
        }
        assert_eq!(manager.get(kept).unwrap().state, State::Paused);
        assert!(fixture.notices.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!fixture.downloads().exists());
        manager.resume(&[kept]);
        assert_eq!(manager.get(kept).unwrap().state, State::Queued);
        assert!(manager.lock().active.is_empty());
        assert!(fixture.notices.lock().unwrap().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn damaged_history_without_backup_is_reported_and_preserved() {
        let fixture = Fixture::new();
        let path = fixture.dir.path().join("downloads.json");
        std::fs::write(&path, b"[{broken history").unwrap();
        let error = fixture
            .try_manager_with(Tools::new(fixture.dir.path(), None))
            .err()
            .unwrap();
        assert!(error.to_string().contains("downloads.json"));
        assert_eq!(std::fs::read(&path).unwrap(), b"[{broken history");
        assert!(!fixture.downloads().exists());
    }

    #[tokio::test]
    async fn damaged_history_recovers_nonempty_backup_and_preserves_original() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/kept.zip".into(),
                file_name: Some("kept.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        manager.shutdown().await;
        drop(manager);
        let path = fixture.dir.path().join("downloads.json");
        std::fs::write(&path, b"[{broken history").unwrap();
        let manager = fixture.manager();
        wait_for_migrations(&manager).await;
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.get(id).unwrap().file_name, "kept.zip");
        assert!(manager.recovery_notice().unwrap().contains("preserved"));
        let damaged = std::fs::read_dir(fixture.dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains("downloads.json.damaged-"))
            .unwrap();
        assert_eq!(std::fs::read(damaged).unwrap(), b"[{broken history");
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn startup_reconciles_committed_http_and_retries_save_before_acknowledging() {
        let fixture = Fixture::new();
        let base = server().await;
        let manager = fixture.manager();
        let url = format!("{base}/committed.zip");
        let id = manager
            .add(AddRequest {
                url: url.clone(),
                file_name: Some("committed.zip".into()),
                headers: vec![("Cookie".into(), "session=fixture".into())],
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let (path, _) = manager.path_of(id).unwrap();
        let root = manager.settings().temp_dir;
        manager.shutdown().await;
        drop(manager);
        // Simulate a core commit followed by process death before the desktop
        // receives completion: the durable history is still Paused.
        let downloader = Downloader::new(sdm_core::ClientConfig {
            proxy: sdm_core::ProxyConfig::Disabled,
            ..sdm_core::ClientConfig::default()
        })
        .unwrap();
        let outcome = downloader
            .start(
                DownloadRequest::new(url, Destination::File(path.clone())).options(
                    DownloadOptions {
                        temp_dir: Some(root.clone()),
                        completion_id: Some(id.to_string()),
                        ..DownloadOptions::default()
                    },
                ),
            )
            .wait()
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Completed { .. }));
        let files = sdm_core::PartialFiles::new(&path, Some(&root));
        assert!(files.completion.exists());
        let history = fixture.dir.path().join("downloads.json");
        let backup = with_suffix(&history, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        let manager = fixture.manager();
        manager.remove(&[id], false);
        wait_for_migrations(&manager).await;
        assert_eq!(manager.get(id).unwrap().state, State::Completed);
        assert!(
            files.completion.exists(),
            "receipt must survive a failed history save"
        );
        let saved: Vec<Record> = store::load_json(&history).unwrap().unwrap();
        assert_eq!(saved[0].state, State::Paused);
        assert_eq!(std::fs::read(&path).unwrap(), DATA);
        assert!(manager.lock().active.is_empty());

        manager.remove(&[id], false);
        assert_eq!(manager.get(id).unwrap().state, State::Completed);
        assert!(
            files.completion.exists(),
            "failed removal must retain the recovery receipt"
        );
        assert_eq!(std::fs::read(&path).unwrap(), DATA);

        std::fs::remove_dir(&backup).unwrap();
        manager.tick(true);
        for _ in 0..500 {
            if !files.completion.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!files.completion.exists());
        let saved: Vec<Record> = store::load_json(&history).unwrap().unwrap();
        assert_eq!(saved[0].state, State::Completed);
        assert_eq!(saved[0].size, Some(DATA.len() as u64));
        assert!(saved[0].headers.is_empty());
        assert_eq!(std::fs::read(path).unwrap(), DATA);
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn failed_removal_retains_paused_data_and_durable_record() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/paused.zip".into(),
                file_name: Some("paused.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let path = manager.path_of(id).unwrap().0;
        let files = sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir));
        std::fs::create_dir_all(files.directory().unwrap()).unwrap();
        std::fs::write(&files.data, b"keep partial data").unwrap();
        let history = fixture.dir.path().join("downloads.json");
        let previous = std::fs::read(&history).unwrap();
        let backup = with_suffix(&history, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        manager.remove(&[id], false);
        assert_eq!(manager.get(id).unwrap().state, State::Paused);
        assert!(
            manager
                .get(id)
                .unwrap()
                .error
                .unwrap()
                .contains("files were kept")
        );
        assert_eq!(std::fs::read(&files.data).unwrap(), b"keep partial data");
        assert_eq!(std::fs::read(history).unwrap(), previous);
        manager.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removal_waits_for_media_cleanup_and_reserves_the_old_path() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let mut request = media_request("http://example.com/video", "same-title");
        request.start = false;
        let old = manager.add_media(request.clone()).unwrap();
        let old_path = manager.path_of(old).unwrap().0;
        let old_files = sdm_core::PartialFiles::new(&old_path, Some(&manager.settings().temp_dir));
        std::fs::create_dir_all(old_files.directory().unwrap()).unwrap();
        std::fs::write(old_files.directory().unwrap().join("leftover"), b"old").unwrap();
        // A user may already have moved the final media file. The retained row
        // must reserve its name until the scheduled cleanup has finished.
        {
            let mut inner = manager.lock();
            inner.records[0].finish(Ok(Outcome::Completed {
                path: old_path.clone(),
                size: 3,
            }));
            inner.cleanup_pending.insert(old);
            inner.dirty = true;
            manager.save(&mut inner);
        }
        manager.retry_cleanup();
        assert!(manager.lock().cleanup_running.contains(&old));
        manager.remove(&[old], false);
        assert!(manager.get(old).unwrap().stopping);
        let new = manager.add_media(request).unwrap();
        let new_path = manager.path_of(new).unwrap().0;
        assert_ne!(
            new_path, old_path,
            "cleanup must retain the old path reservation"
        );
        let new_files = sdm_core::PartialFiles::new(&new_path, Some(&manager.settings().temp_dir));
        std::fs::create_dir_all(new_files.directory().unwrap()).unwrap();
        let marker = new_files.directory().unwrap().join("new-partial");
        std::fs::write(&marker, b"new").unwrap();
        for _ in 0..500 {
            if manager.get(old).is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(manager.get(old).is_none());
        assert_eq!(manager.get(new).unwrap().state, State::Paused);
        assert_eq!(std::fs::read(marker).unwrap(), b"new");
        assert!(!old_files.directory().unwrap().exists());
        let saved: Vec<Record> = store::load_json(&fixture.dir.path().join("downloads.json"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.iter().map(|r| r.id).collect::<Vec<_>>(), vec![new]);
        manager.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_active_removal_pauses_and_keeps_partial_data_until_saved() {
        let fixture = Fixture::new();
        let (base, _) = big_file_server().await;
        let manager = fixture.manager();
        let mut slow = manager.settings();
        slow.speed_limit = 256 * 1024;
        manager.update_settings(slow).unwrap();
        let id = manager
            .add(request(format!("{base}/active.bin"), true))
            .await
            .unwrap();
        for _ in 0..500 {
            if manager.get(id).unwrap().downloaded > 100 * 1024 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(manager.get(id).unwrap().downloaded > 100 * 1024);
        let path = manager.path_of(id).unwrap().0;
        let files = sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir));
        let history = fixture.dir.path().join("downloads.json");
        let previous = std::fs::read(&history).unwrap();
        let backup = with_suffix(&history, ".bak");
        std::fs::remove_file(&backup).unwrap();
        std::fs::create_dir(&backup).unwrap();
        manager.remove(&[id], false);
        let paused = wait_for(&manager, id, State::Paused).await;
        assert!(paused.error.unwrap().contains("files were kept"));
        assert!(paused.downloaded > 100 * 1024);
        assert!(
            files.data.exists(),
            "pause must retain partial bytes before failed row removal"
        );
        assert!(files.state.exists(), "pause must retain resumable progress");
        assert!(!path.exists());
        assert_eq!(std::fs::read(&history).unwrap(), previous);
        std::fs::remove_dir(&backup).unwrap();
        manager.remove(&[id], false);
        assert!(manager.get(id).is_none());
        assert!(!files.directory().unwrap().exists());
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn malformed_completion_receipt_keeps_row_paused_and_preserves_files() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/paused.zip".into(),
                file_name: Some("paused.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let path = manager.path_of(id).unwrap().0;
        let files = sdm_core::PartialFiles::new(&path, Some(&manager.settings().temp_dir));
        manager.shutdown().await;
        drop(manager);
        std::fs::create_dir_all(files.directory().unwrap()).unwrap();
        std::fs::write(&files.completion, b"{broken receipt").unwrap();
        std::fs::write(&files.data, b"keep partial data").unwrap();
        let manager = fixture.manager();
        wait_for_migrations(&manager).await;
        assert_eq!(manager.get(id).unwrap().state, State::Paused);
        assert!(
            manager
                .get(id)
                .unwrap()
                .error
                .unwrap()
                .contains("verify completion")
        );
        assert_eq!(
            std::fs::read(&files.completion).unwrap(),
            b"{broken receipt"
        );
        assert_eq!(std::fs::read(&files.data).unwrap(), b"keep partial data");
        assert!(manager.lock().active.is_empty());
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn invalid_record_ids_recover_backup_or_report_preserved_data() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/kept.zip".into(),
                file_name: Some("kept.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        manager.shutdown().await;
        drop(manager);
        let path = fixture.dir.path().join("downloads.json");
        let records: Vec<Record> = store::load_json(&path).unwrap().unwrap();
        let bytes = serde_json::to_vec(&vec![records[0].clone(), records[0].clone()]).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let manager = fixture.manager();
        wait_for_migrations(&manager).await;
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.get(id).unwrap().file_name, "kept.zip");
        manager.shutdown().await;
        drop(manager);
        std::fs::remove_file(with_suffix(&path, ".bak")).unwrap();
        let mut exhausted = records[0].clone();
        exhausted.id = u64::MAX;
        let bytes = serde_json::to_vec(&vec![exhausted]).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let error = fixture
            .try_manager_with(Tools::new(fixture.dir.path(), None))
            .err()
            .unwrap();
        assert!(error.to_string().contains("invalid or duplicate record ID"));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn exhausted_record_ids_fail_without_overflow_or_partial_batch_adds() {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let id = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/kept.zip".into(),
                file_name: Some("kept.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap();
        let history = fixture.dir.path().join("downloads.json");
        let previous = std::fs::read(&history).unwrap();
        manager.lock().next_id = u64::MAX - 1;
        let error = manager
            .add_batch(BatchRequest {
                links: ["one", "two"]
                    .into_iter()
                    .map(|name| BatchLink {
                        url: format!("http://127.0.0.1:1/{name}.zip"),
                        ..BatchLink::default()
                    })
                    .collect(),
                ..BatchRequest::default()
            })
            .unwrap_err();
        assert!(error.contains("exhausted"));
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.get(id).unwrap().file_name, "kept.zip");
        assert_eq!(manager.lock().next_id, u64::MAX - 1);
        assert_eq!(std::fs::read(&history).unwrap(), previous);

        manager.shutdown().await;
        drop(manager);
        let mut records: Vec<Record> = store::load_json(&history).unwrap().unwrap();
        records[0].id = u64::MAX - 1;
        store::save_json(&history, &records).unwrap();
        let previous = std::fs::read(&history).unwrap();
        let manager = fixture.manager();
        wait_for_migrations(&manager).await;
        let error = manager
            .add(AddRequest {
                url: "http://127.0.0.1:1/overflow.zip".into(),
                file_name: Some("overflow.zip".into()),
                ..AddRequest::default()
            })
            .await
            .unwrap_err();
        assert!(error.contains("exhausted"));
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.lock().next_id, u64::MAX);
        assert_eq!(std::fs::read(history).unwrap(), previous);
        manager.shutdown().await;
    }

    #[test]
    fn names_from_urls() {
        let url =
            url::Url::parse("https://example.com/a/T%E1%BA%A3i%20v%E1%BB%81.zip?x=1").unwrap();
        assert_eq!(name_from_url(&url), "Tải về.zip");
        let url = url::Url::parse("https://example.com/").unwrap();
        assert_eq!(name_from_url(&url), "download");
    }
}
