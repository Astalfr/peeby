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
| `innertube` | InnerTube client: Home, subscriptions, subscribed channels, channel videos and continuations (`browse`), watch page and comments (`next`), streams (VISIONOS `player` + HLS master), Return YouTube Dislike votes, cookies and SAPISIDHASH | No GPUI dependency. Fixture tests in `tests/fixtures/`, live check with `examples/smoke.rs` |
| `player` | libmpv backend: commands, events, frames | Copied from peeby ([ADR 0001](decisions/0001-peeby-foundations.md)), plus `external_audio`, `ytdl`, `log_file` |
| `gpui-video` | GPUI element showing the player's frames (zero-copy D3D11, software fallback) | Copied from peeby, needs `vendor/gpui-pre-windows` |
| `peebtube` | Binary: window, title bar, sidebar, pages (`feed` for Home/subscriptions/channel, `watch`), the player view (`player_view`, controls adapted from peeby's player page), icons (`assets`, Lucide) | `anyhow` for errors |
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

`PlayerView` owns the player, the `VideoView` and the controls. In fullscreen, `AppView` and `WatchView` render the player alone. A watch page covered by another page pauses its player (`set_covered`).

## Navigation

`AppView` owns the title bar (back button, "paste a URL or id" field), the sidebar (Home, Subscriptions, subscribed channels) and a stack of pages. Home and Subscriptions reset the stack; channel and watch pages are pushed. `peebtube <url-or-id>` opens a video at startup.

## Images

`images::ImageStore` (from peeby): GPUI global entity, in-memory LRU of decoded images (256 MB), disk cache in `%LOCALAPPDATA%\peebtube\cache\images`, 8 parallel downloads. The thumbnail size is picked from YouTube's `sources` for the card's physical width.

## Persistence

| Data | Location |
|---|---|
| Session cookies | `%APPDATA%\peebtube\cookies.txt` ([ADR 0004](decisions/0004-cookie-session.md)) |
| Image cache | `%LOCALAPPDATA%\peebtube\cache\images\` |
