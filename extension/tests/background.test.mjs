import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const code = readFileSync(new URL("../src/background.js", import.meta.url), "utf8");
const sender = { url: "https://video.example/watch", tab: { url: "https://video.example/watch" } };
const page = { pageUrl: sender.url, title: "Clip", mediaUrl: "https://cdn.example/clip.mp4" };

async function background({ blocked = [], settings = {}, probeError = null, delay = 0 } = {}) {
  const calls = [];
  let listener;
  let contextListener;
  const tabMessages = [];
  let now = 100_000;
  let error = probeError;
  const event = { addListener() {} };
  const chrome = {
    storage: { local: { async get(defaults) { return { ...defaults, ...settings }; } } },
    runtime: { id: "sdm-test", onInstalled: event, onStartup: event, onMessage: { addListener(fn) { listener = fn; } } },
    cookies: { async getAll() { return [{ name: "session", value: "test-only", domain: "video.example", path: "/", secure: true, hostOnly: true }]; } },
    i18n: { getMessage: (key) => key },
    downloads: { onCreated: event },
    contextMenus: { onClicked: { addListener(fn) { contextListener = fn; } } },
    tabs: { onActivated: event, onUpdated: event, async sendMessage(...args) { tabMessages.push(args); return { ok: true }; } },
    alarms: { create() {}, onAlarm: event },
    action: { setTitle() {} },
  };
  const message = (value, from = sender) => new Promise((resolve) => {
    assert.equal(listener(value, from, resolve), true);
  });
  vm.runInNewContext(code.replace("/* @no-video-sites */ []", JSON.stringify(blocked)), {
    chrome, navigator: { userAgent: "SDM extension test", userAgentData: { brands: [] } },
    URL, AbortSignal, TypeError, setTimeout,
    Date: class extends Date { static now() { return now; } },
    async fetch(url, init) {
      const path = new URL(url).pathname;
      const body = init.body ? JSON.parse(init.body) : null;
      calls.push({ path, body });
      if (path === "/api/ping") return Response.json({ enabled: true, media: true, version: "0.5.0" });
      if (delay && body?.wait) {
        await new Promise((resolve, reject) => {
          const timer = setTimeout(resolve, delay);
          init.signal.addEventListener("abort", () => { clearTimeout(timer); reject(init.signal.reason); }, { once: true });
        });
      }
      if (error) return new Response(error, { status: 422 });
      if (path === "/api/media/add") return Response.json({ id: 42 });
      return body?.wait ? Response.json({ options: [{ id: "v1080", kind: "video" }] }) : new Response(null, { status: 202 });
    },
  });
  await message({ type: "status" });
  return { message, calls, tabMessages, context: (info, tab) => contextListener(info, tab), advance: (ms) => { now += ms; }, fail: (value) => { error = value; } };
}

const mediaCalls = (app) => app.calls.filter(({ path }) => path !== "/api/ping");

test("prefetch deduplicates tabs and fragments for one minute, with page cookies", async () => {
  const app = await background();
  await Promise.all([
    app.message({ type: "video-prefetch", ...page }),
    app.message({ type: "video-prefetch", ...page, pageUrl: `${page.pageUrl}#player` }),
  ]);
  assert.equal(mediaCalls(app).length, 1);
  const [{ path, body }] = mediaCalls(app);
  assert.equal(path, "/api/media/probe");
  assert.equal(body.wait, false);
  assert.equal(body.cookies[0].name, "session");
  assert.equal(body.userAgent, "SDM extension test");
  app.advance(60_001);
  await app.message({ type: "video-prefetch", ...page });
  assert.equal(mediaCalls(app).length, 2);
});

