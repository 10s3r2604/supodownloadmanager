# Roadmap

SDM is built in phases. Each phase ends with something people can use.

## Phase 1: Download engine and command line ✅ (v0.1)

- [x] Probe the server (size, range support, file name, ETag, Last-Modified)
- [x] Multi-connection downloads with dynamic segment splitting
- [x] Pause and resume, progress saved to disk and checked against the server
- [x] Retries with exponential backoff; failing connections hand over their work
- [x] Single-connection fallback for servers without range support or a known size
- [x] Global and per-download speed limits
- [x] Safe file names (Content-Disposition, URL), no accidental overwrites
- [x] SHA-256 verification
- [x] Custom headers, cookies, referer, user agent, HTTP/SOCKS5 proxy
- [x] `sdm` command line tool: parallel downloads, URL lists, progress bars, Ctrl+C to pause
- [x] Integration tests against a misbehaving local server
- [x] CI on Windows, macOS and Linux; release builds for all three

## Phase 2: Desktop app

Built with [Tauri](https://tauri.app) (Rust + web UI), so it stays small (around 10 MB) and fast.

- [ ] Download list with progress, speed, time left and a segment map
- [ ] "Add URL" dialog that shows the file name, size and resume support before starting
- [ ] Pause, resume, cancel, restart, open file, open folder
- [ ] Categories (Video, Music, Documents, Compressed, Programs) with their own folders
- [ ] Settings: default folder, connections, speed limit, proxy, parallel downloads
- [ ] Download list saved between sessions
- [ ] System tray icon and notifications
- [ ] Clipboard monitoring: offer to download copied links
- [ ] Dark and light themes
- [ ] Sparse files on Windows, so pre-allocating multi-gigabyte downloads is instant on NTFS

## Phase 3: Browser integration

- [ ] Extension for Chrome, Edge and Firefox (Manifest V3)
- [ ] Take over browser downloads by file type and size
- [ ] "Download with SDM" in the context menu for links, images and videos
- [ ] Forward cookies, referer and user agent so downloads that need a login work
- [ ] Talk to the desktop app through native messaging

## Phase 4: Power features

- [ ] Video grabber for HLS (`.m3u8`) and DASH streams, merged with ffmpeg
- [ ] Scheduler: start and stop at set times, shut down or sleep when done
- [ ] Several queues with their own limits
- [ ] Saved logins per site
- [ ] Refresh expired download links without losing progress
- [ ] Batch downloads with numbered patterns (`file[001-100].jpg`)

## Phase 5: Distribution

- [ ] Installers: MSI/NSIS for Windows, DMG for macOS, AppImage/deb/rpm for Linux
- [ ] Automatic updates
- [ ] Packages for winget, Scoop, Homebrew and Flathub
- [ ] Translations, starting with English and Vietnamese

Ideas and help are welcome. Open an issue to discuss before starting on a large change.
