import { readFileSync } from "node:fs";
import assert from "node:assert/strict";

const readJson = (path) => JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
const cargo = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
const version = cargo.match(/\[workspace\.package\][\s\S]*?^version = "([^"]+)"/m)?.[1];
assert.ok(version, "Cargo.toml must declare workspace.package.version");
const checks = {
  "apps/desktop/package.json": readJson("apps/desktop/package.json").version,
  "apps/desktop/package-lock.json": readJson("apps/desktop/package-lock.json").version,
  "package-lock root package": readJson("apps/desktop/package-lock.json").packages[""].version,
  "apps/desktop/src-tauri/tauri.conf.json": readJson("apps/desktop/src-tauri/tauri.conf.json").version,
  "workspace sdm-core dependency": cargo.match(/^sdm-core = \{[^\n]*version = "([^"]+)"/m)?.[1],
};
for (const [name, actual] of Object.entries(checks)) {
  assert.equal(actual, version, `${name} must match the application version ${version}`);
}
if (process.env.VERSION) {
  assert.equal(process.env.VERSION, `v${version}`, "Release tag/input must match the application version (including the v prefix)");
}
console.log(`Application versions agree: ${version}${process.env.VERSION ? ` (${process.env.VERSION})` : ""}`);
