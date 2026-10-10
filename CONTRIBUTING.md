# Contributing to SDM

Thank you for helping! Bug reports, ideas, translations, documentation and code
are all welcome. Issues and discussions can be written in English or
Vietnamese (tiếng Việt).

## Reporting a bug

Please include:

- your operating system, browser and SDM version
- what you did, and the link if it is public
- what you expected and what happened
- for the command line tool, the output of the same command with `-vv` added

## Development setup

1. Install [Rust](https://rustup.rs) (latest stable) and [Node.js](https://nodejs.org) 20 or newer.
2. On Linux, install the libraries the desktop app needs. On Debian/Ubuntu:

   ```sh
   sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
   ```

   Other systems: see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

3. Clone the repository:

   ```sh
   git clone https://github.com/10s3r2604/supodownloadmanager
   cd supodownloadmanager
   ```

### Desktop app

```sh
cd apps/desktop
npm install
npm run tauri dev
```

To work on the interface alone, run `npm run dev` and open
http://localhost:1420 in a browser. Outside the app the interface uses a
simulated backend (`src/mock.ts`) with sample downloads.

### Browser extension

```sh
node extension/build.mjs
```

Then load `extension/dist/chromium` in a Chromium browser with "Load unpacked",
or `extension/dist/firefox/manifest.json` in Firefox via `about:debugging`.

### Download engine and command line tool

```sh
cargo run -p sdm-cli -- get https://example.com/file.zip
```

## Before opening a pull request

Run the same checks as CI:

```sh
cargo fmt --all
(cd apps/desktop && npm run build)    # the desktop crate embeds the built interface
node --test extension/tests/*.test.mjs
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep pull requests focused on one change.
- Add tests for new behaviour. The engine's integration tests in
  `crates/sdm-core/tests/download.rs` run against a local server that can
  imitate misbehaving servers; extend it when you need a new behaviour.
- Write code, comments and commit messages in English. Interface texts live in
  `apps/desktop/src/i18n.ts` and `extension/src/_locales`; please add both
  English and Vietnamese (or ask for help with the translation).
- Open an issue first for large changes, so we can agree on the approach.

## Code layout

- `crates/sdm-core`: the download engine. All front-ends use it, so it must
  not depend on any user interface.
- `crates/sdm-cli`: the `sdm` command.
- `apps/desktop/src-tauri`: the desktop app backend (download list, settings,
  browser bridge, tray, notifications).
- `apps/desktop/src`: the desktop app interface (React + TypeScript).
- `extension`: the browser extension.

By contributing you agree that your work is released under the
[GPL-3.0-or-later](LICENSE) license of the project.
