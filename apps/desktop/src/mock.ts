// A simulated backend so the interface can be developed and tested in a
// normal browser (`npm run dev`). It is never used inside the real app.

import type { Api, Browser, Category, Download, Settings, WhenDone } from "./api";

const MB = 1024 * 1024;
const now = Date.now();

const settings: Settings = {
  downloadDir: "/home/demo/Downloads",
  categoryFolders: true,
  connections: 8,
  maxParallel: 3,
  speedLimit: 0,
  proxy: "",
  language: "auto",
  skin: (new URLSearchParams(location.search).get("skin") as Settings["skin"]) || "neon",
  theme: (new URLSearchParams(location.search).get("theme") as Settings["theme"]) || "system",
  progressWindow: true,
  progressStyle: (new URLSearchParams(location.search).get("pstyle") as Settings["progressStyle"]) || "graph",
  completeDialog: true,
  closeToTray: true,
  notifications: true,
  clipboardMonitor: true,
  browserIntegration: true,
  confirmBrowserDownloads: true,
  bridgePort: 17360,
  captureExtensions: ["7z", "exe", "iso", "mkv", "mp3", "mp4", "msi", "pdf", "rar", "zip"],
};

function categoryOf(name: string): Category {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mkv", "webm", "avi", "mov"].includes(ext)) return "video";
  if (["mp3", "flac", "m4a", "wav"].includes(ext)) return "music";
  if (["pdf", "docx", "xlsx", "epub", "txt"].includes(ext)) return "document";
  if (["zip", "rar", "7z", "iso", "gz"].includes(ext)) return "compressed";
  if (["exe", "msi", "dmg", "deb", "apk"].includes(ext)) return "program";
  if (["png", "jpg", "jpeg", "webp"].includes(ext)) return "image";
  return "other";
}

function folderFor(category: Category) {
  const folders: Record<Category, string> = {
    video: "Video",
    music: "Music",
    document: "Documents",
    compressed: "Compressed",
    program: "Programs",
    image: "Images",
    other: "",
  };
  const sub = settings.categoryFolders ? folders[category] : "";
  return sub ? `${settings.downloadDir}/${sub}` : settings.downloadDir;
}

let nextId = 1;
function make(
  fileName: string,
  host: string,
  size: number | null,
  state: Download["state"],
  fraction: number,
  minutesAgo: number,
  extra: Partial<Download> = {},
): Download {
  const category = categoryOf(fileName);
  return {
    id: nextId++,
    url: `https://${host}/files/${encodeURIComponent(fileName)}`,
    fileName,
    path: `${folderFor(category)}/${fileName}`,
    size,
    downloaded: size ? Math.round(size * fraction) : Math.round(fraction * 40 * MB),
    resumable: true,
    category,
    state,
    error: null,
    addedAt: now - minutesAgo * 60_000,
    completedAt: state === "completed" ? now - (minutesAgo - 3) * 60_000 : null,
    elapsed: Math.round(((size ?? 40 * MB) * fraction) / (11 * MB)) * 1000,
    speed: 0,
    eta: null,
    connections: 0,
    segments: [],
    stopping: false,
    ...extra,
  };
}

const downloads: Download[] = [
  make("ubuntu-24.04.2-desktop-amd64.iso", "releases.ubuntu.com", 6_114 * MB, "downloading", 0.42, 2),
  make("Big.Buck.Bunny.1080p.mkv", "download.blender.org", 691 * MB, "downloading", 0.78, 5),
  make("VSCodeUserSetup-x64-1.105.exe", "code.visualstudio.com", 104 * MB, "downloading", 0.18, 6),
  make("Bao-cao-tai-chinh-2025.pdf", "drive.company.vn", 18 * MB, "completed", 1, 60),
  make("Son Tung M-TP - Album 2025.zip", "files.example.vn", 245 * MB, "paused", 0.35, 90),
  make("podcast-episode-128.mp3", "cdn.podcast.fm", 72 * MB, "completed", 1, 60 * 26),
  make("dataset-archive.7z", "data.example.org", 3_210 * MB, "failed", 0.11, 60 * 27, {
    error: "server responded with 403 Forbidden",
  }),
  make("wallpaper-4k-hoi-an.png", "unsplash.com", 9 * MB, "completed", 1, 60 * 50),
];

const whenDone = new Map<number, WhenDone>();

const speeds = new Map<number, number>([
  [1, 9.4 * MB],
  [2, 4.1 * MB],
  [3, 3.1 * MB],
]);

function simulate() {
  const active = downloads.filter((d) => d.state === "downloading");
  for (const d of active) {
    const base = speeds.get(d.id) ?? 3 * MB;
    const speed = Math.max(0, base * (0.85 + Math.random() * 0.3));
    const size = d.size ?? Infinity;
    d.downloaded = Math.min(size, d.downloaded + speed / 2);
    d.elapsed += 500;
    d.speed = Math.round(speed);
    d.connections = 8;
    d.eta = d.size ? Math.ceil((d.size - d.downloaded) / speed) : null;
    d.segments = segmentsFor(d);
    if (d.size && d.downloaded >= d.size) {
      Object.assign(d, { state: "completed", speed: 0, eta: null, connections: 0, segments: [] });
      d.completedAt = Date.now();
    }
  }
  const running = downloads.filter((d) => d.state === "downloading").length;
  const queued = downloads.filter((d) => d.state === "queued");
  for (const d of queued.slice(0, Math.max(0, settings.maxParallel - running))) {
    d.state = "downloading";
  }
}

