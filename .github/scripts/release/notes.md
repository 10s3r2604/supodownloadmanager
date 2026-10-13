**SDM 0.7.5 · Browser extension 0.6.6**

## Highlights

- Remember extensions that have connected once, separately from whether their browser is currently connected. Show the reported version and offer **Update extension** when the bundled version is newer.
- Update the app-managed extension and attempt Reload of its existing card through normal Windows browser controls. Guided confirmation remains available when needed; SDM waits for the extension to report its new version.
- Discover usable quality and a positive estimated size before showing **Supo logo + Download**. Omit unresolved Best/unknown-size choices; selected X feed clips, cancellation and worker restart recovery remain supported.
- Use single-file resolution/bitrate metadata for estimates and avoid duplicated audio format labels.

## Update and refresh the extension

**Windows SDM 0.7.0 or newer:** **Settings → Updates → Check now → Install and restart**, after transfers and sharing stop. Older Windows versions need the latest installer once. Linux/macOS use their normal installers.

After updating SDM, open **Browsers → Update extension** to refresh extension 0.6.6; finish any browser Reload step. Reload open video tabs. Existing IDs/preferences are preserved; older extensions continue to work during setup. Firefox temporary add-ons need reloading after browser restart.

**Tiếng Việt:** **Cài đặt → Cập nhật → Kiểm tra ngay → Cài và khởi động lại**. Sau đó vào **Trình duyệt**, bấm **Cập nhật tiện ích**, hoàn tất Reload nếu được yêu cầu và tải lại tab video. Không cần nhập port để kết nối.

## Downloads

| Your system | Choose |
| --- | --- |
| Windows 10/11 x64 | `Supo.Download.Manager_0.7.5_x64-setup.exe` (recommended), or the MSI |
| macOS Apple Silicon / Intel | `…_aarch64.dmg` / `…_x64.dmg` |
| Ubuntu / Debian / Mint | `…_amd64.deb` |
| Fedora / openSUSE | `….x86_64.rpm` |
| Linux portable | `…_amd64.AppImage` |
| Chromium extension, manual setup | `sdm-extension-chromium-v0.7.5.zip` |
| Firefox extension, manual setup | `sdm-extension-firefox-v0.7.5.zip` |
| Command line | `sdm-cli-…` for your platform |

Use the regular Chromium extension for full video integration. The `store-chromium` ZIP is for store submission and disables YouTube video controls. Store listings are not published yet.

Installers, update packages and `SHA256SUMS` are included. Updater signatures protect package integrity; Windows publisher signing and macOS notarization are separate work. Native capture, browser cookies/referrer, video quality/size and launch/reconnect are verified in Chrome and Edge with the release Windows executable.

[English guide](https://github.com/10s3r2604/supodownloadmanager#readme) · [Hướng dẫn tiếng Việt](https://github.com/10s3r2604/supodownloadmanager/blob/main/README.vi.md) · [Browser setup](https://github.com/10s3r2604/supodownloadmanager/blob/main/docs/BROWSER_EXTENSION.md)
