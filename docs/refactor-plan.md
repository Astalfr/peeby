# Refactor plan: Peeby, one app for YouTube, Jellyfin and Twitch

Status: agreed 2026-11-01, all ten steps done. Tick the boxes as steps land; each step ends with
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` and a run of the app,
and YouTube keeps working after every step.

## Goal

PeebTube becomes **Peeby**, a general media app with one tab ("space") per service: YouTube, Jellyfin, and
later Twitch. UI components and the player are shared; each space only brings its API client, its pages
and the conversion of its data into the shared components' models. The code stays easy to read: the shell
knows no service, the `ui` crate knows no service.

## Decisions

| Topic | Decision |
|---|---|
| Repository | This one (`D:\DEV\peebtube`). The old `D:\DEV\peeby` repo is abandoned: the code needed is copied, nothing else is imported (no CI, packaging or `.claude/` hooks) |
| Name and icon | App, crate and exe are `peeby`, window title "Peeby"; the icon stays peebtube's (round logo with the red play badge) |
| User data | No migration: fresh `%APPDATA%\peeby\` (settings, `cookies.txt`) and `%LOCALAPPDATA%\peeby\`. Cookies are imported again and Jellyfin signed in again. ADR 0004 is updated for the new path |
| Layout | Top bar shared by every space: logo, space tabs, search, notifications bell, settings gear, window buttons. Left sidebar: the open space's entries only (no "Spaces" section, the tabs replace it) |
| Accent | One per space: YouTube red `#FF4E45`, Jellyfin amber `#FF8A3D`, Twitch violet. The theme follows the active tab; tabs carry a small colored badge (as in the mock-up) |
| Navigation | Each space has its own routes and its own page stack; switching tabs keeps each space where it was. A Jellyfin 401 sends only the Jellyfin tab to its login page |
| Switching tabs while playing | **Mini-player** as in Chrome's picture-in-picture: a separate borderless window above every other window (GPUI `WindowKind::PopUp`: `WS_EX_TOPMOST`, no taskbar entry), movable and resizable, outside the main window. The same video keeps playing (same player and video surface, no reload); a button goes back to its space and page, the cross stops it. Within a space, a covered player pauses as today |
| Search | Searches the active space. YouTube search is added (new InnerTube endpoint, fixture first) |
| Bell | Aggregated: one panel, one section per space that has notifications (YouTube now, Twitch later), the badge sums the unread counts |
| Settings and accounts | Behind the gear: general and player sections, then one section per space with its account (YouTube cookies, Jellyfin server and sign-out), storage |
| Twitch | Not implemented now; researched, documented in an ADR and `docs/twitch.md`, and the shared abstractions are checked against it |

## Target architecture

```
crates/
  player/        libmpv backend (peebtube's version: a superset of peeby's)
  gpui-video/    video surface (peebtube's version, with corner radii)
  innertube/     YouTube client, unchanged
  jellyfin-api/  copied from peeby with its fixture tests
  ui/            NEW, no service types:
    theme, motion, assets, images (blurhash, sized decode, disk cache size and clear)
    components: MediaCard, scrolling row, virtual grid, hero carousel, skeletons,
                loader, notice, settings rows, sidebar entry
    player/      generic PlayerView
  peeby/         the binary
    shell/       top bar, tabs, sidebar, per-space page stacks, mini-player, NowPlaying
    spaces/youtube/   today's pages, on top of `ui`
    spaces/jellyfin/  peeby's pages, on top of `ui`
    settings/    general settings plus one section per space
```

Principles:

1. **A space is a module implementing a `Space` trait**: name, accent, sidebar entries, its login page when
   signed out, and optionally a search and notifications. Adding Twitch means adding `spaces/twitch/` and
   one line in the list of spaces.
