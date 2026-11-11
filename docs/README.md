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
| Look and motion | Peeby's visual system, one accent per space (YouTube red, Jellyfin blue) | [0006](decisions/0006-peeby-visual-system.md) |
| Multi-service app (YouTube, Jellyfin, Twitch) | Peeby: one space per service, peeby abandoned, no data migration | [0007](decisions/0007-multi-space-app.md) |
| Mini-player | A window of its own above the others (PopUp), the same video surface, one sound at a time per window | [0008](decisions/0008-mini-player-window.md) |
| Jellyfin | 12.0 or newer, one server, one account, token in the Credential Manager | [0009](decisions/0009-jellyfin-12-single-account.md) |
| Twitch | Helix for the catalog (sign-in required; superseded by 0016), GQL and usher for playback, ads removed by a local playlist proxy, low latency, HEVC above 1080p, the web `auth-token` asked for (optional, with a tutorial), no browser on the host, chat read anonymously with Twitch, 7TV and BTTV emotes | [0010](decisions/0010-twitch-space.md) |
| YouTube lives | Started near their edge, seekable over their DVR window in the shared player, chat polled from `live_chat/get_live_chat` | [0011](decisions/0011-youtube-live-dvr.md) |
| YouTube lives from their start | Lives with DVR from VISIONOS's DASH segments through a local playlist proxy (`innertube::LiveProxy`): the whole broadcast, up to 12 h, MP4 formats only (WebM does not seek); the HLS window as the fallback | [0025](decisions/0025-youtube-lives-from-dash.md) |
| A Twitch live's past | Its recording (the archive VOD, `stream.archiveVideo`) through a second proxy: the seek bar covers the broadcast, a seek back plays the recording, the badge comes back to the live; its position saved as a VOD's | [0026](decisions/0026-twitch-live-rewind-from-its-recording.md) |
| Twitch without the Helix sign-in | The catalog through anonymous GQL (first pages), the web `auth-token` as the sign-in, Helix optional for the followed channels and as the catalog's backup | [0016](decisions/0016-twitch-without-helix-sign-in.md) |
| Sign-in through a WebView | `peeby --login youtube\|twitch`: the service's own sign-in page in WebView2 (InPrivate), its session read once and checked like a manual import; the cookie import and the auth-token tutorial stay as the manual way | [0017](decisions/0017-sign-in-webview.md) |
| YouTube Shorts | A Shorts page playing YouTube's reel sequence (`reel/reel_item_watch` seedless, then `reel/reel_watch_sequence`), one player per Short, looped; metadata, comments and rating from the watch page's endpoints | [0018](decisions/0018-youtube-shorts-page.md) |
| Encrypted cookie jar | The YouTube jar encrypted with DPAPI for the Windows user (the Credential Manager is too small for it); a plain copy only for yt-dlp | [0022](decisions/0022-encrypted-cookie-jar.md) |
| Twitch raids | A live's outgoing raid from Twitch's PubSub (`raid.<id>`, anonymous; GQL shows viewers none), pinned over the chat with its countdown, followed to the raided channel unless the viewer stays | [0024](decisions/0024-twitch-raids-pubsub.md) |
| Twitch Hype Trains | A live's Hype Train read from GQL when the page opens, then followed on PubSub's `hype-train-events-v1.<id>` (anonymous, on the raid's connection), pinned over the chat as Twitch shows it: the channel's color filling the bar with the level's progress, the level, the countdown; unfolded, the level's emotes, the conductors and the record | [0032](decisions/0032-twitch-hype-trains.md) |
| Twitch multistream | Up to six lives from the sidebar's "Multistream": the main one with the whole player, the others muted in a row under it (480p, no points earned); a click swaps them. Chat as a tab per channel or merged (lines mirrored from each channel's panel, marked with its channel; writing to the chosen one). One connection per channel, one rate-limit window for every writer | [0033](decisions/0033-twitch-multistream.md) |
| Sending to a Twitch chat | A second IRC connection signed in with the web `auth-token` that only writes (cleaned lines, 20 messages per 30 s), the generic composer in `ui::chat` with an emote and emoji picker, the channel points under the field | [0023](decisions/0023-twitch-chat-sending.md) |
| 7TV live updates | A live's chat follows 7TV's EventAPI (anonymous WebSocket) for the global set, the channel's set and its 7TV user: changes applied to the session's copies, a REST refetch for whatever it cannot vouch for (new connection, unreadable change, set switch); the gear's reload stays | [0030](decisions/0030-seventv-event-api.md) |
| 7TV cosmetics | The same EventAPI connection follows the channel's `cosmetic.*` and `entitlement.*`: 7TV paints color each letter of a name with its gradient's color (image paints: the average color of each letter's slice of the image; no shadows), the 7TV badge follows the Twitch ones, applied to the lines already shown; personal emotes later | [0031](decisions/0031-seventv-cosmetics.md) |
| Restoring the tabs | Each window's place and tabs (their pages up to the one shown, as each space's route keys) kept in `session.json` every 30 s and at quit, brought back at startup (setting "Restore tabs", on), built when they show, a tab that plays not shown first | [0035](decisions/0035-restore-tabs.md) |
| SponsorBlock | A YouTube video's sponsored parts, reminders, intros… from SponsorBlock's public API, asked by a 4-digit prefix of the id's SHA-256; per category skipped, a button, or ignored; marked on the seek bar in their color | [0034](decisions/0034-sponsorblock.md) |
| YouTube original titles | Titles in the video's own language (on by default): one anonymous oEmbed request per video, cached, since no InnerTube response carries the original | [0021](decisions/0021-youtube-original-titles.md) |
| Tab history | A browser-like history per tab (back, forward), the settings page one of its entries; pages built when they show, players dropped when left behind a sidebar page | [0019](decisions/0019-tab-history.md) |
| Tab windows | Tabs dragged along the bar with Peeby's own drag (window listeners while the button is held, the release seen anywhere); several windows over the same spaces, the window used last taking what has no window, a closed window kept hidden as a spare; a tab torn out to a new window under the pointer or dropped on another window's bar | [0028](decisions/0028-tab-windows.md) |
| Crates and build times | The shell and what the spaces share in `peeby-core`, one crate per space (`space-youtube`, `space-twitch`, `space-jellyfin`), `peeby` only `main.rs`, so that a change in a space rebuilds that space; the dev profile's debug info cut to line tables | [0029](decisions/0029-crates-per-space.md) |
| Notifications and background | Windows notifications with the channel's picture and the bell, per kind (Twitch live and category change, YouTube video and live); the window hidden in the notification area; one instance; start with Windows | [0020](decisions/0020-notifications-and-background.md) |
| Twitch channel points | Read and spent through raw GQL with the auth-token (balance, rewards, predictions; two clicks per spend); the bonus chest claimed on twitch.tv (integrity check); a `minute-watched` report each minute the live plays | [0012](decisions/0012-twitch-channel-points.md) |
| Watch progress | On the account, never local: YouTube's stats pings (history, resume position) from a signed WEB `player`, the cards' `startTimeSeconds` to resume; Twitch VODs through GQL `viewingHistory` and `updateUserViewedVideo` | [0013](decisions/0013-watch-progress-on-the-account.md) |
| Installer and updates | Inno Setup per user, built by CI from a version tag and published as a GitHub Release; installed copies download a newer installer in the background (SHA-256 checked) and show a green button in the title bar that quits, installs and restarts | [0027](decisions/0027-installer-and-updates.md) |
| UI language | English (default) or French, chosen in the settings; both texts at each call site (`ui::i18n::t`, `tf!`), no catalogue; code, comments and docs in English | [0014](decisions/0014-interface-language.md) |

