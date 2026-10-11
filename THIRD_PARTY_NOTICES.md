# Third-party components

SDM's own code is licensed under **GPL-3.0-or-later**, as stated in
[LICENSE](LICENSE) and the workspace manifests. That license does not replace
the licenses or copyright notices of third-party code, fonts or tools.

## Video tools in installers

The release workflow packages the official tools using
[bundle-tools.sh](.github/scripts/bundle-tools.sh). SDM runs them as separate
executables. Check the upstream release used by a particular installer for its
exact version, source and complete license notices.

| Component | Packaged form | License information |
| --- | --- | --- |
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | `yt-dlp_win.zip` on Windows, `yt-dlp_macos` on macOS, `yt-dlp_linux` on Linux | These PyInstaller distributions include GPL-licensed dependencies and are licensed under **GPL v3.0 or later**. yt-dlp's own source uses **Unlicense**. See [Licensing](https://github.com/yt-dlp/yt-dlp#licensing) and [third-party licenses](https://github.com/yt-dlp/yt-dlp/blob/master/THIRD_PARTY_LICENSES.txt). |
| [Deno](https://github.com/denoland/deno) | Official platform executable | **MIT** for Deno's own code, with separately licensed dependencies. See its [LICENSE.md](https://github.com/denoland/deno/blob/main/LICENSE.md). |
| [FFmpeg](https://github.com/yt-dlp/FFmpeg-Builds) | `ffmpeg-master-latest-win64-gpl.zip`, Windows only | A **GPL build**, whose terms include the licenses of its enabled libraries. See the [build project](https://github.com/yt-dlp/FFmpeg-Builds) and [FFmpeg legal information](https://ffmpeg.org/legal.html). macOS and Linux use a separately installed copy of FFmpeg. |

Redistributing these executables requires preserving their notices and
satisfying their applicable source-distribution requirements. The Unlicense
for yt-dlp's own source does not describe the entire standalone executable.

## Libraries and font

| Component | License / notices |
| --- | --- |
| [librqbit](https://github.com/ikatson/rqbit), version 9.0.1 | **Apache-2.0**. The vendored tracker patch retains its [license](third_party/librqbit-tracker-comms/LICENSE); its changes are described in its [README](third_party/librqbit-tracker-comms/README.md). |
| [Tauri](https://github.com/tauri-apps/tauri) | **MIT OR Apache-2.0** for Tauri's own code. Its dependencies retain their own licenses. |
| [React](https://github.com/facebook/react) | **MIT**. |
| [Inter](https://rsms.me/inter/) | **SIL Open Font License 1.1**; the [font license](apps/desktop/src/assets/fonts/Inter-LICENSE.txt) includes its copyright notice. |

This is an overview of key components, not a complete dependency inventory.
Rust and JavaScript versions are recorded in `Cargo.lock` and
`apps/desktop/package-lock.json`. Their dependency manifests record individual
licenses, including MPL-2.0 for some Rust dependencies. Those licenses and any
required notices remain applicable to those components.
