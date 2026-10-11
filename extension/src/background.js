// Supo Download Manager browser extension.
//
// Hands downloads over to the SDM desktop app, which listens on
// http://127.0.0.1:<port>. When the app is not running, the browser keeps
// downloading as usual.

const DEFAULT_PORT = 17360;
const PING_CACHE_MS = 5000;
const PREFETCH_CACHE_MS = 60_000;
const MEDIA_TIMEOUT_MS = 180_000;
const DEFAULT_PREFS = { capture: true, videoButton: true, magnets: true, port: DEFAULT_PORT, minSizeMb: 0 };

// Sites where the video button and menu stay away. The Chrome Web Store
// build fills this in, because the store does not allow extensions that
// download from YouTube; the SDM app itself still can.
const NO_VIDEO_SITES = /* @no-video-sites */ [];

// Used when the app does not send its own list.
const FALLBACK_EXTENSIONS = [
  "7z", "aac", "apk", "avi", "bz2", "deb", "dmg", "exe", "flac", "flv", "gz", "iso", "m4a",
  "m4v", "mkv", "mov", "mp3", "mp4", "msi", "ogg", "pdf", "pkg", "rar", "rpm", "tar", "tgz",
  "torrent", "wav", "webm", "wmv", "xz", "zip",
];

// Content types worth handing over when the file name has no known extension.
const CAPTURE_MIME =
  /^(video|audio)\/|^application\/(zip|x-zip-compressed|x-rar-compressed|vnd\.rar|x-7z-compressed|x-msdownload|x-msi|x-apple-diskimage|vnd\.android\.package-archive|x-iso9660-image|x-tar|gzip|x-gzip|x-bzip2|x-xz|x-bittorrent)$/i;

/** Whether the video button and menu may offer this page. */
function videoAllowed(pageUrl) {
  try {
    const host = new URL(pageUrl).hostname;
    return !NO_VIDEO_SITES.some((site) => host === site || host.endsWith(`.${site}`));
  } catch {
    return false;
  }
}

let lastPing = { at: 0, status: null };
let cachedBrowser = null;
// Links given back to the browser because SDM could not take them.
const givenBack = new Set();
const prefetchedPages = new Map();

async function prefs() {
  return chrome.storage.local.get(DEFAULT_PREFS);
}

