//! The download list: a queue of downloads that survives restarts.

use std::collections::HashMap;
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
    /// Set for BitTorrent downloads; `url` is the magnet link or `.torrent`
    /// address then, and `path` the file or folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentSpec>,
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
    /// A BitTorrent download; `connections` counts peers then.
    pub torrent: bool,
    /// Sharing a finished torrent with others.
    pub seeding: bool,
    /// Checking torrent data that is already on disk.
    pub checking: bool,
    /// Bytes per second sent to other peers.
    pub upload_speed: u64,
    /// Bytes sent to other peers.
    pub uploaded: u64,
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
}

pub type Notify = Box<dyn Fn(Notice) + Send + Sync>;

pub struct Manager {
    inner: Mutex<Inner>,
    store: PathBuf,
    tools: Arc<Tools>,
    torrents: Arc<TorrentEngine>,
    runtime: Handle,
    notify: Notify,
}

struct Inner {
    records: Vec<Record>,
    active: HashMap<u64, Active>,
    downloader: Downloader,
    settings: Settings,
    next_id: u64,
    dirty: bool,
    closing: bool,
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

    fn cancel(&self) {
        match self {
            Job::File(control) => control.cancel(),
            Job::Media(control) => control.cancel(),
            Job::Torrent(control) => control.cancel(),
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
        settings: Settings,
        tools: Arc<Tools>,
        torrents: Arc<TorrentEngine>,
        runtime: Handle,
        notify: Notify,
    ) -> anyhow::Result<Arc<Self>> {
        let downloader = Downloader::new(settings.client_config())?;
        let mut records: Vec<Record> = store::load_json(&store).unwrap_or_default();
        for record in &mut records {
            if matches!(record.state, State::Queued | State::Downloading) {
                record.state = State::Paused;
            }
        }
        let next_id = records.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                records,
                active: HashMap::new(),
                downloader,
                settings,
                next_id,
                dirty: false,
                closing: false,
            }),
            store,
            tools,
            torrents,
            runtime,
            notify,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn downloader(&self) -> Downloader {
        self.lock().downloader.clone()
    }

