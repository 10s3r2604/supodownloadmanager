<p align="center"><img src="assets/logo.svg" alt="Supo Download Manager" width="96"></p>

<h1 align="center">Supo Download Manager</h1>

<p align="center">Free, open-source downloads for Windows, macOS and Linux.<br>Files · video quality selection · Torrent · browser integration</p>

<p align="center">
  <a href="https://github.com/10s3r2604/supodownloadmanager/releases/latest"><img alt="Latest stable release" src="https://img.shields.io/github/v/release/10s3r2604/supodownloadmanager?label=stable&color=12b886"></a>
  <a href="https://github.com/10s3r2604/supodownloadmanager/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/10s3r2604/supodownloadmanager/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="GPL-3.0-or-later" src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue"></a>
</p>

<p align="center">
  <a href="https://github.com/10s3r2604/supodownloadmanager/releases/latest"><b>Download SDM</b></a> ·
  <a href="README.vi.md"><b>Tiếng Việt</b></a> ·
  <a href="#browser-setup">Browser setup</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a>
</p>

![Compact dark SDM interface](docs/images/sdm-idm-en.png)

SDM brings file downloads, videos and Torrent into one compact desktop app. Pause
and resume, choose what to download, and see progress without losing the useful
details. English and Vietnamese are included. No account, subscription, device
limit or ads.

## Download and install

Get installers and extension ZIPs from the [latest stable release](https://github.com/10s3r2604/supodownloadmanager/releases/latest).
**Current stable: SDM 0.7.4**, with browser extension **0.6.5**.

| Your system | Choose this package |
| --- | --- |
| Windows 10 / 11, x64 | `…_x64-setup.exe` (recommended), or `…_x64_en-US.msi` |
| macOS, Apple Silicon | `…_aarch64.dmg` |
| macOS, Intel | `…_x64.dmg` |
| Ubuntu / Debian / Mint, x64 | `…_amd64.deb` |
| Fedora / openSUSE, x64 | `….x86_64.rpm` |
| Linux, x64 portable | `…_amd64.AppImage` |
| Command line | `sdm-cli-…` for your system |

Release assets include `SHA256SUMS` and updater signatures. Windows update packages
are verified by SDM; this is separate from Windows publisher signing and macOS
notarization. See [update and signing details](docs/AUTO_UPDATES.md).

### Update an installed copy

On **Windows SDM 0.7.0 or newer**, open **Settings → Updates → Check now**, then
**Install and restart** after transfers and sharing stop. Older Windows versions
need the latest installer once. Linux and macOS use the normal installers;
automatic installation is not enabled there yet.

After updating the app, refresh an existing extension through
**Browsers → select your browsers → Set up selected**, then reload open video tabs.
Older extensions continue to work while you complete setup.

## Browser setup

1. Open SDM. On first launch it detects installed browsers; tick the ones you use.
2. Choose **Set up selected**. Windows attempts loading through the browser's
   normal controls. Automatic loading is verified on Chrome; follow any remaining
   confirmation steps for your browser.
3. Wait for **Connected**, then download a file or hover a video and click the
   Supo logo + **Download** button.

Reopen this flow from **Browsers** at any time. The extension connects automatically
with SDM 0.7.4; its popup shows connection status, **Try again** and **Open SDM** when
available. Connection diagnostics stay under Advanced.

Chrome, Edge, Cốc Cốc, Brave, Opera, Vivaldi and Firefox are supported. Store listings
are not published yet. If guided setup is needed, use the app's bundled extension
or a release ZIP; public releases are easier to download than CI artifacts.
[Browser setup, ZIP installation and troubleshooting](docs/BROWSER_EXTENSION.md)
include Firefox's temporary add-on limitation.

## What SDM does

| Feature | What you can choose |
| --- | --- |
| File downloads | Up to 32 connections, pause/resume, retries, speed limits and proxies. Resume checks server validators before reusing partial data. |
| Browser captures | A separate 480 × 320 confirmation window with filename, size, resume support and save folder; **Download now**, **Download later** or **Cancel**. |
| Videos | Quality, format and estimated size in the browser menu, including the selected video on an X feed. Video and audio are merged with ffmpeg when needed. |
| Torrent | Magnet links and `.torrent` files, file selection, resume and sharing limits. |
| Queues | Multiple queues, schedules, numbered URL batches and selecting links from a page. |
| Temporary files | SDM's own folder, configurable in **Settings → General → Temporary folder**. Completed or removed downloads clean their working data; paused/failed downloads keep it for resume. A folder change applies to new downloads. |
| Appearance | Compact dark interface by default, plus light/system themes and Neon. Speed charts, connections and Torrent details open on demand. |
| Windows startup | Optional, off by default; opens SDM in the tray when you sign in. |
| Windows updates | Tested stable releases, background download, manual checks and installation after work stops. |

The default main window is **880 × 560**, constrained to the screen and display
scaling. Labels and actions remain readable in smaller windows.

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/images/sdm-browsers-en.png" alt="Detected browsers with selection checkboxes"><br><sub>Select the browsers you use</sub></td>
    <td width="50%"><img src="docs/images/sdm-capture-compact-en.png" alt="Small download confirmation window"><br><sub>Review a captured download</sub></td>
  </tr>
  <tr>
    <td><img src="docs/images/video-button-en.png" alt="Supo logo and Download button with quality and size choices"><br><sub>Choose video quality in the page</sub></td>
    <td><img src="docs/images/sdm-torrent-en.png" alt="Selecting files from a Torrent"><br><sub>Choose Torrent files</sub></td>
  </tr>
</table>

Desktop and extension screenshots use demo data; layouts come from the app.

## Privacy and license

Download history and settings stay on your computer. The extension passes the
selected download's URL, cookies and referrer to SDM locally. SDM contacts the
selected websites, Torrent peers/trackers and update providers as described in
[Privacy](docs/PRIVACY.md). Torrent peers can see your IP address while sharing.

SDM is licensed under [GPL-3.0-or-later](LICENSE). Only download files you have the
right to download. Bundled tools and dependencies keep their own licenses:
[yt-dlp](https://github.com/yt-dlp/yt-dlp) (Unlicense),
[Deno](https://github.com/denoland/deno) (MIT),
[ffmpeg](https://github.com/yt-dlp/FFmpeg-Builds) (GPL build on Windows),
[librqbit](https://github.com/ikatson/rqbit) (Apache-2.0), and
[Hanken Grotesk](https://github.com/marcologous/hanken-grotesk) (SIL OFL).
Windows bundles ffmpeg; install it through your package manager on macOS/Linux for
merging and converting media. SDM is an independent project.

## Build and contribute

Use stable Rust and Node.js 22 or newer. Linux also needs the
[Tauri system libraries](CONTRIBUTING.md#development-setup).

```sh
cd apps/desktop
npm ci
npm run tauri dev
# Local installer without repository signing credentials:
npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

```sh
# From the repository root:
node extension/build.mjs
cargo run -p sdm-cli -- get https://example.com/file.zip
```

[Contributing](CONTRIBUTING.md) covers setup and checks.
[Report a bug](https://github.com/10s3r2604/supodownloadmanager/issues/new/choose),
[see planned work](docs/ROADMAP.md), or read the
[product guidelines](docs/PRODUCT_GUIDELINES.md).
