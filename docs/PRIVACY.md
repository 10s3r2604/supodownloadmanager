# Privacy policy

_Supo Download Manager (SDM) desktop app and browser extension. Last updated: October 2026._

**In short: SDM collects nothing. It has no servers, no accounts, no analytics and no ads.
Everything stays on your computer.**

## The browser extension

The extension exists to hand downloads from your browser to the SDM app on the same
computer. To do that it reads, only when you download something or click its button:

- the address of the file, video page or magnet link,
- the address of the page it came from (the referer),
- the cookies the website set for that address, and the browser's user agent, so that
  downloads which need you to be logged in keep working.

This information is sent **only** to the SDM app on your own computer, at
`http://127.0.0.1` (port 17360 unless you change it). It is never sent to the
developers of SDM or to anyone else. The extension does not read the content of
the pages you visit, does not record your browsing history and does not track you.

Its settings (on/off switches, the port and the minimum file size) are kept in the
browser's extension storage on your computer.

## The desktop app

The app downloads files from the addresses you give it, using the cookies and referer
that came with them, just as your browser would. It keeps its download list and
settings in its data folder on your computer. Cookies are deleted from that list as
soon as a download is complete.

The app contacts other servers only to do what you ask:

- the servers you download from;
- for videos, the video site, through [yt-dlp](https://github.com/yt-dlp/yt-dlp), which
  SDM downloads from its official GitHub releases (and keeps up to date) the first time
  you download a video, together with [ffmpeg](https://ffmpeg.org) on Windows;
- for torrents, BitTorrent trackers, the DHT network and other peers. Like every
  BitTorrent program, SDM shares the parts of a torrent it has with other peers, who
  can see your IP address. You can stop sharing finished torrents in Settings.

## Children

SDM is a general-purpose tool and does not knowingly collect anything from anyone.

## Changes and contact

Changes to this policy are published in this file, with the date above. Questions:
open an issue at <https://github.com/10s3r2604/supodownloadmanager/issues>.

---

## Chính sách quyền riêng tư (tóm tắt tiếng Việt)

SDM **không thu thập bất kỳ dữ liệu nào**: không có máy chủ, không tài khoản, không
thống kê, không quảng cáo. Extension chỉ đọc địa chỉ file/trang video/link magnet bạn
tải, trang nguồn, cookie của trang đó và user agent, rồi gửi **duy nhất** cho ứng dụng
SDM trên chính máy bạn (`127.0.0.1`), để tải được cả các link cần đăng nhập. Không gửi
cho nhà phát triển hay bất kỳ ai khác. Ứng dụng xoá cookie khỏi danh sách tải ngay khi
file tải xong. Khi tải torrent, giống mọi chương trình BitTorrent, SDM chia sẻ dữ liệu
với các máy khác và họ thấy được địa chỉ IP của bạn.