async function browserId() {
  if (cachedBrowser) return cachedBrowser;
  const ua = navigator.userAgent;
  const brands = (navigator.userAgentData?.brands ?? []).map((b) => b.brand);
  let id = "chromium";
  if (/firefox\//i.test(ua)) id = "firefox";
  else if (/edg\//i.test(ua)) id = "edge";
  else if (/opr\//i.test(ua)) id = "opera";
  else if (/coc_coc_browser/i.test(ua) || brands.some((b) => /coc ?coc/i.test(b))) id = "coccoc";
  else if (/vivaldi/i.test(ua) || brands.some((b) => /vivaldi/i.test(b))) id = "vivaldi";
  else if (await isBrave()) id = "brave";
  else if (brands.includes("Google Chrome")) id = "chrome";
  cachedBrowser = id;
  return id;
}

async function isBrave() {
  try {
    return Boolean(navigator.brave && (await navigator.brave.isBrave()));
  } catch {
    return false;
  }
}

async function callApp(path, body, timeout = 2500) {
  const { port } = await prefs();
  return fetch(`http://127.0.0.1:${port}${path}`, {
    method: body ? "POST" : "GET",
    headers: {
      "Content-Type": "application/json",
      "X-SDM-Client": "extension",
      "X-SDM-Browser": await browserId(),
    },
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(timeout),
  });
}

/** Asks the app whether it is running. Returns its status or `null`. */
async function ping(fresh = false) {
  if (!fresh && Date.now() - lastPing.at < PING_CACHE_MS) return lastPing.status;
  let status = null;
  try {
    const response = await callApp("/api/ping");
    if (response.ok) status = await response.json();
  } catch {
    // The app is not running.
  }
  lastPing = { at: Date.now(), status };
  chrome.action.setTitle({
    title: status
      ? `${chrome.i18n.getMessage("extName")} ${status.version}`
      : chrome.i18n.getMessage("statusOffline"),
  });
  return status;
}

function extensionOf(name) {
  const last = (name || "").split(/[?#]/)[0].split(/[\\/]/).pop() || "";
  const dot = last.lastIndexOf(".");
  return dot > 0 ? last.slice(dot + 1).toLowerCase() : "";
}

function baseName(path) {
  return (path || "").split(/[\\/]/).pop() || undefined;
}

function sizeOf(item) {
  if (item.totalBytes > 0) return item.totalBytes;
  if (item.fileSize > 0) return item.fileSize;
  return 0;
}

/** Whether a browser download should go to SDM instead. */
function wanted(item, status, settings) {
  const url = item.finalUrl || item.url;
  if (!/^https?:/i.test(url)) return false; // blob:, data: and file: stay in the browser
  if (givenBack.has(item.url) || givenBack.has(url)) return false;
  if (item.byExtensionId && item.byExtensionId === chrome.runtime.id) return false;

  const size = sizeOf(item);
  if (settings.minSizeMb > 0 && size > 0 && size < settings.minSizeMb * 1024 * 1024) return false;

  const extensions = status.captureExtensions?.length ? status.captureExtensions : FALLBACK_EXTENSIONS;
  const ext = extensionOf(item.filename) || extensionOf(url);
  if (ext && extensions.includes(ext)) return true;
  return CAPTURE_MIME.test(item.mime || "");
}

async function cookieHeader(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ") || undefined;
  } catch {
    return undefined;
  }
}

/** Sends a link to the app. Returns whether the app accepted it. */
async function handOver({ url, referrer, fileName, fileSize }) {
  try {
    const response = await callApp("/api/download", {
      url,
      referrer: referrer || undefined,
      cookies: await cookieHeader(url),
      userAgent: navigator.userAgent,
      fileName: fileName || undefined,
      fileSize: fileSize || undefined,
    });
    return response.ok;
  } catch {
    return false;
  }
}

/** The page's cookies, so videos that need a login can be downloaded. */
async function cookieList(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    return cookies.map((c) => ({
      name: c.name,
      value: c.value,
      domain: c.domain,
      path: c.path,
      secure: c.secure,
      hostOnly: c.hostOnly,
      expires: c.expirationDate ?? null,
    }));
  } catch {
    return [];
  }
}

/** Sends a page with a video to the app. Returns whether the app took it. */
async function handOverVideo({ pageUrl, title, mediaUrl, referrer }) {
  if (!videoAllowed(pageUrl)) return false;
  try {
    const response = await callApp("/api/media", {
      pageUrl,
      title: title || undefined,
      mediaUrl: mediaUrl || undefined,
      referrer: referrer || undefined,
      cookies: await cookieList(pageUrl),
      userAgent: navigator.userAgent,
    });
    return response.ok;
  } catch {
    return false;
  }
}

/** Reads qualities or starts the chosen download without opening the app dialog. */
async function videoRequest(message, sender) {
  const { pageUrl } = message;
  if (
    !/^https?:/i.test(pageUrl || "") ||
    !videoAllowed(pageUrl) ||
    !videoAllowed(sender.url || sender.tab?.url)
  ) {
    return { ok: false, error: chrome.i18n.getMessage("videoUnavailable") };
  }
  const [settings, status] = await Promise.all([prefs(), ping()]);
  if (!settings.videoButton || !status?.enabled || !status?.media) {
    return { ok: false, error: chrome.i18n.getMessage("statusOffline") };
  }
  const prefetch = message.type === "video-prefetch";
  const key = pageUrl.split("#")[0];
  const now = Date.now();
  for (const [page, time] of prefetchedPages) {
    if (now - time >= PREFETCH_CACHE_MS) prefetchedPages.delete(page);
  }
  if (prefetch) {
    if (prefetchedPages.has(key)) return { ok: true };
    prefetchedPages.set(key, now);
  }
  try {
    const source = {
      pageUrl,
      title: message.title || undefined,
      mediaUrl: /^https?:/i.test(message.mediaUrl || "") ? message.mediaUrl : undefined,
      referrer: sender.tab?.url !== pageUrl ? sender.tab?.url : undefined,
      cookies: await cookieList(pageUrl),
      userAgent: navigator.userAgent,
    };
    const add = message.type === "video-add";
    const response = await callApp(
      add ? "/api/media/add" : "/api/media/probe",
      add ? { source, option: message.option } : { ...source, wait: !prefetch },
      prefetch ? 2500 : MEDIA_TIMEOUT_MS,
    );
    if (!response.ok) throw new Error((await response.text()) || chrome.i18n.getMessage("videoFailed"));
    if (prefetch) return { ok: true };
    const result = await response.json();
    return add ? { ok: true, id: result.id } : { ok: true, info: result };
  } catch (error) {
    if (prefetch && prefetchedPages.get(key) === now) prefetchedPages.delete(key);
    return { ok: false, error: error instanceof TypeError ? chrome.i18n.getMessage("statusOffline") : error.message };
  }
}

/** Lets the browser download a link after all. */
function giveBack(url) {
  givenBack.add(url);
  setTimeout(() => givenBack.delete(url), 60_000);
  chrome.downloads.download({ url }).catch(() => {});
}

/**
 * Sends the links of a page (or of the selected part) to the app, which
 * lets the user pick the ones to download.
 */
async function handOverLinks(tabId, { frameId = 0, selection = false } = {}) {
  const page = await chrome.tabs
    .sendMessage(tabId, { type: "collect-links", selection }, { frameId })
    .catch(() => null);
  if (!page) return { ok: false, error: chrome.i18n.getMessage("linksReload") };
  if (!page.links?.length) return { ok: false, error: chrome.i18n.getMessage("linksNone") };
  const status = await ping(true);
  if (!status?.enabled || !status?.links) return { ok: false, error: chrome.i18n.getMessage("statusOffline") };
  try {
    const response = await callApp(
      "/api/links",
      {
        pageUrl: page.pageUrl,
        title: page.title || undefined,
        links: page.links,
        cookies: await cookieHeader(page.pageUrl),
        userAgent: navigator.userAgent,
      },
      10_000,
    );
    if (!response.ok) throw new Error((await response.text()) || chrome.i18n.getMessage("statusOffline"));
    return { ok: true, count: page.links.length };
  } catch (error) {
    return { ok: false, error: error instanceof TypeError ? chrome.i18n.getMessage("statusOffline") : error.message };
  }
}

chrome.downloads.onCreated.addListener(async (item) => {
  const settings = await prefs();
  if (!settings.capture || item.state !== "in_progress") return;
  const status = await ping();
  if (!status?.enabled || !wanted(item, status, settings)) return;

  try {
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch {
    // The download may already be gone.
  }
  const accepted = await handOver({
    url: item.finalUrl || item.url,
    referrer: item.referrer,
    fileName: baseName(item.filename),
    fileSize: sizeOf(item),
  });
  if (!accepted) giveBack(item.url);
});

function createMenus() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "sdm-link",
      title: chrome.i18n.getMessage("menuLink"),
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: "sdm-media",
      title: chrome.i18n.getMessage("menuMedia"),
      contexts: ["image", "audio"],
    });
    chrome.contextMenus.create({
      id: "sdm-video",
      title: chrome.i18n.getMessage("menuVideo"),
      contexts: ["video", "page", "frame"],
    });
    chrome.contextMenus.create({
      id: "sdm-all-links",
      title: chrome.i18n.getMessage("menuAllLinks"),
      contexts: ["page", "frame"],
    });
    chrome.contextMenus.create({
      id: "sdm-selected-links",
      title: chrome.i18n.getMessage("menuSelectedLinks"),
      contexts: ["selection"],
    });
  });
}

