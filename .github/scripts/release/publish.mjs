import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readFile, readdir, stat } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { checkEligibility, getAll, githubApi } from "./eligibility.mjs";

export async function publishRelease({ repository, sha, ciRunId, version, requestedTag, api, directory, notes, upload }) {
  const eligible = () => checkEligibility({ repository, sha, ciRunId, version, requestedTag, api });
  const decision = await eligible();
  if (!decision.eligible) return decision;
  const names = (await readdir(directory)).sort();
  assert.ok(names.includes("latest.json") && names.includes("SHA256SUMS"));
  const body = `${notes}\n\nBuilt from commit ${sha}; full CI run ${ciRunId}.\n`;
  const draft = decision.existingDraft ?? await api("releases", { method: "POST", body: {
    tag_name: decision.tag, target_commitish: sha, name: `Supo Download Manager ${decision.tag}`,
    body, draft: true, prerelease: false, make_latest: "false",
  } });
  assert.equal(draft.draft, true, "Assets can only be changed while the release is a draft");
  assert.equal(draft.target_commitish, sha);
  // Retrying a failed release may replace its unpublished assets. A published
  // release is never overwritten, and no partial manifest is public.
  for (const asset of await getAll(api, `releases/${draft.id}/assets`)) {
    await api(`releases/assets/${asset.id}`, { method: "DELETE" });
  }
  const expected = new Map();
  for (const name of names) {
    const file = join(directory, name);
    const metadata = await stat(file);
    assert.ok(metadata.isFile() && metadata.size > 0);
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(file)) hash.update(chunk);
    const digest = `sha256:${hash.digest("hex")}`;
    await upload({ releaseId: draft.id, name, file, size: metadata.size });
    expected.set(name, { size: metadata.size, digest });
  }
  const uploaded = await getAll(api, `releases/${draft.id}/assets`);
  assert.deepEqual(uploaded.map((asset) => asset.name).sort(), names, "The uploaded release asset set is incomplete");
  for (const asset of uploaded) {
    assert.equal(asset.state, "uploaded");
    assert.equal(asset.size, expected.get(asset.name).size);
    assert.equal(asset.digest, expected.get(asset.name).digest, `Uploaded asset checksum differs: ${asset.name}`);
  }
  // Recheck after all uploads: CI and tags must still bind to the same main
  // commit, with no newer stable release. Only this final API call publishes.
  const finalDecision = await eligible();
  if (!finalDecision.eligible) return finalDecision;
  await api(`releases/${draft.id}`, { method: "PATCH", body: {
    draft: false, prerelease: false, make_latest: "true", target_commitish: sha, body,
  } });
  return { eligible: true, tag: decision.tag, releaseId: draft.id };
}

async function main() {
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GH_TOKEN;
  const config = JSON.parse(await readFile("apps/desktop/src-tauri/tauri.conf.json", "utf8"));
  const result = await publishRelease({
    repository, sha: process.env.RELEASE_SHA, ciRunId: process.env.RELEASE_CI_RUN, version: config.version,
    requestedTag: process.env.REQUESTED_TAG, api: githubApi(repository, token), directory: "release-final",
    notes: await readFile(".github/scripts/release/notes.md", "utf8"),
    upload: async ({ releaseId, name, file, size }) => {
      const response = await fetch(`https://uploads.github.com/repos/${repository}/releases/${releaseId}/assets?name=${encodeURIComponent(name)}`, {
        method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/octet-stream", "Content-Length": String(size), "X-GitHub-Api-Version": "2022-11-28" },
        body: createReadStream(file), duplex: "half", signal: AbortSignal.timeout(600_000),
      });
      assert.ok(response.ok, `Upload failed for ${name}: ${response.status}`);
    },
  });
  console.log(result.eligible ? `Published complete stable release ${result.tag}` : `Release kept unpublished: ${result.reason}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
