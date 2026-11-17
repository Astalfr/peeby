# 0038: The signed-in web_embedded client before yt-dlp, its `n` solved in a resident Deno

- Status: accepted
- Date: 2026-11-16

## Context

From 2026-11-14, YouTube's bot check ("Sign in to confirm you're not a bot", `LOGIN_REQUIRED`)
refuses nearly every video to VISIONOS ([0003](0003-visionos-hls-streams.md)), an anonymous
visitor: 1 play through it in three days against about 43 refusals, while dozens a day played
before. Each refused video went to yt-dlp ([0005](0005-bundled-yt-dlp.md)): 8.5 s median from
`loadfile` to the first frame, no storyboard, no quality menu, no captions. Everything measured
is in [youtube-bot-check.md](../youtube-bot-check.md).

Options measured, signed in:

- VISIONOS with the session: HTTP 400 `INVALID_ARGUMENT` (cookies and SAPISIDHASH refused for this
  client), ignored with either alone.
- `web_embedded` (`WEB_EMBEDDED_PLAYER`): passes the bot check. Its HLS manifest (6 variants,
  144p to 1080p, H.264 + AAC muxed) carries an `n` challenge: unsolved, every segment answers 403.
  A dubbed video has one variant per language at each height (the original, an untagged copy,
  each dub), a dub sometimes with the highest bandwidth: `hls::ladder` takes the original track
  (`acont=original` in the variant's `sgoap`), then the untagged one, before any dub.
  Asked with `adPlaybackContext.pyv`, its answer has no pre-roll ads and the stream is served at
  once (else after the ads could be skipped, about 3.5 s).
- `web_safari`: HLS too, but `pyv` breaks it; `mweb`, `web_creator`, `tv`, `tv_downgraded`,
  `tv_samsung`, `web_music`: no HLS, or unplayable.
- The direct (DASH) formats of every client (VP9, AV1, up to 4K HDR) stop after about a minute of
  content without a PO token (BotGuard attestation). Not pursued here.

`n` differs in every answer: it cannot be cached. yt-dlp's EJS scripts (`github.com/yt-dlp/ejs`,
Unlicense) solve it; yt-dlp starts Deno for each solve (150 ms, 430 ms for a new player). EJS's
`jsc` turns a player into code that sets `_result.n`: evaluated once, its function solves a
challenge in a call.

## Decision

Since [0040](0040-youtube-player-setting.md), a setting puts web_embedded first by default; what
follows describes the VISIONOS-first order, which the setting keeps.

- A video goes through VISIONOS first, as before. When VISIONOS answers `LOGIN_REQUIRED` (or
  refused it in the last 30 minutes, `prefetch::refused`) and a session is set, it goes through
  `Client::embedded_stream`; when that fails, or signed out, through yt-dlp, which stays the last
  resort (age-restricted, non-embeddable videos, a solver broken by a new player). Lives are not
  handled by it (`Unplayable` `LIVE`): they keep their VISIONOS path.
- The request, as yt-dlp 2026.10.26 makes it: the embed page's ytcfg (`/embed/<id>?html5=1`,
  `Referer` a non-YouTube URL), the player JS's `signatureTimestamp`, `encryptedHostFlags`,
  `adPlaybackContext.pyv`. A refusal with `pyv` asks again without it, then waits for the ads
  (`innertube::ad_wait`) before handing the stream to mpv.
- The `n` challenge: `innertube::jsc::Solver`, one Deno process started at the first challenge
  (`deno run` on yt-dlp's two EJS scripts plus `jsc_server.js`, a JSON line per request and
  answer), started again if it dies or does not answer in 15 s. It keeps each player's function:
  the player's code is sent once per process. Deno ends with Peeby (its stdin closes).
- The EJS scripts are pinned with SHA-256 in `scripts/fetch-tools.ps1` (0.8.0, the version
  yt-dlp 2026.10.26 vendors, checked against its SHA3-512) and shipped in `tools\` beside Deno.
- Fixtures: `player_embedded.json` (with `pyv`) and `player_embedded_ads.json` (without, its ads
  trimmed to what the wait reads), redacted; `cargo run -p innertube --example embedded` checks
  the chain live.

## Consequences

- Measured in the app on two refused videos: stream URLs after 1.28 s for the first (Deno's start
  and the player's first parse included), 0.68 s for the next, of which about 0.35 s is
  VISIONOS's refusal; mpv's first frame 0.5 s later (one muxed playlist, where VISIONOS's video
  and audio playlists open one after the other). About 1.2 s from click to picture, against
  8.5 s through yt-dlp, with the storyboard, six qualities and the captions.
- Refused videos play in 1080p H.264 at most (VISIONOS gives VP9).
- The EJS scripts follow YouTube's player: a new player can break them until they are updated
  with yt-dlp. yt-dlp, which updates itself, then takes over. `pyv` is undocumented: if it stops
  working, the stream waits for the ads (about 3.5 s).
- Deno (98 MB) stays in the bundle and now serves Peeby too. Replacing it was measured on
  2026-11-16 and not done ([youtube-bot-check.md](../youtube-bot-check.md), "Removing Deno"):
  a hidden WebView2 runs the solver faster than Deno once started (0.7 s to start), but yt-dlp
  would need QuickJS, its fallback about 2.4 s slower.
- Revisit when the bot check lifts (VISIONOS plays again, no change needed), when a PO token
  source is decided (the DASH formats), or if `web_embedded` starts asking for a PO token.
