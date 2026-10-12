import assert from "node:assert/strict";
import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { checkCiRun, releaseDecision } from "./policy.mjs";

export function githubApi(repository, token, fetchImpl = fetch) {
  assert.match(repository, /^[\w.-]+\/[\w.-]+$/);
  assert.ok(token, "A scoped GitHub token is required");
  return async (path, { method = "GET", body, missing = false } = {}) => {
    const response = await fetchImpl(`https://api.github.com/repos/${repository}/${path}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28", ...(body ? { "Content-Type": "application/json" } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal: AbortSignal.timeout(30_000),
    });
    if (missing && response.status === 404) return null;
    assert.ok(response.ok, `GitHub ${method} ${path} returned ${response.status}`);
    return response.status === 204 ? null : response.json();
  };
}

export async function getAll(api, path, field) {
  const items = [];
  for (let page = 1; ; page++) {
    const response = await api(`${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
    const current = field ? response[field] : response;
    assert.ok(Array.isArray(current));
    items.push(...current);
    if (current.length < 100) return items;
  }
}

export async function resolveTag(api, tag) {
  const reference = await api(`git/ref/tags/${encodeURIComponent(tag)}`, { missing: true });
  if (!reference) return null;
  let object = reference.object;
  for (let depth = 0; object.type === "tag"; depth++) {
    assert.ok(depth < 8, "Too many nested annotated tags");
    object = (await api(`git/tags/${object.sha}`)).object;
  }
  assert.equal(object.type, "commit");
  return object.sha;
}

export async function checkEligibility({ repository, sha, ciRunId, requestedTag, version, api }) {
  assert.match(sha, /^[a-f0-9]{40}$/);
  assert.match(String(ciRunId), /^\d+$/);
  const [main, run, jobs, releases, tag] = await Promise.all([
    api("git/ref/heads/main"),
    api(`actions/runs/${ciRunId}`),
    getAll(api, `actions/runs/${ciRunId}/jobs?filter=latest`, "jobs"),
    getAll(api, "releases"),
    resolveTag(api, `v${version}`),
  ]);
  checkCiRun(run, jobs, repository, sha);
  return releaseDecision({ version, requestedTag, sha, mainSha: main.object.sha, tagSha: tag, releases });
}

async function main() {
  const config = JSON.parse(readFileSync("apps/desktop/src-tauri/tauri.conf.json", "utf8"));
  const decision = await checkEligibility({
    repository: process.env.GITHUB_REPOSITORY,
    sha: process.env.RELEASE_SHA,
    ciRunId: process.env.RELEASE_CI_RUN,
    requestedTag: process.env.REQUESTED_TAG,
    version: config.version,
    api: githubApi(process.env.GITHUB_REPOSITORY, process.env.GH_TOKEN),
  });
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `eligible=${decision.eligible}\nversion=${config.version}\ntag=v${config.version}\n`);
  }
  console.log(decision.eligible ? `Eligible stable release: v${config.version}` : `Release skipped: ${decision.reason}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