chrome.runtime.onInstalled.addListener(createMenus);
chrome.runtime.onStartup.addListener(createMenus);

/** Hides the video menu on sites where it may not be offered. */
async function syncVideoMenu(tabId) {
  try {
    const tab = await chrome.tabs.get(tabId);
    chrome.contextMenus.update("sdm-video", { visible: videoAllowed(tab.url || "") }, () => void chrome.runtime.lastError);
  } catch {
    // The tab is gone.
  }
}

if (NO_VIDEO_SITES.length > 0) {
  chrome.tabs.onActivated.addListener(({ tabId }) => syncVideoMenu(tabId));
  chrome.tabs.onUpdated.addListener((tabId, change, tab) => {
    if (change.url && tab.active) syncVideoMenu(tabId);
  });
}

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "sdm-all-links" || info.menuItemId === "sdm-selected-links") {
    if (tab?.id != null) {
      await handOverLinks(tab.id, { frameId: info.frameId ?? 0, selection: info.menuItemId === "sdm-selected-links" });
    }
    return;
  }
  if (info.menuItemId === "sdm-video") {
    const pageUrl = info.frameUrl || info.pageUrl || tab?.url;
    if (!pageUrl || !/^https?:/i.test(pageUrl)) return;
    await handOverVideo({
      pageUrl,
      title: tab?.title,
      mediaUrl: /^https?:/i.test(info.srcUrl || "") ? info.srcUrl : null,
    });
    return;
  }
  const url = info.menuItemId === "sdm-link" ? info.linkUrl : info.srcUrl;
  if (!url || !/^(https?|magnet):/i.test(url)) return;
  const accepted = await handOver({ url, referrer: info.pageUrl || tab?.url });
  // The browser cannot download magnet links itself.
  if (!accepted && !/^magnet:/i.test(url)) giveBack(url);
});