test("quality requests wait for the app and selections submit the original source", async () => {
  const app = await background();
  const probe = await app.message({ type: "video-probe", ...page });
  assert.equal(probe.ok, true);
  assert.equal(probe.info.options[0].id, "v1080");
  assert.equal(mediaCalls(app)[0].body.wait, true);
  const added = await app.message({ type: "video-add", ...page, option: "v1080" });
  assert.equal(added.id, 42);
  const last = mediaCalls(app).at(-1);
  assert.equal(last.path, "/api/media/add");
  assert.equal(last.body.source.pageUrl, page.pageUrl);
  assert.equal(last.body.source.cookies[0].value, "test-only");
  assert.equal(last.body.option, "v1080");
});

test("slow quality reads outlive the ordinary 2.5-second bridge timeout", async () => {
  const app = await background({ delay: 2700 });
  const result = await app.message({ type: "video-probe", ...page });
  assert.equal(result.ok, true);
});

test("app errors are preserved and failed prefetches may retry", async () => {
  const app = await background({ probeError: "This page needs a login" });
  const result = await app.message({ type: "video-prefetch", ...page });
  assert.equal(result.error, "This page needs a login");
  app.fail(null);
  assert.equal((await app.message({ type: "video-prefetch", ...page })).ok, true);
  assert.equal(mediaCalls(app).length, 2);
});

test("the store build refuses YouTube and embedded YouTube frames for every menu action", async () => {
  const app = await background({ blocked: ["youtube.com", "youtu.be"] });
  for (const type of ["video-prefetch", "video-probe", "video-add"]) {
    assert.equal((await app.message({ type, pageUrl: "https://www.youtube.com/watch?v=clip" })).ok, false);
    assert.equal((await app.message({ type, ...page }, { ...sender, url: "https://www.youtube.com/embed/clip" })).ok, false);
  }
  assert.equal(mediaCalls(app).length, 0);
});

test("disabled video buttons do not read formats in the background", async () => {
  const app = await background({ settings: { videoButton: false } });
  assert.equal((await app.message({ type: "video-prefetch", ...page })).ok, false);
  assert.equal(mediaCalls(app).length, 0);
});

test("X timelines never reach discovery or the desktop picker, including legacy handovers", async () => {
  const app = await background();
  const from = { url: "https://x.com/home", tab: { url: "https://x.com/home" } };
  for (const type of ["video-prefetch", "video-probe", "video-add", "video"]) {
    assert.equal((await app.message({ type, pageUrl: from.url }, from)).ok, false);
  }
  assert.equal((await app.message({ type: "video", pageUrl: "https://x.com/a/status/123" }, from)).ok, false);
  assert.equal(mediaCalls(app).length, 0);
});

test("X quality discovery and selected downloads stay on the quiet bridge routes", async () => {
  const app = await background();
  const from = { url: "https://x.com/home", tab: { url: "https://x.com/home" } };
  const clip = { pageUrl: "https://x.com/a/status/123/video/2", title: "Second clip" };
  assert.equal((await app.message({ type: "video-probe", ...clip }, from)).ok, true);
  assert.equal((await app.message({ type: "video-add", ...clip, option: "v1080" }, from)).ok, true);
  assert.deepEqual(mediaCalls(app).map(c => c.path), ["/api/media/probe", "/api/media/add"]);
  assert.equal(mediaCalls(app)[1].body.source.pageUrl, clip.pageUrl);
  assert.equal(mediaCalls(app)[1].body.option, "v1080");
  assert.equal(mediaCalls(app)[0].body.referrer, from.url);
});

test("the X video context menu opens the quality menu in its original frame", async () => {
  const app = await background();
  await app.context({ menuItemId: "sdm-video", pageUrl: "https://x.com/home", srcUrl: "blob:https://x.com/clip", frameId: 7 }, { id: 42, url: "https://x.com/home" });
  assert.deepEqual(JSON.parse(JSON.stringify(app.tabMessages)), [[42, { type: "video-open-menu", mediaUrl: "blob:https://x.com/clip" }, { frameId: 7 }]]);
  assert.equal(mediaCalls(app).length, 0);
  await app.context({ menuItemId: "sdm-video", pageUrl: page.pageUrl }, { id: 42, url: page.pageUrl });
  assert.equal(mediaCalls(app).at(-1).path, "/api/media");
});
