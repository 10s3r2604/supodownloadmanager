Supo Download Manager 0.7.1 is the first signed stable update for installed 0.7.0 review builds. In SDM, open Settings → Updates → Check now. After the verified package has downloaded and all downloads and Torrent sharing have stopped, select Install and restart. The update keeps the signing key already trusted by 0.7.0.

The app uses a compact dark window with a toolbar, categories and a download table. Browser captures open a small confirmation window. Start with the computer is optional; automatic updates receive tested stable releases.

The update manifest is published only after full CI, all four desktop builds, all CLI packages and the browser extensions pass. SDM verifies update packages with its pinned Tauri signing key. These updater signatures are separate from Windows Authenticode and macOS notarization.

| System | Package |
| --- | --- |
| Windows 10/11 | `Supo.Download.Manager_*_x64-setup.exe` |
| macOS Apple Silicon | `*_aarch64.dmg` |
| macOS Intel | `*_x64.dmg` |
| Ubuntu / Debian | `*_amd64.deb` |
| Fedora / openSUSE | `*.x86_64.rpm` |
| Linux AppImage | `*.AppImage` |
| CLI | `sdm-cli-*` for your system |
| Browser extension | `sdm-extension-chromium-*.zip` or `sdm-extension-firefox-*.zip` |

Windows builds older than 0.7.0 need to install 0.7.1 manually once. Installed 0.7.0 builds can use the in-app updater. Linux and macOS use their normal installers while automatic installation remains disabled. For Chrome/Edge, unzip the extension, enable Developer mode and select Load unpacked. Reload an existing unpacked extension after replacing its files.

See [automatic updates](https://github.com/10s3r2604/supodownloadmanager/blob/main/docs/AUTO_UPDATES.md) for release and signing details.
