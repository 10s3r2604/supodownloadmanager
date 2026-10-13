# Product guidelines

These choices were approved by the repository owner. Use them when developing
SDM or updating its GitHub presentation. Current implementation: **SDM 0.7.5**
and browser extension **0.6.6**. [Changelog](../CHANGELOG.md) records shipped changes;
[Roadmap](ROADMAP.md) records work still to do.

## Identity and interface

- Use **Supo Download Manager**, **SDM** and the Supo assets. Use **Torrent** in
  the interface, documentation and repository description. Preserve standard
  MIME types and upstream license notices needed for interoperability.
- The owner selected [option 3](design/idm-ui-review.md): a compact dark layout
  with Supo colors, toolbar, categories, queues and a download table. Main window:
  **880 × 560** by default; browser confirmation: **480 × 320**. Respect monitor
  work area and DPI, and keep controls usable in smaller windows.
- Keep the ordinary interface minimal. Progress, speed, time left, filename,
  destination, size, resume support and the action appropriate to each state
  remain available. Open detailed charts/connections on demand.
- Maintain English and Vietnamese together. Size controls for their actual
  labels, including Vietnamese, and keep confirmation actions visible when
  content scrolls.

## Browser and video flow

- On first launch, detect installed browsers and let the user tick several.
  **Set up selected** prepares the extension and attempts normal browser UI
  loading on Windows. Chrome automatic loading is verified; other browsers
  retain clear guided confirmation when needed. Do not rewrite browser profiles
  or enterprise policies to force installation.
- Remember observed extension versions separately from live connection. Offer
  **Update extension** for the app-bundled version, reload the existing copy and
  confirm the reported version before showing completion. Preserve newer versions.
- Use the Supo logo and the short caption **Download** on the video button.
  Resolve usable quality and a positive estimated size before revealing it; omit
  unresolved placeholder choices.
  Quality, format and estimated size belong in the page menu. Resolve the
  selected X post/clip from the feed rather than forwarding its Home timeline.
- Browser captures open a small separate window with **Download now / Download
  later / Cancel**, while the main window stays in the background.
- Connect automatically through Native Messaging. Show status, **Try again**
  and **Open SDM** when available. Keep legacy connection and Torrent port
  controls under collapsed advanced diagnostics; do not require ordinary users
  to configure a port.
- Preserve extension IDs/preferences and compatibility with older installed
  clients. Do not resubmit an uncertain native download request over HTTP.

## Temporary data, startup and updates

- Store partial data and resume state in SDM's own temporary folder. Users can
  choose that folder in Settings. New downloads use the new choice; existing
  downloads retain their saved location. Keep paused/failed data for resuming.
  Clean completed/removed working data after durable history bookkeeping.
- Starting with Windows is optional and off by default. Login startup opens
  the tray, without taking focus or showing the main window.
- Automatic updates receive **tested stable releases**, not every main commit.
  A new stable version must pass exact-commit full CI, all package builds and
  signature/checksum checks. Keep manual checks and the off switch available.
- Install after transfers, processing and sharing stop and history is saved.
  Windows SDM 0.7.0+ has an updater; older versions need one manual install.
  Linux/macOS automatic installation remains planned until validated.
- Updater signing protects package integrity. Windows Authenticode and macOS
  notarization are separate work; do not imply that SmartScreen warnings have
  been eliminated. Keep the private key outside Git and preserve the pinned
  public key for installed clients.

## GitHub presentation

- Put the latest stable download and English/Vietnamese installation guidance
  first. Explain which installer/extension ZIP to choose; CI artifacts are
  review builds. Do not advertise unsubmitted store listings as available.
- Use current interface captures and label demo data. Keep historical design
  mockups as reference, separate from screenshots of the shipped interface.
- Keep the current **GPL-3.0-or-later** license consistent across the repository.
  The earlier MIT question was a proposal, not a decision to relicense. Retain
  bundled-tool and dependency licenses.
- Use release notes that describe that version. Preserve published installers,
  signatures, checksums and tags when correcting release description text.
