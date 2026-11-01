# Peeby documentation

Peeby is a native media client for Windows, written in Rust with [GPUI](https://www.gpui.rs/), with one tab ("space") per service: YouTube today, Jellyfin and later Twitch on the way ([refactor-plan.md](refactor-plan.md)). It was PeebTube, a YouTube client reusing the playback stack of peeby, the author's Jellyfin client, which it now replaces ([ADR 0007](decisions/0007-multi-space-app.md)).

## Goals

1. **The YouTube pages that matter**: Home (recommendations), subscriptions and their latest videos, watch page with comments and a like/dislike bar ([Return YouTube Dislike](https://returnyoutubedislike.com)).
2. **Fast playback**: about 1 s from click to first frame, 1080p, hardware decoding.
3. **Nothing installed on the host**: libmpv, yt-dlp and Deno ship with the app.

## Scope decisions

| Topic | Decision | ADR |
|---|---|---|
| Playback, platform, license, GPUI version, libmpv FFI, video surface | Inherited from peeby | [0001](decisions/0001-peeby-foundations.md) |
| YouTube API | Own InnerTube client, lenient parsing, fixture tests | [0002](decisions/0002-own-innertube-client.md) |
| Streams | VISIONOS client, HLS (no JS challenge, no `available_at` wait) | [0003](decisions/0003-visionos-hls-streams.md) |
| Sign-in | Browser cookies (`cookies.txt`), SAPISIDHASH | [0004](decisions/0004-cookie-session.md) |
| Fallback extraction | yt-dlp + Deno bundled, pinned and isolated | [0005](decisions/0005-bundled-yt-dlp.md) |
| Look and motion | Peeby's visual system with a red accent | [0006](decisions/0006-peeby-visual-system.md) |
| Multi-service app (YouTube, Jellyfin, later Twitch) | Peeby: one space per service, peeby abandoned, no data migration | [0007](decisions/0007-multi-space-app.md) |
| UI language | French UI strings; code, comments and docs in English | — |

## Index

- [roadmap.md](roadmap.md): milestones and status
- [refactor-plan.md](refactor-plan.md): the move to Peeby, a multi-space app (YouTube, Jellyfin, Twitch), with its checklists
- [architecture.md](architecture.md): crates, threads, data flow
- [design.md](design.md): palette, layout, pages and motion
- [etude-faisabilite.md](etude-faisabilite.md): the feasibility study (French), with measurements and sources
- [decisions/](decisions/): architecture decision records
- Peeby docs that apply as they are: `D:\DEV\peeby\docs\player\` (mpv options, video surface) and `D:\DEV\peeby\vendor\README.md` (GPUI patch)
