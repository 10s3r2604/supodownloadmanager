// Talks to the Rust side of the app. When the page runs in a normal browser
// (for design work or tests) a simulated backend is used instead.

import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

export type State = "queued" | "downloading" | "paused" | "completed" | "failed";
export type Category =
  | "video"
  | "music"
  | "document"
  | "compressed"
  | "program"
  | "image"
  | "other";
export type Engine = "chromium" | "firefox";
export type Skin = "neon" | "minimal";
export type ProgressStyle = "graph" | "ring" | "classic";
export type Headers = [string, string][];

export interface Segment {
  start: number;
  end: number;
  downloaded: number;
}

export interface Download {
  id: number;
  url: string;
  fileName: string;
  path: string;
  size: number | null;
  downloaded: number;
  resumable: boolean;
  category: Category;
  state: State;
  error: string | null;
  addedAt: number;
  completedAt: number | null;
  /** Milliseconds spent transferring data. */
  elapsed: number;
  speed: number;
  eta: number | null;
  connections: number;
  segments: Segment[];
  stopping: boolean;
  /** A video from a web page, downloaded with yt-dlp. */
  media: boolean;
  /** Video and audio are being merged. */
  processing: boolean;
  /** A Torrent download; `connections` counts peers then. */
  torrent: boolean;
  /** Sharing a finished torrent with others. */
  seeding: boolean;
  /** Checking torrent data already on disk. */
  checking: boolean;
  /** Bytes per second sent to other peers. */
  uploadSpeed: number;
  /** Bytes sent to other peers. */
  uploaded: number;
  /** The queue it waits in. */
  queue: string;
}

/** The queue every download is in unless another one is chosen. */
export const MAIN_QUEUE = "main";

export type PowerAction = "nothing" | "quit" | "sleep" | "hibernate" | "shutdown";

export interface Schedule {
  enabled: boolean;
  /** "HH:MM", local time. */
  start: string | null;
  stop: string | null;
  /** 0 = Monday … 6 = Sunday; empty = once. */
  days: number[];
}

export interface Queue {
  /** Empty for a new queue. */
  id: string;
  /** Empty for the main queue. */
  name: string;
  /** Downloads at the same time; the main queue uses the setting instead. */
  maxParallel: number;
  schedule: Schedule;
  whenDone: PowerAction;
}

/** A power action that happens at `at` unless cancelled. */
export interface PowerCountdown {
  action: PowerAction;
  /** The queue that finished, or null for all downloads. */
  queue: string | null;
  at: number;
}

export interface UpdateState {
  phase: "disabled" | "idle" | "checking" | "downloading" | "ready" | "waiting" | "installing" | "error";
  supported: boolean;
  currentVersion: string;
  version?: string | null;
  downloaded: number;
  total?: number | null;
  lastChecked?: number | null;
  error?: string | null;
}

export interface Settings {
  downloadDir: string;
  tempDir: string;
  categoryFolders: boolean;
  connections: number;
  maxParallel: number;
  speedLimit: number;
  proxy: string;
  language: "auto" | "en" | "vi";
  skin: Skin;
  theme: "system" | "light" | "dark";
  progressWindow: boolean;
  progressStyle: ProgressStyle;
  completeDialog: boolean;
  closeToTray: boolean;
  startWithWindows: boolean;
  autoUpdate: boolean;
  notifications: boolean;
  clipboardMonitor: boolean;
  browserIntegration: boolean;
  browserSetupCompleted: boolean;
  confirmBrowserDownloads: boolean;
  bridgePort: number;
  captureExtensions: string[];
  /** Share finished torrents until this many percent were uploaded; 0 = no, 65535 = until SDM quits. */
  seedRatio: number;
  /** Bytes per second, 0 = unlimited. */
  uploadLimit: number;
  /** 0 = picked automatically. */
  torrentPort: number;
}

/** Share finished torrents until SDM quits. */
export const SEED_FOREVER = 65535;

export interface ProbeResult {
  url: string;
  finalUrl: string;
  size: number | null;
  resumable: boolean;
  fileName: string;
  contentType: string | null;
  category: Category;
  directory: string;
}

export interface AddRequest {
  url: string;
  headers: Headers;
  fileName?: string;
  directory?: string;
  start: boolean;
  queue?: string;
}

/** One of many links added at once. */
export interface BatchLink {
  url: string;
  fileName?: string | null;
  /** Extra headers for this link, such as the cookies of its site. */
  headers?: Headers;
  /** The link's text on the page. */
  text?: string | null;
}

