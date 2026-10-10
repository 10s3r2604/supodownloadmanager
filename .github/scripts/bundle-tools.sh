#!/usr/bin/env bash
# Puts yt-dlp, Deno and (on Windows) ffmpeg into apps/desktop/src-tauri/tools,
# so the installers ship them and videos work without downloading anything.
#
#   bundle-tools.sh <Windows|macOS|Linux> <deno asset, e.g. deno-x86_64-pc-windows-msvc.zip>
set -euo pipefail
os=$1
deno_asset=$2
out=apps/desktop/src-tauri/tools
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$out"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }

# yt-dlp, checked against its published checksums.
ytdlp=https://github.com/yt-dlp/yt-dlp/releases/latest/download
case $os in
  Windows) asset=yt-dlp_win.zip ;;
  macOS) asset=yt-dlp_macos ;;
  Linux) asset=yt-dlp_linux ;;
  *) echo "unknown system $os" >&2; exit 1 ;;
esac
fetch "$ytdlp/SHA2-256SUMS" "$tmp/sums"
expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a { print $1 }' "$tmp/sums")
fetch "$ytdlp/$asset" "$tmp/$asset"
actual=$(sha256 "$tmp/$asset")
[ -n "$expected" ] && [ "$expected" = "$actual" ] || { echo "yt-dlp checksum mismatch" >&2; exit 1; }
if [ "$os" = Windows ]; then
  unzip -q -o "$tmp/$asset" -d "$out/yt-dlp"
  test -f "$out/yt-dlp/yt-dlp.exe"
else
  cp "$tmp/$asset" "$out/yt-dlp"
  chmod +x "$out/yt-dlp"
fi
printf '%s' "$expected" > "$out/yt-dlp.sha256"

# Deno, which yt-dlp needs for all of YouTube's formats.
deno=https://github.com/denoland/deno/releases/latest/download
fetch "$deno/$deno_asset" "$tmp/deno.zip"
fetch "$deno/$deno_asset.sha256sum" "$tmp/deno.sum"
expected=$(grep -oiE '[0-9a-f]{64}' "$tmp/deno.sum" | head -1 | tr 'A-F' 'a-f')
[ "$expected" = "$(sha256 "$tmp/deno.zip")" ] || { echo "Deno checksum mismatch" >&2; exit 1; }
unzip -q -o "$tmp/deno.zip" -d "$out"

# ffmpeg on Windows (other systems use their own), from the builds yt-dlp
# recommends.
if [ "$os" = Windows ]; then
  ffmpeg=https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest
  fetch "$ffmpeg/ffmpeg-master-latest-win64-gpl.zip" "$tmp/ffmpeg.zip"
  fetch "$ffmpeg/checksums.sha256" "$tmp/ffmpeg.sums"
  expected=$(awk '$2 == "ffmpeg-master-latest-win64-gpl.zip" || $2 == "*ffmpeg-master-latest-win64-gpl.zip" { print $1 }' "$tmp/ffmpeg.sums")
  [ -n "$expected" ] && [ "$expected" = "$(sha256 "$tmp/ffmpeg.zip")" ] || { echo "ffmpeg checksum mismatch" >&2; exit 1; }
  unzip -q -j "$tmp/ffmpeg.zip" "*/bin/ffmpeg.exe" -d "$out"
fi

ls -la "$out"
