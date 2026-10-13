# Changelog

## 0.7.3

- Size toolbar buttons to their labels so Vietnamese “Chạy hàng đợi” stays
  inside its button in light and dark themes. Compact windows use more readable
  labels, and the toolbar can scroll when the available width is insufficient.

## 0.7.2

- First-run browser setup detects installed browsers and lets users select several.
  Windows attempts loading through the selected browser's normal controls;
  a browser that requires confirmation keeps clear manual steps. Connection
  status comes from the extension, and older installations keep their setup.
- Prepare extension updates before replacing existing files and keep newer
  unpacked versions. Browser setup runs off the native UI thread.
- Ordinary main commits skip already published versions without failing the
  release workflow; new versions retain exact-CI/signature/checksum gates.

- Browser extension 0.6.4 uses a compact Supo logo + **Download** button on
  videos. The caption stays short during handoff; quality, estimated size
  and progress remain available in the menu.

## 0.7.1

- Publish the signed stable update feed for installed 0.7.0 review builds.
  Use Settings → Updates → Check now, then Install and restart after downloads
  and Torrent sharing have stopped. The updater retains the same public key.
- Include the compact Supo interface, optional Windows startup, Torrent naming,
  browser capture and download recovery improvements documented below.

## 0.7.0

- Use the name **Torrent** consistently in the interface, native file picker,
  status labels, diagnostics and repository documentation.
- Compact IDM-style Supo layout: toolbar, category and queue sidebar, download
  table, selection, search, details and status bar. The default main window is
  880 × 560, constrained to the monitor work area and display scaling.
- Browser confirmation windows are 480 × 320, with file name, size, resume
  capability and destination visible. URL and queue details expand on demand;
  the action buttons remain accessible in a smaller window.
- Optional start with Windows opens SDM in the tray. Registration is per-user,
  quotes executable paths, reflects Task Manager changes and rolls back on save failure.
- Windows receives signed stable updates in the background, with manual check,
  an off switch and installation only after work stops and history is saved.
  Installer failures preserve the verified package and allow retry.
- A versioned main commit can publish a stable release after exact-commit full
  CI, complete platform builds and signature/checksum verification. Development
  installers remain separate review artifacts. See [automatic updates](docs/AUTO_UPDATES.md).

## 0.6.1

- Browser-captured downloads open a small, separate window with file name, size,
  destination and Download now / Download later choices.
- X feed videos resolve the selected post and clip. Choose quality, frame rate,
  format and estimated size directly in the browser menu.
- Video discovery uses short polling requests that survive extension worker
  restarts. Closing the menu cancels its request; discovery has bounded active
  work, queue length and timeouts.
- GitHub EXE/ZIP capture handles redirects and late file-name updates, while
  preserving browser cookies and referrer for the download.
- Choose SDM's temporary folder in Settings. Downloads keep their own temporary
  subfolder, retain paused data and clean completed or removed data, including
  when the destination is on another drive.
- Settings are validated before saving or changing the running configuration.
  Save failures stay visible inside Settings. Saved data has a last-good backup;
  recovery preserves damaged originals and reports the affected paths.
- New downloads are saved before starting. Resume validates complete segment
  coverage and partial-file length, syncs bytes before saved progress, and uses
  HTTP validators to avoid mixing changed server content.
- A completion journal reconciles a downloaded file with history after an
  interrupted final save. Completion cleanup waits for history to be saved.

The browser extension is version 0.6.3. Its video menu requires SDM 0.6.1 or newer.
Existing extension 0.6.2 clients can still use the desktop bridge's legacy API.
