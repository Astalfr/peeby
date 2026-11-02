# Peeby documentation

Peeby is a native media client for Windows, written in Rust with [GPUI](https://www.gpui.rs/), with one tab ("space") per service: YouTube, Jellyfin and Twitch ([refactor-plan.md](refactor-plan.md), [twitch.md](twitch.md)). It was PeebTube, a YouTube client reusing the playback stack of peeby, the author's Jellyfin client, which it now replaces ([ADR 0007](decisions/0007-multi-space-app.md)).

## Goals

1. **The YouTube pages that matter**: Home (recommendations), subscriptions and their latest videos, search, playlists, watch page with comments and a like/dislike bar ([Return YouTube Dislike](https://returnyoutubedislike.com)).
2. **Everything the former peeby did for Jellyfin**: home with what to resume and watch next, libraries, a title's page, search, Direct Play with progress reports, skippable segments and the next episode.
3. **One app**: the services as tabs over the same components and player; a video keeps playing in a mini-player when another tab opens.
4. **Fast playback**: about 1 s from click to first frame, 1080p, hardware decoding.
5. **Nothing installed on the host**: libmpv, yt-dlp and Deno ship with the app.

## Scope decisions

| Topic | Decision | ADR |
|---|---|---|
| Playback, platform, license, GPUI version, libmpv FFI, video surface | Inherited from peeby | [0001](decisions/0001-peeby-foundations.md) |
| YouTube API | Own InnerTube client, lenient parsing, fixture tests | [0002](decisions/0002-own-innertube-client.md) |
| Streams | VISIONOS client, HLS (no JS challenge, no `available_at` wait) | [0003](decisions/0003-visionos-hls-streams.md) |
| Sign-in | Browser cookies (`cookies.txt`), SAPISIDHASH | [0004](decisions/0004-cookie-session.md) |
| Fallback extraction | yt-dlp + Deno bundled, pinned and isolated | [0005](decisions/0005-bundled-yt-dlp.md) |
| Look and motion | Peeby's visual system, one accent per space (YouTube red, Jellyfin amber) | [0006](decisions/0006-peeby-visual-system.md) |
| Multi-service app (YouTube, Jellyfin, Twitch) | Peeby: one space per service, peeby abandoned, no data migration | [0007](decisions/0007-multi-space-app.md) |
| Mini-player | A window of its own above the others (PopUp), the same video surface, one sound at a time | [0008](decisions/0008-mini-player-window.md) |
| Jellyfin | 12.0 or newer, one server, one account, token in the Credential Manager | [0009](decisions/0009-jellyfin-12-single-account.md) |
| Twitch | Helix for the catalog (sign-in required), GQL and usher for playback, ads removed by a local playlist proxy, low latency, HEVC above 1080p, the web `auth-token` asked for (optional, with a tutorial), no browser on the host, chat read anonymously with Twitch and 7TV emotes | [0010](decisions/0010-twitch-space.md) |
| YouTube lives | Started near their edge, seekable over their DVR window (an hour) in the shared player, chat polled from `live_chat/get_live_chat` | [0011](decisions/0011-youtube-live-dvr.md) |
| Twitch channel points | Read and spent through raw GQL with the auth-token (balance, rewards, predictions; two clicks per spend); the bonus chest claimed on twitch.tv (integrity check); a `minute-watched` report each minute the live plays | [0012](decisions/0012-twitch-channel-points.md) |
| UI language | French UI strings; code, comments and docs in English | — |

## Index

- [roadmap.md](roadmap.md): milestones and status
- [refactor-plan.md](refactor-plan.md): the move to Peeby, a multi-space app (YouTube, Jellyfin, Twitch), with its checklists
- [architecture.md](architecture.md): crates, threads, data flow
- [design.md](design.md): palette, layout, pages and motion
- [etude-faisabilite.md](etude-faisabilite.md): the feasibility study (French), with measurements and sources
- [decisions/](decisions/): architecture decision records; the former peeby's are archived in [decisions/peeby/](decisions/peeby/)
- [player/](player/): mpv options, video surface, playback UX, ASS subtitles (from peeby)
- [twitch.md](twitch.md): how the Twitch space works (playback, credentials, chat, Helix), with what was
  checked live; [twitch-plan.md](twitch-plan.md): the technical plan it was built from, step by step, with what
  each step verified
- [jellyfin/](jellyfin/): the Jellyfin API as used, Direct Play (from peeby); [reference/](reference/): Jellyfin's OpenAPI spec
- [../vendor/README.md](../vendor/README.md): the GPUI patch
