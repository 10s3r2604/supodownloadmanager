# Startup and stable updates

SDM 0.7.0 adds optional login startup and signed automatic updates on Windows.
The first signed stable update is 0.7.1. Installed 0.7.0 review builds can receive
it directly through **Settings → Updates → Check now**, then **Install and restart**.
The signing key and update endpoint are unchanged. Windows builds older than
0.7.0 need the 0.7.1 installer once because they do not contain the updater.
Linux and macOS still use their normal installers; their automatic updater is
disabled until installation and restart behavior has been tested on those systems.

## Application settings

- **Settings → General → Start SDM with the computer** is off by default. On
  Windows it registers a quoted executable path with `--autostart` for the
  current user, without administrator rights. Login starts SDM in the tray.
  Opening SDM normally still shows its window. Settings reflects a disable made
  in Windows Task Manager, and restores the previous registration if saving fails.
- **Settings → Updates → Automatically download stable updates** is on by
  default on supported systems. SDM checks shortly after startup and every six
  hours, downloads in the background and verifies both the package signature and
  the signed version. Turning it off prevents automatic checks and installation
  on exit. **Check now** remains available.
- A verified update offers **Install and restart**. Installation waits for
  downloads, media processing, temporary-file cleanup and torrent sharing to
  stop, and requires saved download history and queues. Unsaved Settings changes
  block the button until saved or discarded. An automatic install runs only when
  all SDM windows are hidden, or on a normal exit when the manager is already idle.
  An installer failure leaves SDM usable and allows an explicit retry.

## What makes a commit an update

Commits do not directly become application updates. The release workflow watches
successful **CI** runs from `main` and publishes only when the checked-in stable
application version advances beyond existing public releases.

1. Change the application version consistently in the workspace Cargo manifest,
   Cargo lockfile, desktop package/lockfile and Tauri configuration. Run
   `node .github/scripts/check-version.mjs` and update the release notes in
   `.github/scripts/release/notes.md`.
2. Merge the reviewed change to `main`. All ten CI jobs must pass for that exact
   commit, including native Windows and Chrome/Edge checks. Fork and pull-request
   runs cannot authorize a release.
3. Build and sign Windows x64, Linux x64 and both macOS architectures, plus the
   CLI and browser extension packages. A failed or missing build prevents publication.
4. Verify all updater signatures and their signed versions, merge the four
   targets into one `latest.json`, upload the complete asset set to a draft and
   verify GitHub's uploaded sizes and SHA-256 digests.
5. Recheck the exact CI commit, current `main` HEAD, tag and release history before
   making the draft public and latest. A newer main commit, reused public version
   or mismatched tag prevents publication.

Stable `vX.Y.Z` tags and a manual **Release** workflow run from `main` can retry the
same process. They require the same completed full CI run. A failed attempt can
replace assets in its unpublished draft; public releases are never overwritten.
Development installers from CI are review artifacts and are not update-feed releases.

## Signing setup for the repository owner

The public updater key is pinned in `apps/desktop/src-tauri/tauri.conf.json`.
Add its matching private key as the repository Actions secret
`TAURI_SIGNING_PRIVATE_KEY`. The prepared key has an empty password; if an
encrypted key is used later, also configure `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
Never put a private key in Git, release assets, workflow logs or chat. Keep a
private backup: replacing the public key alone prevents installed clients from
verifying future updates.

The release workflow requires the signing secret and fails if it is absent. CI
can still build review installers with updater artifact generation disabled.
The stable manifest is served at:

`https://github.com/10s3r2604/supodownloadmanager/releases/latest/download/latest.json`

Tauri's updater signature protects package integrity and version identity. It
does not provide Windows Authenticode publisher identification, SmartScreen
reputation or macOS notarization. Those use separate certificates/services.
