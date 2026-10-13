import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { mergeArtifacts, stageArtifacts, verifyUpdaterFile } from "./artifacts.mjs";
import { checkEligibility, resolveTag } from "./eligibility.mjs";
import { publishRelease } from "./publish.mjs";
import { checkCiRun, compareVersions, EXPECTED_CI_JOBS, releaseDecision, stableVersion, UPDATER_PLATFORMS, CLI_TARGETS } from "./policy.mjs";

const sha = "a".repeat(40);
const newerSha = "b".repeat(40);
const repository = "10s3r2604/supodownloadmanager";
const run = { head_sha: sha, event: "push", head_branch: "main", head_repository: { full_name: repository }, path: ".github/workflows/ci.yml", status: "completed", conclusion: "success" };
const jobs = EXPECTED_CI_JOBS.map((name) => ({ name, status: "completed", conclusion: "success" }));
const decision = (extra = {}) => releaseDecision({ version: "0.7.0", sha, mainSha: sha, releases: [{ tag_name: "v0.6.0", draft: false, prerelease: false }], ...extra });

test("stable versions advance numerically, with no prereleases or malformed tags", () => {
  assert.equal(compareVersions("0.10.0", "0.9.9"), 1);
  assert.equal(compareVersions("0.7.0", "0.7.0"), 0);
  for (const value of ["v0.7.0", "0.7.0-beta.1", "0.7.0+build", "00.7.0", "0.7", "0.7.0\n"]) {
    assert.throws(() => stableVersion(value));
  }
  assert.equal(decision().eligible, true);
  assert.throws(() => decision({ version: "0.6.1" }));
});

test("stale commits and duplicate or regressive public versions do not publish", () => {
  assert.equal(decision({ mainSha: newerSha }).eligible, false);
  assert.equal(decision({ releases: [{ tag_name: "v0.7.0", draft: false }] }).eligible, false);
  assert.equal(decision({ releases: [{ tag_name: "v0.10.0", draft: false }] }).eligible, false);
  assert.equal(decision({ releases: [{ tag_name: "v0.9.0-beta.1", draft: false, prerelease: true }] }).eligible, true);
});

test("tags, manual input and previous drafts must bind to the exact version and commit", () => {
  assert.throws(() => decision({ requestedTag: "v0.7.1" }));
  assert.throws(() => decision({ tagSha: newerSha }));
  assert.throws(() => decision({ releases: [{ tag_name: "v0.7.0", draft: true, target_commitish: newerSha }] }));
  assert.equal(decision({ requestedTag: "v0.7.0", tagSha: sha, releases: [{ tag_name: "v0.7.0", draft: true, target_commitish: sha }] }).eligible, true);
});

test("an ordinary commit after publication skips the version whose tag belongs to the previous commit", () => {
  const result = decision({ tagSha: newerSha, releases: [{ tag_name: "v0.7.0", draft: false }] });
  assert.equal(result.eligible, false);
  assert.equal(result.reason, "This version already has a public release");
  assert.throws(() => decision({ tagSha: newerSha, releases: [{ tag_name: "v0.7.0", draft: true, target_commitish: sha }] }));
});

test("full successful main CI is required; fork, PR, skipped and missing jobs fail", () => {
  checkCiRun(run, jobs, repository, sha);
  for (const extra of [
    { head_sha: newerSha }, { event: "pull_request" }, { head_branch: "feature" },
    { head_repository: { full_name: "fork/sdm" } }, { path: ".github/workflows/other.yml" },
    { conclusion: "failure" }, { status: "in_progress" },
  ]) assert.throws(() => checkCiRun({ ...run, ...extra }, jobs, repository, sha));
  assert.throws(() => checkCiRun(run, jobs.slice(1), repository, sha));
  assert.throws(() => checkCiRun(run, [{ ...jobs[0], conclusion: "skipped" }, ...jobs.slice(1)], repository, sha));
  assert.throws(() => checkCiRun(run, [...jobs, { name: "extra", status: "completed", conclusion: "failure" }], repository, sha));
});

