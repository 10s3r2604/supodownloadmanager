Supo Download Manager 0.7.2 adds first-run browser setup. SDM detects installed browsers, lets you select several and prepares the bundled extension for them. On Windows it attempts loading through each selected browser's normal UI controls. Automatic loading is validated on Chrome. Other browsers may need confirmation; the remaining steps stay visible. Connection status comes from the extension. Existing installations keep their setup and can open Browsers to use the new selection flow.

Extension 0.6.4 has a compact Supo logo + Download button. Replace an existing unpacked extension's files, click Reload in the browser extension manager and reload open video tabs. Setup preserves separately installed newer extension versions. Firefox temporary add-ons still require reloading after browser restart; Linux/macOS use the guided setup.

Installed Windows SDM 0.7.0/0.7.1 can update through Settings → Updates → Check now, then Install and restart after transfers and sharing stop. Update packages retain the pinned signing key. These signatures are separate from Windows Authenticode and macOS notarization.

Downloads: Windows x64 setup.exe/MSI; macOS Intel/Apple Silicon DMG; Linux deb/rpm/AppImage; CLI packages; Chromium, Firefox and Chromium store extension ZIPs. The Chromium store build leaves YouTube video controls disabled.

Release publication requires the successful full CI run for the exact main commit, every platform package, valid updater signatures and signed versions, and matching uploaded checksums. Ordinary commits without a new version skip publication safely.
