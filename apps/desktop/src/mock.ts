// A simulated backend so the interface can be developed and tested in a
// normal browser (`npm run dev`). It is never used inside the real app.

import type { Api, Browser, Category, Download, MediaInfo, MediaOption, Settings, ToolStatus, TorrentFile, TorrentInfo, WhenDone } from "./api";
import { categoryOf } from "./model";

const MB = 1024 * 1024;
const now = Date.now();

const settings: Settings = {
  downloadDir: "/home/demo/Downloads",
  categoryFolders: true,
  connections: 8,
  maxParallel: 3,
  speedLimit: 0,
  proxy: "",
  language: (new URLSearchParams(location.search).get("lang") as Settings["language"]) ?? "auto",
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
  captureExtensions: ["7z", "exe", "iso", "mkv", "mp3", "mp4", "msi", "pdf", "rar", "torrent", "zip"],
  seedRatio: 100,
  uploadLimit: 0,
  torrentPort: 0,
};

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
    media: false,
    processing: false,
    torrent: false,
    seeding: false,
    checking: false,
    uploadSpeed: 0,
    uploaded: 0,
    ...extra,
  };
}

const magnet = (hash: string, name: string) => `magnet:?xt=urn:btih:${hash}&dn=${encodeURIComponent(name)}`;

/** The files of the example torrent (Blender's open movies, free to share). */
const openMovies: TorrentFile[] = [
  { index: 0, path: "Big Buck Bunny/Big Buck Bunny.mp4", size: 276_134_947 },
  { index: 1, path: "Big Buck Bunny/Big Buck Bunny.en.srt", size: 140 },
  { index: 2, path: "Big Buck Bunny/poster.jpg", size: 310_380 },
  { index: 3, path: "Sintel/Sintel.mp4", size: 129_241_752 },
  { index: 4, path: "Sintel/Sintel.en.srt", size: 1_652 },
  { index: 5, path: "Sintel/Sintel.vi.srt", size: 1_738 },
  { index: 6, path: "Tears of Steel/tears_of_steel_1080p.mkv", size: 571_694_486 },
  { index: 7, path: "README.txt", size: 2_048 },
];

const downloads: Download[] = [
  make("ubuntu-24.04.2-desktop-amd64.iso", "releases.ubuntu.com", 6_114 * MB, "downloading", 0.42, 2),
  make("debian-13.1.0-amd64-DVD-1.iso", "", 3_978 * MB, "downloading", 0.27, 3, {
    url: magnet("6a9759bffd5c0af65319979fb7832189f4f3c35d", "debian-13.1.0-amd64-DVD-1.iso"),
    torrent: true,
  }),
  make("Big.Buck.Bunny.1080p.mkv", "download.blender.org", 691 * MB, "downloading", 0.78, 5),
  make("VSCodeUserSetup-x64-1.105.exe", "code.visualstudio.com", 104 * MB, "downloading", 0.18, 6),
  make("Bao-cao-tai-chinh-2025.pdf", "drive.company.vn", 18 * MB, "completed", 1, 60),
  make("Son Tung M-TP - Album 2025.zip", "files.example.vn", 245 * MB, "paused", 0.35, 90),
  make("podcast-episode-128.mp3", "cdn.podcast.fm", 72 * MB, "completed", 1, 60 * 26),
  make("dataset-archive.7z", "data.example.org", 3_210 * MB, "failed", 0.11, 60 * 27, {
    error: "server responded with 403 Forbidden",
  }),
  make("wallpaper-4k-hoi-an.png", "unsplash.com", 9 * MB, "completed", 1, 60 * 50),
  make("Sintel", "", 129 * MB, "completed", 1, 45, {
    url: magnet("08ada5a7a6183aae1e09d831df6748d566095a10", "Sintel"),
    torrent: true,
    seeding: true,
    uploaded: 61 * MB,
    category: "video",
    path: "/home/demo/Downloads/Video/Sintel",
  }),
];

const whenDone = new Map<number, WhenDone>();

