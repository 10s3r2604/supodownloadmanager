// Supo Download Manager browser extension.
//
// Native Messaging connects to the desktop without a user-configured port.
// Older desktop builds retain their loopback bridge during migration.

const DEFAULT_PORT = 17360;
const NATIVE_HOST = "com.supodownloadmanager.bridge";
let connectionTransport = "none";
let connectionFailure = null;
let nativeAvailable = false;
const PING_CACHE_MS = 5000;
// Adding a video can read the page again or fetch ffmpeg first.
const MEDIA_ADD_TIMEOUT_MS = 180_000;
const KEEP_AWAKE_MS = 20_000;
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

function isXPage(pageUrl) {
  try {
    return ["x.com", "www.x.com", "twitter.com", "www.twitter.com", "mobile.twitter.com"].includes(new URL(pageUrl).hostname);
  } catch { return false; }
}

let lastPing = { at: 0, status: null };
let pingRequest = 0;
let cachedBrowser = null;
// Links given back to the browser because SDM could not take them.
const givenBack = new Set();
const captureJobs = new Map();
const capturedDownloads = new Set();

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

class ConnectionError extends Error {
  constructor(code, unavailable = false) {
    super(chrome.i18n.getMessage(code === "appNotRunning" ? "statusOffline" : "statusConnectionError"));
    this.code = code;
    this.unavailable = unavailable;
  }
}

async function nativeMessage(message, timeout) {
  if (typeof chrome.runtime.sendNativeMessage !== "function") throw new ConnectionError("notConfigured", true);
  let timer;
  try {
    const result = await Promise.race([
      chrome.runtime.sendNativeMessage(NATIVE_HOST, message),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new ConnectionError("timeout")), timeout); }),
    ]);
    nativeAvailable = true;
    if (result?.error) throw new ConnectionError(result.error, result.error === "notConfigured");
    return result;
  } catch (error) {
    if (error instanceof ConnectionError) throw error;
    // Only failures before host dispatch permit the compatibility request.
    // An uncertain reply/timeout must never submit the same download twice.
    const missing = /native messaging host.*not found|host.*is not registered|access.*host.*forbidden|no such native application/i.test(error?.message || "");
    throw new ConnectionError(missing ? "notConfigured" : "connectionError", missing);
  } finally { clearTimeout(timer); }
}

async function callApp(path, body, timeout = 2500, extraHeaders = {}) {
  // A confirmed older app stays on its supported transport. In particular,
  // an old executable left in a host registration cannot receive a POST twice.
  if (path !== "/api/ping" && connectionTransport === "compatibility" && lastPing.status && !lastPing.status.nativeMessaging) {
    return callCompatibility(path, body, timeout, extraHeaders);
  }
  try {
    const result = await nativeMessage({action:"request", path, browser:await browserId(), origin:(chrome.runtime.getURL?.("") || "").replace(/\/$/, ""), body:body ?? null, headers:extraHeaders, timeoutMs:timeout}, timeout);
    if (!Number.isInteger(result?.status) || typeof result.body !== "string") throw new ConnectionError("connectionError");
    connectionTransport = "native"; connectionFailure = null;
    return {ok:result.status >= 200 && result.status < 300, status:result.status, json:async()=>JSON.parse(result.body), text:async()=>result.body};
  } catch (error) {
    connectionFailure = error.code || "connectionError";
    if (!error.unavailable && path !== "/api/ping") throw error;
  }
  return callCompatibility(path, body, timeout, extraHeaders);
}

async function callCompatibility(path, body, timeout, extraHeaders) {
  const { port } = await prefs();
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method: body ? "POST" : "GET",
    headers: {
      "Content-Type": "application/json",
      "X-SDM-Client": "extension",
      "X-SDM-Browser": await browserId(),
      "X-SDM-Extension-ID": chrome.runtime.id,
      ...extraHeaders,
    },
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(timeout),
  });
  connectionTransport = "compatibility"; connectionFailure = null;
  return response;
}

