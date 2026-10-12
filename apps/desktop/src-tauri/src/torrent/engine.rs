//! The Torrent session and the torrent downloads running in it.
//!
//! The session starts the first time a torrent is needed. librqbit keeps
//! its own list of torrents with their progress (`fastresume`), so a paused
//! torrent continues without checking all its data again. SDM's download
//! list stays in charge: torrents only run while their download runs, or
//! while they share a finished download with others (seeding).

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use librqbit::dht::DhtPersistenceConfig;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig,
    ListenerOptions, ManagedTorrent, Session, SessionOptions, SessionPersistenceConfig,
    TorrentStats, TorrentStatsState,
};
use sdm_core::{Downloader, Outcome, Phase, Progress, SegmentProgress};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::{MAX_TORRENT_FILE, SEED_FOREVER, TorrentInfo, TorrentSource};

type Handle = Arc<ManagedTorrent>;

/// How long to look for the peers of a magnet link before giving up.
const MAGNET_TIMEOUT: Duration = Duration::from_secs(180);
/// The most segments shown for a torrent; neighbouring files are grouped.
const MAX_SEGMENTS: usize = 64;

/// Settings of the Torrent session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineConfig {
    /// TCP port for incoming connections; `0` picks one and keeps it.
    pub port: u16,
    pub download_limit: Option<u64>,
    pub upload_limit: Option<u64>,
    /// Empty for none, or a proxy URL; only SOCKS5 proxies carry
    /// Torrent traffic.
    pub proxy: String,
    /// Share a finished torrent until this many percent of its size were
    /// uploaded: `0` for not at all, [`SEED_FOREVER`] until SDM quits.
    pub seed_ratio: u16,
    /// For tests: no DHT, no local peer discovery, loopback only.
    pub local_only: bool,
}

/// What a torrent is doing besides downloading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TorrentLive {
    /// Bytes per second sent to other peers.
    pub upload_speed: u64,
    /// Bytes sent to other peers since the torrent started.
    pub uploaded: u64,
    /// Sharing a finished download with others.
    pub seeding: bool,
    /// Checking data already on disk.
    pub checking: bool,
}

/// Everything a torrent download needs.
#[derive(Debug, Clone)]
pub struct JobConfig {
    pub info_hash: String,
    /// Files to download; all when missing.
    pub files: Option<Vec<usize>>,
    /// Sizes of the files to download, by index, in torrent order.
    pub sizes: Vec<(usize, u64)>,
    /// The folder the files are written into.
    pub folder: PathBuf,
    /// The file or folder shown in the download list.
    pub path: PathBuf,
}

struct Known {
    info: TorrentInfo,
    /// The magnet link or `.torrent` address it came from.
    link: String,
    /// Peers met while reading a magnet link, to start with.
    peers: Vec<SocketAddr>,
}

struct Seeding {
    handle: Handle,
    /// Bytes uploaded before seeding started.
    base: u64,
    /// Stop after uploading this many bytes; `None` for never.
    target: Option<u64>,
}

