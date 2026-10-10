# Publishing the browser extension

Everything needed to list the SDM extension in the **Chrome Web Store**, **Microsoft
Edge Add-ons** and **Firefox Add-ons (AMO)**: which package goes where, the listing
texts, the answers to the privacy forms, the images, and how to automate updates.

| Store | Package | Account |
| --- | --- | --- |
| Chrome Web Store (also for Cốc Cốc, Brave, Opera, Vivaldi) | `sdm-extension-store-chromium-<version>.zip` | [Developer dashboard](https://chrome.google.com/webstore/devconsole), one-time US$5 fee, identity check |
| Microsoft Edge Add-ons | `sdm-extension-store-chromium-<version>.zip` | [Partner Center](https://partner.microsoft.com/dashboard/microsoftedge/), free |
| Firefox Add-ons | `sdm-extension-firefox-<version>.zip` | [AMO Developer Hub](https://addons.mozilla.org/developers/), free |

The zips are attached to every [GitHub release](https://github.com/10s3r2604/supodownloadmanager/releases),
or build them with `node extension/build.mjs` (see `extension/dist/`).

> **Why a separate "store" build for Chrome and Edge?** The Chrome Web Store rejects
> extensions that download YouTube videos ("facilitating download of YouTube videos" is
> one of its listed reasons for rejection). The store build therefore keeps the
> "Download this video" button and menu away from YouTube; the SDM app itself can still
> download YouTube links pasted into it. The Firefox build and the build shipped with
> the app are unchanged. Never mention YouTube in the Chrome or Edge listing.

## 1. First submission (by hand, once per store)

1. Create the item and upload the zip.
2. Fill in the listing with the texts below (English first, then add Vietnamese as a
   second language).
3. Upload the images from [`docs/store/`](store/).
4. Fill in the privacy section with the answers below; privacy policy URL:
   `https://github.com/10s3r2604/supodownloadmanager/blob/main/docs/PRIVACY.md`
5. Add the reviewer notes below and submit. Reviews take from a day to a few weeks;
   broad host permissions often mean a longer, manual review.
6. Once listed, put the store links into `CHROME_WEB_STORE_URL` and `FIREFOX_ADDONS_URL`
   in `apps/desktop/src-tauri/src/browsers.rs`, so the app's **Install extension** button
   opens the store (Edge, Cốc Cốc, Brave, Opera and Vivaldi can all install from the
   Chrome Web Store).

## 2. Listing texts

**Name:** Supo Download Manager

**Category:** Productivity → Tools (Chrome) · Productivity (Edge) · Download Management (Firefox)

**Short description** (from the manifest, ≤ 132 characters):

- EN: Send downloads, videos and magnet links to Supo Download Manager, the free download accelerator for Windows, macOS and Linux.
- VI: Chuyển file tải, video và link magnet sang Supo Download Manager, trình tăng tốc tải miễn phí cho Windows, macOS và Linux.

**Detailed description (EN):**

```
Supo Download Manager (SDM) is a free, open-source download accelerator for Windows,
macOS and Linux. This extension connects your browser to the SDM app.

• Faster downloads: files you download are handed to SDM, which uses many
  connections at once and can pause and resume at any time, even after a restart.
• Downloads that need a login keep working: the extension passes the page's cookies,
  referer and user agent to SDM on your computer.
• Videos from web pages: hover a video and click "Download this video", then pick the
  quality in SDM.
• Magnet links: click a magnet link and SDM shows what is inside the torrent so you can
  pick the files.
• Right-click any link, image or audio and choose "Download with SDM".
• Choose which file types SDM takes over, set a minimum size, or switch it all off
  from the toolbar button.

The extension needs the free SDM app: https://github.com/10s3r2604/supodownloadmanager
When the app is not running, your browser downloads as usual.

Privacy: nothing is collected. The extension talks only to the SDM app on your own
computer (127.0.0.1). No accounts, no analytics, no ads. Source code (GPL-3.0):
https://github.com/10s3r2604/supodownloadmanager
```

**Detailed description (VI):**

```
Supo Download Manager (SDM) là trình tăng tốc tải xuống miễn phí, mã nguồn mở cho
Windows, macOS và Linux. Extension này kết nối trình duyệt với ứng dụng SDM.

• Tải nhanh hơn: file bạn tải được chuyển sang SDM, tải bằng nhiều kết nối cùng lúc,
  tạm dừng và tải tiếp bất cứ lúc nào, kể cả sau khi tắt máy.
• Link cần đăng nhập vẫn tải được: extension gửi cookie, trang nguồn và user agent cho
  SDM trên máy bạn.
• Tải video trên trang web: rê chuột lên video, bấm "Tải video này" rồi chọn chất lượng.
• Link magnet: bấm vào link magnet, SDM hiện nội dung torrent để bạn chọn tệp muốn tải.
• Chuột phải vào link, ảnh, âm thanh và chọn "Tải bằng SDM".
• Chọn loại file SDM nhận, đặt dung lượng tối thiểu, hoặc tắt hẳn ngay trên thanh công cụ.

Cần cài ứng dụng SDM miễn phí: https://github.com/10s3r2604/supodownloadmanager
Khi ứng dụng không chạy, trình duyệt tải như bình thường.

Quyền riêng tư: không thu thập gì. Extension chỉ nói chuyện với ứng dụng SDM trên chính
máy bạn (127.0.0.1). Không tài khoản, không thống kê, không quảng cáo.
```

## 3. Privacy form answers

**Single purpose:** Hands downloads, video pages and magnet links from the browser over to
the Supo Download Manager desktop app installed on the same computer.

**Permission justifications:**

| Permission | Why it is needed |
| --- | --- |
| `downloads` | To notice downloads of the file types the user chose, cancel them in the browser and hand them to the SDM app, and to give them back to the browser when the app is not running. |
| `contextMenus` | Adds "Download with SDM" to the right-click menu of links, images, audio and video pages. |
| `cookies` | Downloads behind a login need the site's cookies; they are read only for the address being downloaded and sent only to the SDM app on 127.0.0.1. |
| `storage` | Keeps the extension's own settings (switches, port, minimum size). |
| `alarms` | Checks once a minute whether the SDM app is running, to show its status. |
| Host permission `<all_urls>` | Downloads and videos can come from any website, and cookies must be readable for any download address. The extension also talks to the app at http://127.0.0.1. |
| Content scripts on all sites | Show the "Download this video" button when the pointer is over a video, and open magnet links in SDM. They read nothing else from the page. |

**Remote code:** No. All code is in the package; nothing is downloaded or evaluated.

**Data usage:** The extension does not collect or transmit user data off the device. The
download address, referer, cookies and user agent go only to the companion app on the same
computer (127.0.0.1) to perform the download the user started. If a store form insists on
listing data types handled locally, tick *Authentication information* (cookies) and
*Web history* (download addresses) and state "used only locally, never transmitted to the
developer or third parties". Certify: not sold, not used for unrelated purposes, not used
for creditworthiness.

**Firefox:** the manifest declares `data_collection_permissions: { required: ["none"] }`.
The source is not minified or generated, so no source upload is needed.

## 4. Reviewer notes

```
This extension works together with the free, open-source Supo Download Manager desktop
app (GPL-3.0): https://github.com/10s3r2604/supodownloadmanager/releases
Without the app it does nothing and the browser downloads normally.

To test:
1. Install and open the app (Windows .exe/.msi, macOS .dmg or Linux .deb/.AppImage).
2. The toolbar popup shows "Connected to SDM".
3. Download any .zip or .pdf: SDM opens an "Add download" dialog instead of the browser
   downloading it.
4. Right-click a link: "Download link with SDM".
5. Open a page with an HTML5 video (e.g. https://test-videos.co.uk/bigbuckbunny/mp4-h264)
   and hover the video: a "Download this video" button appears.
6. Click a magnet link (e.g. on https://webtorrent.io/free-torrents): SDM shows the files
   of the torrent.
All communication is with http://127.0.0.1:17360 only.
```

## 5. Images

All in [`docs/store/`](store/), English (`-en`) and Vietnamese (`-vi`):

| Image | Size | Chrome | Edge | Firefox |
| --- | --- | --- | --- | --- |
| `screenshot-1…5-*.png` | 1280×800 | up to 5 | up to 10 | yes |
| `promo-small-*.png` | 440×280 | required | optional | – |
| `promo-marquee-*.png` | 1400×560 | optional | – | – |
| `logo-300.png` | 300×300 | – | store logo | – |
| `extension/src/icons/128.png` | 128×128 | store icon | – | – |

## 6. Automatic updates

After the first version is live in a store, later versions can be uploaded by the
**Publish extension** workflow (Actions → Publish extension → Run workflow). Add these
repository secrets (Settings → Secrets and variables → Actions); a store whose secrets
are missing is skipped.

| Store | Secrets | Where to get them |
| --- | --- | --- |
| Chrome | `CWS_CLIENT_ID`, `CWS_CLIENT_SECRET`, `CWS_REFRESH_TOKEN`, `CWS_PUBLISHER_ID`, `CWS_EXTENSION_ID` | Google Cloud OAuth client for the Chrome Web Store API ([guide](https://github.com/fregante/chrome-webstore-upload-keys)); publisher ID from the dashboard URL; extension ID from the item page |
| Edge | `EDGE_API_KEY`, `EDGE_CLIENT_ID`, `EDGE_PRODUCT_ID` | Partner Center → Microsoft Edge → Publish API; product ID on the extension's overview page |
| Firefox | `AMO_JWT_ISSUER`, `AMO_JWT_SECRET` | [AMO API keys](https://addons.mozilla.org/developers/addon/api/key/) |

Bump `VERSION` in `extension/build.mjs` before each upload; stores refuse a version they
already have.
