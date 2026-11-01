# Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                     peebtube (GPUI app)                      │
│        Home · Watch (player + related) · …                   │
│        │                         │                           │
│        ▼                         ▼                           │
│  ┌──────────────┐        ┌──────────────────┐                │
│  │  innertube   │        │  player (libmpv) │──► gpui-video  │
│  │ browse/next/ │        │  HLS, d3d11va    │    element     │
│  │ player, HLS  │        │  audio-files     │                │
│  └──────┬───────┘        └────────┬─────────┘                │
└─────────┼─────────────────────────┼──────────────────────────┘
          │ InnerTube JSON          │ HLS playlists + segments
          ▼                         ▼
     www.youtube.com          *.googlevideo.com
```

## Crates

| Crate | Role | Notes |
|---|---|---|
| `innertube` | InnerTube client: Home, subscriptions, subscribed channels, channel videos, the user's playlists and a playlist's videos, and their continuations (`browse`), watch page and comments (`next`), streams (VISIONOS `player` + HLS master), Return YouTube Dislike votes, cookies and SAPISIDHASH | No GPUI dependency. Fixture tests in `tests/fixtures/`, live check with `examples/smoke.rs` |
| `player` | libmpv backend: commands, events, frames | Copied from peeby ([ADR 0001](decisions/0001-peeby-foundations.md)), plus `external_audio`, `ytdl`, `log_file` |
| `gpui-video` | GPUI element showing the player's frames (zero-copy D3D11, software fallback) | Copied from peeby, needs `vendor/gpui-pre-windows` |
| `peebtube` | Binary: window, title bar, sidebar, pages (`feed` for Home/subscriptions/channel, `channels`, `playlists`, `watch`, `account`, `settings_page`), the play queue of a playlist (`queue`), saving a video to a playlist (`save`), settings (`settings`), bundled tools and their updates (`tools`), the player view (`player_view`, controls adapted from peeby's player page), icons (`assets`, Lucide) | `anyhow` for errors |
| `spikes/yt-play` | Playback spike: VISIONOS or yt-dlp, timings | Kept for measurements |

## Threads and data flow

- **UI thread (GPUI).** Rendering and input only.
- **Network.** `reqwest` needs tokio, GPUI has its own executor. `peebtube::runtime` (from peeby) owns a 2-thread tokio runtime as a GPUI global; `runtime::spawn(cx, future)` returns a future that GPUI tasks await.
- **Player.** `Player::start` runs on GPUI's background executor (libmpv and D3D11 setup block briefly). mpv events and frames come back through channels (peeby's design).

## Startup

1. `settings::load_cookies()` reads `%APPDATA%\peebtube\cookies.txt` (or `PEEBTUBE_COOKIES`) before the UI starts.
2. In the background: `Client::refresh_client_version()` reads the current WEB `clientVersion` and a `visitorData` from the page's ytcfg.
3. The Home page loads `FEwhat_to_watch`. Its response also carries a `visitorData`, kept for the stream resolver.

## Watch page

The stream (`Client::stream`, ~0.2 s), the page metadata (`Client::watch`) and the RYD votes load in parallel; the first comments page follows the metadata. RYD votes are cached for the session (100 requests/min, 10,000/day). mpv starts as soon as the stream is known: video media playlist as the file, audio rendition as `audio-files`, the VISIONOS User-Agent as an HTTP header. Measured: playback about 1.5 s after the page opens, player setup included.

`PlayerView` owns the player, the `VideoView` and the controls, plus the stream's quality ladder and subtitle tracks. A quality change starts a new mpv on the other variant at the current position (the old one is paused, then dropped when the new picture arrives). Subtitle tracks are given to mpv with `sub-add` the first time they are picked; mpv numbers them 1, 2… in that order.

`prefetch` resolves the stream of a card the mouse rests on (300 ms), and the watch page takes it from there. `prefetch::warm` resolves a stream at once (the next video of a queue). In fullscreen, `AppView` and `WatchView` render the player alone. A watch page covered by another page pauses its player (`set_covered`).

## Navigation

`AppView` owns the title bar (back button, "paste a URL or id" field), the sidebar (Home, Subscriptions, Playlists, then Account and Settings) and a stack of pages. Sidebar pages reset the stack; the channels, channel, playlist and watch pages are pushed. `peebtube <url-or-id>` opens a video at startup.

## Playlists

`Client::playlists` reads the library (`FEplaylist_aggregation`, all playlists in one response, "Watch later" and "Liked videos" included), `Client::playlist` a playlist's page (`VL<id>`, header and the first 100 videos) and `Client::playlist_more` the next 100. "Watch later" lists `playlistVideoRenderer`s, the others video lockups; the next page's token is the `continuationItemViewModel` of the list holding the videos (the page has another one beside it).

Playing a playlist creates a `queue::Queue` entity: the loaded videos, the play order (shuffled or not), repeat, and the token of the next videos, which it loads when fewer than 5 are left to play. `AppView::play_queue` pushes a watch page for its current video. That page shows the queue, subscribes to its player (`PlayerEvent::Ended`, `Next`, `Previous`) and to the queue (`QueueEvent::Play`). When the video ends, the queue picks the next one and `play_queue(…, replace: true)` swaps the watch page on top for a new one sharing the queue. Replacing rather than pushing keeps a single player alive for a whole playlist, drops the previous page's pending requests with it, and keeps "back" leading to the playlist. The new page takes the focus, so fullscreen and its keys carry on. Each page warms the next video's stream (`prefetch::warm`), so the next video starts in about 1.3 s.

Saving a video: `Client::playlist_choices` (`playlist/get_add_to_playlist`) lists the playlists and whether each holds the video; `Client::add_to_playlist` and `Client::remove_from_playlist` post `browse/edit_playlist` with the action that dialog's own endpoints carry (`ACTION_ADD_VIDEO`, `ACTION_REMOVE_VIDEO_BY_VIDEO_ID`). No `edit_playlist` response is saved yet: an HTTP success counts as done unless a `status` other than `STATUS_SUCCEEDED` comes back. `save::Session` (a global set by `AppView`) gives the cards the current client; every change ends with a gpui-component notification (toast) in the window it started from.

Shuffling uses a small xorshift generator (no dependency) and mixes the videos loaded so far: a long playlist's next pages are inserted at random places after the current video as they arrive.

## Session

`AppView::set_session` replaces the app's `Client` after a cookie import or a sign-out and rebuilds Home. The image store and the stream prefetch use an anonymous client of their own, so they never need replacing. Pages that need a session (Home signed out, subscriptions, an expired session) offer an "Importer des cookies" button that opens the account page.

## Images

`images::ImageStore` (from peeby): GPUI global entity, in-memory LRU of decoded images (256 MB), disk cache in `%LOCALAPPDATA%\peebtube\cache\images`, 8 parallel downloads. The thumbnail size is picked from YouTube's `sources` for the card's physical width.

## Persistence

| Data | Location |
|---|---|
| Settings (language, region, default quality, volume) | `%APPDATA%\peebtube\settings.toml`, `settings::Settings` global |
| Session cookies | `%APPDATA%\peebtube\cookies.txt` ([ADR 0004](decisions/0004-cookie-session.md)) |
| Image cache | `%LOCALAPPDATA%\peebtube\cache\images\` |