2. **Components take display models, not service types.** `MediaCard` takes a title, subtitle lines, an
   image (URL, size template or blurhash), badges (duration, progress, watched, unplayed count, live,
   viewers), a shape (poster 2:3, thumb 16:9, avatar) and a **single** hover callback (GPUI allows one
   `on_hover` per element; YouTube's hover prefetch needs it for the 1–2 s playback budget). Each space
   converts its own types. Element ids stay unique per list.
3. **The player is owned by the shell, not the page.** A `NowPlaying` session holds the `PlayerView`; the
   watch page (YouTube, embedded, theater mode) or the full-window page (Jellyfin) only displays it, and the
   mini-player displays the same one.
4. **The generic player takes a playback source**: mpv start options with per-source HTTP headers and no
   assumption of a separate audio track; qualities; subtitle tracks (embedded or loaded on demand) and audio
   tracks; chapters and segments; preview tiles (YouTube storyboard and Jellyfin trickplay are the same
   model); a live flag (no seek bar); previous/next availability. It emits events: `Started`, `Progress`,
   `Paused`, `Seeked`, `TracksChanged`, `Stopped` (also when dropped), `Ended`, `Next`, `Previous`,
   `Theater`. The Jellyfin space turns them into its playback reports.

## Steps

### 1. Rename to Peeby
- [x] Crate and binary `peeby`, window title and title-bar mark "Peeby", log filter, thread name
- [x] Icon: `peebtube.ico` renamed, `.rc` and `build.rs` updated, logo unchanged
- [x] Paths `%APPDATA%\peeby\` and `%LOCALAPPDATA%\peeby\`, env vars `PEEBY_COOKIES`, `PEEBY_MPV_LOG`
- [x] New ADR "multi-space app", superseding the "copies" consequence of ADR 0001; ADR 0004 path updated
- [x] CLAUDE.md, README and architecture updated (workspace version 0.2.0, `PEEBY_DISPLAY` to pick the screen)

### 2. `ui` crate
- [x] Move `theme` (accent switchable with `theme::set_accent`, used from step 4), `motion`, `assets`, `runtime`
- [x] Image store with `get_sized` and cache size and clear, downloading through a `Fetch` the app provides
      (blurhash comes with the Jellyfin space, step 6, with the `blurhash` dependency)
- [x] Components: `Picture` and `image_fill`, `MediaCard` (grid, compact, row, numbered), skeletons, loader,
      notice, settings rows (`ui::form`)
- [x] YouTube pages moved onto these components, no visual change (checked on screenshots: Home, watch page)
- Moved to step 6, where Jellyfin is their second user: poster shapes and badges of `MediaCard` (progress,
  watched, unplayed count), scrolling row with arrows, hero carousel, and a virtual grid shared by
  `feed.rs` and the Jellyfin library

### 3. Generic player
- [x] `PlayerView` moved to `ui::player`: it plays a `Source` (start options, qualities with their own
      options, subtitles added on demand, `Previews` tiles) and reads no setting: `Prefs` in, `PlayerEvent::Sound`
      and `Theater` out. YouTube builds its source in `youtube::source` (verified: 8 qualities, 6 subtitle
      tracks, playback 1.44 s)
- [x] Peeby's player features in `ui::player`: audio and subtitle tracks from mpv (named by the source when it
      knows better, `Source::audio_names`/`subtitle_names`) with menus and A/C, subtitle delay (Z/X), chapters
      (marks and titles on the seek bar, menu, Page Up/Down, mojibake repair), segments (`set_segments`, else
      guessed from chapter titles, `segments.rs` with its tests) and the skip button (S), "up next" with its
      countdown (`set_up_next`, `Prefs::autoplay_next`/`autoplay_countdown`), configurable seek steps
      (`Prefs::seek_step`/`seek_step_large`), remaining time, error panel
- [x] Events for Jellyfin's reports: `Started`, `Paused`, `Seeked`, `TracksChanged` (only on a change),
      `Close`; `position()`, `duration()`, `is_paused()`, `selected_tracks()`, `finished()`
- [x] One shortcut table (below), with the `?` help overlay (French)
- [x] Right click on the picture: a menu with "Raccourcis clavier" (the help) and "Statistiques" (mpv's codecs,
      resolution, decoding, frame rates, dropped frames, A/V sync, audio output, bitrates, buffer, read once a
      second through `Player::property`). No shortcut button in the bar. Both panels stay inside the player and
      scroll when it is smaller (checked at 640×360 by sending the clicks to the window, cursor untouched)
- [x] Two layouts: `Layout::Embedded` (YouTube) and `Layout::Window` (top bar with back, title and subtitle,
      window buttons; Esc/Backspace close; wheel volume)
- [x] `cargo run -p ui --example player`: the full-window layout on a video mpv generates (two audio tracks,
      intro segment, "up next"). Verified on screenshots: top bar, skip button, audio button, "up next" card
      and its countdown ending in `Next`
- Left to the Jellyfin space (step 6), which owns the data: reports to the server, track mapping to
  Jellyfin indexes and per-series memory, server segments, trickplay as `Previews`, external subtitle fonts,
  start fullscreen, "remember volume"
- Moved to step 4: the `NowPlaying` session. `PlayerView` is an entity, so the mini-player can hold it
  alongside (or after) its page without changing the player
- **Changed for YouTube:** arrows and J/L now both seek 10 s (Shift: 60 s), as in peeby, instead of 5 s and
  10 s; a help button and `?`; subtitle delay and chapters keys apply too
- **Not verified by hand:** the menus (audio, subtitles, chapters, quality), the help overlay, Z/X and Page
  Up/Down were not clicked or pressed (the tests do not take the mouse or the keyboard)

### 4. Shell
- [x] Top bar: logo, tabs, search (disabled until a space searches), aggregated bell, gear, window buttons
- [x] Sidebar with the active space's entries, folding as today
- [x] One page stack per space, page fade; the `Space` trait (`shell/space.rs`); YouTube under `spaces/youtube/`
- [x] Accent per space on tab switch; a placeholder Jellyfin tab (one "Accueil" page) until step 6
- [x] Settings behind the gear (the current YouTube settings page; unified at step 8); Compte stays in YouTube's
      sidebar until then
- [x] `NowPlaying` and the mini-player window (always on top, outside the main window), ADR 0008. Checked 2026-11-01 with
      `PEEBY_PIP=1 spike-yt-play`: the same `VideoView` moves into a `WindowKind::PopUp` window and back, zero-copy,
      without restarting mpv. To do: give the window its display (`display_id`), drag to move
      (`start_window_move`), resizing (the PopUp style has no frame), and release a moved view's atlas
      entries in every window (`cx.drop_image(…, None)` in `gpui-video`)
- Verified on the second screen with messages posted to the windows (cursor untouched): tabs and accent,
  the mini-player opening on a tab switch with the video playing, "Revenir à la page" (back to YouTube, the
  video playing in the page), "×" (paused, app still open), closing the main window with the mini-player open
  (the app quits), the gear's page, the bell's panel with its YouTube section. **Not verified:** moving the
  mini-player by dragging its video and resizing it from its edges and corners (posted messages carry no real button state, so the
  drag ends at once), its controls showing on a real hover (posted moves get an immediate
  `WM_MOUSELEAVE`; the controls were checked with a test build that always shows them); a notification clicked from
  another tab (it posts `notification/record_interactions`, a write to the account), a queue moving to its next
  video while in the mini-player, and the "one sound at a time" rule (needs a second space that plays)
- Also verified: the sidebar's Abonnements and Compte, back from the watch page, and a language change from
  the gear rebuilding YouTube's Home in the new language (then set back to French). A playing video stops on a
  language change, as before step 4 (every page holds the old client)

### 5. `jellyfin-api`
- [x] Copy the crate and its fixture tests from peeby (17 pass; 5 live tests ignored without a server). Its live
      tests read `PEEBY_TEST_SERVER`, `PEEBY_TEST_USER` and `PEEBY_TEST_PASSWORD` from the environment (no
      `dotenvy`). `keyring`, `uuid` and `blurhash` come with step 6, where they are used
- [x] Peeby's docs that apply here: `docs/jellyfin/`, `docs/player/`, the OpenAPI spec in `docs/reference/`;
      peeby's ADRs archived in `docs/decisions/peeby/`; [ADR 0009](decisions/0009-jellyfin-12-single-account.md)
      (Jellyfin 12+, one server, one account)

### 6. Jellyfin space
- Its player plays in the content panel, under the shared top bar, like YouTube's (agreed 2026-11-01): the
  tabs and the mini-player stay reachable, F gives fullscreen. It uses a new `Layout::Page` (back and title, no
  window buttons); `Layout::Window` stays for the example
- [x] Every item of the parity checklist below, with these changes: the search is the top bar's (scoped to the
      space), not a sidebar entry; the user (avatar or initial, name, server host) is the sidebar's bottom
      entry and opens the space's settings page, which step 8 moves behind the gear; UI strings in French
- Verified on the user's server (signed in from the former peeby's saved token and the server and user put in
  `settings.toml`): Home (banner with logo, "À suivre", "Derniers ajouts"), the sidebar (libraries under
  their heading, user and host), a library (168 posters, badges, progress), a series page (logo, details,
  genres, "Lire E6", seasons, watched episodes), the search from the top bar, a watched movie playing in the
  panel for a few seconds and Escape back to its page (it is still marked watched afterwards; the server
  probably recorded a last played date and a play count),
  the settings page, the device id saved in `[jellyfin]`. YouTube's Home and watch page unchanged
- **Not verified**: the login page, Quick Connect, sign-out and the "session expired" path (signing out
  would revoke the saved token); an episode's playback (segments, skip, up next, previous and next,
  trickplay, tracks carried over, fonts of external ASS subtitles), "Reprendre" (no item in progress), the
  banner's rotation and the rows' arrows on hover, the mini-player with a Jellyfin video

### 7. YouTube search
- [x] Save a redacted `search` response, write and test its parser, results page: `search.json` and
      `search_continuation.json` (anonymous, fr/FR, trimmed, no visitor data), `Client::search` and
      `search_more`, `parse_search` (videos and channels in order; Shorts shelves, playlists and courses left
      out), the page opened by the top bar's search button: a query run on Enter, list rows, a channel row
      opening the channel, next pages while scrolling. Also in `smoke`
- Verified: the parser tests, `smoke` live (15 videos and 1 channel, then 14 and 1), and in the app signed out
  (`PEEBY_COOKIES` pointing to no file, so the test searches stayed out of the user's history): results,
  the next page while scrolling, the channel row opening the channel and back to the results. **Not
  verified:** a signed-in search (it goes to the account's search history)

### 8. Unified settings
- [x] Settings page behind the gear (`shell::settings_page`): general (folded menu), player (shared
      `[player]`: autoplay and countdown, remember volume, seek steps, hardware acceleration, now also
      used by YouTube), then each space's part (`Space::settings`): YouTube (account and cookie import,
      language and region, default quality), Jellyfin (account with server and sign-out, start fullscreen,
      network buffer, languages and subtitles, home rows); storage (image cache), about
- [x] "Compte" left YouTube's sidebar; "Importer des cookies" buttons and Jellyfin's user entry open the
      settings (`SpaceEvent::OpenSettings`); Jellyfin's own settings page is gone
- [x] Aggregated notifications panel (done at step 4)
- Verified on screenshots: the page and its parts (YouTube signed in, Jellyfin signed in), the accent of the
  open space, Jellyfin's user entry opening it, YouTube's sidebar without "Compte". **Not verified:** a
  change of each setting by hand, the cookie import and sign-outs from there (writes), YouTube with hardware
  acceleration off

### 9. Twitch preparation
- [x] ADR and `docs/twitch.md` from the research (below): [twitch.md](twitch.md),
      [ADR 0010](decisions/0010-twitch-space.md), accepted: ads removed by a playlist-rewriting layer; the
      web `auth-token` asked for, optional, with a tutorial (user's answers, 2026-11-01)
- [x] Check that the playback source, `MediaCard`, the space trait and the watch layout cover it: they do;
      to add when the space is built: `Source::live` (no seek bar, live badge), a WebSocket client for the
      chat, animated images, the ad-removing layer if chosen, refreshable GQL settings (table in twitch.md)

### 10. Documentation
- [x] architecture.md (diagram, crates, startup, navigation, persistence), design.md (accent per space, top bar,
      sidebars, settings, YouTube search, Jellyfin pages, motion), roadmap.md, CLAUDE.md, docs/README.md (goals,
      decisions) updated; the stale names (`AppView`, `player_view`, the title bar's bell, the account page) gone

## Shortcuts (proposed single table)

| Key | Action |
|---|---|
| Space, K | Play/pause |
| ←/→, J/L | Seek by the seek step (setting); with Shift, the large step |
| Page Up / Page Down | Previous / next chapter |
| 0–9 | Jump to 0–90 % |
| Home | Back to the start |
| ↑/↓ | Volume ±5 |
| M | Mute |
| A | Cycle audio tracks |
| C | Cycle subtitles, then off |
| Z/X | Subtitle delay ±0.1 s |
| S | Skip the current segment (intro, recap, credits…) |
| N/P | Next / previous (episode or queue) |
| T | Theater mode (embedded layout) |
| F, double-click | Fullscreen |
| Esc | Close menu or help, else leave fullscreen, else close the full-window player |
| Backspace | Close the full-window player |
| ? | Shortcut help |
| Right click | Menu: shortcut help, statistics |

## Jellyfin parity checklist

Everything peeby does today must work in the Jellyfin space. References are to `D:\DEV\peeby\crates\peeby\src`.

### Session (`login.rs`, `app.rs`, `settings.rs`)
- [x] Login card: server, user name, masked password, Enter submits, `https://` added when no scheme
- [x] Server check: `/System/Info/Public`, refuse servers older than 12.0
- [x] Quick Connect: code shown, poll every 3 s, cancel
- [x] French error messages for each failure (`describe`)
- [x] `MediaBrowser` header with client, device name, device id (generated once, kept in settings), token;
      the token never in a URL
- [x] Token in the Windows Credential Manager (`keyring`), server URL and user id in settings
- [x] Session restored at startup; a 401 sends the tab to the login page with "session expired"
- [x] Sign out: `POST /Sessions/Logout`, token and user id deleted, server URL kept
- [x] User avatar and name (`/Users/Me`, initial on an accent disc as fallback)

### Sidebar
- [x] Home, then one entry per video library (film, tv or folder icon); the search is the top bar's
- [x] Server host name shown

### Home (`home.rs`)
- [x] Parallel loads: resume (16), next up (16, with the next-up options), views, latest per library not hidden
- [x] Hero: 440 px, up to 5 items from resume and next up (one per series), backdrop cross-fade 700 ms,
      logo or title, meta line (S#:E# · name · "N min restantes", or year), 3-line synopsis (spoiler rule),
      Resume/Play and Details buttons, dots, rotation every 8 s paused on hover, backdrops preloaded
- [x] Rows "Reprendre" (play), "À suivre" (play), "Derniers ajouts dans …" (details), each switchable
- [x] Skeleton rows cross-faded into content; empty state

### Library (`library.rs`)
- [x] Item types per library type (Movies, TvShows, BoxSets, other)
- [x] All pages of 200 loaded in sequence until the total, first load shown as it arrives, reload swaps at once
- [x] Virtualized grid of 160 px posters, columns from the viewport, header with title and item count

### Search (`search.rs`)
- [x] 300 ms debounce, a keystroke cancels the pending search
- [x] Movies, series and episodes, limit 60; titles as posters, "Épisodes" as thumbs; empty state

### Detail (`detail.rs`)
- [x] Hero 540 px: backdrop and gradient, logo or title, meta line (year, runtime, rating badge, star and
      community rating, series status), genre pills, 4-line synopsis
- [x] Play button: movie Resume/Play; series: episode in progress, else first unwatched, else first of the
      season shown, labelled "Reprendre E5" / "Lire E3"
- [x] Season pivot when there is more than one season; initial season: requested, else first with unwatched
      episodes, else first
- [x] Episode rows: 224 px thumbnail (spoiler blur), play overlay on hover, progress bar (0.5–99.5 %),
      "N. title", watched check, duration, "Reprendre à m:ss", 2-line synopsis (spoiler rule)
- [x] Skeleton cross-fade

### Cards and images (`ui.rs`, `images.rs`)
- [x] Shapes Poster 160 px, LargePoster 192 px (2:3), Thumb 288 px (16:9)
- [x] Image choice per shape (series poster for episodes; thumb, then backdrop, then primary), logos,
      parent backdrop fallback
- [x] Server-sized WebP at quality 85, blurhash placeholder, 200 ms fade-in
- [x] Progress bar, watched badge, unplayed count badge, play overlay on play cards, series name as its own link
- [x] Episodes and seasons open the series page on that season
- [x] Horizontal rows with hover arrows and an 80 % glide over 280 ms
- [x] Mojibake repair of chapter titles, HTML overview to plain text

### Player (`player_page.rs`, `tracks.rs`, `segments.rs`)
- [x] Playback info and direct play only: refuse transcoding with its reasons
- [x] Static stream URL, `Authorization` header given to mpv
- [x] External subtitles (delivery URL, else the subtitle stream URL); fonts of external ASS downloaded to
      the cache and given to mpv
- [x] Resume position from the user data
- [x] Reports: Playing on load, Progress every 10 s and on pause, seek and track change, Stopped once
      (also when the page is dropped); mark played at the end; the page underneath refreshed
- [x] Track mapping Jellyfin index ↔ mpv id; choice order: series prefs of the session, then the language
      settings, then the server defaults; language normalization and best match (title, forced)
- [x] Segments from the server, else from chapter titles (rules of `docs/player/playback-ux.md`, 5–240 s);
      skip button "Passer l'intro / le récap / le générique / l'aperçu", S key
- [x] Up next: from the credits tail or 20 s before the end; countdown with autoplay, "Lire maintenant",
      "Annuler"; seeking back resets it; watched only past the up-next point
- [x] Previous/next episode buttons (disabled at the ends), P/N, track prefs carried over, fullscreen kept
- [x] Trickplay thumbnails on the seek bar
- [x] Chapters: gaps on the seek bar, hover title, chapters menu, Page Up/Down
- [x] Audio and subtitle menus, subtitle delay line, help overlay
- [x] Top bar with back, title and subtitle (series · S#:E# · name, or name · year); the window buttons are the
      shell's (the player is in the panel)
- [x] Click toggles pause, double-click fullscreen, wheel volume, pause pulse, OSD 1.5 s, buffering
      overlay, error panel with Back
- [x] Software decoding and software surface options (`PEEBY_SOFTWARE_VIDEO=1` too)

### Settings (`settings.rs`, `settings_page.rs`)
- [x] Playback: autoplay next, countdown (5/10/15/20/30 s), start fullscreen, remember volume, seek step
      (5/10/15/30 s), large step (30/60/120/300 s)
- [x] Languages: audio (server default, jpn, eng, fra, spa, deu, ita, kor, zho), subtitles (server default,
      off, fra, eng, spa, deu, ita, por), subtitle scale (0.75–2)
- [x] Network buffer (150/300/600/1000 MiB), hardware acceleration
- [x] Home: next-up window (7/30/90/365 days, no limit), rewatching, show hero, continue watching, next up,
      one switch per library for "latest", hide spoilers
- [x] Storage: image cache size and Clear
- [x] About: server name and version, address, user, device, app version, sign out

## Twitch: what the architecture must allow

From the research of 2026-11-01 (sources go into `docs/twitch.md` at step 9):

- **Playback**: GQL `PlaybackAccessToken`, then the usher HLS playlist (v2 paths). Persisted-query hashes and
  the Client-ID go stale: treat them as refreshable settings, like `WEB_CLIENT_VERSION`. Clips are direct MP4.
- **Ads** are stitched into the live playlist; removing them needs a playlist-rewriting layer between Twitch
  and mpv (local proxy or libmpv stream callback). The web token of a Turbo or subscribed account removes them.
- **Live**: no seek bar, uptime and viewer count instead, audio muxed with the video, `Referer`/`Origin`
  headers for mpv. Low latency would need the same rewriting layer.
- **Two credentials**: a Helix user token (Device Code Grant, public client, no secret; single-use refresh
  tokens to persist like the rotated cookies) for followed channels, categories, search, VODs and clips; the
  web `auth-token` for GQL (recommendations, ad-free playback). Both are secrets under ADR 0004's rules.
- **Chat**: anonymous read over IRC WebSocket (`justinfan`), emotes from the tags, then badges and
  7TV/BTTV/FFZ. The watch layout needs a side-panel slot for it.
- **Categories** are a browse page of their own; image URLs are size templates (`{width}x{height}` and
  `%{width}x%{height}`).
- **Open risk**: Twitch's Client-Integrity fallback needs a Chromium browser, which conflicts with
  "nothing installed on the host". The normal path must not depend on it.
