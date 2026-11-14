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
- mpv starts yt-dlp in Peeby's working directory, where yt-dlp writes its format check's test file (`NamedTemporaryFile(dir='.')`, for formats it must test). Started with Windows, that directory is `C:\Windows\System32`: yt-dlp failed with `Permission denied` and mpv, left with the watch page, ended with `MPV_ERROR_UNKNOWN_FORMAT` (-17) (sUjPMssQgWc, 2026-11-14). Peeby therefore makes the cache folder its working directory at startup (`tools::set_working_dir`); every path of the app is absolute.
- The fallback forces HLS to avoid the `available_at` wait: `bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/…` (about 3–4 s to start). It also sets `sub-langs=none`, because mpv opens external subtitles before playing.

In the app (`tools.rs`, `watch.rs`): when `Client::stream` fails with anything but a network error, mpv's `ytdl_hook` runs the bundled yt-dlp with those options and a copy of the session cookies. Members-only videos (VISIONOS answers `UNPLAYABLE` to anyone, being an anonymous visitor) get one check first: a WEB `player` request with the session (`Client::check_members_only`). When it offers the channel's membership (`errorScreen.playerLegacyDesktopYpcOfferRenderer.offerId == "sponsors_only_video"`, not translated, unlike `reason`), the player says the video is for members (`Error::MembersOnly`, its text asking a signed-out user to sign in with a member's account) without running yt-dlp; when it would play (the account is a member), yt-dlp plays it with the cookies. Measured on an age-restricted video: playback after 8.1 s, 1080p, hardware decoding. yt-dlp found no HLS for it, so mpv waited for `available_at` (4 s).

YouTube's bot check ("Sign in to confirm you're not a bot", `LOGIN_REQUIRED`) refuses some videos to every anonymous client (VISIONOS, tv_samsung, mweb, android_vr) from an IP for hours, while others still play (2026-11-14: three videos refused, opened again and again, others playing through VISIONOS meanwhile). There is no challenge to show the user: it is not a captcha, and signed in, yt-dlp plays them (tv_samsung, web_embedded with the cookies). So a video VISIONOS refused with `LOGIN_REQUIRED` goes straight to yt-dlp for 30 minutes (`prefetch::refused`), hover prefetch included: no second request that feeds the check, about a second less before yt-dlp starts. Only that video: skipping VISIONOS for every video would slow down those it still plays.

yt-dlp also takes over when `Client::stream` succeeds but mpv cannot open what it got: some videos (E-Q2tEgJ1ZI, 2026-11-07) have a master playlist whose media playlists all answer 404, and mpv ends with `MPV_ERROR_LOADING_FAILED` (-13). The source carries a `fallback` (`ui::player::Source::fallback`), which the player calls at most once, only when mpv fails before loading the file; the yt-dlp start carries none, so a second failure shows its error. Lives keep the error: they start at their edge, which yt-dlp's options do not.

## Consequences

- About 110 MB more in the bundle (Deno). QuickJS-NG (2 MB) was measured and rejected: 5.6 s per JS challenge.
- yt-dlp needs updates more often than the app: at startup, at most once a day, the app runs `yt-dlp -U` in the background with no console window (`tools::update_ytdlp`, stamp in `%LOCALAPPDATA%\peebtube\yt-dlp\last-update`). yt-dlp checks the release's `SHA2-256SUMS` itself. The `tools/` folder must stay writable: per-user install only.
- Measured with the single-file build: age-restricted video playing after 5.4 s (8.1 s earlier with "onedir", the difference comes from YouTube's wait, not from yt-dlp).
