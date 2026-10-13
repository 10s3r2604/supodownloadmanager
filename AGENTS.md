# Working on Supo Download Manager

Read [product guidelines](docs/PRODUCT_GUIDELINES.md) before changing the app,
extension, documentation or repository presentation. They record the owner's
chosen defaults and behavior. Keep README.md and README.vi.md consistent.

- Use the existing checkout. Create a Git worktree only when the user asks.
- Keep the Supo identity and the name Torrent in user-facing text.
- Keep the compact dark default, short video button and hidden connection
  diagnostics. Preserve useful choices and readable English/Vietnamese labels.
- Document verified behavior accurately, including browser confirmation and
  platform limitations. Distinguish updater signatures from publisher signing.
- Documentation-only changes do not require an app version bump. Check links,
  asset paths and current instructions instead of adding unrelated tests.
- During GitHub CI, inspect metadata while the workflow is running. Retrieve
  logs only after the workflow and all its jobs have completed.
- Preserve dependency licenses, protocol identifiers and private signing keys.
  Never place private keys, cookies or credential-bearing URLs in Git or logs.
