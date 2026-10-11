import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const code = () => readFileSync(new URL("../src/background.js", import.meta.url), "utf8");
const sender = { url: "https://video.example/watch", tab: { id: 17, url: "https://video.example/watch" }, frameId: 2, documentId: "document-a" };
const page = { pageUrl: sender.url, title: "Clip", mediaUrl: "https://cdn.example/clip.mp4" };

// The desktop keeps client attachments independently of the worker lifetime.
// Reusing this fake app across worker VMs catches accidental in-memory ownership.
function desktop() {
  const clients = new Map();
  let error = null;
  return {
    clients,
    fail(value) { error = value; },
    complete(clientId) { clients.get(clientId).result = { state: "done", info: { options: [{ id: "v1080", kind: "video" }] } }; },
    async fetch(path, body) {
      if (error) return new Response(error, { status: 422 });
      if (path === "/api/media/probe/start") {
        if (!clients.has(body.clientId)) clients.set(body.clientId, { source: body.source, result: { state: "pending" } });
        return Response.json(clients.get(body.clientId).result);
      }
      if (path === "/api/media/probe/status") {
        const client = clients.get(body.clientId);
        return client ? Response.json(client.result) : new Response("Probe is no longer available", { status: 404 });
      }
      if (path === "/api/media/probe/cancel") {
        clients.delete(body.clientId);
        return Response.json({ cancelled: true });
      }
      if (path === "/api/media/add") return Response.json({ id: 42 });
      if (path === "/api/media") return new Response(null, { status: 202 });
      assert.fail(`Unexpected desktop route ${path}`);
    },
  };
}

async function background({ blocked = [], settings = {}, app = desktop(), mediaProbeJobs = true, deferStartupPing = false } = {}) {
  const calls = [];
  const deadlines = new WeakMap();
  let listener;
  let contextListener;
  const tabMessages = [];
  let now = 100_000;
  let startupPing;
  let pingCount = 0;
  let cookieReads = 0;
  const event = { addListener() {} };
  const chrome = {
    storage: { local: { async get(defaults) { return { ...defaults, ...settings }; } } },
    runtime: { id: "sdm-test", onInstalled: event, onStartup: event, onMessage: { addListener(fn) { listener = fn; } } },
    cookies: { async getAll() { ++cookieReads; return [{ name: "session", value: "test-only", domain: "video.example", path: "/", secure: true, hostOnly: true }]; } },
    i18n: { getMessage: (key) => key },
    downloads: { onCreated: event, onChanged: event, onDeterminingFilename: event },
    contextMenus: { onClicked: { addListener(fn) { contextListener = fn; } } },
    tabs: { onActivated: event, onUpdated: event, async sendMessage(...args) { tabMessages.push(args); return { ok: true }; } },
    alarms: { create() {}, onAlarm: event },
    action: { setTitle() {} },
  };
  const message = (value, from = sender) => new Promise((resolve) => {
    assert.equal(listener(value, from, resolve), true);
  });
  vm.runInNewContext(code().replace("/* @no-video-sites */ []", JSON.stringify(blocked)), {
    chrome, navigator: { userAgent: "SDM extension test", userAgentData: { brands: [] } },
    URL, TypeError, setTimeout,
    AbortSignal: { timeout(ms) {
      const signal = new AbortController().signal;
      deadlines.set(signal, ms);
      return signal;
    } },
    Date: class extends Date { static now() { return now; } },
    async fetch(url, init) {
      const path = new URL(url).pathname;
      const body = init.body ? JSON.parse(init.body) : null;
      calls.push({ path, body, timeout: deadlines.get(init.signal), headers: init.headers });
      if (path === "/api/ping") {
        if (++pingCount === 1 && deferStartupPing) return new Promise(resolve => {
          startupPing = () => resolve(new Response(null, { status: 503 }));
        });
        return Response.json({ enabled: true, media: true, mediaProbeJobs, version: "0.5.0" });
      }
      return app.fetch(path, body);
    },
  });
  await message({ type: "status" });
  return { message, calls, tabMessages, desktop: app, finishStartupPing: () => startupPing(), context: (info, tab) => contextListener(info, tab), advance: (ms) => { now += ms; }, fail: (value) => app.fail(value), cookieReads: () => cookieReads };
}

const mediaCalls = (app) => app.calls.filter(({ path }) => path !== "/api/ping");

test("prefetch and quality promotion retain the caller's client ID with authenticated source", async () => {
  const app = await background();
  const clientId = "prefetch-owner";
  assert.equal((await app.message({ type: "video-prefetch", ...page, clientId })).ok, true);
  assert.equal((await app.message({ type: "video-probe", ...page, clientId })).state, "pending");
  assert.equal(mediaCalls(app).length, 2);
  for (const { path, body, timeout, headers } of mediaCalls(app)) {
    assert.equal(path, "/api/media/probe/start");
    assert.deepEqual(Object.keys(body).sort(), ["clientId", "source"]);
    assert.equal(body.clientId, clientId);
    assert.equal(body.source.pageUrl, page.pageUrl);
    assert.equal(body.source.mediaUrl, page.mediaUrl);
    assert.equal(body.source.cookies[0].name, "session");
    assert.equal(body.source.userAgent, "SDM extension test");
    assert.equal(headers["X-SDM-Probe-Owner"], "sdm-test:17:2:document-a");
    assert.ok(timeout > 0 && timeout <= 2500, `start timeout ${timeout} must be short`);
  }
  assert.equal(app.desktop.clients.size, 1);
});

