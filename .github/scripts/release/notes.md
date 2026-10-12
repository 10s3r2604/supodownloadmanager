Supo Download Manager uses a compact dark window with a toolbar, categories and a download table. Browser captures open a small confirmation window. Start with the computer is optional; automatic updates receive tested stable releases.

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

Install 0.7.0 manually once to enable automatic updates on Windows. Linux and macOS use their normal installers while automatic installation remains disabled. For Chrome/Edge, unzip the extension, enable Developer mode and select Load unpacked. Reload an existing unpacked extension after replacing its files.

See [automatic updates](https://github.com/10s3r2604/supodownloadmanager/blob/main/docs/AUTO_UPDATES.md) for release and signing details.