function segmentsFor(d: Download) {
  if (!d.size) return [];
  const count = 8;
  const chunk = d.size / count;
  const done = d.downloaded / d.size;
  return Array.from({ length: count }, (_, i) => {
    const wobble = 0.75 + ((i * 37) % 10) / 20;
    const fraction = Math.min(1, done * wobble);
    return {
      start: Math.round(i * chunk),
      end: Math.round((i + 1) * chunk),
      downloaded: Math.round(chunk * fraction),
    };
  });
}

setInterval(simulate, 500);

const browsers: Browser[] = [
  { id: "chrome", name: "Google Chrome", engine: "chromium", executable: "/usr/bin/google-chrome", extensionsPage: "chrome://extensions", connected: true },
  { id: "coccoc", name: "Cốc Cốc", engine: "chromium", executable: "/usr/bin/coccoc-browser", extensionsPage: "chrome://extensions", connected: false },
  { id: "edge", name: "Microsoft Edge", engine: "chromium", executable: "/usr/bin/microsoft-edge", extensionsPage: "edge://extensions", connected: false },
  { id: "firefox", name: "Firefox", engine: "firefox", executable: "/usr/bin/firefox", extensionsPage: "about:debugging#/runtime/this-firefox", connected: false },
];

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const byIds = (ids: number[]) => downloads.filter((d) => ids.includes(d.id));

export const mockApi: Api = {
  async listDownloads() {
    return clone(downloads);
  },
  async getDownload(id) {
    const found = downloads.find((d) => d.id === id);
    return found ? clone(found) : null;
  },
  async openProgressWindow(id) {
    window.open(`${location.pathname}${location.search}#progress/${id}`, "_blank", "width=480,height=320");
  },
  async getWhenDone(id) {
    return whenDone.get(id) ?? { showDialog: settings.completeDialog, openFile: false };
  },
  async setWhenDone(id, choice) {
    whenDone.set(id, choice);
  },
  async closeWindow() {},
  async minimizeWindow() {},
  async setWindowTitle(title) {
    document.title = title;
  },
  async resizeWindow() {},
  async probe(url) {
    await wait(400);
    const parsed = new URL(url);
    const name = decodeURIComponent(parsed.pathname.split("/").filter(Boolean).pop() ?? "download");
    const category = categoryOf(name);
    return {
      url,
      finalUrl: url,
      size: 1_480 * MB,
      resumable: true,
      fileName: name,
      contentType: "application/octet-stream",
      category,
      directory: folderFor(category),
    };
  },
  async add(request) {
    const name = request.fileName || new URL(request.url).pathname.split("/").pop() || "download";
    const d = make(name, new URL(request.url).host, 1_480 * MB, request.start ? "queued" : "paused", 0, 0, { url: request.url });
    if (request.directory) d.path = `${request.directory}/${name}`;
    downloads.push(d);
    return d.id;
  },
  async pause(ids) {
    for (const d of byIds(ids)) {
      if (d.state === "downloading" || d.state === "queued") {
        Object.assign(d, { state: "paused", speed: 0, eta: null, connections: 0, segments: [] });
      }
    }
  },
  async resume(ids) {
    for (const d of byIds(ids)) {
      if (d.state === "paused" || d.state === "failed") Object.assign(d, { state: "queued", error: null });
    }
  },
  async remove(ids) {
    for (const id of ids) {
      const index = downloads.findIndex((d) => d.id === id);
      if (index >= 0) downloads.splice(index, 1);
    }
  },
  async pauseAll() {
    await this.pause(downloads.map((d) => d.id));
  },
  async resumeAll() {
    await this.resume(downloads.map((d) => d.id));
  },
  async open() {},
  async showInFolder() {},
  async getSettings() {
    return clone(settings);
  },
  async saveSettings(next) {
    Object.assign(settings, next);
    return clone(settings);
  },
  async pickFolder() {
    return "/home/demo/Videos";
  },
  async listBrowsers() {
    return clone(browsers);
  },
  async installExtension(id) {
    const browser = browsers.find((b) => b.id === id)!;
    return {
      method: "manual",
      browser: browser.name,
      engine: browser.engine,
      extensionDir: `/home/demo/.local/share/com.supodownloadmanager.app/extension/${browser.engine}`,
      extensionsPage: browser.extensionsPage,
    };
  },
  async openExtensionFolder() {},
  async appInfo() {
    return { version: "0.2.0", bridgePort: 17360, bridgeError: null };
  },
  async onAddPrompt(handler) {
    const listener = (event: Event) => handler((event as CustomEvent).detail);
    window.addEventListener("sdm:add-prompt", listener);
    return () => window.removeEventListener("sdm:add-prompt", listener);
  },
  async onClipboardLink(handler) {
    const listener = (event: Event) => handler((event as CustomEvent).detail);
    window.addEventListener("sdm:clipboard-link", listener);
    return () => window.removeEventListener("sdm:clipboard-link", listener);
  },
};