export interface BatchRequest {
  links: BatchLink[];
  headers?: Headers;
  directory?: string;
  start: boolean;
  queue?: string;
}

/** The links of a web page, sent by the browser extension. */
export interface LinksPrompt {
  pageUrl: string;
  title: string | null;
  links: BatchLink[];
}

/** A download offered by the browser extension. */
export interface AddPrompt {
  url: string;
  fileName: string | null;
  fileSize?: number | null;
  directory?: string | null;
  headers: Headers;
}

export interface Browser {
  id: string;
  name: string;
  engine: Engine;
  executable: string;
  extensionsPage: string;
  connected: boolean;
}

export interface InstallInfo {
  method: "store" | "manual" | "automatic";
  browser: string;
  engine: Engine;
  extensionDir: string;
  extensionsPage: string;
  setupStep?: string | null;
}

/** A cookie as the browser extension reads it. */
export interface Cookie {
  name: string;
  value: string;
  domain: string;
  path?: string;
  secure?: boolean;
  hostOnly?: boolean;
  expires?: number | null;
}

/** A page with a video. */
export interface MediaSource {
  pageUrl: string;
  mediaUrl?: string | null;
  title?: string | null;
  cookies?: Cookie[];
  userAgent?: string | null;
  referrer?: string | null;
}

export interface MediaOption {
  id: string;
  kind: "video" | "audio";
  height: number | null;
  fps: number | null;
  ext: string;
  vcodec: string | null;
  acodec: string | null;
  size: number | null;
  needsFfmpeg: boolean;
  format: string;
  merge: string | null;
  audio: string | null;
}

export interface MediaInfo {
  title: string;
  thumbnail: string | null;
  duration: number | null;
  site: string;
  url: string;
  options: MediaOption[];
  default: string | null;
  videoDir: string;
  audioDir: string;
}

export interface ToolStatus {
  ytdlp: boolean;
  ffmpeg: boolean;
  ffmpegInstallable: boolean;
  ytdlpSizeMb: number;
  ffmpegSizeMb: number;
}

export interface ToolProgress {
  tool: "yt-dlp" | "ffmpeg";
  downloaded: number;
  total: number | null;
}

export interface AddMediaRequest {
  source: MediaSource;
  option: MediaOption;
  title: string;
  directory?: string;
  start: boolean;
}

/** A magnet link, a .torrent address or a .torrent file. */
export interface TorrentSource {
  link?: string | null;
  path?: string | null;
  headers?: [string, string][];
}

export interface TorrentFile {
  index: number;
  /** Path inside the torrent, with "/" between folders. */
  path: string;
  size: number;
}

export interface TorrentInfo {
  infoHash: string;
  name: string;
  size: number;
  files: TorrentFile[];
  private: boolean;
  comment: string | null;
  pieces: number;
  pieceLength: number;
  directory: string;
  /** The download that already has this torrent. */
  existing: number | null;
}

export interface AddTorrentRequest {
  infoHash: string;
  /** Indexes of the chosen files; all when missing. */
  files?: number[] | null;
  directory?: string;
  start: boolean;
}

/** What happens when a download finishes. */
export interface WhenDone {
  showDialog: boolean;
  openFile: boolean;
}

export interface AppInfo {
  version: string;
  bridgePort: number;
  bridgeError: string | null;
}