pub struct TorrentEngine {
    dir: PathBuf,
    session: RwLock<Option<Arc<Session>>>,
    starting: tokio::sync::Mutex<()>,
    config: Mutex<EngineConfig>,
    known: Mutex<HashMap<String, Known>>,
    seeding: Mutex<HashMap<String, Seeding>>,
    /// Torrents a download asked for; others are kept paused.
    claimed: Mutex<HashSet<String>>,
    cancel_inspect: Notify,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TorrentEngine {
    /// `dir` holds the session state and the `.torrent` files.
    pub fn new(dir: PathBuf, config: EngineConfig) -> Arc<Self> {
        Arc::new(Self {
            dir,
            session: RwLock::new(None),
            starting: tokio::sync::Mutex::new(()),
            config: Mutex::new(config),
            known: Mutex::new(HashMap::new()),
            seeding: Mutex::new(HashMap::new()),
            claimed: Mutex::new(HashSet::new()),
            cancel_inspect: Notify::new(),
        })
    }

    pub fn config(&self) -> EngineConfig {
        lock(&self.config).clone()
    }

    /// Applies new settings. Speed limits apply right away; the port and
    /// proxy at the next start.
    pub fn set_config(&self, config: EngineConfig) {
        if let Some(session) = self.current() {
            session
                .ratelimits
                .set_download_bps(bps(config.download_limit));
            session.ratelimits.set_upload_bps(bps(config.upload_limit));
        }
        *lock(&self.config) = config;
    }

    fn current(&self) -> Option<Arc<Session>> {
        self.session
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The running session, started on first use.
    pub async fn session(self: &Arc<Self>) -> Result<Arc<Session>, String> {
        if let Some(session) = self.current() {
            return Ok(session);
        }
        let _starting = self.starting.lock().await;
        if let Some(session) = self.current() {
            return Ok(session);
        }
        let config = self.config();
        let port = self.port(&config);
        let session = match create(&self.dir, &config, port).await {
            Err(error) if port != 0 => {
                tracing::warn!(%error, port, "Torrent port unavailable, using another one");
                create(&self.dir, &config, 0).await
            }
            result => result,
        }
        .map_err(|error| format!("Torrent could not start: {error:#}"))?;
        tracing::info!(addr = ?session.listen_addr(), "Torrent session started");
        *self.session.write().unwrap_or_else(PoisonError::into_inner) = Some(session.clone());

        // Torrents restored from the last run wait until their download
        // asks for them.
        let restored: Vec<Handle> =
            session.with_torrents(|torrents| torrents.map(|(_, t)| t.clone()).collect());
        for handle in restored {
            let engine = Arc::clone(self);
            let session = session.clone();
            tokio::spawn(async move {
                let _ = handle.wait_until_initialized().await;
                let hash = handle.info_hash().as_string();
                if !lock(&engine.claimed).contains(&hash) && !handle.is_paused() {
                    let _ = session.pause(&handle).await;
                }
            });
        }
        tokio::spawn(watch_seeding(Arc::downgrade(self)));
        Ok(session)
    }

    /// The port to listen on: the configured one, or one picked once and
    /// remembered.
    fn port(&self, config: &EngineConfig) -> u16 {
        if config.port != 0 || config.local_only {
            return config.port;
        }
        let file = self.dir.join("port");
        if let Some(port) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|&p| p >= 1024)
        {
            return port;
        }
        let port = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
            .and_then(|l| l.local_addr())
            .map_or(0, |a| a.port());
        if port != 0 {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(file, port.to_string());
        }
        port
    }

    /// Where other peers can reach this session.
    pub fn listen_addr(&self) -> Option<SocketAddr> {
        self.current()?.listen_addr()
    }

    pub fn torrent_file(&self, info_hash: &str) -> PathBuf {
        self.dir
            .join("torrents")
            .join(format!("{info_hash}.torrent"))
    }

    /// Reads a torrent from a magnet link, a `.torrent` address or a file,
    /// and remembers it for [`Self::known`].
    pub async fn inspect(
        self: &Arc<Self>,
        source: &TorrentSource,
        downloader: &Downloader,
    ) -> Result<TorrentInfo, String> {
        let work = self.read_source(source, downloader);
        tokio::select! {
            result = work => result,
            () = self.cancel_inspect.notified() => Err("cancelled".into()),
        }
    }

    async fn read_source(
        self: &Arc<Self>,
        source: &TorrentSource,
        downloader: &Downloader,
    ) -> Result<TorrentInfo, String> {
        let link = source.link.as_deref().map(str::trim).unwrap_or_default();
        let (bytes, peers, link) = if let Some(path) = &source.path {
            let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
            if size > MAX_TORRENT_FILE {
                return Err("this file is too big to be a .torrent file".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            (bytes, Vec::new(), String::new())
        } else if super::is_magnet(link) {
            let (bytes, peers) = self.resolve_magnet(link).await?;
            (bytes, peers, link.to_owned())
        } else if link.starts_with("http://") || link.starts_with("https://") {
            let fetched = downloader
                .fetch(link, &source.headers, MAX_TORRENT_FILE)
                .await
                .map_err(|e| e.to_string())?;
            (fetched.data, Vec::new(), link.to_owned())
        } else {
            return Err("this is neither a magnet link nor a .torrent file".into());
        };

        let info = super::parse(&bytes)?;
        let file = self.torrent_file(&info.info_hash);
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
        let link = if link.is_empty() {
            super::magnet_link(&info.info_hash, &info.name)
        } else {
            link
        };
        lock(&self.known).insert(
            info.info_hash.clone(),
            Known {
                info: info.clone(),
                link,
                peers,
            },
        );
        Ok(info)
    }

    /// Asks the swarm for the `.torrent` data of a magnet link.
    async fn resolve_magnet(
        self: &Arc<Self>,
        link: &str,
    ) -> Result<(Vec<u8>, Vec<SocketAddr>), String> {
        if let Some(hash) = super::magnet_info_hash(link)
            && let Ok(bytes) = std::fs::read(self.torrent_file(&hash))
        {
            return Ok((bytes, Vec::new()));
        }
        let session = self.session().await?;
        let options = AddTorrentOptions {
            list_only: true,
            ..AddTorrentOptions::default()
        };
        let added = tokio::time::timeout(
            MAGNET_TIMEOUT,
            session.add_torrent(AddTorrent::from_url(link), Some(options)),
        )
        .await
        .map_err(|_| "no one sharing this torrent was found".to_owned())?
        .map_err(|e| format!("{e:#}"))?;
        match added {
            AddTorrentResponse::ListOnly(list) => {
                Ok((list.torrent_bytes.to_vec(), list.seen_peers))
            }
            AddTorrentResponse::AlreadyManaged(_, handle) => {
                let bytes = handle
                    .with_metadata(|m| m.torrent_bytes.to_vec())
                    .map_err(|e| format!("{e:#}"))?;
                Ok((bytes, Vec::new()))
            }
            AddTorrentResponse::Added(..) => Err("unexpected answer from Torrent".into()),
        }
    }

    /// Stops reading magnet links (the user closed the dialog).
    pub fn cancel_inspections(&self) {
        self.cancel_inspect.notify_waiters();
    }

    /// A torrent read before, with the link it came from.
    pub fn known(&self, info_hash: &str) -> Option<(TorrentInfo, String)> {
        if let Some(known) = lock(&self.known).get(info_hash) {
            return Some((known.info.clone(), known.link.clone()));
        }
        let bytes = std::fs::read(self.torrent_file(info_hash)).ok()?;
        let info = super::parse(&bytes).ok()?;
        let link = super::magnet_link(&info.info_hash, &info.name);
        Some((info, link))
    }

    /// Starts or resumes a torrent download. Must be called inside a Tokio
    /// runtime.
    pub fn start(self: &Arc<Self>, config: JobConfig) -> TorrentJob {
        let control = TorrentControl(Arc::new(ControlShared {
            progress: Mutex::new(Progress {
                phase: Phase::Connecting,
                total: Some(config.sizes.iter().map(|s| s.1).sum()),
                resumable: true,
                ..Progress::default()
            }),
            live: Mutex::new(TorrentLive::default()),
            stop: AtomicU8::new(RUNNING),
            wake: Notify::new(),
        }));
        lock(&self.claimed).insert(config.info_hash.clone());
        let task = tokio::spawn(run(Arc::clone(self), config, control.clone()));
        TorrentJob { control, task }
    }

    async fn add_or_resume(
        &self,
        session: &Arc<Session>,
        config: &JobConfig,
    ) -> Result<Handle, String> {
        let id: librqbit::dht::Id20 = config
            .info_hash
            .parse()
            .map_err(|_| "broken torrent info hash".to_owned())?;
        if let Some(handle) = session.get(id.into()) {
            if handle.only_files() != config.files {
                let wanted: HashSet<usize> = match &config.files {
                    Some(files) => files.iter().copied().collect(),
                    None => handle
                        .with_metadata(|m| (0..m.file_infos.len()).collect())
                        .map_err(|e| format!("{e:#}"))?,
                };
                session
                    .update_only_files(&handle, &wanted)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
            }
            if handle.is_paused() {
                session
                    .unpause(&handle)
                    .await
                    .map_err(|e| format!("{e:#}"))?;
            }
            return Ok(handle);
        }
        let bytes = std::fs::read(self.torrent_file(&config.info_hash))
            .map_err(|_| "the .torrent data of this download is missing".to_owned())?;
        let peers = lock(&self.known)
            .get(&config.info_hash)
            .map(|k| k.peers.clone())
            .filter(|p| !p.is_empty());
        let options = AddTorrentOptions {
            only_files: config.files.clone(),
            overwrite: true,
            output_folder: Some(config.folder.to_string_lossy().into_owned()),
            initial_peers: peers,
            ..AddTorrentOptions::default()
        };
        match session
            .add_torrent(AddTorrent::from_bytes(bytes), Some(options))
            .await
            .map_err(|e| format!("{e:#}"))?
        {
            AddTorrentResponse::Added(_, handle)
            | AddTorrentResponse::AlreadyManaged(_, handle) => Ok(handle),
            AddTorrentResponse::ListOnly(_) => Err("unexpected answer from Torrent".into()),
        }
    }

    /// A finished torrent: share it with others for a while, or stop it.
    async fn finished(&self, session: &Arc<Session>, handle: &Handle, size: u64) {
        let ratio = self.config().seed_ratio;
        if ratio == 0 {
            let _ = session.pause(handle).await;
            return;
        }
        let target = (ratio != SEED_FOREVER).then(|| size.saturating_mul(ratio.into()) / 100);
        lock(&self.seeding).insert(
            handle.info_hash().as_string(),
            Seeding {
                handle: handle.clone(),
                base: handle.stats().uploaded_bytes,
                target,
            },
        );
    }

    /// Upload statistics of a finished torrent that is being shared.
    pub fn seeding(&self, info_hash: &str) -> Option<TorrentLive> {
        let seeding = lock(&self.seeding);
        let entry = seeding.get(info_hash)?;
        let stats = entry.handle.stats();
        Some(TorrentLive {
            upload_speed: stats
                .live
                .as_ref()
                .map_or(0, |l| speed(l.upload_speed.mbps)),
            uploaded: stats.uploaded_bytes.saturating_sub(entry.base),
            seeding: true,
            checking: false,
        })
    }

    /// Stops sharing a finished torrent.
    pub async fn stop_seeding(&self, info_hash: &str) {
        let entry = lock(&self.seeding).remove(info_hash);
        if let (Some(entry), Some(session)) = (entry, self.current()) {
            let _ = session.pause(&entry.handle).await;
        }
    }

    /// Removes a torrent from the session, deleting its files when
    /// `delete_files` is set, and forgets its `.torrent` data.
    pub async fn forget(self: &Arc<Self>, info_hash: &str, delete_files: bool) {
        lock(&self.seeding).remove(info_hash);
        lock(&self.claimed).remove(info_hash);
        lock(&self.known).remove(info_hash);
        if let Ok(id) = info_hash.parse::<librqbit::dht::Id20>() {
            let session = match self.current() {
                Some(session) => Ok(session),
                // Its progress may still be in the saved session.
                None if self.dir.join("session").exists() => self.session().await,
                None => Err(String::new()),
            };
            if let Ok(session) = session
                && session.get(id.into()).is_some()
                && let Err(error) = session.delete(id.into(), delete_files).await
            {
                tracing::warn!(%error, info_hash, "could not remove a torrent");
            }
        }
        let _ = std::fs::remove_file(self.torrent_file(info_hash));
    }

    /// Pauses every torrent so the next start finds them paused, and
    /// stops the session.
    pub async fn shutdown(&self) {
        lock(&self.seeding).clear();
        let Some(session) = self.current() else {
            return;
        };
        let handles: Vec<Handle> =
            session.with_torrents(|torrents| torrents.map(|(_, t)| t.clone()).collect());
        for handle in handles {
            if !handle.is_paused() {
                let _ = session.pause(&handle).await;
            }
        }
        session.stop().await;
        *self.session.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

async fn create(dir: &Path, config: &EngineConfig, port: u16) -> anyhow::Result<Arc<Session>> {
    std::fs::create_dir_all(dir)?;
    let socks = config
        .proxy
        .starts_with("socks5")
        .then(|| config.proxy.clone());
    let (listen_addr, announce_port): (SocketAddr, _) = if config.local_only {
        // librqbit does not tell trackers about loopback ports by itself.
        let port = match port {
            0 => std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
                .local_addr()?
                .port(),
            port => port,
        };
        ((Ipv4Addr::LOCALHOST, port).into(), Some(port))
    } else if ipv6_available() {
        ((Ipv6Addr::UNSPECIFIED, port).into(), None)
    } else {
        ((Ipv4Addr::UNSPECIFIED, port).into(), None)
    };
    let ipv4_only = config.local_only || listen_addr.is_ipv4();
    let options = SessionOptions {
        dht: (!config.local_only).then(|| DhtSessionConfig {
            persistence: Some(DhtPersistenceConfig {
                config_filename: Some(dir.join("dht.json")),
                ..DhtPersistenceConfig::default()
            }),
            ..DhtSessionConfig::default()
        }),
        fastresume: true,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(dir.join("session")),
        }),
        listen: Some(ListenerOptions {
            listen_addr,
            announce_port,
            enable_upnp_port_forwarding: !config.local_only,
            ipv4_only,
            ..ListenerOptions::default()
        }),
        connect: socks.map(|proxy_url| ConnectionOptions {
            proxy_url: Some(proxy_url),
            ..ConnectionOptions::default()
        }),
        ratelimits: LimitsConfig {
            download_bps: bps(config.download_limit),
            upload_bps: bps(config.upload_limit),
        },
        disable_local_service_discovery: config.local_only,
        ipv4_only,
        client_name_and_version: Some(format!("SupoDownloadManager/{}", env!("CARGO_PKG_VERSION"))),
        ..SessionOptions::default()
    };
    Session::new_with_opts(dir.join("downloads"), options).await
}

/// Some computers have IPv6 switched off completely.
fn ipv6_available() -> bool {
    std::net::UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).is_ok()
}

fn bps(limit: Option<u64>) -> Option<NonZeroU32> {
    limit.and_then(|l| NonZeroU32::new(u32::try_from(l).unwrap_or(u32::MAX)))
}

/// librqbit reports speeds in MiB/s.
fn speed(mib_per_second: f64) -> u64 {
    (mib_per_second * 1024.0 * 1024.0).max(0.0) as u64
}

/// Stops sharing finished torrents once they uploaded enough.
async fn watch_seeding(engine: std::sync::Weak<TorrentEngine>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;
        let Some(engine) = engine.upgrade() else {
            return;
        };
        let Some(session) = engine.current() else {
            return;
        };
        let done: Vec<(String, Handle)> = lock(&engine.seeding)
            .iter()
            .filter(|(_, s)| {
                let stats = s.handle.stats();
                !matches!(stats.state, TorrentStatsState::Live)
                    || s.target
                        .is_some_and(|t| stats.uploaded_bytes.saturating_sub(s.base) >= t)
            })
            .map(|(hash, s)| (hash.clone(), s.handle.clone()))
            .collect();
        for (hash, handle) in done {
            lock(&engine.seeding).remove(&hash);
            if !handle.is_paused() {
                let _ = session.pause(&handle).await;
            }
        }
    }
}

const RUNNING: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

struct ControlShared {
    progress: Mutex<Progress>,
    live: Mutex<TorrentLive>,
    stop: AtomicU8,
    wake: Notify,
}

/// A remote control for a torrent download, like `sdm_core::DownloadControl`.
#[derive(Clone)]
pub struct TorrentControl(Arc<ControlShared>);

impl TorrentControl {
    pub fn progress(&self) -> Progress {
        lock(&self.0.progress).clone()
    }

