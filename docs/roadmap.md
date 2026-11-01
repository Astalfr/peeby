# Roadmap

Status legend: ✅ done · 🚧 in progress · ⏳ not started

| Milestone | Status | Content | Exit criterion |
|---|---|---|---|
| **Study and spikes** | ✅ | Feasibility study, Home via InnerTube, playback spike (yt-dlp, then VISIONOS HLS), bundled tools | [etude-faisabilite.md](etude-faisabilite.md), ADRs 0001–0005 |
| **1: foundations** | ✅ | Workspace, CLAUDE.md, docs; `innertube` (Home, watch page, streams, cookies) with fixture tests; app shell, Home grid, watch page with autoplay and related videos | Open a video from a URL and from a related video, ~1 s to playback; signed-in Home shows recommendations |
| **2: YouTube pages** | ✅ (RSS fallback moved to the backlog) | Subscriptions list and feed (`FEsubscriptions`, RSS fallback), comments (`commentEntityPayload`) and replies, Return YouTube Dislike bar (cache, attribution), channel page, infinite scroll | — |
| **3: player comfort** | ✅ | Quality choice, subtitles on demand (`sub-add`), prefetch on hover, yt-dlp fallback ([ADR 0005](decisions/0005-bundled-yt-dlp.md)) | — |
| **4: session and settings** | ✅ | Cookie import UI, `Set-Cookie` persistence, settings page (language, quality), yt-dlp updater | — |
| **Peeby refactor** | ✅ | One space per service (YouTube, Jellyfin), shared `ui` crate and player, tabs, mini-player, YouTube search, unified settings ([refactor-plan.md](refactor-plan.md), ADRs 0007–0010) | All ten steps of the plan; peeby's Jellyfin features in the Jellyfin space |
| **Twitch space** | 🚧 | Helix catalog, GQL/usher playback with the ad-removing layer, `auth-token` with its tutorial, read-only chat ([twitch.md](twitch.md), [ADR 0010](decisions/0010-twitch-space.md), plan: [twitch-plan.md](twitch-plan.md)) | Live and VOD playback without ads, followed channels, chat |
| **5: distribution** | ⏳ | CI, bundle with libmpv, yt-dlp and Deno in `tools/`, installer (peeby's scripts) | Downloadable build |

## Twitch space

- 7TV emotes in the chat (2026-11-01, step 9): `twitch::seventv` (global and channel sets, the channel's aliases, still frames of animated emotes), loaded once per channel and session from the room's id, words replaced by emotes. 5 new tests on 3 saved responses. Verified in the app on a busy channel. Animated and zero-width rendering, badges, BTTV and FFZ come later.

- Step 9 done (2026-11-01): the read-only chat beside a live. Generic in `ui::chat` (model, `ChatPanel`: a virtual list of 500 messages following the newest, held while scrolled up, emotes inline, deletions greyed, the room's modes; `watch_layout`: the watch page with a chat column), so that YouTube lives can plug in later; `twitch::chat` (IRC parser, anonymous WebSocket with reconnection) and `spaces::twitch::chat` in between. Emote ranges proved to count code points. 19 new tests, 9 of them on a fixture of 21 anonymized IRC lines. Verified in the app on three busy channels; the panel example holds 120 frames a second at 50 messages a second. **Not verified:** a reconnection, the resume button's click, a deletion seen in the app. Badges, third-party and animated emotes come next.

- Step 8 done (2026-11-01): VODs (through the proxy, seekable, storyboard previews on the seek bar), clips (direct MP4s from GQL) and a channel's page (header, live banner, VODs and clips tabs); `peeby` opens `twitch.tv/videos/<id>` and clip links too. 7 new tests on 5 new fixtures. Verified in the app: a VOD in 0.99 s with a preview on hover, a clip in 1.34 s, the channel page. **Not verified:** the clips tab by click, a subscriber-only VOD in the app; muted segments are not shown.

- Step 7 done (2026-11-01): the Helix catalog. `twitch::helix` (followed lives and channels, users, top categories, a category's lives, search; a refused token refreshed once) tested on 8 fixtures saved by `cargo run -p peeby --example twitch_capture` (trimmed, every trace of the account replaced). Pages: "Suivies" (lives as cards, then the offline channels), "Parcourir" (categories) and a category's lives, the search from the top bar; the sidebar lists the followed channels on air, refreshed every 2 minutes. Verified in the app signed in, from screenshots. **Not verified:** the offline section, the sidebar's refresh, a 429.

- Step 6 done (2026-11-01): lives play in the Twitch tab. `peeby twitch.tv/<login>` (or a name typed on "Suivies", a placeholder until step 7) opens a watch page: the shared player in live mode through the ad-removing proxy, the channel's header from a GQL `Channel` query (avatar, title, category, viewers, uptime), a column kept for the chat, theater mode, the ad notice when the backup cannot cover a break, a default quality setting. Verified signed in with the user's auth-token: playback after 1.33 s, 6 qualities, the live badge with uptime and viewers. **Not verified:** the ad notice in the app, the mini-player with a live, the offline and unknown-channel messages in the app.

- Step 5 done (2026-11-01): the Twitch tab (violet, `TW`) with its sign-in card (Device Code Grant with Peeby's Client-ID: the code shown large, twitch.tv/activate opened, polling), `twitch::auth` (device flow, single-flight refresh storing each single-use refresh token before use, validation at startup and hourly, revoke), the secrets in the Credential Manager (`twitch|refresh`, `twitch|auth-token`), and its settings part (account, the optional auth-token with its tutorial, checked through GQL `currentUser`). 6 new tests on 5 new fixtures; `smoke` gets a device code live. Checked on screenshots: the tab, the card, the settings part; then with the user: a real sign-in and auth-token, and the session restored after a restart. **Not verified:** the hourly validation, sign-out (it would end the user's session).

- Step 4 done (2026-11-01): the shared player plays lives (`Source::live`: no seek bar, seeking keys or remaining time, an "EN DIRECT" badge with uptime and viewers from `set_live_info`, quality changes at the live edge, mpv's window duration ignored) and shows a page's message over the picture (`set_notice`, for a removed ad). `PEEBY_LIVE=1 cargo run -p ui --example player` shows both; checked on a screenshot, with the normal mode unchanged. The app still opens and plays a YouTube video (screenshot). **Not verified:** YouTube's control bar in the app (it had hidden before the screenshot, no mouse moved), Jellyfin, the live keys pressed.

- Step 3 done (2026-11-01): ads removed. `twitch::ads` reads media playlists (Amazon titles, stitched-ad date ranges, live sequence), `twitch::splice` fills an ad break with an ad-free backup playlist (`playerType=popout`) aligned on Twitch's live sequence, and `twitch::Proxy` serves the result to mpv from `127.0.0.1` (random key, playlists only, segments straight from the CDN). `Client::play` asks for both tokens together. Measured in the spike: 1.35 s to playback through the proxy; a forced pre-roll (`site`) covered by the backup, 1.51 s, no stall. 8 new tests on the saved reload sequence (98 in the workspace). **Not verified:** a mid-roll, the no-backup notice, the switch by eye.

- Step 2 done (2026-11-01): the `twitch` crate's GQL playback token (raw query, `playerType=embed`, Client-ID read from twitch.tv's page), usher v2 then v1, master variants (v2 `IVS-*` and v1 formats, landscape/portrait, codecs, audio only), and errors (`NotLoggedIn`, `NotFound`, `Offline`, `SubscriberOnly`, `IntegrityRequired`). 13 tests on 6 new fixtures; `cargo run -p twitch --example smoke` passes live. **Not verified:** `IntegrityRequired` against a real answer, the auth-token path, clips and storyboards (step 8).

- Step 1 of [twitch-plan.md](twitch-plan.md) done (2026-11-01): `spikes/twitch-play` plays a live in 0.98–1.29 s and a public VOD in 1.29 s, anonymously: the raw GQL token query, `playerType=embed` (no pre-roll, unlike `site`), usher v2, 1080p60 H.264 on the GPU. Redacted fixtures in `crates/twitch/tests/fixtures/` (masters v1 and v2, 25 reloads across a pre-roll, a backup playlist, GQL tokens, a subscriber-only VOD's 403). The `twitch` crate exists, empty. **Not verified:** a mid-roll, HEVC/AV1 (capped at 1080p signed out), anything signed in.

## Peeby refactor

- Step 10 done (2026-11-01): the docs reread and brought up to date (architecture, design, docs/README.md, CLAUDE.md). The refactor plan is complete; what was not verified is listed under each step of [refactor-plan.md](refactor-plan.md).
- Step 9 done (2026-11-01): [twitch.md](twitch.md) (the research with its sources, and what the architecture covers) and [ADR 0010](decisions/0010-twitch-space.md): ads removed by a playlist-rewriting layer, the web `auth-token` asked for with a tutorial (the user's choices). No code.
- Step 8 done (2026-11-01): one settings page behind the gear, with the app's sections, a shared `[player]` section used by both spaces, and a part per space (`Space::settings`); YouTube's account moved there from its sidebar. Verified on screenshots; see the plan.
- Step 7 done (2026-11-01): YouTube search (`Client::search`, `search_more`, `parse_search` tested on two new redacted fixtures, a results page behind the top bar's search button with next pages on scroll). Verified signed out; see the plan.
- Step 6 done (2026-11-01): the Jellyfin space (`spaces/jellyfin/`): login with Quick Connect, token in the Credential Manager, Home with its banner and rows, libraries as a virtual grid, a title's page, search from the top bar, playback in the content panel (`Layout::Page`) with reports, track mapping, segments, trickplay and adjacent episodes, its settings page. Shared pieces added to `ui`: BlurHash placeholders and per-server image routes, logos, posters with progress and badges, scrolling rows, the grid layout, `plain_text`. Verified on the user's server and on screenshots; see the plan for what was not.
- Step 5 done (2026-11-01): `jellyfin-api` copied from peeby with its fixture tests (17 pass), its live tests reading credentials from the environment; peeby's Jellyfin and player docs and the OpenAPI spec copied, its ADRs archived in `decisions/peeby/`, [ADR 0009](decisions/0009-jellyfin-12-single-account.md). **Not verified:** the live tests (no Jellyfin server given yet).
- Step 4 done (2026-11-01): the shell (top bar with tabs, search, aggregated notifications and gear; per-space sidebar and page stacks; accent per space), YouTube under `spaces/youtube/`, a placeholder Jellyfin tab, the mini-player window above the others ([ADR 0008](decisions/0008-mini-player-window.md)). Verified on screenshots with posted clicks; see the plan for what was not.
- Step 3 done (2026-11-01): peeby's player features in `ui::player` (tracks, subtitle delay, chapters, segments and skip, "up next", help, seek steps, full-window layout, events for reports); `cargo run -p ui --example player` shows them on a generated video. Verified: clippy, tests (segments, text), the example on screenshots, YouTube playback 1.48 s. **Not verified:** menus and keys by hand. Arrows now seek 10 s on YouTube too.
- Step 3 started (2026-11-01): the player moved to `ui::player` and plays a generic `Source`; the mini-player approach was checked with `PEEBY_PIP=1 spike-yt-play` (zero-copy video in an always-on-top `PopUp` window and back). Verified: clippy, tests, playback 1.44 s with 8 qualities and 6 subtitle tracks. **Not verified:** the quality and subtitle menus and the storyboard by hand (same code, not clicked).
- Step 2 done (2026-11-01): `ui` crate (theme with a switchable accent, motion, icons, image store behind a `Fetch`, `Picture`, `MediaCard` in four layouts, skeletons, notices, settings rows); YouTube pages use it, no visual change. Verified: clippy, tests (2 new), Home and watch page on screenshots, playback 1.47 s.
- Step 1 done (2026-11-01): PeebTube renamed Peeby (crate, exe, window, `%APPDATA%\peeby`, `PEEBY_*`), version 0.2.0, [ADR 0007](decisions/0007-multi-space-app.md); `PEEBY_DISPLAY` picks the screen the window opens on. Verified: build, clippy, tests, and `peeby <url>` playing after about 1.5 s. **Not verified:** which screen the window lands on with `PEEBY_DISPLAY=secondary` (no warning, so a second display was found). Next steps and checklists: [refactor-plan.md](refactor-plan.md).

## Notifications

- Done (2026-11-01): the bell of subscribed channels, on the watch page (right of the channel) and on the channels page's tiles: "Toutes", "Personnalisées" or "Aucune" (those the channel offers), the icon showing the current level. Parsed and tested on `subscriptions_channels.json` and a new trimmed, redacted signed-in watch page (`next_signed_in.json`); the signed-out `next.json` carries a bell too but is not subscribed, so it shows none.
- Verified by driving the app: the watch page of a subscribed channel shows its bell ("Toutes"), the menu opens with the current level checked; the channels page shows the 475 bells (all three icons seen), a tile's bell opens its menu without opening the channel, and the grid still scrolls. `smoke` reads 475 bells out of 475 channels live.
- **Not verified:** no level was changed: `notification/modify_channel_preference` was never sent (writing to the account needs the user's go-ahead), so its response has no fixture yet, and the toasts and the revert on failure were not seen. Scrolling smoothness of the channels page with 475 bells was not measured.
- Then: the **inbox**, a bell at the right of the title bar with the unseen count, the notifications in sections as on youtube.com, next pages on scroll, a click opening the video and marking it read. youtube.com reads it with `browse` `FEnotifications_inbox` (found from the browser's own request): `notification/get_notification_menu` answers an empty inbox. Parsers tested on 3 new redacted fixtures (inbox, its next page, the unseen count); `smoke` reads the inbox, its next page and the count live. Fix on the way: the channel bells' levels were not forgotten on a session change (`bell::reset` ran at startup only).
- Verified by driving the app: the bell in the title bar, the panel (Important, Autres notifications, unread dots, avatars, thumbnails), next pages while scrolling (down to notifications 12 days old), no warning in the log.
- **Not verified:** the badge with a count above 0 (the count was 0 during the test), whether YouTube resets its count when the panel opens, clicking a notification: `notification/record_interactions` was never sent (writing to the account needs the user's go-ahead) and its body field name (`serializedRecordNotificationInteractionsRequest`) is not confirmed by a capture. The notifications' "⋮" menu (hide, fewer, turn off) is not offered.

## Smooth feeds

- Done (2026-11-01): the feeds (Home, subscriptions in grid and list mode, channel) are a virtual list, so a scroll builds only the rows in view instead of every loaded card; their header stays at the top. List rows take the whole width; grid cards are at least 416 px and widen to fill the row.
- Verified: the app starts and Home loads (22 videos). **Not verified:** scrolling smoothness, the next page loading at the end, resizing and folding the sidebar (cards per row), the grid/list switch, by hand.

## Like and dislike

- Done (2026-11-01): the watch page's like and dislike buttons rate the video (`like/like`, `like/dislike`, `like/removelike` with the page's `params`); a click on the active one clears the rating. They show the user's rating (`likeStatusEntity`, parsed and tested on `next.json`), change at once and go back with a toast if YouTube refuses; signed out, a toast asks for cookies. The two halves sit close together on a pill as wide as its content; the RYD link moved to the attribution line.
- **Not verified:** not driven in the app, and no rating was sent (writing to the account needs the user's go-ahead), so the `like/*` responses have no fixture yet and the signed-in `likeStatus` is untested.

## Theater mode and folding sidebar

- Done (2026-11-01): theater mode on the watch page ("Mode cinéma" button and T key; the player across the page above the details and related videos), a burger button at the left of the title bar that folds the sidebar to its icons (labels as tooltips). Both are saved in `settings.toml`.
- **Not verified:** not driven in the app yet (build, clippy and tests only): the theater layout and its height, the button and the T key, the folded sidebar and its tooltips.

## Save to playlist

- Done (2026-11-01): "⋮" menu and "Watch later" shortcut on grid cards and list rows (hover only), "Enregistrer" on the watch page (since then a "⋮" menu with "Enregistrer dans une playlist"), the list of playlists with checks, toasts. `playlist/get_add_to_playlist` is parsed and tested on a redacted fixture.
- Then: toasts at the bottom right; an "x" on the videos of editable playlists (page and queue panel) removes the entry (`ACTION_REMOVE_VIDEO` by `setVideoId`, parsed and tested on the "Watch later" fixture).
- Then: a spinner replaces the "x" and the "Watch later" button while their request runs; "À regarder plus tard" in the sidebar, above Playlists; a custom loader (ring and stretching arc); the "Watch later" shortcut shows a check on videos already in it (ids loaded at startup) and then removes them; a reverse-order button on playlist pages (saved per playlist); the URL field of the title bar is gone.
- Verified: the card buttons appear on hover. **Not verified:** the popover and its list in the app (the test run was stopped), the "x" buttons and the toasts' position on screen, and any actual add or remove: the `browse/edit_playlist` request was never sent (writing to the account needs the user's go-ahead), so its response has no fixture yet.

## Playlists

- Done (2026-11-01): "Playlists" in the sidebar: the user's playlists, a playlist's page (header, numbered videos, next pages on scroll), "Tout lire" and "Aléatoire". Playing a playlist shows its queue next to the player: a video that ends plays the next one, previous/next buttons and P/N keys, shuffle, repeat, next pages loaded as the end comes near, next stream resolved ahead. Parsers tested on 4 new redacted fixtures (library, "Watch later", a public playlist and its continuation); `smoke` checks the library, "Liked videos" and its next page live.
- Verified by driving the app: the playlists page (15 playlists), "Watch later" (17 videos), "Tout lire" (playback after 1.7 s), N, then the end of a video playing the next one (1.3 s, prefetched), the same in fullscreen (it stays fullscreen, Escape still works), playing from row 4, shuffle, back to the playlist page.
- **Not verified:** reaching the end of a loaded page during playback (the queue loads the next one first; only its request was checked live), repeat at the end of a playlist.

## Visual refresh

- Done (2026-11-01): peeby's visual system ([design.md](design.md), [ADR 0006](decisions/0006-peeby-visual-system.md)): card hover (lift, red ring, play button), image fade-in, page fade, skeletons on the feeds and the watch page, Fluent sidebar, settings rows, account card, centered empty and error states.
- Then: the subscribed channels moved from the sidebar to a **Channels page** (grid with a filter, opened from "Toutes les chaînes" on the subscriptions page); grid rows align cards at the top (titles at most two lines); feeds drop repeated videos, which made hovered cards jump (shared hover state).
- Subscriptions page: grid or list mode, saved in `settings.toml` (`subscriptions_layout`). Verified: the switch, the rows, hover, and the saved value.
- Verified by driving the mouse: hover on Home and on the subscriptions page (one hover event in, one out, even at the bottom edge of a card), rows aligned at the top with one- and two-line titles, the channels page (475 channels, filter, hover, a click opens the channel). Playback: 1.7 s on a test video. **Not verified:** page fades by eye.

## Milestone 4 status

- Done (2026-11-01):
  - **Account page** (sidebar "Compte"): session state, export instructions, "Importer cookies.txt…" (file picker; checked with a Home request before it replaces the session file), "Se déconnecter". The app switches session without restarting. Pages that need a session link to it; an expired session says so.
  - **yt-dlp updates**: single-file build (it can update itself, "onedir" cannot), `yt-dlp -U` at most once a day in the background. Verified: "yt-dlp is up to date (stable@2026.10.26)", then the fallback played an age-restricted video in 5.4 s.
- **Not verified:** the import flow in the UI (file picker, valid file), since no fresh cookies were available; the expired-session state was seen in the logs only.
- **Settings page** (sidebar "Réglages"), saved in `%APPDATA%\peebtube\settings.toml` (`serde` + `toml`, agreed with the user):
  - language and region of YouTube (11 presets): the clients are rebuilt at once, Home reloads;
  - default quality (2160p to 360p), used by the watch page and the hover prefetch;
  - volume and mute, kept across launches.
  Writes are delayed 0.5 s and replace each other (a volume drag makes dozens of changes). Unknown or missing keys take their defaults.
  Verified: a file with `max_height = 720` gives a 1280x720 stream at launch. The page itself was not clicked in a test run.

## Milestone 3 status

- Done (2026-11-01):
  - **Quality menu**: the whole HLS ladder (2160p to 144p, VP9 preferred). Changing quality restarts mpv on the new variant at the current position: verified with the `player` crate directly, 1080p to 720p, playback again after about 1.0–1.1 s, subtitle track kept.
  - **Subtitles on demand**: the VISIONOS player response lists the caption tracks (`captions.playerCaptionsTracklistRenderer`); mpv fetches one as WebVTT (`&fmt=vtt`) with `sub-add` when it is picked (menu or C). Verified: loaded and selected in about 0.1 s. Machine translations (`&tlang=`) answer HTTP 429 for this client, so they are not offered.
  - **Prefetch on hover**: resting 300 ms on a video card resolves its stream; the watch page then starts mpv without waiting for the resolver (saves the 0.2–0.4 s). Cache of 64 streams, 4 h.
  - yt-dlp fallback (see milestone 2).
- **Not verified:** the menus and the hover prefetch in the UI (not clicked in a test run). The app logs the ladder and tracks it receives: 8 qualities and 6 tracks on a test video.

## Milestone 2 status

- Done and verified (2026-11-01), signed in:
  - `cargo run -p innertube --example smoke -- <cookies.txt>` passes all endpoints live: Home (23 videos), subscriptions (111 videos, more), 475 subscribed channels, channel videos and continuation, watch page, comments (20, total count), stream, RYD votes.
  - App: signed-in Home grid; sidebar with Home, Subscriptions and the subscribed channels; watch page with playback in about 1.6 s, comments and the RYD bar.
  - Player controls adapted from peeby's player page (`player_view.rs`): seek bar with a storyboard thumbnail and the time over the hovered point (YouTube's `playerStoryboardSpecRenderer` sprite sheets, largest level) (click and drag: the playhead follows the mouse until the release), volume slider, play/pause, ±10 s, fullscreen (only the player is shown), auto-hide, OSD, pause pulse, shortcuts (K/space, J/L, arrows, 0–9, M, F, Esc). Volume is kept for the session.
  - Fixtures from signed-in responses are trimmed (3–4 items) and stripped of account data (`datasyncId`, visitorData, top bar).
- **Not verified:** clicks in the UI (sidebar, channel link, "Plus de vidéos", "Plus de commentaires", player buttons, volume drag, fullscreen) — same code paths as the logged ones, but not clicked in a test run.
- Then (same day):
  - Comment replies ("N réponses", then "Plus de réponses"), tested on a real replies page (`comment_replies.json`).
  - Infinite scroll on feeds and comments: the next page starts within about a screen of the end; a "Réessayer" button only after a failure.
  - yt-dlp fallback when VISIONOS fails (verified on an age-restricted video: 8.1 s, see ADR 0005).
  - Cookie rotation: the session was invalidated after about an hour because `Set-Cookie` updates were ignored. They are now applied and saved (ADR 0004). **The cookies must be exported again**, and the fix is not verified over a long session yet.
- RSS fallback for subscriptions: moved to the backlog (the signed-in feed works).

## Milestone 1 status

- Done and verified (2026-11-01):
  - `innertube`: 11 tests on real fixtures, including a feed continuation (continuations carry `logged_in` too). `cargo run -p innertube --example home` answers `logged_in=false` with YouTube's message.
  - App, signed out:
    - Home shows the empty state with the cookie instructions;
    - `peebtube <url>` plays 1080p VP9 with d3d11va and audio, about 1.4–1.6 s after the page opens;
    - title, channel, description and related videos with thumbnails are shown;
    - a covered watch page pauses its player, even when the player starts after being covered (`peebtube <id1> <id2>`). "Back" does not resume by itself.
  - The VISIONOS resolver uses its own anonymous visitorData, separate from the signed-in session's.
  - InnerTube errors are shown in French (`ui::error_text`).
- **Not verified:**
  - signed-in Home (needs the user's cookies);
  - clicks in the UI: the URL field, a related card, "Plus de vidéos", the back button (wired, same code paths as the command line);
  - Claude Code hooks (fmt, clippy gate): not installed yet, see CLAUDE.md "Ask before".

## Backlog

- RSS fallback for the subscriptions feed (`feeds/videos.xml?channel_id=`), for use without a session.
- Shorts shelves (`shortsLockupViewModel`) on Home and subscriptions.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| YouTube blocks the VISIONOS client | No playback | yt-dlp fallback, values tracked against yt-dlp's source |
| YouTube changes renderers | Empty feeds or pages | Lenient parsing, fixtures, `logged_in` checks |
| Cookies expire or rotate | Signed out | `Error::NotLoggedIn` shown to the user; persist `Set-Cookie` (milestone 4) |
| Copied crates drift from peeby | Duplicate fixes | Shared repository once both stabilize |