export interface Api {
  listDownloads(): Promise<Download[]>;
  getDownload(id: number): Promise<Download | null>;
  openProgressWindow(id: number): Promise<void>;
  getWhenDone(id: number): Promise<WhenDone>;
  setWhenDone(id: number, choice: WhenDone): Promise<void>;
  /** Controls the window the page runs in. */
  closeWindow(): Promise<void>;
  showWindow(): Promise<void>;
  getAddPrompt(): Promise<AddPrompt>;
  minimizeWindow(): Promise<void>;
  setWindowTitle(title: string): Promise<void>;
  resizeWindow(width: number, height: number): Promise<void>;
  probe(url: string, headers: Headers): Promise<ProbeResult>;
  add(request: AddRequest): Promise<number>;
  pause(ids: number[]): Promise<void>;
  resume(ids: number[]): Promise<void>;
  remove(ids: number[], deleteFiles: boolean): Promise<void>;
  pauseAll(): Promise<void>;
  resumeAll(): Promise<void>;
  open(id: number): Promise<void>;
  showInFolder(id: number): Promise<void>;
  getUpdateState(): Promise<UpdateState>;
  checkForUpdates(): Promise<UpdateState>;
  installUpdate(): Promise<UpdateState>;
  getSettings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Settings>;
  pickFolder(): Promise<string | null>;
  pickTorrentFile(): Promise<string | null>;
  listBrowsers(): Promise<Browser[]>;
  installExtension(id: string): Promise<InstallInfo>;
  completeBrowserSetup(): Promise<Settings>;
  openExtensionFolder(engine: Engine): Promise<void>;
  appInfo(): Promise<AppInfo>;
  onAddPrompt(handler: (prompt: AddPrompt) => void): Promise<UnlistenFn>;
  onClipboardLink(handler: (url: string) => void): Promise<UnlistenFn>;
  mediaTools(): Promise<ToolStatus>;
  /** Downloads yt-dlp, and ffmpeg when asked, if they are missing. */
  mediaPrepare(ffmpeg: boolean): Promise<ToolStatus>;
  mediaProbe(source: MediaSource): Promise<MediaInfo>;
  /** What yt-dlp is doing while it reads the page. */
  mediaProbeStage(pageUrl: string): Promise<string | null>;
  cancelMediaProbe(pageUrl: string): Promise<void>;
  addMedia(request: AddMediaRequest): Promise<number>;
  onMediaPrompt(handler: (source: MediaSource) => void): Promise<UnlistenFn>;
  inspectTorrent(source: TorrentSource): Promise<TorrentInfo>;
  cancelTorrentInspect(): Promise<void>;
  addTorrent(request: AddTorrentRequest): Promise<number>;
  torrentFiles(id: number): Promise<TorrentFile[]>;
  /** Magnet links and .torrent files SDM was started with. */
  takePendingTorrents(): Promise<TorrentSource[]>;
  magnetHandler(): Promise<{ supported: boolean; registered: boolean }>;
  registerMagnetHandler(): Promise<void>;
  onTorrentPrompt(handler: (source: TorrentSource) => void): Promise<UnlistenFn>;
  /** Files dropped on the window. */
  onFileDrop(handler: (paths: string[]) => void): Promise<UnlistenFn>;
  onToolProgress(handler: (progress: ToolProgress) => void): Promise<UnlistenFn>;
  /** The links a pattern such as `file[1-10].zip` stands for. */
  expandBatch(pattern: string): Promise<string[]>;
  addBatch(request: BatchRequest): Promise<number[]>;
  /** Gives an unfinished download a new link, keeping its progress. */
  changeAddress(id: number, url: string): Promise<void>;
  onLinksPrompt(handler: (prompt: LinksPrompt) => void): Promise<UnlistenFn>;
  listQueues(): Promise<Queue[]>;
  /** Adds a queue (with an empty id) or changes one. */
  saveQueue(queue: Queue): Promise<Queue>;
  deleteQueue(id: string): Promise<void>;
  startQueue(id: string): Promise<void>;
  stopQueue(id: string): Promise<void>;
  moveToQueue(ids: number[], queue: string): Promise<void>;
  /** What happens once every download is done. */
  getAfterAll(): Promise<PowerAction>;
  setAfterAll(action: PowerAction): Promise<void>;
  powerCountdown(): Promise<PowerCountdown | null>;
  cancelPowerAction(): Promise<void>;
  onPowerCountdown(handler: (countdown: PowerCountdown | null) => void): Promise<UnlistenFn>;
}