// Tell the app every minute that the extension is installed and working.
chrome.alarms.create("ping", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "ping") ping(true);
});
ping(true);

chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (message?.type === "status") {
    ping(true).then(reply);
    return true; // reply asynchronously
  }
  if (message?.type === "video-enabled") {
    Promise.all([prefs(), ping()]).then(([settings, status]) =>
      reply(
        Boolean(
          settings.videoButton && status?.enabled && status?.media && videoAllowed(sender.tab?.url || sender.url),
        ),
      ),
    );
    return true;
  }
  if (message?.type === "magnets-enabled") {
    Promise.all([prefs(), ping()]).then(([settings, status]) =>
      reply(Boolean(settings.magnets && status?.enabled && status?.torrent)),
    );
    return true;
  }
  if (message?.type === "magnet" && /^magnet:\?/i.test(message.url || "")) {
    handOver({ url: message.url, referrer: sender.tab?.url }).then((ok) => reply({ ok }));
    return true;
  }
  if (message?.type === "video" && /^https?:/i.test(message.pageUrl || "")) {
    handOverVideo({
      pageUrl: message.pageUrl,
      title: message.title,
      mediaUrl: message.mediaUrl,
      referrer: sender.tab?.url !== message.pageUrl ? sender.tab?.url : undefined,
    }).then((ok) => reply({ ok }));
    return true;
  }
  if (message?.type === "page-links") {
    chrome.tabs
      .query({ active: true, currentWindow: true })
      .then(([tab]) => (tab?.id != null ? handOverLinks(tab.id) : { ok: false, error: chrome.i18n.getMessage("linksNone") }))
      .then(reply)
      .catch(() => reply({ ok: false, error: chrome.i18n.getMessage("statusOffline") }));
    return true;
  }
  if (["video-prefetch", "video-probe", "video-add"].includes(message?.type)) {
    videoRequest(message, sender).then(reply).catch(() => reply({ ok: false, error: chrome.i18n.getMessage("videoFailed") }));
    return true;
  }
  return false;
});