const speeds = new Map<number, number>([
  [1, 9.4 * MB],
  [2, 5.2 * MB],
  [3, 4.1 * MB],
  [4, 3.1 * MB],
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
    d.connections = d.torrent ? 24 + Math.round(Math.random() * 6) : 8;
    d.eta = d.size ? Math.ceil((d.size - d.downloaded) / speed) : null;
    d.segments = segmentsFor(d);
    if (d.torrent) {
      d.uploadSpeed = Math.round(380_000 * (0.7 + Math.random() * 0.6));
      d.uploaded += d.uploadSpeed / 2;
    }
    if (d.size && d.downloaded >= d.size) {
      Object.assign(d, { state: "completed", speed: 0, eta: null, connections: 0, segments: [] });
      d.completedAt = Date.now();
    }
  }
  for (const d of downloads.filter((d) => d.seeding)) {
    d.uploadSpeed = Math.round(620_000 * (0.6 + Math.random() * 0.8));
    d.uploaded += d.uploadSpeed / 2;
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

const query = new URLSearchParams(location.search);
const tools: ToolStatus = {
  ytdlp: query.get("tools") !== "0",
  ffmpeg: query.get("tools") !== "0" && query.get("ffmpeg") !== "0",
  ffmpegInstallable: true,
  ytdlpSizeMb: 18,
  ffmpegSizeMb: 66,
};

function option(id: string, kind: MediaOption["kind"], height: number | null, ext: string, size: number, merge: boolean, extra: Partial<MediaOption> = {}): MediaOption {
  return {
    id,
    kind,
    height,
    fps: height ? 30 : null,
    ext,
    vcodec: kind === "video" ? (height && height > 1080 ? "VP9" : "H.264") : null,
    acodec: ext === "webm" ? "Opus" : "AAC",
    size,
    needsFfmpeg: merge,
    format: id,
    merge: merge ? ext : null,
    audio: null,
    ...extra,
  };
}

function mediaInfo(pageUrl: string): MediaInfo {
  const youtube = /youtu/.test(pageUrl);
  if (!youtube) {
    return {
      title: "Big Buck Bunny (2008)",
      thumbnail: null,
      duration: 596,
      site: "HTML5MediaEmbed",
      url: pageUrl,
      options: [
        option("v1080", "video", 1080, "mp4", 276 * MB, false, { fps: 60 }),
        option("v720", "video", 720, "mp4", 164 * MB, false),
        option("v480", "video", 480, "mp4", 98 * MB, false),
        option("mp3", "audio", null, "mp3", 9.5 * MB, true, { audio: "mp3", acodec: "MP3" }),
      ],
      default: "v1080",
      videoDir: `${settings.downloadDir}/Video`,
      audioDir: `${settings.downloadDir}/Music`,
    };
  }
  return {
    title: "Sơn Tùng M-TP | Chúng Ta Của Tương Lai | Official Music Video",
    thumbnail: null,
    duration: 263,
    site: "Youtube",
    url: pageUrl,
    options: [
      option("v2160", "video", 2160, "webm", 412 * MB, true),
      option("v1440", "video", 1440, "webm", 205 * MB, true),
      option("v1080", "video", 1080, "mp4", 96 * MB, true, { fps: 60 }),
      option("v720", "video", 720, "mp4", 48 * MB, true),
      option("v480", "video", 480, "mp4", 22 * MB, true),
      option("v360", "video", 360, "mp4", 14 * MB, false),
      option("m4a", "audio", null, "m4a", 4.2 * MB, false),
      option("mp3", "audio", null, "mp3", 6.3 * MB, true, { audio: "mp3", acodec: "MP3" }),
    ],
    default: "v1080",
    videoDir: `${settings.downloadDir}/Video`,
    audioDir: `${settings.downloadDir}/Music`,
  };
}

function torrentInfo(): TorrentInfo {
  return {
    infoHash: "c9e15763f722f23e98a29decdfae341b98d53056",
    name: "Blender Open Movies",
    size: openMovies.reduce((sum, f) => sum + f.size, 0),
    files: openMovies,
    private: false,
    comment: "Big Buck Bunny, Sintel and Tears of Steel (CC BY, Blender Foundation)",
    pieces: 3754,
    pieceLength: 262_144,
    directory: `${settings.downloadDir}/Video`,
    existing: null,
  };
}

if (query.get("torrent")) {
  setTimeout(() => {
    window.dispatchEvent(
      new CustomEvent("sdm:torrent-prompt", {
        detail: { link: magnet("c9e15763f722f23e98a29decdfae341b98d53056", "Blender Open Movies") },
      }),
    );
  }, 800);
}

if (query.get("media")) {
  setTimeout(() => {
    window.dispatchEvent(
      new CustomEvent("sdm:media-prompt", {
        detail:
          query.get("media") === "generic"
            ? { pageUrl: "https://media.example.org/films/big-buck-bunny", title: "Big Buck Bunny" }
            : { pageUrl: "https://www.youtube.com/watch?v=zoEtcR5EW08", title: "YouTube" },
      }),
    );
  }, 800);
}

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
    return { version: "0.4.0", bridgePort: 17360, bridgeError: null };
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
  async mediaTools() {
    return { ...tools };
  },
  async mediaPrepare(ffmpeg) {
    const fetchTool = async (tool: "yt-dlp" | "ffmpeg", total: number) => {
      for (let done = 0; done <= total; done += total / 12) {
        window.dispatchEvent(new CustomEvent("sdm:tool-progress", { detail: { tool, downloaded: Math.round(done), total } }));
        await wait(150);
      }
    };
    if (!tools.ytdlp) await fetchTool("yt-dlp", 18 * MB);
    tools.ytdlp = true;
    if (ffmpeg && !tools.ffmpeg) await fetchTool("ffmpeg", 66 * MB);
    if (ffmpeg) tools.ffmpeg = true;
    return { ...tools };
  },
  async mediaProbe(source) {
    await wait(query.get("slowprobe") ? 60_000 : 900);
    if (/broken/.test(source.pageUrl)) throw new Error("Unsupported URL: " + source.pageUrl);
    return mediaInfo(source.pageUrl);
  },
  async mediaProbeStage() {
    return "[youtube] zoEtcR5EW08: Downloading tv client config";
  },
  async addMedia(request) {
    const d = make(`${request.title}.${request.option.ext}`, new URL(request.source.pageUrl).host, request.option.size, request.start ? "downloading" : "paused", 0, 0, {
      url: request.source.pageUrl,
      media: true,
    });
    downloads.push(d);
    return d.id;
  },
  async onMediaPrompt(handler) {
    const listener = (event: Event) => handler((event as CustomEvent).detail);
    window.addEventListener("sdm:media-prompt", listener);
    return () => window.removeEventListener("sdm:media-prompt", listener);
  },
  async pickTorrentFile() {
    return "/home/demo/Downloads/blender-open-movies.torrent";
  },
  async inspectTorrent(source) {
    await wait(source.path ? 300 : query.get("slowtorrent") ? 60_000 : 1400);
    if (/broken/.test(source.link ?? source.path ?? "")) throw new Error("no one sharing this torrent was found");
    return torrentInfo();
  },
  async cancelTorrentInspect() {},
  async addTorrent(request) {
    const info = torrentInfo();
    const files = request.files ? info.files.filter((f) => request.files?.includes(f.index)) : info.files;
    const size = files.reduce((sum, f) => sum + f.size, 0);
    const d = make(info.name, "", size, request.start ? "downloading" : "paused", 0, 0, {
      url: magnet(info.infoHash, info.name),
      torrent: true,
      category: "video",
      path: `${request.directory ?? info.directory}/${info.name}`,
    });
    downloads.push(d);
    return d.id;
  },
  async torrentFiles(id) {
    const d = downloads.find((x) => x.id === id);
    if (!d?.torrent) return [];
    if (d.fileName === "Sintel") return openMovies.filter((f) => f.path.startsWith("Sintel/"));
    if (d.fileName.endsWith(".iso")) return [{ index: 0, path: d.fileName, size: d.size ?? 0 }];
    return openMovies;
  },
  async takePendingTorrents() {
    return [];
  },
  async magnetHandler() {
    return { supported: true, registered: query.get("magnet") === "1" };
  },
  async registerMagnetHandler() {},
  async onTorrentPrompt(handler) {
    const listener = (event: Event) => handler((event as CustomEvent).detail);
    window.addEventListener("sdm:torrent-prompt", listener);
    return () => window.removeEventListener("sdm:torrent-prompt", listener);
  },
  async onFileDrop() {
    return () => {};
  },
  async onToolProgress(handler) {
    const listener = (event: Event) => handler((event as CustomEvent).detail);
    window.addEventListener("sdm:tool-progress", listener);
    return () => window.removeEventListener("sdm:tool-progress", listener);
  },
};