const tauriApi: Api = {
  listDownloads: () => invoke("list_downloads"),
  getDownload: (id) => invoke("get_download", { id }),
  openProgressWindow: (id) => invoke("open_progress_window", { id }),
  getWhenDone: (id) => invoke("get_when_done", { id }),
  setWhenDone: (id, choice) => invoke("set_when_done", { id, choice }),
  closeWindow: () => getCurrentWindow().close(),
  async showWindow() {
    await getCurrentWindow().show();
    await getCurrentWindow().setFocus();
  },
  getAddPrompt: () => invoke("get_add_prompt"),
  minimizeWindow: () => getCurrentWindow().minimize(),
  setWindowTitle: (title) => getCurrentWindow().setTitle(title),
  async resizeWindow(width, height) {
    // Pin the size with min = max: GTK will not shrink a window that is not resizable.
    const window = getCurrentWindow();
    const monitor = await currentMonitor().catch(() => null);
    if (monitor) {
      const scale = Number.isFinite(monitor.scaleFactor) && monitor.scaleFactor > 0 ? monitor.scaleFactor : 1;
      width = Math.min(width, Math.max(1, Math.floor(monitor.workArea.size.width / scale - 24)));
      height = Math.min(height, Math.max(1, Math.floor(monitor.workArea.size.height / scale - 24)));
    }
    const size = new LogicalSize(width, height);
    await window.setMaxSize(null);
    await window.setMinSize(size);
    await window.setMaxSize(size);
    await window.setSize(size);
  },
  probe: (url, headers) => invoke("probe_url", { url, headers }),
  add: (request) => invoke("add_download", { request }),
  pause: (ids) => invoke("pause_downloads", { ids }),
  resume: (ids) => invoke("resume_downloads", { ids }),
  remove: (ids, deleteFiles) => invoke("remove_downloads", { ids, deleteFiles }),
  pauseAll: () => invoke("pause_all"),
  resumeAll: () => invoke("resume_all"),
  open: (id) => invoke("open_download", { id }),
  showInFolder: (id) => invoke("show_in_folder", { id }),
  getUpdateState: () => invoke("get_update_state"),
  checkForUpdates: () => invoke("check_for_updates"),
  installUpdate: () => invoke("install_update"),
  getSettings: () => invoke("get_settings"),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  pickFolder: () => invoke("pick_folder"),
  pickTorrentFile: () => invoke("pick_torrent_file"),
  listBrowsers: () => invoke("list_browsers"),
  installExtension: (id) => invoke("install_extension", { id }),
  completeBrowserSetup: () => invoke("complete_browser_setup"),
  openExtensionFolder: (engine) => invoke("open_extension_folder", { engine }),
  appInfo: () => invoke("app_info"),
  onAddPrompt: (handler) => listen<AddPrompt>("sdm:add-prompt", (e) => handler(e.payload)),
  onClipboardLink: (handler) => listen<string>("sdm:clipboard-link", (e) => handler(e.payload)),
  mediaTools: () => invoke("media_tools"),
  mediaPrepare: (ffmpeg) => invoke("media_prepare", { ffmpeg }),
  mediaProbe: (source) => invoke("media_probe", { source }),
  mediaProbeStage: (pageUrl) => invoke("media_probe_stage", { pageUrl }),
  cancelMediaProbe: (pageUrl) => invoke("cancel_media_probe", { pageUrl }),
  addMedia: (request) => invoke("add_media", { request }),
  onMediaPrompt: (handler) => listen<MediaSource>("sdm:media-prompt", (e) => handler(e.payload)),
  inspectTorrent: (source) => invoke("inspect_torrent", { source }),
  cancelTorrentInspect: () => invoke("cancel_torrent_inspect"),
  addTorrent: (request) => invoke("add_torrent", { request }),
  torrentFiles: (id) => invoke("torrent_files", { id }),
  takePendingTorrents: () => invoke("take_pending_torrents"),
  magnetHandler: () => invoke("magnet_handler"),
  registerMagnetHandler: () => invoke("register_magnet_handler"),
  onTorrentPrompt: (handler) => listen<TorrentSource>("sdm:torrent-prompt", (e) => handler(e.payload)),
  onFileDrop: (handler) =>
    getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop") handler(event.payload.paths);
    }),
  onToolProgress: (handler) => listen<ToolProgress>("sdm:tool-progress", (e) => handler(e.payload)),
  expandBatch: (pattern) => invoke("expand_batch", { pattern }),
  addBatch: (request) => invoke("add_batch", { request }),
  changeAddress: (id, url) => invoke("change_address", { id, url }),
  onLinksPrompt: (handler) => listen<LinksPrompt>("sdm:links-prompt", (e) => handler(e.payload)),
  listQueues: () => invoke("list_queues"),
  saveQueue: (queue) => invoke("save_queue", { queue }),
  deleteQueue: (id) => invoke("delete_queue", { id }),
  startQueue: (id) => invoke("start_queue", { id }),
  stopQueue: (id) => invoke("stop_queue", { id }),
  moveToQueue: (ids, queue) => invoke("move_to_queue", { ids, queue }),
  getAfterAll: () => invoke("get_after_all"),
  setAfterAll: (action) => invoke("set_after_all", { action }),
  powerCountdown: () => invoke("power_countdown"),
  cancelPowerAction: () => invoke("cancel_power_action"),
  onPowerCountdown: (handler) => listen<PowerCountdown | null>("sdm:power", (e) => handler(e.payload)),
};

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: Api = inTauri ? tauriApi : (await import("./mock")).mockApi;