## Index

- [roadmap.md](roadmap.md): milestones and status
- [features-plan.md](features-plan.md): the next features the user picked (2026-11-11), not started
- [release.md](release.md): making a release (version bump, tag, CI), building the installer locally
- [refactor-plan.md](refactor-plan.md): the move to Peeby, a multi-space app (YouTube, Jellyfin, Twitch), with its checklists
- [architecture.md](architecture.md): crates, threads, data flow
- [design.md](design.md): palette, layout, pages and motion; the logo, its sources and the proposals: [logo/](logo/README.md)
- [etude-faisabilite.md](etude-faisabilite.md): the feasibility study (French), with measurements and sources
- [decisions/](decisions/): architecture decision records; the former peeby's are archived in [decisions/peeby/](decisions/peeby/)
- [player/](player/): mpv options, video surface, playback UX, ASS subtitles (from peeby)
- [twitch.md](twitch.md): how the Twitch space works (playback, credentials, chat, Helix), with what was
  checked live; [twitch-plan.md](twitch-plan.md): the technical plan it was built from, step by step, with what
  each step verified
- [jellyfin/](jellyfin/): the Jellyfin API as used, Direct Play (from peeby); [reference/](reference/): Jellyfin's OpenAPI spec
- [../vendor/README.md](../vendor/README.md): the GPUI patch
