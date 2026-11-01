# Roadmap

Status legend: ✅ done · 🚧 in progress · ⏳ not started

| Milestone | Status | Content | Exit criterion |
|---|---|---|---|
| **Study and spikes** | ✅ | Feasibility study, Home via InnerTube, playback spike (yt-dlp, then VISIONOS HLS), bundled tools | [etude-faisabilite.md](etude-faisabilite.md), ADRs 0001–0005 |
| **1: foundations** | ✅ | Workspace, CLAUDE.md, docs; `innertube` (Home, watch page, streams, cookies) with fixture tests; app shell, Home grid, watch page with autoplay and related videos | Open a video from a URL and from a related video, ~1 s to playback; signed-in Home shows recommendations |
| **2: YouTube pages** | ✅ (RSS fallback moved to the backlog) | Subscriptions list and feed (`FEsubscriptions`, RSS fallback), comments (`commentEntityPayload`) and replies, Return YouTube Dislike bar (cache, attribution), channel page, infinite scroll | — |
| **3: player comfort** | ✅ | Quality choice, subtitles on demand (`sub-add`), prefetch on hover, yt-dlp fallback ([ADR 0005](decisions/0005-bundled-yt-dlp.md)) | — |
| **4: session and settings** | ✅ | Cookie import UI, `Set-Cookie` persistence, settings page (language, quality), yt-dlp updater | — |
| **5: distribution** | ⏳ | CI, bundle with libmpv, yt-dlp and Deno in `tools/`, installer (peeby's scripts) | Downloadable build |

## Notifications

- Done (2026-11-01): the bell of subscribed channels, on the watch page (right of the channel) and on the channels page's tiles: "Toutes", "Personnalisées" or "Aucune" (those the channel offers), the icon showing the current level. Parsed and tested on `subscriptions_channels.json` and a new trimmed, redacted signed-in watch page (`next_signed_in.json`); the signed-out `next.json` carries a bell too but is not subscribed, so it shows none.
- Verified by driving the app: the watch page of a subscribed channel shows its bell ("Toutes"), the menu opens with the current level checked; the channels page shows the 475 bells (all three icons seen), a tile's bell opens its menu without opening the channel, and the grid still scrolls. `smoke` reads 475 bells out of 475 channels live.
- **Not verified:** no level was changed: `notification/modify_channel_preference` was never sent (writing to the account needs the user's go-ahead), so its response has no fixture yet, and the toasts and the revert on failure were not seen. Scrolling smoothness of the channels page with 475 bells was not measured.
- Then: the **inbox**, a bell at the right of the title bar with the unseen count, the notifications in sections as on youtube.com, next pages on scroll, a click opening the video and marking it read. youtube.com reads it with `browse` `FEnotifications_inbox` (found from the browser's own request): `notification/get_notification_menu` answers an empty inbox. Parsers tested on 3 new redacted fixtures (inbox, its next page, the unseen count); `smoke` reads the inbox, its next page and the count live. Fix on the way: the channel bells' levels were not forgotten on a session change (`bell::reset` ran at startup only).
- Verified by driving the app: the bell in the title bar, the panel (Important, Autres notifications, unread dots, avatars, thumbnails), next pages while scrolling (down to notifications 12 days old), no warning in the log.
- **Not verified:** the badge with a count above 0 (the count was 0 during the test), whether YouTube resets its count when the panel opens, clicking a notification: `notification/record_interactions` was never sent (writing to the account needs the user's go-ahead) and its body field name (`serializedRecordNotificationInteractionsRequest`) is not confirmed by a capture. The notifications' "⋮" menu (hide, fewer, turn off) is not offered.

## Smooth feeds

- Done (2026-11-01): the feeds (Home, subscriptions in grid and list mode, channel) are a virtual list, so a scroll builds only the rows in view instead of every loaded card; their header stays at the top. List rows take the whole width; grid cards are 416 px.
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
  - Player controls adapted from peeby's player page (`player_view.rs`): seek bar with time preview (click and drag: the playhead follows the mouse until the release), volume slider, play/pause, ±10 s, fullscreen (only the player is shown), auto-hide, OSD, pause pulse, shortcuts (K/space, J/L, arrows, 0–9, M, F, Esc). Volume is kept for the session.
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