test("annotated tags resolve to their commit and an absent tag is allowed", async () => {
  assert.equal(await resolveTag(async () => null, "v0.7.0"), null);
  const api = async (path) => path.startsWith("git/ref/")
    ? { object: { type: "tag", sha: newerSha } }
    : { object: { type: "commit", sha } };
  assert.equal(await resolveTag(api, "v0.7.0"), sha);
});

function fakeGitHub({ mainSha = sha } = {}) {
  const calls = [];
  let draft = null;
  const assets = [];
  const api = async (path, options = {}) => {
    const method = options.method ?? "GET";
    calls.push({ path, method, body: options.body });
    if (path === "git/ref/heads/main") return { object: { sha: mainSha } };
    if (path === "actions/runs/123") return run;
    if (path.startsWith("actions/runs/123/jobs?")) return { jobs };
    if (path.startsWith("git/ref/tags/")) return null;
    if (path.startsWith("releases?")) return draft ? [draft] : [];
    if (path === "releases" && method === "POST") {
      draft = { ...options.body, id: 42 };
      return draft;
    }
    if (path.startsWith("releases/42/assets?")) return assets;
    if (path === "releases/42" && method === "PATCH") {
      draft = { ...draft, ...options.body };
      return draft;
    }
    throw new Error(`Unexpected API call: ${method} ${path}`);
  };
  return { api, calls, assets, setMainSha: (value) => { mainSha = value; } };
}

test("API eligibility requires all checks, without mutating releases", async () => {
  const github = fakeGitHub();
  assert.equal((await checkEligibility({ repository, sha, ciRunId: 123, version: "0.7.0", api: github.api })).eligible, true);
  assert.ok(github.calls.every((call) => call.method === "GET"));
});

