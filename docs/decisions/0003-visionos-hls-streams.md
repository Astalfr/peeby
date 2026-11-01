# 0003: Streams from the VISIONOS client, as HLS

- Status: accepted
- Date: 2026-11-01

## Context

Playback must start within 1–2 s of the click. With yt-dlp through mpv's `ytdl_hook` it took **8–8.8 s**:
- yt-dlp takes about 3 s: web page, player API, player JS, JS challenge with Deno (0.8 s; QuickJS-NG 5.6 s).
- YouTube delivers DASH (`https`) URLs with an `available_at` about 5 s in the future. Before it the server answers 403, after it 206 (verified). This wait cannot be avoided for DASH.
- HLS (`m3u8_native`) formats have no wait.

Player clients compared one by one:
- `tv_simply` and `android_vr` give 360p only;
- `web_safari` imposes the wait;
- **`visionos`** gives HLS up to 2160p, with no JS challenge (`REQUIRE_JS_PLAYER: False` in yt-dlp) and no wait. Its master playlist URL is signed by the server.

## Decision

`innertube::Client::stream`:
1. `POST /youtubei/v1/player` with the VISIONOS context of yt-dlp 2026.10.26 and a session `visitorData`. Without it most videos answer `LOGIN_REQUIRED`; on `LOGIN_REQUIRED` the visitorData is refreshed once.
2. GET the master playlist and pick the best variant up to 1080p, VP9 before AVC at equal height.
3. Variants are video only: the audio rendition (`EXT-X-MEDIA`, group named by `AUDIO=`) goes to mpv as `audio-files`. Playlists and segments are fetched with the client's Safari User-Agent.

Measured on 5 videos:
- resolution: 0.16–0.37 s;
- mpv to first frame: about 0.66 s;
- **click to playback: about 1–1.5 s**;
- start at 150 s: 0.95 s.

Stream URLs stay valid for 6 h.

## Consequences

- No Python or JS runtime on the hot path.
- VISIONOS is undocumented and can be blocked at any time. yt-dlp's source at the pinned version is the reference for its values. The fallback is yt-dlp ([0005](0005-bundled-yt-dlp.md)).
- No AV1 in the HLS ladder (VP9 and AVC only).
- Not verified yet with this client: age-restricted and members-only videos, lives, Shorts.