test("selected downloads submit the original authenticated source", async () => {
  const app = await background();
  const added = await app.message({ type: "video-add", ...page, option: "v1080" });
  assert.equal(added.id, 42);
  const last = mediaCalls(app).at(-1);
  assert.equal(last.path, "/api/media/add");
  assert.equal(last.body.source.pageUrl, page.pageUrl);
  assert.equal(last.body.source.cookies[0].value, "test-only");
  assert.equal(last.body.option, "v1080");
});

test("qualities can take more than 30 seconds while all bridge requests remain short", async () => {
  const app = await background();
  const clientId = "slow-owner";
  const start = await app.message({ type: "video-probe", ...page, clientId });
  assert.equal(start.ok, true);
  assert.equal(start.state, "pending");
  app.advance(45_001);
  const pending = await app.message({ type: "video-probe-status", clientId });
  assert.equal(pending.ok, true);
  assert.equal(pending.state, "pending");
  app.desktop.complete(clientId);
  const result = await app.message({ type: "video-probe-status", clientId });
  assert.equal(result.ok, true);
  assert.equal(result.state, "done");
  assert.equal(result.info.options[0].id, "v1080");
  assert.deepEqual(mediaCalls(app).map(call => call.path), ["/api/media/probe/start", "/api/media/probe/status", "/api/media/probe/status"]);
  for (const { timeout } of mediaCalls(app)) assert.ok(timeout > 0 && timeout <= 2500, `probe timeout ${timeout} must be short`);
});

test("a replacement worker resumes a job with only the content script's client ID", async () => {
  const app = desktop();
  const clientId = "survives-restart";
  const first = await background({ app });
  assert.equal((await first.message({ type: "video-probe", ...page, clientId })).state, "pending");
  const replacement = await background({ app });
  replacement.advance(60_001);
  assert.equal((await replacement.message({ type: "video-probe-status", clientId })).state, "pending");
  app.complete(clientId);
  assert.equal((await replacement.message({ type: "video-probe-status", clientId })).info.options[0].id, "v1080");
  assert.deepEqual(mediaCalls(replacement).map(call => call.body), [{ clientId }, { clientId }]);
  assert.equal(replacement.cookieReads(), 0);
});

test("cancelling one menu leaves another owner's attachment to the same source intact", async () => {
  const app = await background();
  const other = { ...sender, tab: { id: 18, url: sender.tab.url }, frameId: 3, documentId: "document-b" };
  await Promise.all([
    app.message({ type: "video-probe", ...page, clientId: "owner-a" }),
    app.message({ type: "video-probe", ...page, clientId: "owner-b" }, other),
  ]);
  assert.equal(app.desktop.clients.size, 2);
  const cancelled = await app.message({ type: "video-probe-cancel", clientId: "owner-a" });
  assert.equal(cancelled.ok, true);
  assert.equal(app.desktop.clients.has("owner-a"), false);
  assert.equal(app.desktop.clients.has("owner-b"), true);
  assert.equal((await app.message({ type: "video-probe-status", clientId: "owner-b" }, other)).state, "pending");
  app.desktop.complete("owner-b");
  assert.equal((await app.message({ type: "video-probe-status", clientId: "owner-b" }, other)).state, "done");
  const calls = mediaCalls(app).filter(call => call.path === "/api/media/probe/cancel");
  assert.deepEqual(calls.map(call => call.body), [{ clientId: "owner-a" }]);
  assert.equal(calls[0].headers["X-SDM-Probe-Owner"], "sdm-test:17:2:document-a");
  const otherCalls = mediaCalls(app).filter(call => call.body.clientId === "owner-b");
  assert.ok(otherCalls.every(call => call.headers["X-SDM-Probe-Owner"] === "sdm-test:18:3:document-b"));
  assert.ok(calls[0].timeout > 0 && calls[0].timeout <= 2500);
});

test("terminal desktop job errors become failed content-script replies", async () => {
  const app = await background();
  const clientId = "terminal-error-owner";
  await app.message({ type: "video-probe", ...page, clientId });
  app.desktop.clients.get(clientId).result = { state: "error", error: "This page needs a login" };
  const result = await app.message({ type: "video-probe-status", clientId });
  assert.equal(result.ok, false);
  assert.equal(result.error, "This page needs a login");
});

