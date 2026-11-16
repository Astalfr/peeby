# YouTube's bot check and the ways around a slow start

Findings of 2026-11-16, from the release logs, the pinned yt-dlp source (2026.10.26) and live
probes. The first lead (a resolver chain without yt-dlp, with Deno kept running as the `n`
solver) is implemented: [ADR 0038](decisions/0038-signed-in-web-embedded-fallback.md). The rest
is still material for later decisions.

Each claim says how it is known: **measured** (a probe or the logs, that day, one IP, one
account), **source** (yt-dlp's code or wiki), or **not tested**.

## The problem

YouTube's bot check ("Sign in to confirm you're not a bot", `LOGIN_REQUIRED`) refuses anonymous
`player` requests from the user's IP. VISIONOS, Peeby's resolver ([ADR 0003](decisions/0003-visionos-hls-streams.md)),
is anonymous, so the refused videos go to the bundled yt-dlp, which is slow and gives the player
no storyboard, no quality menu and no captions.

Measured in the release logs (`%LOCALAPPDATA%\peeby\logs`):

| Period | Played by VISIONOS | Refused |
|---|---|---|
| 2026-11-09 → 11-13 | dozens a day (69 in one session) | 0 |
| 2026-11-14 09:00 → 11-16 | 1 | about 43 |

- The switch happened at once, on 2026-11-14 in the morning. No change of the InnerTube client
  falls there: the cause is on YouTube's side.
- Not every video is refused: the anonymous `smoke` example still played its fixed video on
  11-16, while a Short and a live were refused. ADR 0005's "some videos are refused" understates
  it: for the videos the user opens, nearly all are.
- Click to stream URLs through VISIONOS, when it works: median 0.22 s, p90 0.30 s (48 plays).
- mpv, from `loadfile` to the first frame: median 0.94 s directly (every source), **8.5 s**
  through yt-dlp (p90 10.5 s; 37 plays).

### Where yt-dlp's time goes (measured)

`yt-dlp -J` on a refused video, cookies given: about 4 s for a normal video, about 10 s for a
6 h one. For the 4 s: Python start 0.8 s, watch page 0.7 s, embed config and two `player`
requests 0.7 s, Deno 0.7 s, manifests 0.6 s. mpv opens the stream only after that. Since mpv's
`ytdl_hook` runs it, Peeby gets nothing back but the playback.

Chapters are not lost: they come from `next` (`watch.rs`, `load_page`), not from the stream.

## What does not work

### A signed-in VISIONOS (measured)

| VISIONOS `player` request | Answer |
|---|---|
| anonymous | `LOGIN_REQUIRED` |
| cookies + `Authorization` (SAPISIDHASH) | HTTP 400 `INVALID_ARGUMENT` |
| the same, with the session's visitorData | HTTP 400 |
| cookies alone | `LOGIN_REQUIRED` (ignored) |
| `Authorization` alone | `LOGIN_REQUIRED` (ignored) |

YouTube recognizes the web session and refuses it for this client. yt-dlp classes VISIONOS as
not supporting cookies (source). The real Vision Pro app signs in, presumably with a Google
OAuth token of its own app, which Peeby cannot get without posing as it (not verified, and not
pursued: same line as Twitch's integrity check).

### The other clients (measured, signed in, video `jUDhLDxVGXw`)

| Client | Result |
|---|---|
| `web_safari` | HLS, 1080p H.264, with the ad wait (below); `pyv` breaks it (`UNPLAYABLE`) |
| `mweb` | no ads, no HLS: direct (DASH) URLs only, locked after about a minute (below) |
| `web_creator` | no ads, direct URLs only, locked the same way |
| `tv`, `tv_downgraded` | `UNPLAYABLE` "The page needs to be reloaded"; `tv_downgraded` answers with `pyv`, direct URLs only |
| `tv_samsung` | direct URLs only, with the ad wait |
| `web_music` | `UNPLAYABLE` |

From the source, not tested: `android_vr` has every format answering 403 since 2026-08-17;
`ios`, `android` and `tv_simply` need a PO token or take no cookies; OAuth sign-in for the TV
client was removed from yt-dlp (from memory, end of 2024).

### The direct formats: VP9, AV1, 1440p, 4K, HDR (measured)

Every signed-in client above returns `adaptiveFormats` with plain URLs (no `signatureCipher`),
up to 2160p HDR on a 4K video. With `n` solved, a range request works for the first 45 s of
content and answers 403 from 90 s on, audio included, on `web_embedded`, `mweb`,
`tv_downgraded` and `web_creator`. This is the PO token (BotGuard attestation) gate: other
clients report the same stop after 45–60 s. Without a PO token, the ceiling is HLS: 1080p,
H.264 + AAC.

yt-dlp's PO Token Guide says Premium accounts need no such token for the streams (source, not
tested; the test account gets ads, so it is not Premium).

## What works: a signed-in `web_embedded`

The client of YouTube's embedded player takes the web session and passes the bot check
(measured on three videos VISIONOS refused: `jUDhLDxVGXw`, `7YYPsowcKSU`, `SosvBGv6CJM`).

### The request

Values from yt-dlp 2026.10.26 (`_base.py`, `_video.py`):

1. `GET https://www.youtube.com/embed/<id>?html5=1`, with the cookies, the Safari User-Agent
   below and `Referer: https://www.reddit.com/` (any non-YouTube URL). From its ytcfg:
   `INNERTUBE_CLIENT_VERSION`, `VISITOR_DATA`, `jsUrl` (the player JS; `PLAYER_JS_URL` is
   absent on this page), `encryptedHostFlags`, `LOGGED_IN`.
2. The player JS, once per version: `signatureTimestamp` (five digits after
   `signatureTimestamp:`), and the code the `n` solver needs.
3. `POST /youtubei/v1/player`:
   - `context.client`: `clientName` `WEB_EMBEDDED_PLAYER`, the page's `clientVersion`
     (default `2.20260708.00.00`), the User-Agent, `hl`, `gl`, `visitorData`;
   - `context.thirdParty.embedUrl`: `https://www.reddit.com/`;
   - `playbackContext.contentPlaybackContext`: `html5Preference: HTML5_PREF_WANTS`,
     `signatureTimestamp`, `encryptedHostFlags`;
   - `contentCheckOk`, `racyCheckOk`;
   - headers: `X-Youtube-Client-Name: 56`, `X-Youtube-Client-Version`, `Origin` and `X-Origin`
     `https://www.youtube.com`, `X-Goog-Visitor-Id`, `Cookie`, `Authorization`,
     `X-Goog-AuthUser: 0`, `X-Youtube-Bootstrap-Logged-In: true`.

User-Agent: `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)`.

What each piece is for (measured): without the page's ytcfg or without `signatureTimestamp`,
the answer is `ERROR` "video unavailable"; without `encryptedHostFlags` too; with all three,
`OK` in 0.2–0.55 s, with the storyboard spec, the caption tracks and `hlsManifestUrl`
(6 variants, 144p to 1080p, `avc1` + `mp4a`).

### The `n` parameter

The manifest's path holds `/n/<challenge>/`. It must be transformed by a function of the
player JS and put back in the path before the manifest is fetched:

- unsolved: master and variant playlists answer 200, **every segment 403**, still after 16 s;
- solved (yt-dlp's EJS solver 0.8.0 in the bundled Deno): segments answer 200, 77–500 Mbit/s.

`n` is different in every `player` answer, even for the same video and session: results
cannot be cached, so each play needs the solver. With Deno started for each solve: 430 ms the
first time (the whole player, 1.6 MB), 150 ms from EJS's preprocessed player (2.3 MB), of
which 64 ms is Deno's own start. A solver kept loaded should answer in milliseconds (not
measured).

EJS (`github.com/yt-dlp/ejs`, release 0.8.0): `yt.solver.lib.min.js` and
`yt.solver.core.min.js`, SHA3-512 in yt-dlp's `jsc/_builtin/vendor/_info.py`. Input, as
`ejs.py` builds it: the two scripts, then
`jsc({type: "player", player: <code>, requests: [{type: "n", challenges: [...]}], output_preprocessed: true})`.

### The ad wait, and `pyv`

With a normal request, segments answer 403 until about 3.3–3.8 s after the `player` answer.
The answer carries a pre-roll ad (10–15 s, skippable after 5 s) and the server serves nothing
before it could have been skipped; yt-dlp computes its `available_at` from the same fields
(`_get_available_at_timestamp`). The wait counts from the `player` answer, not from the first
segment request.

Adding `playbackContext.adPlaybackContext: { "pyv": true }` (yt-dlp's
`use_ad_playback_context`) removes it:

| `web_embedded` request | Ads in the answer | First HLS segment |
|---|---|---|
| normal | 10–15 s, skippable after 5 s | 403, then 200 at +3.4–3.8 s |
| with `pyv` | none | 200 at the first try |

yt-dlp's README says the option only works on `mweb` and `web_music`, and must not be used
with a Premium account (premium formats are lost). YouTube documents none of it: it can stop
working, and waiting about 3.5 s is the fallback.

### Measured costs, one refused video, no prefetch

| Step | Time |
|---|---|
| embed page | 0.15–0.3 s (per video; whether its ytcfg can be reused across videos is not tested) |
| player JS | 0.03 s once cached by the CDN, once per player version |
| `player` | 0.15–0.35 s |
| `n` | 0.15 s with a new Deno each time |
| master + variant playlists | 0.1 s + 0.2 s |

About 0.6–0.8 s from the click to the URLs, against 0.2 s for VISIONOS and 4–10 s for yt-dlp.

### Limits and open points

- 1080p H.264 at most (VISIONOS gives VP9).
- "Embeddable videos only" (PO Token Guide). Age-restricted, members-only and non-embeddable
  videos are not tested; `web_safari` (HLS, with the ad wait) is the candidate behind it.
- On the 6 h video, segments far from the start came at 3–6 Mbit/s, barely above the stream's
  bitrate; on a short popular video, 60–190 Mbit/s everywhere. To measure before relying on
  seeks in long videos (the yt-dlp fallback plays the same HLS).
- The solver scripts follow YouTube's player: they need updates as yt-dlp does.

## Leads, by expected gain

1. **A resolver chain without yt-dlp** (done, [ADR 0038](decisions/0038-signed-in-web-embedded-fallback.md)):
   VISIONOS, then `web_embedded` + `pyv`, then yt-dlp. In the app: stream URLs 0.68 s after the
   click once Deno runs (about 0.35 s of it VISIONOS's refusal), first frame 0.5 s later.
   `web_safari` behind it is not done; yt-dlp stays the last resort until the untested kinds of
   videos are checked.
2. **The JS engine for `n`**, which decides whether Deno (110 MB) can go:
   - a hidden WebView2: `wry` is already a dependency ([ADR 0017](decisions/0017-sign-in-webview.md)),
     V8 is on the host, nothing to ship; a process of its own, as the sign-in window;
   - QuickJS or Boa in the process: a new dependency; ADR 0005 measured 5.6 s per challenge
     with QuickJS-NG, so only with the player kept loaded (paid once per player version);
   - Deno kept running: nothing new, the bundle stays as it is. Chosen (ADR 0038): a solve
     then costs a call, the engine brings no speed, only the bundle's 110 MB.
3. **`tv_samsung`'s direct formats**: an mpv log of the yt-dlp fallback (2026-11-16) shows it
   playing AV1 1080p60 as DASH, likely from `tv_samsung`, the one client whose direct formats were
   not checked past 90 s. If they play to the end without a PO token, VP9, AV1 and above 1080p are
   open without BotGuard (with the ad wait, which `pyv` removes on `tv_downgraded`). To measure.
4. **PO tokens from a hidden WebView2** running YouTube's own page signed in (the way the
   Twitch bonus is claimed, [ADR 0012](decisions/0012-twitch-channel-points.md)): would open
   VP9, AV1, 1440p, 4K and HDR. A large piece of work, not tested, and a decision about where
   the project's line on bot checks stands.
5. **mpv's start** (about 1 s today). web_embedded's muxed playlist gave its first frame in
   0.5 s in the app, which supports the idea below. In one YouTube play, mpv opened the video playlist
   (0.75 s with its probing), only then the audio playlist (0.2 s), then the segments. A local
   proxy (as `innertube::LiveProxy`) serving one playlist with both, already fetched; the
   first segment prefetched on a long hover or on mouse down; less probing
   (`demuxer-lavf-*`). Estimated 0.3–0.6 s, not measured.
6. **Feeding the bot check less** (a hypothesis): the hover prefetch (`prefetch.rs`, since
   before 2026-11-07) sends an anonymous `player` request after 300 ms on a card, mostly for
   videos never played, and each launch takes a new anonymous visitorData. Longer hover delay,
   a cap per minute, no anonymous request while refused, one visitorData kept across launches.
   Third-party write-ups agree that repeated requests prolong a flag; none is authoritative.
7. **On the user's side**: another IP (VPN off or another server, router restart) often lifts
   the flag; reported durations range from minutes to days.

Set aside: implementing SABR (needs a PO token anyway), third-party proxies (Invidious, Piped:
privacy, reliability).

## How the probes were run

- A plain copy of the debug profile's cookies (`--export-cookies`), deleted afterwards.
- Read-only `player` requests and media fetches from the account; no stats ping, so nothing
  written to the history.
- No URL, cookie or token printed: statuses, counts and timings only.
- Temporary code only (an `innertube` example, then Python scripts outside the repository).

## Sources

- yt-dlp 2026.10.26: `yt_dlp/extractor/youtube/_base.py` (clients), `_video.py` (player
  request, `n` in manifests, `available_at`), `jsc/_builtin/` (EJS, Deno), README (extractor
  arguments)
- [PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide) (last edited 2026-07-12)
- [yt-dlp #12045](https://github.com/yt-dlp/yt-dlp/issues/12045), [#15865](https://github.com/yt-dlp/yt-dlp/issues/15865) (the bot check with cookies), [#14404](https://github.com/yt-dlp/yt-dlp/issues/14404) (JS runtimes)
- [VacuumTube #189](https://github.com/shy1132/VacuumTube/issues/189), [ytcast-open](https://github.com/XeTK/ytcast-open), [MediaServiceCore #37](https://github.com/yuliskov/MediaServiceCore/pull/37), [Invidious #5814](https://github.com/iv-org/invidious/pull/5814) (PO tokens, SABR)
