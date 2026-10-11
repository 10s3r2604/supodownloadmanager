#!/usr/bin/env bash
# Drafts can be resumed; a published version must not be rebuilt or overwritten.
set -euo pipefail

if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
  echo 'Use a version tag such as v0.5.1.' >&2
  exit 1
fi
if release_response=$(gh api "repos/$GITHUB_REPOSITORY/releases/tags/$VERSION"); then
  if [[ $(jq -r '.draft' <<< "$release_response") != true ]]; then
    echo "Release $VERSION is already published; use a new version." >&2
    exit 1
  fi
elif ! jq -e '.status == "404"' <<< "$release_response" >/dev/null; then
  echo 'Unable to check release status; refusing to build a release.' >&2
  exit 1
fi
