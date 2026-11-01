# 0005: yt-dlp and Deno bundled, as a fallback

- Status: accepted (the fallback is not wired into the app yet)
- Date: 2026-11-01

## Context

VISIONOS ([0003](0003-visionos-hls-streams.md)) can fail: client blocked, age or membership restrictions. yt-dlp follows YouTube's changes within days. It must not have to be installed on the host.

## Decision

- `scripts/fetch-tools.ps1` downloads pinned, SHA-256-checked builds into `third_party/yt-dlp/`:
  - yt-dlp 2026.10.26, `yt-dlp_win.zip` ("onedir": nothing unpacked to `%TEMP%` per run, `--version` in 0.4 s);
  - Deno 2.9.8, the JS runtime yt-dlp needs for YouTube.
- The release bundle ships the same layout in `tools/` next to the executable.
- yt-dlp runs isolated from the host: `--ignore-config`, `--cache-dir` in the app's folder, `--js-runtimes deno:<path>`.
- The fallback forces HLS to avoid the `available_at` wait: `bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/…` (about 3–4 s to start). It also sets `sub-langs=none`, because mpv opens external subtitles before playing.

## Consequences

- About 110 MB more in the bundle (Deno). QuickJS-NG (2 MB) was measured and rejected: 5.6 s per JS challenge.
- yt-dlp needs updates more often than the app: plan an in-app updater that checks `SHA2-256SUMS`.
