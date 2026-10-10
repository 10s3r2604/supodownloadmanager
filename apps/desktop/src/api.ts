// Talks to the Rust side of the app. When the page runs in a normal browser
// (for design work or tests) a simulated backend is used instead.

import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
}

export interface Settings {
  downloadDir: string;
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
  notifications: boolean;
  clipboardMonitor: boolean;
  browserIntegration: boolean;
  confirmBrowserDownloads: boolean;
  bridgePort: number;
  captureExtensions: string[];
}

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
}

/** A download offered by the browser extension. */
export interface AddPrompt {
  url: string;
  fileName: string | null;
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
  method: "store" | "manual";
  browser: string;
  engine: Engine;
  extensionDir: string;
  extensionsPage: string;
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
  getSettings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Settings>;
  pickFolder(): Promise<string | null>;
  listBrowsers(): Promise<Browser[]>;
  installExtension(id: string): Promise<InstallInfo>;
  openExtensionFolder(engine: Engine): Promise<void>;
  appInfo(): Promise<AppInfo>;
  onAddPrompt(handler: (prompt: AddPrompt) => void): Promise<UnlistenFn>;
  onClipboardLink(handler: (url: string) => void): Promise<UnlistenFn>;
}

const tauriApi: Api = {
  listDownloads: () => invoke("list_downloads"),
  getDownload: (id) => invoke("get_download", { id }),
  openProgressWindow: (id) => invoke("open_progress_window", { id }),
  getWhenDone: (id) => invoke("get_when_done", { id }),
  setWhenDone: (id, choice) => invoke("set_when_done", { id, choice }),
  closeWindow: () => getCurrentWindow().close(),
  minimizeWindow: () => getCurrentWindow().minimize(),
  setWindowTitle: (title) => getCurrentWindow().setTitle(title),
  async resizeWindow(width, height) {
    // Pin the size with min = max: GTK will not shrink a window that is not resizable.
    const window = getCurrentWindow();
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
  getSettings: () => invoke("get_settings"),
  saveSettings: (settings) => invoke("save_settings", { settings }),
  pickFolder: () => invoke("pick_folder"),
  listBrowsers: () => invoke("list_browsers"),
  installExtension: (id) => invoke("install_extension", { id }),
  openExtensionFolder: (engine) => invoke("open_extension_folder", { engine }),
  appInfo: () => invoke("app_info"),
  onAddPrompt: (handler) => listen<AddPrompt>("sdm:add-prompt", (e) => handler(e.payload)),
  onClipboardLink: (handler) => listen<string>("sdm:clipboard-link", (e) => handler(e.payload)),
};

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: Api = inTauri ? tauriApi : (await import("./mock")).mockApi;
