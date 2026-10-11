import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const code = readFileSync(new URL("../src/background.js", import.meta.url), "utf8");
const release = "https://github.com/owner/repo/releases/download/v1/setup.exe";
const asset = "https://release-assets.githubusercontent.com/github-production-release-asset/123/uuid?signature=test";
const item = (overrides = {}) => ({ id: 1, url: release, finalUrl: asset, filename: "C:\\Downloads\\setup.exe", mime: "application/octet-stream", state: "in_progress", totalBytes: 4096, referrer: "https://github.com/owner/repo/releases", ...overrides });

async function harness({ settings = {}, enabled = true, extensions = ["exe", "zip"], accepted = true, cancelError = false, eraseError = false } = {}) {
  const listeners = {}, calls = [], cancelled = [], erased = [], returned = [], items = new Map();
  const event = { addListener() {} };
  const prefs = { ...settings };
  const downloadEvent = (name) => ({ addListener(fn) { listeners[name] = fn; } });
  const chrome = {
    runtime: { id: "sdm-test", onInstalled: event, onStartup: event, onMessage: { addListener(fn) { listeners.message = fn; } } },
    storage: { local: { async get(defaults) { return { ...defaults, ...prefs }; } } },
    cookies: { async getAll({ url }) { return [{ name: new URL(url).hostname === "github.com" ? "github-session" : "asset-session", value: "test-only" }]; } },
    downloads: {
      onCreated: downloadEvent("created"), onChanged: downloadEvent("changed"), onDeterminingFilename: downloadEvent("determining"),
      async cancel(id) { if (cancelError) throw new Error("already complete"); cancelled.push(id); },
      async erase({ id }) { if (eraseError) throw new Error("history unavailable"); erased.push(id); return [id]; },
      async search({ id }) { return items.has(id) ? [items.get(id)] : []; },
      async download(options) { returned.push(options); return 99; },
    },
    contextMenus: { onClicked: event }, tabs: { onActivated: event, onUpdated: event },
    alarms: { create() {}, onAlarm: event }, action: { setTitle() {} }, i18n: { getMessage: key => key },
  };
  vm.runInNewContext(code, {
    chrome, navigator: { userAgent: "Chrome capture test" }, URL, AbortSignal, TypeError,
    setTimeout(...args) { const timer = setTimeout(...args); timer.unref(); return timer; },
    async fetch(url, init) {
      const path = new URL(url).pathname;
      if (path === "/api/ping") return Response.json({ enabled, captureExtensions: extensions, version: "fixture" });
      calls.push({ path, body: JSON.parse(init.body) });
      return new Response(null, { status: accepted ? 202 : 503 });
    },
  });
  await new Promise(resolve => listeners.message({ type: "status" }, {}, resolve));
  const created = async download => { items.set(download.id, download); await listeners.created(download); };
  const determining = download => new Promise(resolve => {
    items.set(download.id, download);
    let suggestions = 0;
    assert.equal(listeners.determining(download, suggestion => { suggestions++; resolve({ suggestions, suggestion }); }), true);
  });
  const changed = async (download, delta) => { items.set(download.id, download); await listeners.changed({ id: download.id, ...delta }); };
  return { created, determining, changed, calls, cancelled, erased, returned, prefs };
}

test("GitHub EXE waits for the filename and uses the original URL and GitHub cookies", async () => {
  const app = await harness();
  await app.created(item({ filename: "" }));
  assert.equal(app.calls.length, 0);
  const suggestion = await app.determining(item());
  assert.equal(suggestion.suggestions, 1);
  assert.equal(suggestion.suggestion, undefined);
  assert.deepEqual(app.cancelled, [1]);
  assert.deepEqual(app.calls.map(c => c.path), ["/api/download"]);
  assert.equal(app.calls[0].body.url, release);
  assert.equal(app.calls[0].body.cookies, "github-session=test-only");
  assert.equal(app.calls[0].body.fileName, "setup.exe");
  assert.equal(app.calls[0].body.fileSize, 4096);
  assert.equal(app.calls[0].body.referrer, item().referrer);
});

test("GitHub ZIP is captured after a redirect to an opaque octet-stream URL", async () => {
  const app = await harness();
  const zip = item({ url: release.replace("setup.exe", "bundle.zip"), filename: "" });
  await app.created(zip);
  await app.determining({ ...zip, filename: "bundle.zip" });
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].body.url, zip.url);
  assert.equal(app.calls[0].body.fileName, "bundle.zip");
});

test("a later filename update can capture a download with no suffix in either URL", async () => {
  const app = await harness();
  const initial = item({ url: "https://example.org/download/123", filename: "" });
  await app.created(initial);
  await app.changed({ ...initial, filename: "archive.zip" }, { filename: { current: "archive.zip" } });
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].body.fileName, "archive.zip");
});

test("creation, filename determination and change events hand a download over once", async () => {
  const app = await harness();
  await Promise.all([app.created(item()), app.determining(item()), app.changed(item(), { filename: { current: item().filename } })]);
  assert.deepEqual(app.cancelled, [1]);
  assert.equal(app.calls.length, 1);
});

test("disabled capturing or app integration leaves the file in Chrome", async () => {
  for (const options of [{ settings: { capture: false } }, { enabled: false }]) {
    const app = await harness(options);
    assert.equal((await app.determining(item())).suggestions, 1);
    assert.equal(app.cancelled.length, 0);
    assert.equal(app.calls.length, 0);
  }
});

test("configured extensions and a minimum size are respected", async () => {
  for (const options of [{ extensions: ["pdf"] }, { extensions: [] }, { settings: { minSizeMb: 1 } }]) {
    const app = await harness(options);
    await app.determining(item());
    assert.equal(app.cancelled.length, 0);
    assert.equal(app.calls.length, 0);
  }
});

test("a rejected handover returns the original link to Chrome without recapturing it", async () => {
  const app = await harness({ accepted: false });
  await app.determining(item());
  assert.equal(app.returned.length, 1);
  assert.equal(app.returned[0].url, release);
  await app.created(item({ id: 2 }));
  await app.determining(item({ id: 2 }));
  assert.equal(app.calls.length, 1);
});

test("a download that Chrome already completed is not duplicated in SDM", async () => {
  const app = await harness({ cancelError: true });
  assert.equal((await app.determining(item())).suggestions, 1);
  assert.equal(app.calls.length, 0);
});

test("a history cleanup failure does not prevent a successful handover", async () => {
  const app = await harness({ eraseError: true });
  await app.determining(item());
  assert.equal(app.calls.length, 1);
  assert.equal(app.returned.length, 0);
});

test("blob downloads, completed files and extension-initiated fallback downloads stay in Chrome", async () => {
  for (const overrides of [{ url: "blob:https://github.com/file", finalUrl: "blob:https://github.com/file" }, { state: "complete" }, { byExtensionId: "sdm-test" }]) {
    const app = await harness();
    await app.determining(item(overrides));
    assert.equal(app.cancelled.length, 0);
    assert.equal(app.calls.length, 0);
  }
});
