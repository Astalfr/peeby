# Refactor plan: Peeby, one app for YouTube, Jellyfin and Twitch

Status: agreed 2026-11-01, step 1 done. Tick the boxes as steps land; each step ends with
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
| Switching tabs while playing | **Mini-player**: the same video keeps playing in a floating window at the bottom right (same video surface, no reload); a click goes back to its space and page, the cross stops it. Within a space, a covered player pauses as today |
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
- [ ] Move `theme` (accent per space, re-applied on tab switch), `motion`, `assets`, `runtime`
- [ ] Image store: union of both versions (URL keys, blurhash, `get_sized`, cache size and clear), with a
      fetcher that does not depend on a service client
- [ ] Components: `MediaCard` and its shapes, list row, compact card, scrolling row with arrows, virtual
      grid (from `feed.rs`), hero carousel, skeletons, loader, notice, settings rows
- [ ] YouTube pages moved onto these components, no visual change

### 3. Generic player
- [ ] `PlayerView` takes a playback source and emits events (see principles); YouTube's stream, captions
      and storyboard become one source among others
- [ ] Port peeby's player features one at a time (see the checklist below)
- [ ] One shortcut table (below), with the `?` help overlay
- [ ] Two layouts: embedded and full window (own window buttons, Esc/Backspace closes)

### 4. Shell
- [ ] Top bar: logo, tabs, search, bell, gear, window buttons
- [ ] Sidebar with the active space's entries, folding as today
- [ ] One page stack per space, page fade
- [ ] `NowPlaying` and the mini-player
- [ ] YouTube as the only space

### 5. `jellyfin-api`
- [ ] Copy the crate and its fixture tests from peeby; workspace dependencies `keyring`, `uuid`, `url`,
      `blurhash` (needed by the Jellyfin code)

### 6. Jellyfin space
- [ ] Every item of the parity checklist below

### 7. YouTube search
- [ ] Save a redacted `search` response, write and test its parser, results page

### 8. Unified settings
- [ ] Settings page behind the gear: general, player, YouTube (language, region, quality, cookies),
      Jellyfin (home rows, server, sign-out), storage, about
- [ ] Aggregated notifications panel

### 9. Twitch preparation
- [ ] ADR and `docs/twitch.md` from the research (below)
- [ ] Check that the playback source, `MediaCard`, the space trait and the watch layout cover it

### 10. Documentation
- [ ] architecture.md, design.md, roadmap.md, CLAUDE.md, README updated

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

## Jellyfin parity checklist

Everything peeby does today must work in the Jellyfin space. References are to `D:\DEV\peeby\crates\peeby\src`.

### Session (`login.rs`, `app.rs`, `settings.rs`)
- [ ] Login card: server, user name, masked password, Enter submits, `https://` added when no scheme
- [ ] Server check: `/System/Info/Public`, refuse servers older than 12.0
- [ ] Quick Connect: code shown, poll every 3 s, cancel
- [ ] French error messages for each failure (`describe`)
- [ ] `MediaBrowser` header with client, device name, device id (generated once, kept in settings), token;
      the token never in a URL
- [ ] Token in the Windows Credential Manager (`keyring`), server URL and user id in settings
- [ ] Session restored at startup; a 401 sends the tab to the login page with "session expired"
- [ ] Sign out: `POST /Sessions/Logout`, token and user id deleted, server URL kept
- [ ] User avatar and name (`/Users/Me`, initial on an accent disc as fallback)

### Sidebar
- [ ] Home, Search, then one entry per video library (film, tv or folder icon)
- [ ] Server host name shown

