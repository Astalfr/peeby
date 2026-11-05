<p align="center">
  <img src="docs/logo/peeb.webp" alt="Peeby" width="120">
</p>

<h1 align="center">Peeby</h1>

<p align="center">
  A native media client for Windows, written in Rust with <a href="https://www.gpui.rs/">GPUI</a>:<br>
  YouTube, Jellyfin and Twitch in one app, with playback through libmpv.
</p>

---

Peeby gives each service a **space** of its own: YouTube, Jellyfin and Twitch. A space can have
as many tabs as you open, and each tab keeps its own history. All spaces share the same player
(libmpv, hardware decoding). When you leave a page, its video keeps playing in a mini-player
window. Playback aims to start within 1–2 s of the click.

> **Status:** version 1.0. Download the installer from the
> [latest release](https://github.com/Astalfr/peeby/releases/latest); Peeby then updates itself
> (a green **Update** button in the title bar when a new version is ready).

## Features

**YouTube** (InnerTube API, signed in with your account's cookies)
- Home, subscriptions, search, playlists, channels, Shorts, history, watch later.
- Watch page with comments, chapters, captions and a like/dislike bar from
  [Return YouTube Dislike](https://returnyoutubedislike.com).
- Lives, seekable over their DVR window, with their chat.
- Titles in the video's own language (optional), notifications for new videos and lives.
- Streams come as HLS from the VISIONOS client, with a bundled yt-dlp as the fallback.

**Jellyfin** (12.0 or newer, one server, one account)
- Home with what to resume and what to watch next, libraries, a title's page, search.
- Direct Play with progress reports, skippable segments, the next episode, external ASS subtitles.

**Twitch** (works without signing in)
- Browse, categories, search, channel pages, lives, VODs and clips.
- Ads removed by a local playlist proxy, low latency, HEVC/AV1 above 1080p when offered.
- Live chat with Twitch, 7TV and BTTV emotes and badges. With the web `auth-token`, you can also
  write in the chat and use channel points: balance, rewards, predictions. Predictions, polls
  and outgoing raids are pinned over the chat, as on Twitch; a raid is followed to its channel.
- The followed channels in the sidebar and notifications when they go live (optional Helix
  connection).

**App**
- Interface in English or French.
- Windows notifications, a notification area icon, a single instance, optional start with Windows.

## Requirements

- **Windows 10 or 11**, x64. Peeby is Windows only.
- **Rust** through [rustup](https://rustup.rs). The toolchain (1.98.0, MSVC) is pinned in
  [rust-toolchain.toml](rust-toolchain.toml), and rustup installs it on the first build.
- **Visual Studio Build Tools** with the "Desktop development with C++" workload (the MSVC linker
  and the Windows SDK).
- **PowerShell 7** (`pwsh`) to run the fetch scripts.
- **7-Zip**, either in `PATH` or in `Program Files`: the libmpv archive is a `.7z`.
- **WebView2 Runtime** for the sign-in windows. Windows 11 includes it.

Nothing else needs installing on the machine: libmpv, yt-dlp and Deno are downloaded into
`third_party/` (git-ignored). The scripts pin each one with its SHA-256.

## Install

Run `peeby-setup-<version>-x64.exe` from the [latest release](https://github.com/Astalfr/peeby/releases/latest).
It installs Peeby for your Windows user (`%LOCALAPPDATA%\Programs\Peeby`, no admin rights) with
libmpv, yt-dlp and Deno. The installer is not signed, so Windows SmartScreen may warn on the first
install ("More info" → "Run anyway").

Installed copies check GitHub for a new release shortly after they start and then every few
hours. When one is downloaded and verified, a green **Update** button shows in the title bar: a
click closes Peeby, installs the update and starts it again ([ADR 0027](docs/decisions/0027-installer-and-updates.md)).
Releases are made with [docs/release.md](docs/release.md).

## Build from source

```powershell
git clone https://github.com/Astalfr/peeby.git
cd peeby

# libmpv: required to play anything
pwsh scripts/fetch-mpv.ps1

# yt-dlp + Deno: YouTube's fallback extraction
pwsh scripts/fetch-tools.ps1

# Build and run (the release build is the one to use day to day)
cargo run -p peeby --release
```

The release executable is `target\release\peeby.exe`. It finds `libmpv-2.dll` next to itself,
or else in `third_party/mpv/`.

> If `fetch-mpv.ps1` fails with a 404, the pinned libmpv release is gone (shinchiro only keeps
> recent ones). Bump the version, URL and SHA-256 at the top of the script.

## First start

- **YouTube:** sign in from the space's account entry. A window opens Google's own sign-in page,
  and Peeby keeps the session cookies. You can also import a `cookies.txt` exported from your
  browser.
- **Jellyfin:** enter your server's address, user name and password in the space.
- **Twitch:** works without signing in. To write in chats, use channel points and get resume
  positions, add the web `auth-token`: sign in through the window, or follow the tutorial in the
  settings. To list your followed channels, also connect Helix (a device code to approve on
  twitch.tv).

You can also run the sign-in window on its own:
`cargo run -p peeby -- --login youtube|twitch > session.txt`. It writes the session to stdout, so
redirect it to a file and never into a log.

## Usage

```powershell
peeby                                   # start
peeby <youtube-url-or-id>…              # open these videos (the last one on top)
peeby https://www.twitch.tv/<login>     # open a live, a VOD (twitch.tv/videos/<id>) or a clip
peeby --background                      # start hidden in the notification area
```

- **Single instance:** while Peeby runs, a new launch hands it its arguments and exits. Closing
  the window only hides it in the notification area; quit from the tray icon.
- **Debug and release builds are separate profiles:** a debug build (`peeby-dev`) has its own
  settings, cookies, credentials and instance. It runs next to the release build and signs in on
  its own.
- `PEEBY_DISPLAY=secondary` (or `=2`) opens the window on another screen.

## Development

| Task | Command |
| --- | --- |
| Build | `cargo build` |
| Run (debug) | `cargo run -p peeby` |
| Lint (must pass) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format | `cargo fmt --all` (2-space indent) |
| Tests | `cargo test --workspace` |
| One test | `cargo test -p <crate> <test_name>` |

Debug builds, examples and spikes are console apps. To keep a console window from opening on
your screen, start them with `Start-Process -NoNewWindow` and redirect their output to files, or
use `cargo run` in the current shell.

### Tools and examples

| What | Command |
| --- | --- |
| YouTube Home feed in the terminal | `cargo run -p innertube --example home -- [cookies.txt] [subscriptions]` |
| Live check of every InnerTube endpoint (counts only) | `cargo run -p innertube --example smoke -- [cookies.txt]` |
| Plain copy of the cookie jar for those examples | `cargo run -p peeby -- --export-cookies > copy.txt` |
| YouTube playback spike, with timings | `cargo run -p spike-yt-play --release -- <url>` (`PEEBY_RESOLVER=ytdl` for yt-dlp) |
| Twitch live check (counts only) | `cargo run -p twitch --example smoke -- [login]` |
| Twitch playback spike | `cargo run -p spike-twitch-play --release -- [login \| twitch.tv/videos/<id> \| vod:<login>]` |
| Twitch fixtures capture | `cargo run -p peeby --example twitch_capture` (see [CLAUDE.md](CLAUDE.md) for its modes) |
| Chat panel alone, with frame timings | `cargo run -p ui --example chat --release` (`PEEBY_RATE=50`) |
| Fake Twitch chat with every chat feature (debug builds) | `cargo run -p peeby -- --fake-chat`, or "Fake chat" in the Twitch sidebar |
| Jellyfin live tests | set `PEEBY_TEST_SERVER`, `PEEBY_TEST_USER`, `PEEBY_TEST_PASSWORD`, then `cargo test -p jellyfin-api --test live -- --ignored --nocapture` |

Keep the exported cookie copy (`copy.txt`) private: never commit it or log it. The examples
write the rotated cookies back to it.

### Workspace

```
crates/
  peeby/         the app: shell, spaces (youtube, jellyfin, twitch), settings, desktop integration
  ui/            shared UI: theme, components, images, chat panel, player, i18n (knows no service)
  player/        libmpv bindings, loaded at runtime
  gpui-video/    the video surface (D3D11 textures in GPUI)
  innertube/     YouTube's InnerTube client
  jellyfin-api/  Jellyfin client
  twitch/        Twitch: GQL, Helix, usher, ad-removing proxy, chat
spikes/          playback experiments with timings
vendor/          patched GPUI Windows platform (see vendor/README.md)
scripts/         fetch-mpv.ps1, fetch-tools.ps1
docs/            design, architecture, decisions (ADRs), roadmap
```

Contributor rules (code style, secrets, logging, the InnerTube and Twitch conventions) are in
[CLAUDE.md](CLAUDE.md).

## Where Peeby keeps its data

| What | Where |
| --- | --- |
| Settings | `%APPDATA%\peeby\settings.toml` |
| YouTube cookies (encrypted with DPAPI) | `%APPDATA%\peeby\cookies.dat` |
| Jellyfin and Twitch tokens | Windows Credential Manager |
| Image cache | `%LOCALAPPDATA%\peeby\images\` |
| Logs (kept 7 days; mpv logs are opt-in) | `%LOCALAPPDATA%\peeby\logs\` |

Cookies and tokens never go to logs, URLs or error messages. Stream URLs (which carry your IP)
are logged at debug level only.

> Playing a video while signed in writes to your accounts: YouTube history and resume positions,
> Jellyfin play reports, Twitch VOD positions.

## Documentation

Start with [docs/README.md](docs/README.md), which lists the goals, the scope decisions and an
index. Then:
- [docs/architecture.md](docs/architecture.md): crates, threads, data flow.
- [docs/roadmap.md](docs/roadmap.md): milestones and status.
- [docs/twitch.md](docs/twitch.md): how the Twitch space works.
- [docs/decisions/](docs/decisions/): architecture decision records.

## Credits

- Dislike counts from [Return YouTube Dislike](https://returnyoutubedislike.com).
- Playback by [mpv](https://mpv.io) (Windows builds by
  [shinchiro](https://github.com/shinchiro/mpv-winbuild-cmake)), with a fallback through
  [yt-dlp](https://github.com/yt-dlp/yt-dlp) and [Deno](https://deno.com).
- UI built with [GPUI](https://www.gpui.rs/) and
  [gpui-component](https://github.com/longbridge/gpui-component).

Peeby is not affiliated with YouTube, Google, Jellyfin or Twitch.

## License

[GPL-3.0-or-later](LICENSE).
