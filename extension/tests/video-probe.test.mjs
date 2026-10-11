import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const code = readFileSync(new URL("../src/video.js", import.meta.url), "utf8");
const flush = () => new Promise(resolve => setImmediate(resolve));
const done = { ok: true, state: "done", info: { title: "Clip", options: [{ id: "v1080", kind: "video", height: 1080, fps: 30, ext: "mp4" }] } };
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};

// A small DOM model exercises the actual content-script event handlers without
// exposing private implementation state or installing a browser dependency.
class Element {
  constructor(tagName) {
    this.tagName = tagName.toUpperCase();
    this.children = [];
    this.attributes = new Map();
    this.listeners = new Map();
    this.style = {};
    this.className = "";
    this.hidden = false;
    this.isConnected = true;
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.get(name); }
  addEventListener(name, callback) {
    const listeners = this.listeners.get(name) || [];
    listeners.push(callback);
    this.listeners.set(name, listeners);
  }
  dispatch(name, extra = {}) {
    const event = { preventDefault() {}, stopPropagation() {}, detail: 1, composedPath: () => [this], ...extra };
    return Promise.all((this.listeners.get(name) || []).map(callback => callback(event)));
  }
  appendChild(child) {
    if (child.parentNode) child.parentNode.children = child.parentNode.children.filter(value => value !== child);
    child.parentNode = this;
    this.children.push(child);
    return child;
  }
  append(...children) { for (const child of children) this.appendChild(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  matches(selector) {
    if (selector === ":popover-open") return Boolean(this.popoverOpen);
    if (selector.startsWith(".")) return this.className.split(" ").includes(selector.slice(1));
    if (selector === "button:not(:disabled)") return this.tagName === "BUTTON" && !this.disabled;
    return this.tagName.toLowerCase() === selector;
  }
  querySelectorAll(selector) {
    return this.children.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  attachShadow() {
    const root = new Element("shadow-root");
    const trigger = new Element("button");
    trigger.className = "trigger";
    trigger.appendChild(new Element("span"));
    const menu = new Element("div");
    menu.className = "menu";
    menu.hidden = true;
    root.append(trigger, menu);
    this.shadow = root;
    return root;
  }
  focus() { this.focused = true; }
  showPopover() { this.popoverOpen = true; }
  hidePopover() { this.popoverOpen = false; }
  getBoundingClientRect() { return { left: 50, right: 690, top: 50, bottom: 410, width: 640, height: 360 }; }
}

function content({ respond, crypto: cryptoAPI } = {}) {
  let now = 100_000;
  let serial = 0;
  let timerSerial = 0;
  let result = { ok: true, state: "pending" };
  const timers = new Map();
  const messages = [];
  const document = new Element("document");
  document.title = "Clip";
  document.body = new Element("body");
  document.documentElement = document.body;
  document.createElement = tag => new Element(tag);
  const video = new Element("video");
  video.currentSrc = "https://cdn.example/clip.mp4";
  document.body.appendChild(video);
  document.appendChild(document.body);
  const windowEvents = new Element("window");
  const location = { href: "https://video.example/watch" };
  const schedule = (callback, delay = 0) => {
    const id = ++timerSerial;
    timers.set(id, { at: now + delay, callback });
    return id;
  };
  vm.runInNewContext(code, {
    window: {}, document, location, URL,
    innerWidth: 1000, innerHeight: 800,
    crypto: cryptoAPI || { randomUUID: () => `content-client-${++serial}` },
    Date: class extends Date { static now() { return now; } },
    setTimeout: schedule, clearTimeout: id => timers.delete(id),
    requestAnimationFrame: callback => schedule(callback),
    addEventListener: (...args) => windowEvents.addEventListener(...args),
    chrome: {
      i18n: { getMessage: key => key },
      runtime: {
        onMessage: { addListener() {} },
        async sendMessage(message) {
          messages.push(JSON.parse(JSON.stringify(message)));
          if (message.type === "video-enabled") return true;
          const custom = respond?.(message);
          if (custom !== undefined) return custom;
          if (message.type === "video-probe-cancel") return { ok: true };
          return result;
        },
      },
    },
  });
  const advance = async (ms) => {
    const target = now + ms;
    for (;;) {
      const next = [...timers].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > target) break;
      timers.delete(next[0]);
      now = next[1].at;
      next[1].callback();
      await flush();
    }
    now = target;
    await flush();
  };
  const host = () => document.body.querySelector("sdm-video-button");
  const menu = () => host().shadow.querySelector(".menu");
  return {
    messages, document, location, video, advance,
    finish() { result = done; },
    async hover() { await document.dispatch("mousemove", { clientX: 100, clientY: 100 }); await advance(0); },
    open: () => host().shadow.querySelector(".trigger").dispatch("click"),
    keepHover: () => host().dispatch("mouseenter"),
    close: () => document.dispatch("keydown", { key: "Escape" }),
    menu,
    scroll: () => windowEvents.dispatch("scroll"),
  };
}

test("a content-owned client ID follows prefetch, menu promotion, and polls past 30 seconds", async () => {
  const page = content();
  await page.hover();
  const prefetch = page.messages.find(message => message.type === "video-prefetch");
  assert.ok(prefetch.clientId);
  const opening = page.open();
  await flush();
  const probe = page.messages.find(message => message.type === "video-probe");
  assert.equal(probe.clientId, prefetch.clientId);
  await page.advance(31_000);
  assert.equal(page.menu().hidden, false);
  assert.equal(page.menu().querySelector(".loading").textContent, "videoFindingQualities");
  const polls = page.messages.filter(message => message.type === "video-probe-status");
  assert.ok(polls.length >= 30);
  assert.ok(polls.every(message => message.clientId === prefetch.clientId));
  assert.ok(polls.every(message => Object.keys(message).sort().join(",") === "clientId,type"));
  page.finish();
  await page.advance(1000);
  await opening;
  assert.equal(page.menu().querySelector(".quality").textContent, "1080p HD");
});

test("opening after a long hover renews an expired prefetch lease without showing its error", async () => {
  let oldId;
  let expiredAttempts = 0;
  const page = content({ respond: message => {
    if (message.type === "video-probe" && message.clientId === oldId) {
      ++expiredAttempts;
      return { ok: false, error: "Probe interest expired" };
    }
    if (message.type === "video-probe-status") return done;
  } });
  await page.hover();
  oldId = page.messages.find(message => message.type === "video-prefetch").clientId;
  await page.keepHover();
  await page.advance(26_000);
  assert.equal(page.messages.filter(message => message.type === "video-prefetch").length, 1);
  assert.equal(page.messages.filter(message => message.type === "video-probe-status").length, 0);
  const opening = page.open();
  await flush();
  const cancellations = page.messages.filter(message => message.type === "video-probe-cancel");
  assert.deepEqual(cancellations, [{ type: "video-probe-cancel", clientId: oldId }]);
  const starts = page.messages.filter(message => message.type === "video-prefetch");
  assert.equal(starts.length, 2);
  assert.notEqual(starts[1].clientId, oldId);
  assert.equal(page.messages.find(message => message.type === "video-probe").clientId, starts[1].clientId);
  assert.equal(page.menu().querySelector(".loading").textContent, "videoFindingQualities");
  await page.advance(1000);
  await opening;
  assert.equal(page.menu().querySelector(".quality").textContent, "1080p HD");
  assert.equal(page.menu().querySelector(".error"), null);
  assert.equal(page.menu().querySelectorAll("button").some(button => button.textContent === "videoRetry"), false);
  assert.equal(expiredAttempts, 0);
  assert.equal(page.messages.find(message => message.type === "video-probe-status").clientId, starts[1].clientId);
});

test("HTTP pages without randomUUID retain a cryptographic client ID through start, poll, and cancel", async () => {
  let randomReads = 0;
  const page = content({ crypto: {
    getRandomValues(bytes) {
      ++randomReads;
      assert.equal(bytes.BYTES_PER_ELEMENT, 1);
      assert.equal(bytes.length, 16);
      bytes.set(Array.from({ length: 16 }, (_, index) => index + 1));
      return bytes;
    },
  } });
  page.location.href = "http://video.example/watch";
  await page.hover();
  const opening = page.open();
  await flush();
  await page.advance(1000);
  await page.close();
  await opening;
  const owned = page.messages.filter(message => message.clientId);
  assert.deepEqual(owned.map(message => message.type), ["video-prefetch", "video-probe", "video-probe-status", "video-probe-cancel"]);
  assert.ok(owned.every(message => message.clientId === "0102030405060708090a0b0c0d0e0f10"));
  assert.match(owned[0].clientId, /^[a-f0-9]{32}$/);
  assert.equal(randomReads, 1, "promotion and polling must reuse the original lease ID");
});

for (const type of ["video-probe", "video-probe-status"]) {
  test(`a rejected ${type} recovers after one delay using the same lease ID`, async () => {
    let attempts = 0;
    const page = content({ respond: message => {
      if (message.type !== type) return;
      if (++attempts === 1) throw new Error("The message port closed before a response was received");
      return done;
    } });
    await page.hover();
    const opening = page.open();
    await flush();
    if (type === "video-probe-status") await page.advance(1000);
    assert.equal(attempts, 1);
    assert.equal(page.menu().querySelector(".loading").textContent, "videoFindingQualities");
    await page.advance(999);
    assert.equal(attempts, 1, "the retry must wait before reopening a runtime port");
    await page.advance(1);
    await opening;
    assert.equal(attempts, 2);
    const attemptsSent = page.messages.filter(message => message.type === type);
    assert.deepEqual(attemptsSent[0], attemptsSent[1]);
    assert.equal(attemptsSent[0].clientId, page.messages.find(message => message.type === "video-prefetch").clientId);
    assert.equal(page.menu().querySelector(".quality").textContent, "1080p HD");
  });

  test(`three rejected ${type} messages show an error and stop automatic retries`, async () => {
    let attempts = 0;
    const page = content({ respond: message => {
      if (message.type === type) {
        ++attempts;
        throw new Error("The message port closed before a response was received");
      }
    } });
    await page.hover();
    const opening = page.open();
    await flush();
    await page.advance(type === "video-probe-status" ? 3000 : 2000);
    await opening;
    assert.equal(attempts, 3);
    assert.equal(page.menu().querySelector(".error").textContent, "videoFailed");
    assert.equal(page.menu().querySelector(".error").getAttribute("role"), "alert");
    assert.ok(page.menu().querySelectorAll("button").some(button => button.textContent === "videoRetry"));
    await page.advance(10_000);
    assert.equal(attempts, 3);
  });

  test(`closing during a ${type} retry delay cancels and wakes without another attempt`, async () => {
    let attempts = 0;
    const page = content({ respond: message => {
      if (message.type === type) {
        ++attempts;
        throw new Error("The message port closed before a response was received");
      }
    } });
    await page.hover();
    let finished = false;
    page.open().then(() => { finished = true; });
    await flush();
    if (type === "video-probe-status") await page.advance(1000);
    assert.equal(attempts, 1);
    await page.close();
    await flush();
    assert.equal(finished, true, "closing must wake the runtime retry wait");
    assert.equal(page.menu().hidden, true);
    const id = page.messages.find(message => message.type === "video-probe").clientId;
    assert.deepEqual(page.messages.filter(message => message.type === "video-probe-cancel"), [{ type: "video-probe-cancel", clientId: id }]);
    await page.advance(1500);
    assert.equal(attempts, 1);
  });

  test(`a terminal ${type} desktop error is displayed without transport retries`, async () => {
    let attempts = 0;
    const page = content({ respond: message => {
      if (message.type === type) {
        ++attempts;
        return { ok: false, error: "This page needs a login" };
      }
    } });
    await page.hover();
    const opening = page.open();
    await flush();
    if (type === "video-probe-status") await page.advance(1000);
    await opening;
    assert.equal(page.menu().querySelector(".error").textContent, "This page needs a login");
    await page.advance(10_000);
    assert.equal(attempts, 1);
  });
}

test("closing a pending menu cancels its lease and wakes its polling wait", async () => {
  const page = content();
  await page.hover();
  let finished = false;
  page.open().then(() => { finished = true; });
  await flush();
  await page.close();
  await flush();
  assert.equal(finished, true, "closing must release a pending content-script promise");
  assert.equal(page.menu().hidden, true);
  const id = page.messages.find(message => message.type === "video-probe").clientId;
  assert.deepEqual(page.messages.filter(message => message.type === "video-probe-cancel"), [{ type: "video-probe-cancel", clientId: id }]);
  await page.advance(1500);
  assert.equal(page.messages.filter(message => message.type === "video-probe-status").length, 0);
});

test("closing while start is in flight cancels immediately and ignores a late result", async () => {
  const start = deferred();
  const page = content({ respond: message => message.type === "video-probe" ? start.promise : undefined });
  await page.hover();
  const opening = page.open();
  await flush();
  const probe = page.messages.find(message => message.type === "video-probe");
  await page.close();
  assert.deepEqual(page.messages.filter(message => message.type === "video-probe-cancel"), [{ type: "video-probe-cancel", clientId: probe.clientId }]);
  start.resolve(done);
  await opening;
  assert.equal(page.menu().hidden, true);
  assert.equal(page.menu().querySelector(".choice"), null);
  await page.advance(1500);
  assert.equal(page.messages.filter(message => message.type === "video-probe-status").length, 0);
});

test("an old start reply cannot overwrite a reopened menu's newer qualities", async () => {
  const oldStart = deferred();
  let starts = 0;
  const newer = { ...done, info: { options: [{ id: "v720", kind: "video", height: 720, ext: "mp4" }] } };
  const page = content({ respond: message => {
    if (message.type === "video-probe") return ++starts === 1 ? oldStart.promise : newer;
  } });
  await page.hover();
  const oldOpening = page.open();
  await flush();
  await page.close();
  await page.open();
  assert.equal(page.menu().querySelector(".quality").textContent, "720p HD");
  oldStart.resolve(done);
  await oldOpening;
  assert.equal(page.menu().hidden, false);
  assert.equal(page.menu().querySelector(".quality").textContent, "720p HD");
  const ids = page.messages.filter(message => message.type === "video-probe").map(message => message.clientId);
  assert.notEqual(ids[0], ids[1]);
});

for (const [name, changeSource] of [
  ["changing page URL", page => { page.location.href = "https://video.example/second-clip"; }],
  ["changing media URL on the same page", page => { page.video.currentSrc = "https://cdn.example/second-clip.mp4"; }],
]) {
  test(`${name} releases the old lease and reopening obtains a new client ID`, async () => {
    const page = content();
    await page.hover();
    const opening = page.open();
    await flush();
    const first = page.messages.find(message => message.type === "video-probe").clientId;
    changeSource(page);
    await page.scroll();
    await flush();
    assert.equal(page.menu().hidden, true);
    assert.equal(page.messages.find(message => message.type === "video-probe-cancel").clientId, first);
    await opening;
    page.finish();
    await page.open();
    const starts = page.messages.filter(message => message.type === "video-probe");
    assert.equal(starts.length, 2);
    assert.notEqual(starts[1].clientId, first);
    assert.equal(starts[1].pageUrl, page.location.href);
    assert.equal(starts[1].mediaUrl, page.video.currentSrc);
    assert.equal(page.menu().querySelector(".quality").textContent, "1080p HD");
  });
}
