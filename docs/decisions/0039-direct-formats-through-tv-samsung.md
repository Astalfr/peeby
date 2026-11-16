# 0039: Higher qualities of refused videos from tv_samsung, through a chunking proxy

- Status: accepted
- Date: 2026-11-16

## Context

Videos YouTube's bot check refuses to VISIONOS play through web_embedded's HLS
([0038](0038-signed-in-web-embedded-fallback.md)): 1080p H.264 at most. Every signed-in client
also returns direct (DASH) formats, VP9, AV1 and Opus up to 4K HDR. Measured on 2026-11-16 (two
refused videos, 2 h at 1440p and a 4K HDR one, 1 MB ranges at 0, 3, 10, 50 and 97 % of each
file, `n` solved):

| Client | Direct formats |
|---|---|
| `tv_samsung`, without `adPlaybackContext.pyv` | 206 everywhere, to the end, 30–100 Mbit/s |
| `tv_samsung`, `tv_downgraded`, `web_embedded` with `pyv` | 403 after the first tens of seconds |

The lock follows the pre-roll ad, not the client: with its ad (15 s, skippable after 5 s), the
whole file is served, once the ad could have been skipped (about 5 s after the answer);
without it, a PO token is wanted. This is likely what yt-dlp's fallback played (an mpv log
showed AV1 1080p60 as DASH).

Played as they are, mpv's requests (a range to the end of the file) are throttled: 21 Mbit/s
for a 1440p60 VP9 of about 11–13 Mbit/s, the picture frozen about 20 s at the start and 14 s
after a seek. A 10 MB request with a `&range=` parameter (yt-dlp's `http_chunk_size`) comes at
200–430 Mbit/s.

The user chose an entry in the quality menu over switching by itself.

## Decision

- When a video plays from web_embedded's HLS, `Client::tv_offer` asks the signed-in
  `tv_samsung` client (`TVHTML5` 5.20260707, yt-dlp's values, the embed page's ytcfg and player
  JS as for web_embedded), never with `pyv`. Its direct formats (`innertube::direct_formats`):
  from 1080p up, per height VP9 SDR, else AV1 SDR, else VP9 HDR (H.264 is left to the HLS); the
  audio Opus before AAC, the original track of a dubbed video. Their `n` solved by the resident
  Deno ([0038](0038-signed-in-web-embedded-fallback.md)).
- An answer without a pre-roll ad offers nothing: its formats would stop after a minute.
- Once served (`DirectOffer::served`), `innertube::DirectProxy` serves them on a local port:
  mpv's ranges answered as successive 10 MB `&range=` requests; the signed URLs never reach mpv.
- The player gets them above its HLS ladder (`PlayerView::add_qualities`, labels such as
  `2160p60 VP9`), with the proxy kept alive as long as the source (`Source::held`), only while
  the source it started is still the one playing. Nothing switches by itself.

## Consequences

- Measured: the entries come about 5.4 s after the start in the app (a 4K video: 2160p60,
  1440p60 and 1080p60 VP9); through the proxy, in the playback spike, a 1440p60 VP9 + Opus plays
  0.61 s after `loadfile` and a seek to 1 h plays again within 2 s, without A/V drift or dropped
  frames, against a 20 s freeze without the proxy.
- One more `player` request per refused video (tv_samsung), read-only.
- **Not verified:** a switch from the menu in the app (the same mpv options as the spike's), AV1
  and HDR on screen, a video without a pre-roll ad (nothing offered: whether its formats really
  stop is inferred from the `pyv` measurements), URLs past their 6 hours.
- Revisit if YouTube locks these formats without a PO token too, or if a video can switch
  without a restart.
