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
- [x] CI on Windows, macOS and Linux; release builds for all three

## Phase 2: Desktop app ✅ (v0.2)

Built with [Tauri](https://tauri.app) (Rust + web interface), so it stays small and fast.

- [x] Download list with progress, speed, time left and a segment map
- [x] "Add download" dialog that shows the file name, size and resume support before starting
- [x] Pause, resume, delete (optionally moving files to the Trash), open file, show in folder
- [x] Categories (Video, Music, Documents, Compressed, Programs, Images) with their own folders
- [x] Download queue with a limit on parallel downloads
- [x] Settings: folder, connections, speed limit, proxy, language, theme
- [x] Download list saved between sessions; quitting pauses and saves everything
- [x] System tray icon and notifications
- [x] Clipboard monitoring: offer to download copied links
- [x] English and Vietnamese, light and dark themes
- [ ] Sparse files on Windows, so pre-allocating multi-gigabyte downloads is instant on NTFS
- [x] Start with the computer (optional, opens in the tray)
- [x] Signed stable automatic updates on Windows; see [release setup](AUTO_UPDATES.md)
- [ ] Automatic update installation/restart validation on Linux and macOS

## Phase 3: Browser integration ✅ (v0.2)

- [x] Extension for Chrome, Edge, Cốc Cốc, Brave, Opera, Vivaldi and Firefox (Manifest V3)
- [x] Take over browser downloads by file type and size
- [x] "Download with SDM" in the context menu for links, images, videos and audio
- [x] Forward cookies, referer and user agent so downloads that need a login work
- [x] Local bridge that only browser extensions can use
- [x] Find installed browsers and guide the extension installation from the app
- [ ] Publish in the Chrome Web Store, Edge Add-ons and on addons.mozilla.org (one-click install);
      packages, listing and images are ready, see [STORE.md](STORE.md)

## Phase 4: Looks, progress windows and videos ✅ (v0.3)

- [x] Compact IDM-style toolbar and download table, dark by default; light/system themes and on-demand speed charts, connections and torrent details
- [x] A progress window for every download, in three styles (speed graph, ring, classic)
- [x] "Show a dialog when done" and "Open the file when done" per download, "Don't show this dialog again"
- [x] Video grabber: YouTube, Facebook, X, TikTok and ~1800 other sites through yt-dlp, with a quality picker
- [x] HLS (`.m3u8`) and DASH streams, video and sound merged with ffmpeg
- [x] "Download this video" button on videos in the browser, with the page's cookies
- [x] yt-dlp downloaded on first use (SHA-256 checked) and kept up to date; ffmpeg.exe fetched on Windows
- [ ] Playlists and channels
- [ ] Subtitles

## Phase 5: Torrent ✅ (v0.4)

Built on [librqbit](https://github.com/ikatson/rqbit), a Torrent engine in Rust.

- [x] Magnet links and `.torrent` files (from a link, a file, drag and drop or the command line)
- [x] "Add torrent" dialog with the files to choose, as a folder tree
- [x] Torrents in the same list, queue and progress windows as other downloads; peers and upload speed
- [x] Fast resume after a restart, without checking all the data again
- [x] Sharing finished torrents back until a ratio, with an upload limit
- [x] Browser: magnet link clicks and `.torrent` downloads go to SDM
- [x] Optionally open magnet links from other programs with SDM (Windows, Linux)
- [ ] Per-torrent peer list and piece map
- [ ] Creating torrents

## Phase 6: Power features (in progress, v0.6)

- [x] Scheduler: start and stop at set times, shut down or sleep when done
- [x] Several queues with their own limits
- [ ] Saved logins per site
- [x] Refresh expired download links without losing progress
- [x] "Download all links" on a page, batch downloads with numbered patterns

## Phase 7: Distribution

- [x] Installers built by CI: MSI/NSIS for Windows, DMG for macOS, deb/rpm/AppImage for Linux
- [ ] Code signing for Windows and macOS
- [ ] Packages for winget, Scoop, Homebrew and Flathub
- [ ] More languages

Ideas and help are welcome. Open an issue to discuss before starting on a large change.
