# Contributing to SDM

Thank you for helping! Bug reports, ideas, translations, documentation and code
are all welcome. Issues and discussions can be written in English or
Vietnamese (tiếng Việt).

## Reporting a bug

Please include:

- your operating system and SDM version (`sdm --version`)
- the command you ran, and the URL if it is public
- what you expected and what happened
- the output of the same command with `-vv` added

## Development setup

1. Install [Rust](https://rustup.rs) (latest stable).
2. Clone the repository and build:

   ```sh
   git clone https://github.com/10s3r2604/supodownloadmanager
   cd supodownloadmanager
   cargo build
   ```

3. Run the command line tool from source:

   ```sh
   cargo run -p sdm-cli -- get https://example.com/file.zip
   ```

## Before opening a pull request

Run the same checks as CI:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep pull requests focused on one change.
- Add tests for new behaviour. The integration tests in
  `crates/sdm-core/tests/download.rs` run against a local server that can
  imitate misbehaving servers; extend it when you need a new behaviour.
- Write code, comments and commit messages in English.
- Open an issue first for large changes, so we can agree on the approach.

## Code layout

- `crates/sdm-core`: the download engine. All front-ends (command line,
  desktop app, browser integration) use it, so it must not depend on any user
  interface.
- `crates/sdm-cli`: the `sdm` command.

By contributing you agree that your work is released under the
[GPL-3.0-or-later](LICENSE) license of the project.