### Home (`home.rs`)
- [ ] Parallel loads: resume (16), next up (16, with the next-up options), views, latest per library not hidden
- [ ] Hero: 440 px, up to 5 items from resume and next up (one per series), backdrop cross-fade 700 ms,
      logo or title, meta line (S#:E# · name · "N min restantes", or year), 3-line synopsis (spoiler rule),
      Resume/Play and Details buttons, dots, rotation every 8 s paused on hover, backdrops preloaded
- [ ] Rows "Reprendre" (play), "À suivre" (play), "Derniers ajouts dans …" (details), each switchable
- [ ] Skeleton rows cross-faded into content; empty state

### Library (`library.rs`)
- [ ] Item types per library type (Movies, TvShows, BoxSets, other)
- [ ] All pages of 200 loaded in sequence until the total, first load shown as it arrives, reload swaps at once
- [ ] Virtualized grid of 160 px posters, columns from the viewport, header with title and item count

### Search (`search.rs`)
- [ ] 300 ms debounce, a keystroke cancels the pending search
- [ ] Movies, series and episodes, limit 60; titles as posters, "Épisodes" as thumbs; empty state

### Detail (`detail.rs`)
- [ ] Hero 540 px: backdrop and gradient, logo or title, meta line (year, runtime, rating badge, star and
      community rating, series status), genre pills, 4-line synopsis
- [ ] Play button: movie Resume/Play; series: episode in progress, else first unwatched, else first of the
      season shown, labelled "Reprendre E5" / "Lire E3"
- [ ] Season pivot when there is more than one season; initial season: requested, else first with unwatched
      episodes, else first
- [ ] Episode rows: 224 px thumbnail (spoiler blur), play overlay on hover, progress bar (0.5–99.5 %),
      "N. title", watched check, duration, "Reprendre à m:ss", 2-line synopsis (spoiler rule)
- [ ] Skeleton cross-fade

### Cards and images (`ui.rs`, `images.rs`)
- [ ] Shapes Poster 160 px, LargePoster 192 px (2:3), Thumb 288 px (16:9)
- [ ] Image choice per shape (series poster for episodes; thumb, then backdrop, then primary), logos,
      parent backdrop fallback
- [ ] Server-sized WebP at quality 85, blurhash placeholder, 200 ms fade-in
- [ ] Progress bar, watched badge, unplayed count badge, play overlay on play cards, series name as its own link
- [ ] Episodes and seasons open the series page on that season
- [ ] Horizontal rows with hover arrows and an 80 % glide over 280 ms
- [ ] Mojibake repair of chapter titles, HTML overview to plain text

### Player (`player_page.rs`, `tracks.rs`, `segments.rs`)
- [ ] Playback info and direct play only: refuse transcoding with its reasons
- [ ] Static stream URL, `Authorization` header given to mpv
- [ ] External subtitles (delivery URL, else the subtitle stream URL); fonts of external ASS downloaded to
      the cache and given to mpv
- [ ] Resume position from the user data
- [ ] Reports: Playing on load, Progress every 10 s and on pause, seek and track change, Stopped once
      (also when the page is dropped); mark played at the end; the page underneath refreshed
- [ ] Track mapping Jellyfin index ↔ mpv id; choice order: series prefs of the session, then the language
      settings, then the server defaults; language normalization and best match (title, forced)
- [ ] Segments from the server, else from chapter titles (rules of `docs/player/playback-ux.md`, 5–240 s);
      skip button "Passer l'intro / le récap / le générique / l'aperçu", S key
- [ ] Up next: from the credits tail or 20 s before the end; countdown with autoplay, "Lire maintenant",
      "Annuler"; seeking back resets it; watched only past the up-next point
- [ ] Previous/next episode buttons (disabled at the ends), P/N, track prefs carried over, fullscreen kept
- [ ] Trickplay thumbnails on the seek bar
- [ ] Chapters: gaps on the seek bar, hover title, chapters menu, Page Up/Down
- [ ] Audio and subtitle menus, subtitle delay line, help overlay
- [ ] Top bar with back, title and subtitle (series · S#:E# · name, or name · year), window buttons
- [ ] Click toggles pause, double-click fullscreen, wheel volume, pause pulse, OSD 1.5 s, buffering
      overlay, error panel with Back
- [ ] Software decoding and software surface options (`PEEBY_SOFTWARE_VIDEO=1` too)

### Settings (`settings.rs`, `settings_page.rs`)
- [ ] Playback: autoplay next, countdown (5/10/15/20/30 s), start fullscreen, remember volume, seek step
      (5/10/15/30 s), large step (30/60/120/300 s)
- [ ] Languages: audio (server default, jpn, eng, fra, spa, deu, ita, kor, zho), subtitles (server default,
      off, fra, eng, spa, deu, ita, por), subtitle scale (0.75–2)
- [ ] Network buffer (150/300/600/1000 MiB), hardware acceleration
- [ ] Home: next-up window (7/30/90/365 days, no limit), rewatching, show hero, continue watching, next up,
      one switch per library for "latest", hide spoilers
- [ ] Storage: image cache size and Clear
- [ ] About: server name and version, address, user, device, app version, sign out

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