test("status and cancellation carry no source credentials and do not reread cookies", async () => {
  const app = await background();
  const clientId = "credentials-owner";
  const from = { url: "https://embed.example/frame", tab: { id: 17, url: "https://embed.example/watch" } };
  await app.message({ type: "video-probe", ...page, clientId }, from);
  const source = mediaCalls(app)[0].body.source;
  assert.equal(source.referrer, from.tab.url);
  assert.equal(source.cookies[0].value, "test-only");
  assert.equal(source.userAgent, "SDM extension test");
  assert.equal(app.cookieReads(), 1);
  await app.message({ type: "video-probe-status", ...page, clientId }, from);
  await app.message({ type: "video-probe-cancel", ...page, clientId }, from);
  assert.equal(app.cookieReads(), 1);
  assert.deepEqual(mediaCalls(app).slice(1).map(call => call.body), [{ clientId }, { clientId }]);
});

test("old desktop apps fail discovery promptly without a long legacy fetch", async () => {
  const app = await background({ mediaProbeJobs: false });
  for (const type of ["video-prefetch", "video-probe"]) {
    const result = await app.message({ type, ...page, clientId: "legacy-owner" });
    assert.equal(result.ok, false);
    assert.equal(result.error, "videoFailed");
  }
  assert.equal(mediaCalls(app).length, 0);
});

test("app errors are preserved and failed prefetches may retry", async () => {
  const app = await background();
  app.fail("This page needs a login");
  const result = await app.message({ type: "video-prefetch", ...page, clientId: "retry-owner" });
  assert.equal(result.error, "This page needs a login");
  app.fail(null);
  assert.equal((await app.message({ type: "video-prefetch", ...page, clientId: "retry-owner" })).ok, true);
  assert.equal(mediaCalls(app).length, 2);
});

test("the store build refuses YouTube and embedded YouTube frames for every menu action", async () => {
  const app = await background({ blocked: ["youtube.com", "youtu.be"] });
  for (const type of ["video-prefetch", "video-probe", "video-add"]) {
    assert.equal((await app.message({ type, pageUrl: "https://www.youtube.com/watch?v=clip", clientId: "blocked-owner" })).ok, false);
    assert.equal((await app.message({ type, ...page, clientId: "blocked-owner" }, { ...sender, url: "https://www.youtube.com/embed/clip" })).ok, false);
  }
  assert.equal(mediaCalls(app).length, 0);
});

test("disabled video buttons do not read formats in the background", async () => {
  const app = await background({ settings: { videoButton: false } });
  assert.equal((await app.message({ type: "video-prefetch", ...page, clientId: "disabled-owner" })).ok, false);
  assert.equal(mediaCalls(app).length, 0);
});

test("X timelines never reach discovery or the desktop picker, including legacy handovers", async () => {
  const app = await background();
  const from = { url: "https://x.com/home", tab: { id: 17, url: "https://x.com/home" } };
  for (const type of ["video-prefetch", "video-probe", "video-add", "video"]) {
    assert.equal((await app.message({ type, pageUrl: from.url, clientId: "timeline-owner" }, from)).ok, false);
  }
  assert.equal((await app.message({ type: "video", pageUrl: "https://x.com/a/status/123" }, from)).ok, false);
  assert.equal(mediaCalls(app).length, 0);
});

test("X quality discovery and selected downloads stay on the quiet bridge routes", async () => {
  const app = await background();
  const from = { url: "https://x.com/home", tab: { id: 17, url: "https://x.com/home" } };
  const clip = { pageUrl: "https://x.com/a/status/123/video/2", title: "Second clip" };
  assert.equal((await app.message({ type: "video-probe", ...clip, clientId: "x-owner" }, from)).ok, true);
  assert.equal((await app.message({ type: "video-add", ...clip, option: "v1080" }, from)).ok, true);
  assert.deepEqual(mediaCalls(app).map(c => c.path), ["/api/media/probe/start", "/api/media/add"]);
  assert.equal(mediaCalls(app)[1].body.source.pageUrl, clip.pageUrl);
  assert.equal(mediaCalls(app)[1].body.option, "v1080");
  assert.equal(mediaCalls(app)[0].body.source.referrer, from.url);
});

test("the X video context menu opens the quality menu in its original frame", async () => {
  const app = await background();
  await app.context({ menuItemId: "sdm-video", pageUrl: "https://x.com/home", srcUrl: "blob:https://x.com/clip", frameId: 7 }, { id: 42, url: "https://x.com/home" });
  assert.deepEqual(JSON.parse(JSON.stringify(app.tabMessages)), [[42, { type: "video-open-menu", mediaUrl: "blob:https://x.com/clip" }, { frameId: 7 }]]);
  assert.equal(mediaCalls(app).length, 0);
  await app.context({ menuItemId: "sdm-video", pageUrl: page.pageUrl }, { id: 42, url: page.pageUrl });
  assert.equal(mediaCalls(app).at(-1).path, "/api/media");
});

test("a stale startup ping cannot hide videos after a newer successful connection", async () => {
  const app = await background({ deferStartupPing: true });
  app.finishStartupPing();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(await app.message({ type: "video-enabled" }), true);
});
