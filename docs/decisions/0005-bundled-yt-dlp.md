# 0005: yt-dlp and Deno bundled, as a fallback

- Status: accepted
- Date: 2026-11-01

## Context

VISIONOS ([0003](0003-visionos-hls-streams.md)) can fail: client blocked, age or membership restrictions. yt-dlp follows YouTube's changes within days. It must not have to be installed on the host.

## Decision

- `scripts/fetch-tools.ps1` downloads pinned, SHA-256-checked builds into `third_party/yt-dlp/`:
  - yt-dlp 2026.10.26, the single-file `yt-dlp.exe`. The "onedir" `yt-dlp_win.zip` starts faster (`--version` in 0.4 s against 1.05 s) but cannot update itself (`win_dir` is in yt-dlp's `_NON_UPDATEABLE_REASONS`). Since yt-dlp is only the fallback, updates matter more than 0.65 s;
  - Deno 2.9.8, the JS runtime yt-dlp needs for YouTube.
- The release bundle ships the same layout in `tools/` next to the executable.
- yt-dlp runs isolated from the host: `--ignore-config`, `--cache-dir` in the app's folder, `--js-runtimes deno:<path>`.
- The fallback forces HLS to avoid the `available_at` wait: `bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/…` (about 3–4 s to start). It also sets `sub-langs=none`, because mpv opens external subtitles before playing.

In the app (`tools.rs`, `watch.rs`): when `Client::stream` fails with anything but a network error, mpv's `ytdl_hook` runs the bundled yt-dlp with those options and a copy of the session cookies. Measured on an age-restricted video: playback after 8.1 s, 1080p, hardware decoding. yt-dlp found no HLS for it, so mpv waited for `available_at` (4 s).

## Consequences

- About 110 MB more in the bundle (Deno). QuickJS-NG (2 MB) was measured and rejected: 5.6 s per JS challenge.
- yt-dlp needs updates more often than the app: at startup, at most once a day, the app runs `yt-dlp -U` in the background with no console window (`tools::update_ytdlp`, stamp in `%LOCALAPPDATA%\peebtube\yt-dlp\last-update`). yt-dlp checks the release's `SHA2-256SUMS` itself. The `tools/` folder must stay writable: per-user install only.
- Measured with the single-file build: age-restricted video playing after 5.4 s (8.1 s earlier with "onedir", the difference comes from YouTube's wait, not from yt-dlp).
