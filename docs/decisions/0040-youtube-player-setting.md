# 0040: A setting for the YouTube player tried first, web_embedded by default, and toasts

- Status: accepted
- Date: 2026-11-16

## Context

Since 2026-11-14 YouTube's bot check refuses nearly every video to VISIONOS on the user's
connection ([0038](0038-signed-in-web-embedded-fallback.md)): each video first spent about
0.35 s on that refusal before web_embedded played it, and the hover prefetch's anonymous
VISIONOS requests fed the check for videos never played. VISIONOS still has the edge when it is
not refused: VP9 at once, and lives with their whole DVR window ([0025](0025-youtube-lives-from-dash.md)).
The user wanted to choose, web_embedded by default, and to be told when VISIONOS is refused and
when web_embedded falls back to yt-dlp.

## Decision

- A setting, "Player" / "Lecteur" in YouTube's part of the settings (`youtube_player`,
  `YouTubePlayer`): `embedded` (default, "Signed-in" / "Connecté") or `visionos`.
- `embedded`, signed in: web_embedded first, then VISIONOS (lives, videos it cannot embed, an
  expired session), then yt-dlp. The hover prefetch resolves no stream (`prefetch::fetch`): its
  VISIONOS stream would not be used. It still asks for the signed-in `next` answer and loads
  the poster, as with any player (2026-11-24, docs/responsiveness-plan.md, item 5).
- `visionos`, or signed out: as before, VISIONOS (or the hover prefetch's stream), web_embedded
  when the bot check refuses it (signed in), then yt-dlp.
- Toasts, in the player's own window (`PlayerView::toast`, pushed at its next render, deferred):
  - the bot check refused VISIONOS, and what plays instead (web_embedded, or yt-dlp; signed
    out, a hint to sign in); at most once every 30 minutes, since once the connection is taken
    for a bot every video would raise it;
  - web_embedded could not play the video, with the reason, and yt-dlp plays it: each time.

## Consequences

- With the default, a refused video no longer waits for VISIONOS's refusal, and no anonymous
  request leaves for a hover. A live costs one web_embedded `player` request (about 0.3–0.5 s)
  before VISIONOS plays it; VP9 comes from the quality menu a few seconds after the start
  ([0039](0039-direct-formats-through-tv-samsung.md)).
- Measured in the app (debug build): default mode, a refused video's stream after 0.80 s with
  Deno's start, no VISIONOS request; `visionos` mode with web_embedded made to fail (a solver
  script hidden), both toasts logged and yt-dlp playing after 9.3 s. The toasts' look on screen
  is not checked.
