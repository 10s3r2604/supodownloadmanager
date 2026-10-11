// Builds the extension into extension/dist/:
//
//   chromium        Chrome, Edge, Cốc Cốc, Brave, Opera, Vivaldi, installed
//                   from the folder the SDM app ships
//   chromium-store  the same for the Chrome Web Store and Edge Add-ons, which
//                   do not allow extensions that download from YouTube: the
//                   video button and menu stay away from YouTube there
//   firefox         Firefox and addons.mozilla.org
//
//   node extension/build.mjs

import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "0.5.1";
const root = dirname(fileURLToPath(import.meta.url));

const icons = { 16: "icons/16.png", 32: "icons/32.png", 48: "icons/48.png", 128: "icons/128.png" };
const base = {
  manifest_version: 3,
  name: "__MSG_extName__",
  description: "__MSG_extDescription__",
  default_locale: "en",
  version: VERSION,
  icons,
  permissions: ["downloads", "contextMenus", "cookies", "storage", "alarms"],
  host_permissions: ["<all_urls>"],
  action: { default_popup: "popup.html", default_title: "__MSG_extName__", default_icon: icons },
  content_scripts: [
    {
      matches: ["http://*/*", "https://*/*"],
      js: ["video.js"],
      all_frames: true,
      run_at: "document_idle",
    },
    {
      matches: ["http://*/*", "https://*/*"],
      js: ["magnet.js"],
      all_frames: true,
      run_at: "document_idle",
    },
  ],
};

/** Sites the store build leaves alone (see above). */
const NO_VIDEO_SITES = ["youtube.com", "youtu.be", "youtube-nocookie.com", "youtubekids.com", "googlevideo.com"];

const chromium = {
  ...base,
  minimum_chrome_version: "116",
  background: { service_worker: "background.js" },
};

const targets = {
  chromium,
  "chromium-store": {
    ...chromium,
    content_scripts: base.content_scripts.map((script) =>
      script.js.includes("video.js")
        ? { ...script, exclude_matches: NO_VIDEO_SITES.map((site) => `*://*.${site}/*`) }
        : script,
    ),
  },
  firefox: {
    ...base,
    background: { scripts: ["background.js"] },
    browser_specific_settings: {
      gecko: {
        id: "sdm@supodownloadmanager.github.io",
        strict_min_version: "140.0",
        data_collection_permissions: { required: ["none"] },
      },
    },
  },
};

for (const [name, manifest] of Object.entries(targets)) {
  const out = join(root, "dist", name);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  cpSync(join(root, "src"), out, { recursive: true });
  writeFileSync(join(out, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  if (name === "chromium-store") {
    const background = join(out, "background.js");
    const code = readFileSync(background, "utf8");
    const marked = "/* @no-video-sites */ []";
    if (!code.includes(marked)) throw new Error("background.js lost its @no-video-sites marker");
    writeFileSync(background, code.replace(marked, JSON.stringify(NO_VIDEO_SITES)));
  }
}
console.log(`Extension ${VERSION} built into ${Object.keys(targets).map((t) => `extension/dist/${t}`).join(", ")}`);