/** Asks the app whether it is running. Returns its status or `null`. */
async function ping(fresh = false) {
  if (!fresh && Date.now() - lastPing.at < PING_CACHE_MS) return lastPing.status;
  const request = ++pingRequest;
  let status = null;
  try {
    const response = await callApp("/api/ping");
    if (response.ok) status = await response.json();
  } catch {
    // The app is not running.
  }
  // A slow startup check on the previous port must not overwrite a newer
  // successful connection and hide the video button for its cache interval.
  if (request !== pingRequest) return lastPing.status;
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

  const extensions = Array.isArray(status.captureExtensions) ? status.captureExtensions : FALLBACK_EXTENSIONS;
  const names = [item.filename, item.url, item.finalUrl].filter(Boolean);
  const suffixes = names.map(extensionOf).filter(Boolean);
  if (suffixes.some((ext) => extensions.includes(ext))) return true;
  if (suffixes.length) return false;
  if (!extensions.length) return false;
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
    // X videos are selected in the page menu. Never send its Home feed to
    // the desktop picker, including messages from an older content script.
    if (isXPage(pageUrl)) return false;
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

/** Short requests let content scripts resume jobs after an MV3 worker restart. */
async function videoRequest(message, sender) {
  const probeHeaders = {
    "X-SDM-Probe-Owner": `${chrome.runtime.id}:${sender.tab?.id ?? "extension"}:${sender.frameId ?? 0}:${sender.documentId || ""}`,
  };
  const statusRequest = message.type === "video-probe-status";
  const cancel = message.type === "video-probe-cancel";
  try {
    if (statusRequest || cancel) {
      const response = await callApp(cancel ? "/api/media/probe/cancel" : "/api/media/probe/status",
        { clientId: message.clientId }, 2500, probeHeaders);
      if (!response.ok) throw new Error((await response.text()) || chrome.i18n.getMessage("videoFailed"));
      if (cancel) return { ok: true };
      const result = await response.json();
      return result.state === "error" ? { ok: false, error: result.error } : { ok: true, ...result };
    }
    const { pageUrl } = message;
    if (isXPage(pageUrl) && !/^\/(?:[^/]+\/status|i\/web\/status|statuses)\/\d+(?:\/|$)/.test(new URL(pageUrl).pathname)) {
      return { ok: false, error: chrome.i18n.getMessage("videoPostNotFound") };
    }
    if (!/^https?:/i.test(pageUrl || "") || !videoAllowed(pageUrl) || !videoAllowed(sender.url || sender.tab?.url)) {
      return { ok: false, error: chrome.i18n.getMessage("videoUnavailable") };
    }
    const [settings, status] = await Promise.all([prefs(), ping()]);
    if (!settings.videoButton || !status?.enabled || !status?.media) {
      return { ok: false, error: chrome.i18n.getMessage("statusOffline") };
    }
    const add = message.type === "video-add";
    // Older desktops retain their legacy protocol for older extensions. A new
    // extension asks the user to update instead of holding an MV3 fetch open.
    if (!add && !status.mediaProbeJobs) return { ok: false, error: chrome.i18n.getMessage("videoFailed") };
    const source = {
      pageUrl,
      title: message.title || undefined,
      mediaUrl: /^https?:/i.test(message.mediaUrl || "") ? message.mediaUrl : undefined,
      referrer: sender.tab?.url !== pageUrl ? sender.tab?.url : undefined,
      cookies: await cookieList(pageUrl),
      userAgent: navigator.userAgent,
    };
    const response = add
      ? await keepingAwake(() => callApp("/api/media/add", { source, option: message.option }, MEDIA_ADD_TIMEOUT_MS, probeHeaders))
      : await callApp("/api/media/probe/start", { source, clientId: message.clientId }, 2500, probeHeaders);
    if (!response.ok) throw new Error((await response.text()) || chrome.i18n.getMessage("videoFailed"));
    const result = await response.json();
    if (add) return { ok: true, id: result.id };
    return result.state === "error" ? { ok: false, error: result.error } : { ok: true, ...result };
  } catch (error) {
    return { ok: false, error: error instanceof TypeError ? chrome.i18n.getMessage("statusOffline") : error.message };
  }
}

/**
 * Chrome stops an idle extension service worker after 30 seconds, even
 * while it waits for a reply. Calling an extension API now and then keeps
 * it awake during a slow request.
 */
async function keepingAwake(task) {
  let done = false;
  const tick = () => {
    if (done) return;
    chrome.runtime.getPlatformInfo?.()?.catch?.(() => {});
    setTimeout(tick, KEEP_AWAKE_MS)?.unref?.();
  };
  setTimeout(tick, KEEP_AWAKE_MS)?.unref?.();
  try {
    return await task();
  } finally {
    done = true;
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

// Downloads the user resumed in the browser. Taking one over would erase
// the part Chrome already has, so they stay in the browser.
const resumedInBrowser = new Set();
const STARTED_LONG_AGO_MS = 60_000;

/** A download that was already running before, not one that just started. */
function alreadyUnderway(item) {
  if (resumedInBrowser.has(item.id)) return true;
  const started = Date.parse(item.startTime || "");
  return item.bytesReceived > 0 && started > 0 && Date.now() - started > STARTED_LONG_AGO_MS;
}

async function captureDownload(item) {
  if (capturedDownloads.has(item.id) || item.state !== "in_progress" || !item.filename) return;
  if (alreadyUnderway(item)) return;
  const settings = await prefs();
  if (!settings.capture) return;
  const status = await ping();
  if (!status?.enabled || !wanted(item, status, settings)) return;

  try {
    await chrome.downloads.cancel(item.id);
  } catch {
    // Do not also create an SDM download if Chrome already completed it.
    return;
  }
  capturedDownloads.add(item.id);
  setTimeout(() => capturedDownloads.delete(item.id), 60_000);
  await chrome.downloads.erase({ id: item.id }).catch(() => {});
  // Keep the original request URL and its cookies. GitHub's final release
  // asset URL is signed, temporary and no longer belongs to github.com.
  const url = /^https?:/i.test(item.url || "") ? item.url : item.finalUrl;
  const accepted = await handOver({
    url,
    referrer: item.referrer,
    fileName: baseName(item.filename),
    fileSize: sizeOf(item),
  });
  if (!accepted) giveBack(url);
}

// Chrome may create the item before naming it, then fire several events
// for the same download. Serialize those snapshots to hand it over once.
function queueCapture(item) {
  const previous = captureJobs.get(item.id) || Promise.resolve();
  const job = previous.catch(() => {}).then(() => captureDownload(item));
  captureJobs.set(item.id, job);
  job.finally(() => {
    if (captureJobs.get(item.id) === job) captureJobs.delete(item.id);
  }).catch(() => {});
  return job;
}

chrome.downloads.onCreated.addListener((item) => queueCapture(item));
chrome.downloads.onDeterminingFilename?.addListener((item, suggest) => {
  // This event has the response's tentative filename, before Save As.
  // Always release it, including skipped downloads and connection failures.
  queueCapture(item).finally(() => suggest()).catch(() => {});
  return true;
});
chrome.downloads.onChanged.addListener(async (delta) => {
  if (capturedDownloads.has(delta.id)) return;
  if (delta.state?.previous === "interrupted") {
    resumedInBrowser.add(delta.id);
    return;
  }
  if (!delta.filename && !delta.url && !delta.finalUrl && !delta.mime && !delta.fileSize && !delta.totalBytes && delta.state?.current !== "in_progress") return;
  const [item] = await chrome.downloads.search({ id: delta.id }).catch(() => []);
  if (item) await queueCapture(item);
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
    if (isXPage(pageUrl)) {
      if (tab?.id != null) await chrome.tabs.sendMessage(tab.id,
        { type: "video-open-menu", mediaUrl: info.srcUrl }, { frameId: info.frameId ?? 0 }).catch(() => {});
      return;
    }
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
  const popupUrl = chrome.runtime.getURL?.("popup.html");
  const extensionPage = !sender.tab || (typeof popupUrl === "string" && sender.url === popupUrl);
  if (message?.type === "connection-status" && extensionPage) {
    ping(true).then(status=>reply({status, transport:status ? connectionTransport : "none", error:connectionFailure, canLaunch:nativeAvailable}));
    return true;
  }
  if (message?.type === "open-app" && extensionPage) {
    nativeMessage({action:"launch"}, 5000).then(result=>reply({ok:result?.launched === true})).catch(()=>reply({ok:false}));
    return true;
  }
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
  if (["video-prefetch", "video-probe", "video-probe-status", "video-probe-cancel", "video-add"].includes(message?.type)) {
    videoRequest(message, sender).then(reply).catch(() => reply({ ok: false, error: chrome.i18n.getMessage("videoFailed") }));
    return true;
  }
  return false;
});
