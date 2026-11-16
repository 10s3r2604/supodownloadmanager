**SDM 0.7.6 · Browser extension 0.6.6**

## Highlights

- **Release a finished Torrent's files.** A finished Torrent used to keep its files open, so an installer unpacking next to its own data (or the game's own setup) could not read them. Files are now released when a transfer completes, when sharing is stopped, and right after a restart.
- Drag completed files from their file icon in all three completion window styles to a folder or another app. Drags use copy mode and reject unfinished or missing downloads.
- Look progress windows up by an in-memory index and sync only the requested download, and check ranged-transfer checkpoints every four seconds instead of continuously. The list keeps refreshing smoothly during large transfers.
- Keep Windows browser setup diagnostics in one script scope and retry address-bar navigation until focus is confirmed.

## Update and refresh the extension

**Windows SDM 0.7.0 or newer:** **Settings → Updates → Check now → Install and restart**, after transfers and sharing stop. Older Windows versions need the latest installer once. Linux/macOS use their normal installers.

After updating SDM, open **Browsers → Update extension** to refresh extension 0.6.6; finish any browser Reload step. Reload open video tabs. Existing IDs/preferences are preserved; older extensions continue to work during setup. Firefox temporary add-ons need reloading after browser restart.

**Tiếng Việt:** **Cài đặt → Cập nhật → Kiểm tra ngay → Cài và khởi động lại**. Sau đó vào **Trình duyệt**, bấm **Cập nhật tiện ích**, hoàn tất Reload nếu được yêu cầu và tải lại tab video. Không cần nhập port để kết nối.

## Downloads

| Your system | Choose |
| --- | --- |
| Windows 10/11 x64 | `Supo.Download.Manager_0.7.6_x64-setup.exe` (recommended), or the MSI |
| macOS Apple Silicon / Intel | `…_aarch64.dmg` / `…_x64.dmg` |
| Ubuntu / Debian / Mint | `…_amd64.deb` |
| Fedora / openSUSE | `….x86_64.rpm` |
| Linux portable | `…_amd64.AppImage` |
| Chromium extension, manual setup | `sdm-extension-chromium-v0.7.6.zip` |
| Firefox extension, manual setup | `sdm-extension-firefox-v0.7.6.zip` |
| Command line | `sdm-cli-…` for your platform |

Use the regular Chromium extension for full video integration. The `store-chromium` ZIP is for store submission and disables YouTube video controls. Store listings are not published yet.

Installers, update packages and `SHA256SUMS` are included. Updater signatures protect package integrity; Windows publisher signing and macOS notarization are separate work. Native capture, browser cookies/referrer, video quality/size and launch/reconnect are verified in Chrome and Edge with the release Windows executable.

[English guide](https://github.com/10s3r2604/supodownloadmanager#readme) · [Hướng dẫn tiếng Việt](https://github.com/10s3r2604/supodownloadmanager/blob/main/README.vi.md) · [Browser setup](https://github.com/10s3r2604/supodownloadmanager/blob/main/docs/BROWSER_EXTENSION.md)
