# Privacy policy

_Supo Download Manager (SDM) desktop app and browser extension. Last updated: October 2026._

SDM has no developer-operated account service, analytics or ads. Download history
and settings stay on your computer. File downloads, video discovery, Torrent
connections and update checks contact the services described below.

## The browser extension

The extension exists to hand downloads from your browser to the SDM app on the same
computer. To do that it reads, only when you download something or click its button:

- the address of the file, video page or magnet link,
- the address of the page it came from (the referer),
- the cookies the website set for that address, and the browser's user agent, so that
  downloads which need you to be logged in keep working.

This information is handed to the SDM app on your own computer, at
`http://127.0.0.1` (port 17360 unless you change it). It is never sent to the
developers of SDM. The extension inspects links and video elements on the current
page to display download controls and identify the selected video. It does not
record your browsing history or send analytics.

Its settings (on/off switches, the port and the minimum file size) are kept in the
browser's extension storage on your computer.

## The desktop app

The app downloads files from the addresses you give it, using the cookies and referer
that came with them, just as your browser would. It keeps its download list and
settings in its data folder on your computer. Cookies are deleted from that list as
soon as a download is complete.

The app contacts these services to perform downloads and enabled update checks:

- the servers you download from;
- for videos, the video site through [yt-dlp](https://github.com/yt-dlp/yt-dlp).
  Installers bundle yt-dlp and Deno, plus [ffmpeg](https://ffmpeg.org) on Windows.
  SDM checks official GitHub releases to update its own copy of yt-dlp and downloads
  missing tools when needed;
- for Torrent downloads, trackers, the DHT network and other peers. SDM shares
  the parts of a torrent it has with other peers, who
  can see your IP address. You can stop sharing finished torrents in Settings.
- for Windows automatic updates, GitHub release metadata and package downloads.
  These checks are enabled by default and can be turned off in
  **Settings → Updates**. SDM does not attach download history, browser cookies or
  an SDM account identifier to these checks.

## Children

SDM is a general-purpose tool and does not knowingly collect anything from anyone.

## Changes and contact

Changes to this policy are published in this file, with the date above. Questions:
open an issue at <https://github.com/10s3r2604/supodownloadmanager/issues>.

---

## Chính sách quyền riêng tư (tóm tắt tiếng Việt)

SDM không có dịch vụ tài khoản, thống kê hay quảng cáo của nhà phát triển. Lịch sử
tải và cài đặt được lưu trên máy bạn. Extension đọc địa chỉ tải, trang nguồn, cookie
và user agent cần cho lần tải rồi chuyển cho SDM trên chính máy (`127.0.0.1`).
Extension kiểm tra link và phần tử video trên trang để hiện nút tải, không ghi lại
lịch sử duyệt web hay gửi thống kê cho nhà phát triển. SDM dùng thông tin đó để tải
từ website và xoá cookie khỏi danh sách tải khi file hoàn tất.

Khi tải Torrent, SDM kết nối tracker, DHT và các máy khác; việc chia sẻ dữ liệu để
tải tiếp cho người khác làm họ thấy được địa chỉ IP của bạn. Có thể dừng chia sẻ
torrent đã hoàn tất trong Cài đặt. Cập nhật tự động trên Windows kết nối GitHub
để kiểm tra và tải bản phát hành, không gửi lịch sử tải hay cookie trình duyệt;
bạn có thể tắt trong **Cài đặt → Cập nhật**.
