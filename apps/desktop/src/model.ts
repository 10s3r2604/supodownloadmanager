// View logic shared by both looks: filters, groups, totals and speed history.

import type { Category, Download, Settings } from "./api";

export type Filter =
  | "all"
  | "active"
  | "downloading"
  | "queued"
  | "paused"
  | "completed"
  | "failed"
  | "unfinished"
  | `cat:${Category}`;

export const CATEGORIES: Category[] = ["video", "music", "document", "compressed", "program", "image", "other"];

export function matches(filter: Filter, d: Download): boolean {
  switch (filter) {
    case "all":
      return true;
    case "active":
      return d.state === "downloading" || d.state === "queued";
    case "unfinished":
      return d.state !== "completed";
    case "downloading":
    case "queued":
    case "paused":
    case "completed":
    case "failed":
      return d.state === filter;
    default:
      return d.category === filter.slice(4);
  }
}

const EXTENSIONS: [Category, string[]][] = [
  ["video", ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp", "ts", "m2ts", "vob", "rmvb"]],
  ["music", ["mp3", "wav", "flac", "aac", "ogg", "oga", "m4a", "wma", "opus", "aiff", "ape"]],
  ["document", ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "csv", "epub", "mobi"]],
  ["compressed", ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "img", "cab"]],
  ["program", ["exe", "msi", "msix", "dmg", "pkg", "deb", "rpm", "apk", "appimage", "flatpak", "bin", "run"]],
  ["image", ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "tif", "tiff", "heic", "avif", "psd"]],
];

/** The category of a file name, like the app's own sorting. */
export function categoryOf(name: string): Category {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return EXTENSIONS.find(([, list]) => list.includes(ext))?.[0] ?? "other";
}

export function hostOf(url: string): string {
  if (/^magnet:/i.test(url)) return "BitTorrent";
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
}

export type Group = "active" | "today" | "yesterday" | "older";

/** Downloads in progress first, the rest by the day they finished or were added. */
export function groupOf(d: Download, now = new Date()): Group {
  if (d.state === "downloading" || d.state === "queued") return "active";
  const day = new Date(d.completedAt ?? d.addedAt);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (day.getTime() >= startOfToday) return "today";
  if (day.getTime() >= startOfToday - 86_400_000) return "yesterday";
  return "older";
}

export interface Totals {
  active: number;
  speed: number;
  connections: number;
  /** Seconds until every running download is done. */
  remaining: number | null;
  downloadedToday: number;
  speedLimit: number | null;
}

export function totals(downloads: Download[], settings: Settings | null): Totals {
  const running = downloads.filter((d) => d.state === "downloading");
  const speed = running.reduce((sum, d) => sum + d.speed, 0);
  const left = running.reduce((sum, d) => sum + (d.size == null ? 0 : Math.max(0, d.size - d.downloaded)), 0);
  const today = new Date().toDateString();
  const finishedToday = downloads
    .filter((d) => d.state === "completed" && d.completedAt && new Date(d.completedAt).toDateString() === today)
    .reduce((sum, d) => sum + (d.size ?? 0), 0);
  return {
    active: running.length,
    speed,
    connections: running.reduce((sum, d) => sum + d.connections, 0),
    remaining: speed > 0 ? left / speed : null,
    downloadedToday: finishedToday + running.reduce((sum, d) => sum + d.downloaded, 0),
    speedLimit: settings && settings.speedLimit > 0 ? settings.speedLimit : null,
  };
}

/** Recent speeds, kept between polls of the download list. */
export interface History {
  /** Total speed, one sample per poll. */
  total: number[];
  /** Speed of each running download. */
  perDownload: Map<number, number[]>;
  /** Smoothed speed of each unfinished segment, keyed by its first byte. */
  segments: Map<number, Map<number, number>>;
  previous: Map<number, { at: number; downloaded: Map<number, number> }>;
}

export const emptyHistory = (): History => ({
  total: [],
  perDownload: new Map(),
  segments: new Map(),
  previous: new Map(),
});

const TOTAL_SAMPLES = 100;
const ROW_SAMPLES = 30;

export function recordHistory(history: History, downloads: Download[], now = Date.now()): History {
  const running = downloads.filter((d) => d.state === "downloading");
  const next: History = {
    total: [...history.total, running.reduce((sum, d) => sum + d.speed, 0)].slice(-TOTAL_SAMPLES),
    perDownload: new Map(),
    segments: new Map(),
    previous: new Map(),
  };
  for (const d of running) {
    next.perDownload.set(d.id, [...(history.perDownload.get(d.id) ?? []), d.speed].slice(-ROW_SAMPLES));
    const before = history.previous.get(d.id);
    const smoothed = new Map<number, number>();
    if (before) {
      const seconds = (now - before.at) / 1000;
      const old = history.segments.get(d.id);
      for (const s of d.segments) {
        if (s.start + s.downloaded >= s.end || seconds <= 0) continue;
        const raw = Math.max(0, (s.downloaded - (before.downloaded.get(s.start) ?? s.downloaded)) / seconds);
        const last = old?.get(s.start);
        smoothed.set(s.start, last == null ? raw : last * 0.6 + raw * 0.4);
      }
    }
    next.segments.set(d.id, smoothed);
    next.previous.set(d.id, { at: now, downloaded: new Map(d.segments.map((s) => [s.start, s.downloaded])) });
  }
  return next;
}

/** Splits a download into `count` equal blocks with how full each one is (0–1). */
export function blocks(d: Download, count = 8): number[] {
  if (d.state === "completed") return Array(count).fill(1);
  if (d.size && d.segments.length > 0) {
    const size = d.size;
    return Array.from({ length: count }, (_, i) => {
      const from = (size / count) * i;
      const to = (size / count) * (i + 1);
      let done = 0;
      for (const s of d.segments) {
        const filledEnd = s.start + s.downloaded;
        done += Math.max(0, Math.min(to, filledEnd) - Math.max(from, s.start));
      }
      return Math.min(1, done / (to - from));
    });
  }
  const fraction = d.size ? d.downloaded / d.size : 0;
  return Array.from({ length: count }, (_, i) => Math.min(1, Math.max(0, fraction * count - i)));
}
