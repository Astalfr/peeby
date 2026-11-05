# 0025: YouTube lives from their DASH segments, through a local proxy

- Status: accepted
- Date: 2026-11-05

## Context

Since [ADR 0011](0011-youtube-live-dvr.md), a live plays from VISIONOS's HLS playlists, which are a
window of an hour at most. The user wants youtube.com's behavior: go back to the broadcast's start, and
come back to the live. ADR 0011 had put aside going further back, because it would cost a proxy or a
second stream path. Measured on 2026-11-05, anonymous (`live_probe`, then `spike-yt-play`):

- **HLS keeps nothing reliable past its window.** Its segment URLs end in `/sq/<N>/`, after the signed
  part, so older numbers can be asked for. On two 24/7 lives that worked 8 to 9 h back. On most
  ordinary broadcasts it did not: on two of four, nothing before the window (404), and on a third,
  an hour more. A missing segment answers 404 or 200 with an empty body.
- **DASH keeps the whole broadcast.** The same VISIONOS answer has `adaptiveFormats` with direct URLs
  (no `available_at`, no JS challenge) and `maxDvrDurationSec` 43200 (12 h, youtube.com's limit). `&sq=0`
  answered on every ordinary broadcast tried.
- **Each DASH segment is a whole file**: `ftyp`, `moov`, `emsg`, `moof`, `mdat` (MP4), or WebM.
  The `moov` is the same across a broadcast: one hash per format over 50 segments from 0 to the edge,
  on four lives with restarts. Listed in an HLS playlist, mpv plays them one after the other.
  FFmpeg warns once per segment ("duplicated MOOV", and for WebM an unknown-length element), harmlessly.
  - From the start: first frame 0.83 s after load.
  - At the edge: 1.1 s, and 3 minutes there with no 404 and no buffering (77 playlist reloads, a
    29.97 fps live).
  - VP9 1440p60: 1.0 s.
- **The edge** comes in every answer's headers: `x-head-seqnum` (the newest segment) and
  `x-head-time-millis`. A `HEAD` request gives them without a body. Segments are not always as long as
  announced: 4.9675 s on average on a 29.97 fps live that announces 5. That is 0.65 %, over 100 s off
  after 4 h if the announced length were used.
- **The DASH ladder goes higher** than the HLS one: VP9 up to 2160p60 where HLS stopped at 1080p60.
- **Twelve hours of signed URLs** (about 900 bytes each) would make an 8 MB playlist on every reload.
  mpv follows redirects for segments.

## Decision

- A live with DVR (`isLive` and `isLiveDvrEnabled`) plays from its DASH formats
  (`innertube::parse_live_formats`: MP4 AVC/AV1/AAC, WebM VP9) through `innertube::LiveProxy`. It is a
  local HTTP server on 127.0.0.1 with a random key, like `twitch::Proxy`. Lives without DVR, videos,
  and a proxy that fails to start keep the HLS path. An HLS live keeps its one-hour window.
- **Playlists.** On each reload the proxy writes a live media playlist per format. It lists the
  segment numbers from the first one kept to the estimated edge, minus one, each `EXTINF` the
  measured length. Each URI is a short local path (`s/<track>/<sq>`) answered with a 302 to the
  signed URL. `MEDIA-SEQUENCE` is the first number, and it slides once the window is full.
- **The edge** is read with `HEAD` when the proxy starts (the chosen video and the audio, together),
  then at most every 10 s while mpv reloads, and estimated in between. A segment's length is
  `x-head-time-millis / x-head-seqnum` when within 5 % of the announced one.
- **The first segment kept** is the DVR length back from the edge, served on most lives. If it is
  gone (24/7 lives keep less than they announce), a search in the background halves the gap to the
  edge. Each step asks for one byte (`Range: bytes=0-0`): 206 is kept, 404 or an empty body is gone,
  while a `HEAD` answers 200 even for a gone segment. The result waits until mpv has read the
  playlists it started on (the quality picked and the audio), since its start came from the first
  window. Then the playlists start there and the space shortens the player's window
  (`LiveProxy::settled`, `PlayerView::set_dvr_window`).
- **Expiry.** The signed URLs are resolved again (VISIONOS `player`) 30 min before they expire. A
  live that is no longer `isLive` gets `#EXT-X-ENDLIST`; an edge stalled for 60 s triggers that check,
  once per stalled minute.
- **Qualities**: one per height, VP9 over AVC at equal height (as HLS's ladder); the best AAC.
- **The player**: unchanged DVR mode (ADR 0011), the window up to 12 h. The source keeps the proxy
  alive (`Source::keep_alive`), so it stops when the page drops the source. The live badge stays the
  "back to live" button. Read-ahead stays capped at `READAHEAD_SECS`: YouTube answers 401 after
  about 160 segments in two seconds.
- The signed URLs stay in the proxy's memory: never logged, never in an error, and not in the
  playlists. Only the local paths and the redirects to mpv carry them.
- Amends [ADR 0003](0003-visionos-hls-streams.md): streams still come from VISIONOS, as HLS for videos
  and lives without DVR, as DASH segments through the proxy for lives with DVR.

## Consequences

- A live can be watched from its start (up to 12 h), and resolves faster: 0.16 s against about 0.3 s
  for the HLS master and media playlist. Lives get 1440p and 2160p when broadcast in them.
- A prefetched live starts at the proxy's edge as of the click, not as of the prefetch. A stale
  prefetched live is dropped at the next prefetch, which stops its proxy.
- Seeking within a segment relies on FFmpeg's timestamps. `EXTINF` is an average, so a seek may land a
  few seconds off on a live whose segments vary.
- One `HEAD` every 10 s per playing format, plus a `player` request every 5.5 h.
- Revisit if YouTube stops serving `adaptiveFormats` URLs to VISIONOS, adds an `available_at` to them,
  or mpv stops accepting repeated `moov`s; the HLS window remains the fallback.
- Checks: `cargo run -p innertube --example live_probe -- [id | query] [--serve <s>]` (anonymous: the
  proxy's window, its playlist and both ends through the redirects; `--serve` keeps it running for the
  spike's `PEEBY_RESOLVER=raw`).
