import assert from "node:assert/strict";
import { createHash, createPublicKey, verify } from "node:crypto";
import { createReadStream } from "node:fs";
import { copyFile, mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { CLI_TARGETS, stableVersion, UPDATER_PLATFORMS } from "./policy.mjs";

function base64(value) {
  const trimmed = value.trim();
  assert.match(trimmed, /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/, "Invalid Minisign base64");
  return Buffer.from(trimmed, "base64");
}

// Tauri wraps Minisign keys/signatures in base64. Verify both the package and
// trusted comment, using the same ED/prehashed format as minisign-verify.
export async function verifyUpdaterFile(file, signature, pubkey) {
  const keyLines = base64(pubkey).toString("utf8").trim().split(/\r?\n/);
  assert.equal(keyLines.length, 2, "Invalid Tauri public key");
  assert.ok(keyLines[0].startsWith("untrusted comment: "));
  const key = base64(keyLines[1]);
  assert.equal(key.length, 42);
  assert.equal(key.subarray(0, 2).toString(), "Ed");
  const lines = base64(signature).toString("utf8").trim().split(/\r?\n/);
  assert.equal(lines.length, 4, "Invalid Tauri updater signature");
  assert.ok(lines[0].startsWith("untrusted comment: "));
  assert.ok(lines[2].startsWith("trusted comment: "));
  const packet = base64(lines[1]);
  const global = base64(lines[3]);
  assert.equal(packet.length, 74);
  assert.equal(global.length, 64);
  assert.equal(packet.subarray(0, 2).toString(), "ED", "Release bundles must use prehashed Minisign signatures");
  assert.deepEqual(packet.subarray(2, 10), key.subarray(2, 10), "The updater signature uses a different signing key");
  const publicKey = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), key.subarray(10)]), format: "der", type: "spki",
  });
  const blake2 = createHash("blake2b512");
  const sha256 = createHash("sha256");
  let size = 0;
  for await (const chunk of createReadStream(file)) {
    blake2.update(chunk);
    sha256.update(chunk);
    size += chunk.length;
  }
  assert.ok(size > 0, "An updater package cannot be empty");
  assert.ok(verify(null, blake2.digest(), publicKey, packet.subarray(10)), "Updater package signature verification failed");
  const comment = lines[2].slice("trusted comment: ".length);
  assert.ok(verify(null, Buffer.concat([packet.subarray(10), Buffer.from(comment)]), publicKey, global), "Updater trusted-comment signature verification failed");
  return { size, sha256: sha256.digest("hex"), comment };
}

async function walk(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const result = [];
  for (const entry of entries) {
    const file = join(directory, entry.name);
    // Tauri leaves unpacked macOS bundles and AppImage staging trees here.
    // Their packaged archives sit alongside them; staging trees contain
    // internal symlinks (e.g. .DirIcon) and must not become release assets.
    if (entry.isDirectory() && !entry.name.endsWith(".app") && !entry.name.endsWith(".AppDir")) result.push(...await walk(file));
    else if (entry.isFile()) result.push(file);
    else assert.ok(entry.isDirectory(), `Unexpected symlink in release artifacts: ${entry.name}`);
  }
  return result;
}

function assetName(file, platform, version) {
  const name = basename(file);
  if (name.endsWith(".app.tar.gz") || name.endsWith(".app.tar.gz.sig")) {
    return `Supo.Download.Manager_${version}_${platform}${name.endsWith(".sig") ? ".app.tar.gz.sig" : ".app.tar.gz"}`;
  }
  return name.replaceAll(" ", ".");
}

function updaterSuffix(platform) {
  if (platform === "windows-x86_64") return "-setup.exe";
  if (platform === "linux-x86_64") return ".AppImage";
  return ".app.tar.gz";
}

export async function stageArtifacts({ bundleDir, outputDir, platform, version, pubkey }) {
  stableVersion(version);
  assert.ok(UPDATER_PLATFORMS.includes(platform));
  const paths = (await walk(bundleDir)).filter((file) => /\.(?:exe|msi|dmg|deb|rpm|AppImage|app\.tar\.gz)(?:\.sig)?$/.test(file));
  const updater = paths.filter((file) => file.endsWith(updaterSuffix(platform)));
  assert.equal(updater.length, 1, `Expected exactly one ${platform} updater package`);
  const signature = (await readFile(`${updater[0]}.sig`, "utf8")).trim();
  const checked = await verifyUpdaterFile(updater[0], signature, pubkey);
  const signedVersion = checked.comment.match(/(?:^|\t)version:([^\t]+)/)?.[1];
  assert.equal(signedVersion, version, "Signed updater version differs from the checked-in version");
  await mkdir(outputDir, { recursive: true });
  const used = new Set();
  for (const file of paths) {
    const name = assetName(file, platform, version);
    assert.ok(!used.has(name), `Duplicate staged asset: ${name}`);
    used.add(name);
    await copyFile(file, join(outputDir, name));
  }
  const fragment = { version, platform, asset: assetName(updater[0], platform, version), signature, ...checked };
  await writeFile(join(outputDir, `fragment-${platform}.json`), `${JSON.stringify(fragment, null, 2)}\n`);
  return fragment;
}