    /// Applies new settings to running and future downloads.
    pub fn update_settings(self: &Arc<Self>, settings: Settings) -> anyhow::Result<()> {
        {
            let mut inner = self.lock();
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
                    stopping: active.is_some_and(|a| a.stopping),
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
            let path = inner.free_path(&directory, &file_name);
            let id = inner.next_id;
            inner.next_id += 1;
            inner.records.push(Record {
                id,
                url,
                headers: request.headers,
                file_name: file_name_of(&path),
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
            });
            inner.dirty = true;
            self.save(&mut inner);
            id
        };
        self.schedule();
        Ok(id)
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
            let path = inner.free_path(&directory, &file_name);
            let id = inner.next_id;
            inner.next_id += 1;
            inner.records.push(Record {
                id,
                url: page.to_string(),
                headers: Vec::new(),
                file_name: file_name_of(&path),
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
            });
            inner.dirty = true;
            self.save(&mut inner);
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
            let id = inner.next_id;
            inner.next_id += 1;
            inner.records.push(Record {
                id,
                url: link,
                headers: Vec::new(),
                file_name: file_name_of(&path),
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
            });
            inner.dirty = true;
            self.save(&mut inner);
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

    /// Removes downloads from the list. Unfinished data is always deleted;
    /// finished files are moved to the trash only when `delete_file` is set.
    pub fn remove(&self, ids: &[u64], delete_file: bool) {
        let mut inner = self.lock();
        for &id in ids {
            if let Some(active) = inner.active.get_mut(&id) {
                active.remove = Some(delete_file);
                active.stopping = true;
                active.control.cancel();
                continue;
            }
            if let Some(index) = inner.records.iter().position(|r| r.id == id) {
                let record = inner.records.remove(index);
                self.discard(&record, delete_file);
                inner.dirty = true;
            }
        }
        self.save(&mut inner);
    }

    /// Copies live progress into the stored records and saves them now and
    /// then. Call this regularly.
    pub fn tick(&self, save: bool) {
        let mut inner = self.lock();
        inner.sync_progress();
        if save {
            self.save(&mut inner);
        }
    }

    /// Pauses everything and waits (up to a few seconds) until all progress
    /// is safely on disk. Used when the app quits.
    pub async fn shutdown(&self) {
        {
            let mut inner = self.lock();
            inner.closing = true;
            for active in inner.active.values_mut() {
                active.stopping = true;
                active.control.pause();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.lock().active.is_empty() && Instant::now() < deadline {
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

    /// Starts queued downloads while there are free slots.
    fn schedule(self: &Arc<Self>) {
        let started = {
            let mut inner = self.lock();
            if inner.closing {
                return;
            }
            let free = usize::from(inner.settings.max_parallel).saturating_sub(inner.active.len());
            let next: Vec<u64> = inner
                .records
                .iter()
                .filter(|r| r.state == State::Queued && !inner.active.contains_key(&r.id))
                .take(free)
                .map(|r| r.id)
                .collect();
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
        let (url, path, headers, media, torrent) = (
            record.url.clone(),
            record.path.clone(),
            record.headers.clone(),
            record.media.clone(),
            record.torrent.clone(),
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
                this.finished(id, result);
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
                url,
                spec,
                directory: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                stem: path
                    .file_stem()
                    .map_or_else(|| "video".into(), |s| s.to_string_lossy().into_owned()),
                work_dir: self.tools.work_dir(id),
                speed_limit: inner.downloader.speed_limit(),
                proxy: inner.settings.proxy.clone(),
            };
            let job = media::job::start(config);
            let control = job.control();
            self.runtime.spawn(async move {
                let result = job.wait().await;
                this.finished(id, result);
            });
            Job::Media(control)
        } else {
            let request = DownloadRequest {
                url,
                destination: Destination::File(path),
                headers,
                options: DownloadOptions {
                    connections: inner.settings.connections.into(),
                    ..DownloadOptions::default()
                },
            };
            let handle = inner.downloader.start(request);
            let control = handle.control();
            self.runtime.spawn(async move {
                let result = handle.wait().await.map_err(|e| e.to_string());
                this.finished(id, result);
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

    fn finished(self: &Arc<Self>, id: u64, result: Result<Outcome, String>) {
        let notice = {
            let mut inner = self.lock();
            let Some(active) = inner.active.remove(&id) else {
                return;
            };
            let progress = active.control.progress();
            let index = inner.records.iter().position(|r| r.id == id);
            let notice = match (index, active.remove) {
                (Some(index), Some(delete_file)) => {
                    let record = inner.records.remove(index);
                    self.discard(&record, delete_file);
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
            inner.dirty = true;
            self.save(&mut inner);
            notice
        };
        if let Some(notice) = notice {
            (self.notify)(notice);
        }
        self.schedule();
    }

    /// Deletes what is left of a download that leaves the list.
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
    }
}

/// Deletes what is left of a download that leaves the list.
fn discard_files(record: &Record, delete_file: bool, tools: &Tools) {
    if record.media.is_some() {
        let _ = std::fs::remove_dir_all(tools.work_dir(record.id));
    }
    for path in [
        sdm_core::partial_data_path(&record.path),
        sdm_core::partial_state_path(&record.path),
    ] {
        let _ = std::fs::remove_file(path);
    }
    if delete_file
        && record.state == State::Completed
        && record.path.exists()
        && let Err(error) = trash::delete(&record.path)
    {
        tracing::warn!(%error, path = %record.path.display(), "could not move the file to the trash");
    }
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
            self.manager_with(Tools::new(self.dir.path()))
        }

        fn manager_with(&self, tools: Tools) -> Arc<Manager> {
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
            .unwrap()
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
        let mut records: Vec<Record> = store::load_json(&store).unwrap();
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
        assert!(manager.list().is_empty());
        assert!(!sdm_core::partial_data_path(&path).exists());
        assert!(!sdm_core::partial_state_path(&path).exists());
        assert!(done.path.exists(), "finished files stay unless asked");
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
            sdm_core::partial_state_path(&path).exists(),
            "progress was saved"
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

    #[test]
    fn names_from_urls() {
        let url =
            url::Url::parse("https://example.com/a/T%E1%BA%A3i%20v%E1%BB%81.zip?x=1").unwrap();
        assert_eq!(name_from_url(&url), "Tải về.zip");
        let url = url::Url::parse("https://example.com/").unwrap();
        assert_eq!(name_from_url(&url), "download");
    }
}
