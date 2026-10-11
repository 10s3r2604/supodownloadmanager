# Browser extension

The SDM extension hands downloads from your browser over to the SDM desktop
app. It works in Chrome, Edge, Cốc Cốc, Brave, Opera, Vivaldi, Chromium and
Firefox.

<img src="images/extension-popup.png" alt="Extension popup" width="320">

## What it does

- **Takes over downloads.** When the browser starts downloading a file of a
  type chosen in SDM (Settings → Browser & clipboard), the extension cancels
  the browser download and sends the link to SDM, together with the page's
  cookies, the referer and the browser's user agent. Downloads that need a
  login therefore work in SDM too.
  It also checks when Chrome determines or updates the file name, so GitHub
  release assets served through an opaque redirect as `application/octet-stream`
  are captured as EXE/ZIP. SDM receives the original request URL and its cookies.
  Multiple download events create only one SDM handover; a refused handover
  returns the link to the browser.
- **Puts a "Download this video" button on videos.** Hover any video that is
  at least 200 × 112 pixels and the button appears in its top right corner
  (also in full screen). Click to choose quality, format and estimated file
  size right on the page. SDM discovers formats in the background; a download
  starts only after you choose an option. On X feeds the extension finds the
  hovered video's post and retains its clip index. Errors and **Try again**
  stay in the page menu, and the X video context menu opens that same picker.
  The button can be turned off in the popup. SDM must be running.

  <img src="images/video-button-en.png" alt="Download this video" width="416">

- **Opens magnet links in SDM.** A click on a magnet link shows SDM's torrent
  dialog instead of asking which program to use (when SDM is running; can be
  turned off in the popup). `.torrent` files the browser downloads go to SDM too.
- **Adds "Download link with SDM"** to the right-click menu of links (also
  magnet links), images and audio, and **"Download the video on this page with SDM"** to the menu of
  videos and pages. Sites such as YouTube replace the video's own menu, so
  right-click next to the player there.
- **Falls back gracefully.** When SDM is not running, the browser downloads as
  usual. When SDM refuses a link, the extension gives it back to the browser.

The popup shows whether SDM is running and lets you turn capturing and the
video button off, set a minimum file size and change the port.

## Installing

### The easy way

Open SDM, click **Browsers**, choose your browser and click **Install
extension**. SDM opens the browser's extension page and shows the steps.

Browsers do not allow other programs to install extensions silently, so the
last click always happens in the browser. Once the extension is published in
the Chrome Web Store and on addons.mozilla.org, that click is simply "Add to
browser". Until then it is installed from a folder:

### Chrome, Edge, Cốc Cốc, Brave, Opera, Vivaldi

1. In SDM, click **Browsers → Install extension** next to your browser. SDM
   copies the extension to its data folder and opens the extension page
   (`chrome://extensions`, `edge://extensions`, ...).
2. Turn on **Developer mode**.
3. Click **Load unpacked** and choose the folder SDM shows (or drag the folder
   onto the page).

When updating an unpacked extension, replace its files and click **Reload**
on its card in the browser's extension page, then reload open video tabs.
An app update alone does not reload an already installed unpacked extension.
If loading a new folder, remove the old SDM extension first so only one is active.

### Firefox

1. In SDM, click **Browsers → Install extension** next to Firefox. Firefox
   opens `about:debugging`.
2. Click **Load Temporary Add-on…** and choose `manifest.json` in the folder
   SDM shows.

Firefox removes temporary add-ons when it restarts. A permanent version will be
available on addons.mozilla.org.

## How the extension talks to SDM

SDM runs a small HTTP server on `127.0.0.1:17360` (configurable). It only
accepts requests that:

- carry the `X-SDM-Client` header, which a web page could only send after a
  CORS preflight that SDM never approves, and
- come from a browser extension (`chrome-extension://`, `moz-extension://`)
  when an `Origin` is present.

So websites cannot start downloads in SDM. By default SDM also shows the
"Add download" dialog for every download from the browser, so nothing is
downloaded without your confirmation.

| Endpoint             | Purpose                                                  |
| -------------------- | -------------------------------------------------------- |
| `GET /api/ping`      | Is SDM running? Returns its version and settings          |
| `POST /api/download` | Hand over a download: `url`, `referrer`, `cookies`, `userAgent`, `fileName`, `fileSize` |
| `POST /api/media`    | Hand over a page with a video: `pageUrl`, `title`, `mediaUrl`, `cookies` (list), `userAgent`, `referrer` |
| `POST /api/media/probe` | Read qualities in the background for the page menu |
| `POST /api/media/add` | Start the quality explicitly selected in the page menu |

The cookies of a video page go only to the SDM app on your computer. SDM
writes them into the download's private work folder for yt-dlp and deletes
them as soon as the video is complete.

## Building

```sh
node extension/build.mjs
```

This writes `extension/dist/chromium`, `extension/dist/chromium-store` and
`extension/dist/firefox`. The desktop app bundles the first and the last; the
store build is the one for the Chrome Web Store and Edge Add-ons (see
[STORE.md](STORE.md)). The release workflow publishes all three as zip files.

## Publishing (maintainers)

See [STORE.md](STORE.md) for the store packages, listing texts, privacy answers, images
and the publishing workflow.
