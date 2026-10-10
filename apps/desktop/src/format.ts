import type { Language } from "./i18n";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number | null | undefined, language: Language): string {
  if (bytes == null) return "";
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  const number = new Intl.NumberFormat(language, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
  return `${number} ${UNITS[unit]}`;
}

/** Seconds as `1:02:05` or `2:05`. */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return "";
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rest = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${rest}` : `${m}:${rest}`;
}

/** Only the time for today, the date and time otherwise. */
export function formatDate(ms: number, language: Language): string {
  const today = new Date().toDateString() === new Date(ms).toDateString();
  return new Intl.DateTimeFormat(
    language,
    today ? { timeStyle: "short" } : { dateStyle: "short", timeStyle: "short" },
  ).format(ms);
}

export function percent(downloaded: number, size: number | null): number | null {
  if (!size) return size === 0 ? 100 : null;
  return Math.min(100, (downloaded / size) * 100);
}

/** A friendly duration: "45 giây", "6 phút", "1 giờ 20 phút". */
export function formatHuman(seconds: number | null | undefined, language: Language): string {
  if (seconds == null || !Number.isFinite(seconds)) return "";
  const s = Math.max(1, Math.round(seconds));
  const vi = language === "vi";
  if (s < 60) return vi ? `${s} giây` : `${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return vi ? `${m} phút` : `${m} min`;
  const h = Math.floor(m / 60);
  const rest = m % 60;
  if (vi) return rest ? `${h} giờ ${rest} phút` : `${h} giờ`;
  return rest ? `${h} h ${rest} min` : `${h} h`;
}

/** A speed with "/s", e.g. "9,9 MB/s". */
export function formatSpeed(bytesPerSecond: number, language: Language): string {
  return `${formatBytes(bytesPerSecond, language)}/s`;
}
