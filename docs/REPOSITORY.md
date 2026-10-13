# GitHub repository presentation

Keep the GitHub **About** fields, READMEs and release descriptions aligned with
the [product guidelines](PRODUCT_GUIDELINES.md). English documentation starts at
[README.md](../README.md); Vietnamese starts at [README.vi.md](../README.vi.md).

## About fields

**Description**

```text
Free, open-source download manager for Windows, macOS and Linux. Resume files, choose video quality, manage Torrent downloads and connect your browsers.
```

**Website**

```text
https://github.com/10s3r2604/supodownloadmanager/releases/latest
```

**Topics**

```text
download-manager download video-downloader torrent magnet-links browser-extension native-messaging rust tauri react windows macos linux open-source
```

## Release descriptions

- Lead with what changed in that version and how installed users update.
- Link the latest stable release and the correct installer/extension packages.
- Explain extension refresh and any browser confirmation still needed.
- Keep technical publication provenance, platform availability and signing
  claims accurate. Updater signatures do not imply Authenticode/notarization.
- Correct prose without replacing published installers, update manifests,
  signatures, checksums or tags. Older releases can point users to the latest.
- Update `.github/scripts/release/notes.md` when a new app version is prepared.
  Documentation and presentation changes alone do not bump the app version.

## Images

Current interface captures are in [images/](images/). They use demo data and must
show the shipped layout, including browser selection and collapsed diagnostics.
Historical options stay in [design/](design/), marked as design references.

The existing [social-preview.png](store/social-preview.png) can be uploaded under
**Settings → General → Social preview** (1280 × 640). Repository APIs do not expose
that upload through the ordinary About edit. Browser-store assets are in
[store/](store/); confirm screenshots and reviewer notes against the current app
before submitting a listing. See [STORE.md](STORE.md).
