import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = vm.createContext({ URL, window: { __sdmVideoButton: true } });
vm.runInContext(readFileSync(new URL("../src/video.js", import.meta.url), "utf8"), context);
const source = (...args) => JSON.parse(JSON.stringify(context.sdmVideoSource(...args)));
const link = (href, time = false) => ({ getAttribute: () => href, querySelector: () => time ? {} : null, contains: () => false });
const scope = (links, parentElement = null, tagName = "DIV") => ({ querySelectorAll: () => links, matches: () => false, parentElement, tagName });
const player = (parentElement, article) => ({ parentElement, closest: () => article, currentSrc: "blob:https://x.com/player", src: "" });

test("X Home resolves each hovered video's own timestamp permalink", () => {
  for (const id of ["123", "456"]) {
    const article = scope([link("/quoted/status/999"), link(`/author/status/${id}?s=20`, true)], null, "ARTICLE");
    assert.equal(source(player(article, article), "https://x.com/home", "Home / X").pageUrl, `https://x.com/author/status/${id}`);
  }
});

test("a quoted video uses its nearest post and retains the chosen clip index", () => {
  const article = scope([link("/outer/status/123", true)], null, "ARTICLE");
  const quote = scope([link("/quoted/status/456/video/1?ref=embed", true)], article);
  assert.equal(source(player(quote, article), "https://x.com/home", "Home / X").pageUrl, "https://x.com/quoted/status/456/video/1");
});

test("the link containing a player takes priority over the post timestamp", () => {
  const own = link("/author/status/123/video/2");
  own.contains = () => true;
  const article = scope([link("/author/status/123", true), own], null, "ARTICLE");
  assert.equal(source(player(article, article), "https://x.com/home", "Home").pageUrl, "https://x.com/author/status/123/video/2");
});

test("malformed or foreign links cannot replace the X post", () => {
  const article = scope([link("http://[", true), link("https://evil.example/a/status/99", true), link("javascript:alert(1)"), link("/i/web/status/456", true)], null, "ARTICLE");
  assert.equal(source(player(article, article), "https://twitter.com/search?q=test", "Search").pageUrl, "https://twitter.com/i/web/status/456");
});

test("does not search other posts when this player has no post link", () => {
  const body = scope([link("/unrelated/status/999", true)]);
  const article = scope([], body, "ARTICLE");
  assert.equal(source(player(article, article), "https://x.com/home", "Home").pageUrl, "https://x.com/home");
  assert.equal(source(player(body, null), "https://x.com/home", "Home").pageUrl, "https://x.com/home");
});

test("other sites retain the page title and direct media URL", () => {
  const video = { currentSrc: "https://cdn.example/video.mp4" };
  assert.deepEqual(source(video, "https://example.org/watch#player", "Clip"), {
    pageUrl: "https://example.org/watch#player", title: "Clip", mediaUrl: "https://cdn.example/video.mp4",
  });
});
