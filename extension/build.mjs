// Builds the extension for Chromium based browsers (Chrome, Edge, Cốc Cốc,
// Brave, Opera, Vivaldi) and for Firefox into extension/dist/.
//
//   node extension/build.mjs

import { cpSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "0.3.0";
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
  ],
};

const targets = {
  chromium: {
    ...base,
    minimum_chrome_version: "116",
    background: { service_worker: "background.js" },
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
}
console.log("Extension built into extension/dist/chromium and extension/dist/firefox");