async function fixture(t) {
  const dir = await mkdtemp(join(tmpdir(), "sdm-release-test-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}

function signingKey() {
  const pair = generateKeyPairSync("ed25519");
  const rawPublic = pair.publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const keyId = randomBytes(8);
  const packet = Buffer.concat([Buffer.from("Ed"), keyId, rawPublic]);
  const pubkey = Buffer.from(`untrusted comment: test public key\n${packet.toString("base64")}\n`).toString("base64");
  const signature = (data, comment = "timestamp:1\tfile:fixture\tversion:0.7.0") => {
    const sig = sign(null, createHash("blake2b512").update(data).digest(), pair.privateKey);
    const global = sign(null, Buffer.concat([sig, Buffer.from(comment)]), pair.privateKey);
    const text = `untrusted comment: test signature\n${Buffer.concat([Buffer.from("ED"), keyId, sig]).toString("base64")}\ntrusted comment: ${comment}\n${global.toString("base64")}\n`;
    return Buffer.from(text).toString("base64");
  };
  return { pubkey, signature };
}

test("verifies the public Minisign reference vector used by the Rust verifier", async (t) => {
  const dir = await fixture(t);
  const file = join(dir, "test");
  await writeFile(file, "test");
  const pubkey = Buffer.from("untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n").toString("base64");
  const signature = Buffer.from("untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1556193335\tfile:test\ny/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==\n").toString("base64");
  assert.equal((await verifyUpdaterFile(file, signature, pubkey)).size, 4);
  await writeFile(file, "Test");
  await assert.rejects(verifyUpdaterFile(file, signature, pubkey), /verification failed/);
});

test("rejects wrong keys, package tampering and changes to the trusted comment", async (t) => {
  const dir = await fixture(t);
  const file = join(dir, "setup.exe");
  const key = signingKey();
  await writeFile(file, "installer bytes");
  const signature = key.signature(Buffer.from("installer bytes"));
  await verifyUpdaterFile(file, signature, key.pubkey);
  await assert.rejects(verifyUpdaterFile(file, signature, signingKey().pubkey), /different signing key/);
  const tampered = Buffer.from(Buffer.from(signature, "base64").toString().replace("version:0.7.0", "version:9.9.9")).toString("base64");
  await assert.rejects(verifyUpdaterFile(file, tampered, key.pubkey), /trusted-comment/);
  await writeFile(file, "installer bytez");
  await assert.rejects(verifyUpdaterFile(file, signature, key.pubkey), /verification failed/);
});

async function completeArtifacts(t) {
  const dir = await fixture(t);
  const inputDir = join(dir, "input");
  const outputDir = join(dir, "final");
  const key = signingKey();
  for (const platform of UPDATER_PLATFORMS) {
    const bundleDir = join(dir, `bundle-${platform}`);
    await mkdir(bundleDir, { recursive: true });
    const filename = platform.startsWith("windows") ? "Supo Download Manager_0.7.0_x64-setup.exe"
      : platform.startsWith("linux") ? "Supo.Download.Manager_0.7.0_amd64.AppImage" : "Supo Download Manager.app.tar.gz";
    const data = Buffer.from(`binary-${platform}`);
    await writeFile(join(bundleDir, filename), data);
    await writeFile(join(bundleDir, `${filename}.sig`), key.signature(data));
    await stageArtifacts({ bundleDir, outputDir: join(inputDir, platform), platform, version: "0.7.0", pubkey: key.pubkey });
  }
  const toolsDir = join(inputDir, "tools");
  await mkdir(toolsDir, { recursive: true });
  for (const target of CLI_TARGETS) {
    await writeFile(join(toolsDir, `sdm-cli-v0.7.0-${target}.${target.includes("windows") ? "zip" : "tar.gz"}`), "archive");
  }
  for (const browser of ["chromium", "firefox", "store-chromium"]) await writeFile(join(toolsDir, `sdm-extension-${browser}-v0.7.0.zip`), "extension");
  return { dir, inputDir, outputDir, version: "0.7.0", repository, pubkey: key.pubkey, notes: "Stable release", pubDate: "2026-10-12T00:00:00Z" };
}

test("merges all four signed targets, unique macOS packages and checksums together", async (t) => {
  const params = await completeArtifacts(t);
  const manifest = await mergeArtifacts(params);
  assert.deepEqual(Object.keys(manifest.platforms).sort(), [...UPDATER_PLATFORMS].sort());
  assert.match(manifest.platforms["windows-x86_64"].url, /\/v0\.7\.0\/Supo\.Download\.Manager_0\.7\.0_x64-setup\.exe$/);
  assert.notEqual(manifest.platforms["darwin-aarch64"].url, manifest.platforms["darwin-x86_64"].url);
  assert.ok((await readdir(params.outputDir)).includes("SHA256SUMS"));
  assert.ok(!(await readdir(params.outputDir)).some((name) => name.startsWith("fragment-")));
});

test("a missing platform prevents a partial update manifest", async (t) => {
  const params = await completeArtifacts(t);
  await rm(join(params.inputDir, "linux-x86_64"), { recursive: true });
  await assert.rejects(mergeArtifacts(params), /every signed desktop target/);
});

test("a changed staged package is rejected at final merge", async (t) => {
  const params = await completeArtifacts(t);
  await writeFile(join(params.inputDir, "windows-x86_64", "Supo.Download.Manager_0.7.0_x64-setup.exe"), "modified binary");
  await assert.rejects(mergeArtifacts(params), /verification failed/);
});

test("a duplicate platform or wrong fragment version is rejected", async (t) => {
  const params = await completeArtifacts(t);
  const file = join(params.inputDir, "darwin-x86_64", "fragment-darwin-x86_64.json");
  const fragment = JSON.parse(await readFile(file, "utf8"));
  await writeFile(file, JSON.stringify({ ...fragment, platform: "darwin-aarch64" }));
  await assert.rejects(mergeArtifacts(params), /Duplicate updater platform/);
  await writeFile(file, JSON.stringify({ ...fragment, version: "0.7.1" }));
  await assert.rejects(mergeArtifacts(params), /0.7.1/);
});

test("missing CLI or extension packages prevent publication", async (t) => {
  const params = await completeArtifacts(t);
  const name = "sdm-extension-firefox-v0.7.0.zip";
  await rm(join(params.inputDir, "tools", name));
  await assert.rejects(mergeArtifacts(params), /Missing browser extension/);
});

test("stage fails without a signature or with a mismatched signed version", async (t) => {
  const dir = await fixture(t);
  const bundleDir = join(dir, "bundle");
  await mkdir(bundleDir);
  const key = signingKey();
  const file = join(bundleDir, "Supo_0.7.0_x64-setup.exe");
  await writeFile(file, "binary");
  const params = { bundleDir, outputDir: join(dir, "staged"), version: "0.7.0", platform: "windows-x86_64", pubkey: key.pubkey };
  await assert.rejects(stageArtifacts(params), /ENOENT/);
  await writeFile(`${file}.sig`, key.signature(Buffer.from("binary"), "timestamp:1\tversion:0.8.0"));
  await assert.rejects(stageArtifacts(params), /Signed updater version/);
  await writeFile(`${file}.sig`, key.signature(Buffer.from("binary"), "timestamp:1\tfile:fixture"));
  await assert.rejects(stageArtifacts(params), /Signed updater version/);
});

test("AppImage staging excludes the generated AppDir and its icon symlink", async (t) => {
  const dir = await fixture(t);
  const bundleDir = join(dir, "bundle", "appimage");
  const appDir = join(bundleDir, "Supo.Download.Manager.AppDir");
  const iconDir = join(appDir, "usr", "share", "icons");
  await mkdir(iconDir, { recursive: true });
  await writeFile(join(iconDir, "icon.png"), "unpacked icon");
  await symlink(iconDir, join(appDir, ".DirIcon"), "junction");
  const name = "Supo.Download.Manager_0.7.0_amd64.AppImage";
  const data = Buffer.from("signed AppImage");
  const key = signingKey();
  await writeFile(join(bundleDir, name), data);
  await writeFile(join(bundleDir, `${name}.sig`), key.signature(data));
  const outputDir = join(dir, "staged");
  await stageArtifacts({ bundleDir: join(dir, "bundle"), outputDir, platform: "linux-x86_64", version: "0.7.0", pubkey: key.pubkey });
  assert.deepEqual((await readdir(outputDir)).sort(), [name, `${name}.sig`, "fragment-linux-x86_64.json"].sort());
  assert.deepEqual(await readFile(join(outputDir, name)), data);
});

async function publicationFixture(t) {
  const params = await completeArtifacts(t);
  await mergeArtifacts(params);
  const github = fakeGitHub();
  const upload = async ({ name, file, size }) => {
    const digest = `sha256:${createHash("sha256").update(await readFile(file)).digest("hex")}`;
    github.assets.push({ name, size, state: "uploaded", digest });
  };
  return { github, args: { repository, sha, ciRunId: 123, version: "0.7.0", api: github.api, directory: params.outputDir, notes: "Release notes", upload } };
}

test("publication only clears draft after every upload and complete checksum verification", async (t) => {
  const { github, args } = await publicationFixture(t);
  const result = await publishRelease(args);
  assert.equal(result.eligible, true);
  const writes = github.calls.filter((call) => call.method !== "GET");
  assert.equal(writes[0].method, "POST");
  assert.equal(writes[0].body.draft, true);
  assert.equal(writes.at(-1).method, "PATCH");
  assert.equal(writes.at(-1).body.draft, false);
  assert.equal(writes.at(-1).body.make_latest, "true");
});

test("an upload failure leaves the release draft and never advertises its manifest", async (t) => {
  const { github, args } = await publicationFixture(t);
  args.upload = async () => { throw new Error("Upload failed"); };
  await assert.rejects(publishRelease(args), /Upload failed/);
  assert.ok(!github.calls.some((call) => call.method === "PATCH"));
});

test("uploaded checksum mismatch prevents publication", async (t) => {
  const { github, args } = await publicationFixture(t);
  const upload = args.upload;
  args.upload = async (item) => { await upload(item); github.assets.at(-1).digest = "sha256:wrong"; };
  await assert.rejects(publishRelease(args), /checksum differs/);
  assert.ok(!github.calls.some((call) => call.method === "PATCH"));
});

test("a newer main commit arriving during uploads keeps the previous build unpublished", async (t) => {
  const { github, args } = await publicationFixture(t);
  const upload = args.upload;
  args.upload = async (item) => { await upload(item); github.setMainSha(newerSha); };
  assert.equal((await publishRelease(args)).eligible, false);
  assert.ok(!github.calls.some((call) => call.method === "PATCH"));
});
