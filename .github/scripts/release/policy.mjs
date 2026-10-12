import assert from "node:assert/strict";

export const EXPECTED_CI_JOBS = [
  "Test browser download capture (Chrome)",
  "Test browser video menus (Edge)",
  "Format and lint",
  ...["ubuntu-latest", "windows-latest", "macos-latest"].flatMap((os) => [
    `Test engine and CLI (${os})`, `Test desktop app (${os})`,
  ]),
];
export const UPDATER_PLATFORMS = ["windows-x86_64", "linux-x86_64", "darwin-aarch64", "darwin-x86_64"];
export const CLI_TARGETS = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin"];

export function stableVersion(value) {
  assert.match(value, /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/, "Only stable SemVer versions can be published");
  return value;
}

export function compareVersions(a, b) {
  const left = stableVersion(a).split(".").map(BigInt);
  const right = stableVersion(b).split(".").map(BigInt);
  for (let i = 0; i < left.length; i++) {
    if (left[i] !== right[i]) return left[i] > right[i] ? 1 : -1;
  }
  return 0;
}

export function checkCiRun(run, jobs, repository, sha) {
  assert.equal(run.head_sha, sha, "CI must test the exact release commit");
  assert.equal(run.event, "push", "Release CI must originate from a push to main");
  assert.equal(run.head_branch, "main");
  assert.equal(run.head_repository?.full_name, repository, "Fork CI cannot authorize a release");
  assert.equal(run.path?.split("@")[0], ".github/workflows/ci.yml");
  assert.equal(run.status, "completed");
  assert.equal(run.conclusion, "success", "Full CI must pass before releasing");
  for (const name of EXPECTED_CI_JOBS) {
    assert.ok(jobs.some((job) => job.name === name), `Missing required CI job: ${name}`);
  }
  assert.ok(jobs.length >= EXPECTED_CI_JOBS.length);
  for (const job of jobs) {
    assert.equal(job.status, "completed", `CI job is still running: ${job.name}`);
    assert.equal(job.conclusion, "success", `CI job did not pass: ${job.name}`);
  }
}

export function releaseDecision({ version, requestedTag, sha, mainSha, tagSha, releases }) {
  stableVersion(version);
  assert.ok(compareVersions(version, "0.7.0") >= 0, "The signed updater starts at 0.7.0");
  const tag = `v${version}`;
  if (requestedTag) assert.equal(requestedTag, tag, "Release tag/input must match the checked-in version");
  if (mainSha !== sha) return { eligible: false, reason: "A newer main commit superseded this build" };
  if (tagSha) assert.equal(tagSha, sha, "The existing version tag points at a different commit");
  const matches = releases.filter((release) => release.tag_name === tag);
  assert.ok(matches.length <= 1, "Multiple releases use this version tag");
  if (matches[0] && !matches[0].draft) return { eligible: false, reason: "This version already has a public release" };
  if (matches[0]) assert.equal(matches[0].target_commitish, sha, "A previous draft was built from a different commit");
  for (const release of releases.filter((item) => !item.draft && !item.prerelease)) {
    if (!/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(release.tag_name)) continue;
    if (compareVersions(version, release.tag_name.slice(1)) <= 0) {
      return { eligible: false, reason: "The checked-in version does not advance the stable release" };
    }
  }
  return { eligible: true, tag, existingDraft: matches[0] ?? null };
}