export async function mergeArtifacts({ inputDir, outputDir, version, repository, pubkey, notes, pubDate }) {
  stableVersion(version);
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  assert.ok(Number.isFinite(Date.parse(pubDate)));
  const files = await walk(inputDir);
  const fragments = files.filter((file) => basename(file).startsWith("fragment-") && file.endsWith(".json"));
  assert.equal(fragments.length, UPDATER_PLATFORMS.length, "The release must have every signed desktop target");
  const platforms = {};
  const byName = new Map();
  for (const file of files.filter((item) => !fragments.includes(item))) {
    const name = basename(file);
    assert.ok(!byName.has(name), `Duplicate release asset: ${name}`);
    assert.ok((await stat(file)).size > 0, `Empty release asset: ${name}`);
    byName.set(name, file);
  }
  for (const file of fragments) {
    const fragment = JSON.parse(await readFile(file, "utf8"));
    assert.equal(fragment.version, version);
    assert.ok(UPDATER_PLATFORMS.includes(fragment.platform), "Unexpected updater platform");
    assert.ok(!platforms[fragment.platform], `Duplicate updater platform: ${fragment.platform}`);
    assert.equal(fragment.asset, basename(fragment.asset));
    assert.ok(fragment.asset.endsWith(updaterSuffix(fragment.platform)));
    assert.ok(byName.has(fragment.asset), `Missing updater package: ${fragment.asset}`);
    assert.equal((await readFile(byName.get(`${fragment.asset}.sig`), "utf8")).trim(), fragment.signature);
    const checked = await verifyUpdaterFile(byName.get(fragment.asset), fragment.signature, pubkey);
    assert.equal(checked.comment.match(/(?:^|\t)version:([^\t]+)/)?.[1], version,
      "Signed updater version differs from the checked-in version");
    assert.equal(checked.size, fragment.size);
    assert.equal(checked.sha256, fragment.sha256, "A staged updater package changed after signing");
    platforms[fragment.platform] = {
      signature: fragment.signature,
      url: `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(fragment.asset)}`,
    };
  }
  assert.deepEqual(Object.keys(platforms).sort(), [...UPDATER_PLATFORMS].sort());
  for (const target of CLI_TARGETS) {
    const file = `sdm-cli-v${version}-${target}.${target.includes("windows") ? "zip" : "tar.gz"}`;
    assert.ok(byName.has(file), `Missing command-line release package: ${file}`);
  }
  for (const browser of ["chromium", "firefox", "store-chromium"]) {
    assert.ok(byName.has(`sdm-extension-${browser}-v${version}.zip`), `Missing browser extension: ${browser}`);
  }
  await mkdir(outputDir, { recursive: true });
  for (const [name, file] of byName) await copyFile(file, join(outputDir, name));
  const manifest = { version, notes, pub_date: pubDate, platforms };
  await writeFile(join(outputDir, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  const sums = [];
  for (const file of (await walk(outputDir)).sort()) {
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(file)) hash.update(chunk);
    sums.push(`${hash.digest("hex")}  ${basename(file)}`);
  }
  await writeFile(join(outputDir, "SHA256SUMS"), `${sums.join("\n")}\n`);
  return manifest;
}

async function main() {
  const config = JSON.parse(await readFile("apps/desktop/src-tauri/tauri.conf.json", "utf8"));
  const common = { version: config.version, pubkey: config.plugins?.updater?.pubkey };
  assert.ok(common.pubkey, "A pinned updater public key must be configured");
  if (process.argv[2] === "stage") {
    await stageArtifacts({ ...common, bundleDir: process.env.BUNDLE_DIR, outputDir: "release-desktop", platform: process.env.UPDATER_PLATFORM });
  } else {
    assert.equal(process.argv[2], "merge");
    await mergeArtifacts({
      ...common, inputDir: "release-input", outputDir: "release-final", repository: process.env.GITHUB_REPOSITORY,
      notes: await readFile(".github/scripts/release/notes.md", "utf8"), pubDate: new Date().toISOString(),
    });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();
