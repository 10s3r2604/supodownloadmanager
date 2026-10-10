The release workflow puts yt-dlp, Deno and (on Windows) ffmpeg here with
`.github/scripts/bundle-tools.sh`, so the installers ship them. Local builds
work without them: SDM then downloads yt-dlp on first use.
