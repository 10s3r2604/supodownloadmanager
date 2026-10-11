import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = vm.createContext({ URL, window: { __sdmLinks: true } });
vm.runInContext(readFileSync(new URL("../src/links.js", import.meta.url), "utf8"), context);
const collect = (...args) => JSON.parse(JSON.stringify(context.sdmPageLinks(...args)));

const element = (attributes, extra = {}) => ({ getAttribute: (name) => attributes[name] ?? null, ...extra });
const page = (anchors, media = []) => ({
  querySelectorAll: (selector) => (selector.startsWith("a[") ? anchors : media),
});

test("collects http links and media once, with their text", () => {
  const root = page(
    [
      element({ href: "files/a.zip" }, { textContent: "  Get\n  A " }),
      element({ href: "/files/a.zip#readme" }, { textContent: "again" }),
      element({ href: "javascript:void(0)" }),
      element({ href: "mailto:me@example.com" }),
      element({ href: "http://[" }),
      element({ href: "https://cdn.example.net/b.mp4" }, { title: "Clip" }),
    ],
    [element({ src: "logo.png" }, { alt: "Logo" }), element({}, { currentSrc: "https://cdn.example.net/b.mp4" })],
  );
  assert.deepEqual(collect(root, null, "https://example.org/files/"), [
    { url: "https://example.org/files/files/a.zip", text: "Get A" },
    { url: "https://example.org/files/a.zip", text: "again" },
    { url: "https://cdn.example.net/b.mp4", text: "Clip" },
    { url: "https://example.org/files/logo.png", text: "Logo" },
  ]);
});

test("keeps only links in the selection, up to the limit", () => {
  const inside = element({ href: "/in.zip" });
  const root = page([inside, element({ href: "/out.zip" })]);
  const selection = { containsNode: (node) => node === inside };
  assert.deepEqual(collect(root, selection, "https://example.org/"), [{ url: "https://example.org/in.zip" }]);
  const many = page(Array.from({ length: 10 }, (_, i) => element({ href: `/f${i}` })));
  assert.equal(collect(many, null, "https://example.org/", 3).length, 3);
});