    pub fn live(&self) -> TorrentLive {
        *lock(&self.0.live)
    }

    pub fn pause(&self) {
        self.stop(PAUSE);
    }

    /// Stops the download; its data is deleted by [`TorrentEngine::forget`].
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
}

pub struct TorrentJob {
    control: TorrentControl,
    task: JoinHandle<Result<Outcome, String>>,
}

impl TorrentJob {
    pub fn control(&self) -> TorrentControl {
        self.control.clone()
    }

    pub async fn wait(self) -> Result<Outcome, String> {
        self.task
            .await
            .unwrap_or_else(|e| Err(format!("the download crashed: {e}")))
    }
}

async fn run(
    engine: Arc<TorrentEngine>,
    config: JobConfig,
    control: TorrentControl,
) -> Result<Outcome, String> {
    let session = engine.session().await?;
    let handle = engine.add_or_resume(&session, &config).await?;
    let mut resumed_from = None;
    loop {
        match control.0.stop.load(Ordering::SeqCst) {
            RUNNING => {}
            how => {
                if !handle.is_paused() {
                    let _ = session.pause(&handle).await;
                }
                return Ok(if how == PAUSE {
                    Outcome::Paused {
                        path: Some(config.path.clone()),
                        resumable: true,
                    }
                } else {
                    Outcome::Cancelled
                });
            }
        }

        let stats = handle.stats();
        if matches!(stats.state, TorrentStatsState::Error) {
            return Err(stats
                .error
                .unwrap_or_else(|| "Torrent stopped with an error".into()));
        }
        let checking = matches!(stats.state, TorrentStatsState::Initializing { .. });
        let mut now = progress(&stats, &config, resumed_from.unwrap_or(0));
        if !checking && resumed_from.is_none() {
            resumed_from = Some(now.downloaded);
            now.resumed_from = now.downloaded;
        }
        *lock(&control.0.progress) = now;
        *lock(&control.0.live) = TorrentLive {
            upload_speed: stats
                .live
                .as_ref()
                .map_or(0, |l| speed(l.upload_speed.mbps)),
            uploaded: stats.uploaded_bytes,
            seeding: false,
            checking,
        };
        if !checking && stats.finished {
            // librqbit counts whole pieces; the files chosen are what count.
            let size = config.sizes.iter().map(|s| s.1).sum();
            engine.finished(&session, &handle, size).await;
            return Ok(Outcome::Completed {
                path: config.path.clone(),
                size,
            });
        }
        if matches!(stats.state, TorrentStatsState::Paused) {
            // Paused from somewhere else (e.g. the session is stopping).
            let _ = session.unpause(&handle).await;
        }

        tokio::select! {
            () = control.0.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }
}

fn progress(stats: &TorrentStats, config: &JobConfig, resumed_from: u64) -> Progress {
    let live = stats.live.as_ref();
    let checking = matches!(stats.state, TorrentStatsState::Initializing { .. });
    let total: u64 = config.sizes.iter().map(|s| s.1).sum();
    // librqbit counts whole pieces, which may stick out of the chosen files.
    let downloaded = if checking {
        0
    } else if stats.file_progress.is_empty() {
        stats.progress_bytes.min(total)
    } else {
        config
            .sizes
            .iter()
            .map(|&(index, size)| {
                stats
                    .file_progress
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .min(size)
            })
            .sum()
    };
    Progress {
        phase: if checking {
            Phase::Connecting
        } else {
            Phase::Downloading
        },
        file_name: None,
        path: Some(config.path.clone()),
        total: Some(total),
        downloaded,
        speed: live.map_or(0, |l| speed(l.download_speed.mbps)),
        connections: live.map_or(0, |l| l.snapshot.peer_stats.live as usize),
        resumable: true,
        segments: segments(&config.sizes, &stats.file_progress),
        resumed_from,
        error: stats.error.clone(),
    }
}

/// One segment per file, or per group of neighbouring files when there are
/// many, so the download list can draw what is done.
fn segments(sizes: &[(usize, u64)], have: &[u64]) -> Vec<SegmentProgress> {
    let total: u64 = sizes.iter().map(|s| s.1).sum();
    if total == 0 {
        return Vec::new();
    }
    let group = if sizes.len() > MAX_SEGMENTS {
        total.div_ceil(MAX_SEGMENTS as u64)
    } else {
        0
    };
    let mut out: Vec<SegmentProgress> = Vec::new();
    let mut offset = 0;
    for &(index, size) in sizes {
        let done = have.get(index).copied().unwrap_or(0).min(size);
        match out.last_mut() {
            Some(last) if last.end - last.start < group => {
                last.end += size;
                last.downloaded += done;
            }
            _ => out.push(SegmentProgress {
                start: offset,
                end: offset + size,
                downloaded: done,
            }),
        }
        offset += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::torrent::testing::{data, make_torrent, tracker};
    use crate::torrent::{TorrentSource, magnet_link};

    fn engine(dir: &Path, seed_ratio: u16, limit: Option<u64>) -> Arc<TorrentEngine> {
        TorrentEngine::new(
            dir.to_path_buf(),
            EngineConfig {
                local_only: true,
                seed_ratio,
                download_limit: limit,
                ..EngineConfig::default()
            },
        )
    }

    fn job(info: &TorrentInfo, files: Option<Vec<usize>>, folder: &Path) -> JobConfig {
        JobConfig {
            info_hash: info.info_hash.clone(),
            sizes: info
                .files
                .iter()
                .filter(|f| files.as_ref().is_none_or(|s| s.contains(&f.index)))
                .map(|f| (f.index, f.size))
                .collect(),
            files,
            folder: folder.to_path_buf(),
            path: folder.to_path_buf(),
        }
    }

    async fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !check() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// One session shares a folder; others download it from a magnet link
    /// and a .torrent file, pause, restart and continue.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn shares_and_downloads_torrents() {
        let root = tempfile::tempdir().unwrap();
        let shared = root.path().join("shared/Pack");
        std::fs::create_dir_all(shared.join("sub")).unwrap();
        let a = data(1_500_000, 1);
        let b = data(700_000, 2);
        std::fs::write(shared.join("a.bin"), &a).unwrap();
        std::fs::write(shared.join("sub/b.bin"), &b).unwrap();
        let tracker = tracker().await;
        let torrent = root.path().join("Pack.torrent");
        std::fs::write(&torrent, make_torrent(&shared, &tracker).await).unwrap();
        let downloader = Downloader::new(sdm_core::ClientConfig::default()).unwrap();
        let from_file = TorrentSource {
            path: Some(torrent),
            ..TorrentSource::default()
        };

        // The seeder checks the data it has and starts sharing it.
        let seeder = engine(&root.path().join("seeder"), SEED_FOREVER, None);
        let info = seeder.inspect(&from_file, &downloader).await.unwrap();
        assert_eq!(info.name, "Pack");
        assert_eq!(info.files.len(), 2);
        let outcome = seeder.start(job(&info, None, &shared)).wait().await;
        assert!(
            matches!(
                outcome,
                Ok(Outcome::Completed {
                    size: 2_200_000,
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert!(seeder.seeding(&info.info_hash).is_some());
        assert!(seeder.listen_addr().is_some());

        // A magnet link: the .torrent data comes from the seeder, found
        // through the tracker. Only one file is wanted.
        let magnet = engine(&root.path().join("magnet"), 0, None);
        let tracker_param: String =
            url::form_urlencoded::byte_serialize(tracker.as_bytes()).collect();
        let link = TorrentSource {
            link: Some(format!(
                "{}&tr={tracker_param}",
                magnet_link(&info.info_hash, "Pack")
            )),
            ..TorrentSource::default()
        };
        let resolved = magnet.inspect(&link, &downloader).await.unwrap();
        assert_eq!(resolved.files, info.files);
        let b_index = resolved
            .files
            .iter()
            .find(|f| f.path == "sub/b.bin")
            .unwrap()
            .index;
        let target = root.path().join("magnet-downloads/Pack");
        let outcome = magnet
            .start(job(&resolved, Some(vec![b_index]), &target))
            .wait()
            .await;
        assert!(
            matches!(outcome, Ok(Outcome::Completed { size: 700_000, .. })),
            "{outcome:?}"
        );
        assert_eq!(std::fs::read(target.join("sub/b.bin")).unwrap(), b);
        magnet.shutdown().await;

        // A slow download is paused, SDM restarts, and it continues.
        let dir = root.path().join("leecher");
        let target = root.path().join("downloads/Pack");
        let leecher = engine(&dir, 0, Some(300_000));
        let info = leecher.inspect(&from_file, &downloader).await.unwrap();
        let first = leecher.start(job(&info, None, &target));
        let control = first.control();
        wait_until("some progress", || control.progress().downloaded > 200_000).await;
        control.pause();
        let paused = first.wait().await;
        assert!(matches!(paused, Ok(Outcome::Paused { .. })), "{paused:?}");
        leecher.shutdown().await;

        let leecher = engine(&dir, 0, None);
        let second = leecher.start(job(&info, None, &target));
        let control = second.control();
        let done = tokio::time::timeout(Duration::from_secs(60), second.wait())
            .await
            .expect("download did not finish");
        assert!(
            matches!(
                done,
                Ok(Outcome::Completed {
                    size: 2_200_000,
                    ..
                })
            ),
            "{done:?}"
        );
        assert!(
            control.progress().resumed_from > 0,
            "the restarted download began from scratch"
        );
        assert_eq!(std::fs::read(target.join("a.bin")).unwrap(), a);
        assert_eq!(std::fs::read(target.join("sub/b.bin")).unwrap(), b);

        // Removing a torrent with its data deletes the files.
        leecher.forget(&info.info_hash, true).await;
        assert!(!target.join("a.bin").exists());
        assert!(!leecher.torrent_file(&info.info_hash).exists());
        leecher.shutdown().await;
        seeder.shutdown().await;
    }

    #[test]
    fn groups_many_files_into_segments() {
        let few = segments(&[(0, 100), (2, 50)], &[100, 7, 20]);
        assert_eq!(
            few,
            vec![
                SegmentProgress {
                    start: 0,
                    end: 100,
                    downloaded: 100
                },
                SegmentProgress {
                    start: 100,
                    end: 150,
                    downloaded: 20
                },
            ]
        );

        let sizes: Vec<(usize, u64)> = (0..1000).map(|i| (i, 10)).collect();
        let have = vec![10; 1000];
        let many = segments(&sizes, &have);
        assert!(many.len() <= MAX_SEGMENTS, "{}", many.len());
        assert_eq!(many.last().unwrap().end, 10_000);
        assert_eq!(many.iter().map(|s| s.downloaded).sum::<u64>(), 10_000);
        assert!(segments(&[], &[]).is_empty());
    }
}
