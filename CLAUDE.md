# Peeby

Native media client for Windows, written in Rust with GPUI: one "space" per service (YouTube, Jellyfin and Twitch), each
with as many tabs as the user opens (ADR 0015). Twitch: docs/twitch.md, ADR 0010, docs/twitch-plan.md. Playback through libmpv, YouTube streams from the InnerTube API, Jellyfin's by Direct Play.
Project docs: docs/README.md (start there). Decisions: docs/decisions/. Progress: docs/roadmap.md.
The playback stack (`player`, `gpui-video`, `vendor/gpui-pre-windows`), `jellyfin-api` and the Jellyfin pages came from
the former Jellyfin-only peeby (`D:\DEV\peeby`, abandoned, ADR 0007); import nothing else from there.

## Commands
- Build: `cargo build` · run: `cargo run -p peeby` (optionally `-- <youtube-url-or-id>…` to open videos, the last on top);
  `PEEBY_DISPLAY=secondary` (or `=2`) opens the window on another screen: use it for test runs
- Test runs never open a console window on the user's screen: debug builds, examples and spikes are console
  apps, so start them with `Start-Process -NoNewWindow` (redirect their output to files), or `cargo run` in
  the current shell; helper scripts follow the same rule
- Lint (must pass): `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all` (2-space indent, see rustfmt.toml)
- Tests: `cargo test --workspace` · single test: `cargo test -p <crate> <test_name>`
- Home feed from the terminal: `cargo run -p innertube --example home -- [cookies.txt] [subscriptions]`
  (give it a copy of the cookies while Peeby runs: both write the rotated cookies back)
- Jellyfin live tests (ignored by default): set `$env:PEEBY_TEST_SERVER`, `PEEBY_TEST_USER`, `PEEBY_TEST_PASSWORD`,
  then `cargo test -p jellyfin-api --test live -- --ignored --nocapture`
- Live check of every endpoint (counts only): `cargo run -p innertube --example smoke -- [cookies.txt]`
  (PowerShell: `"$env:APPDATA\peeby\cookies.txt"`; `%APPDATA%` only works in cmd)
- libmpv: `pwsh scripts/fetch-mpv.ps1` fills third_party/mpv/ (git-ignored, required to run the player)
- yt-dlp + Deno: `pwsh scripts/fetch-tools.ps1` fills third_party/yt-dlp/ (git-ignored, for the fallback and the spike).
  Single-file `yt-dlp.exe` on purpose: the app updates it with `yt-dlp -U`, which "onedir" builds cannot do.
- Playback spike with timings: `cargo run -p spike-yt-play --release -- <url>` (`PEEBY_RESOLVER=ytdl` for yt-dlp)
- Twitch live check (counts only): `cargo run -p twitch --example smoke -- [login]`
- Twitch Helix fixtures: `cargo run -p peeby --example twitch_capture` (the user's session from the Credential
  Manager; spends the refresh token and saves its successor: never while Peeby runs; replaces the account's data; `-- badges` saves only the chat badges; `-- master <login>` prints a live's
  variants with every codec and the auth-token and saves its master redacted, without spending the refresh token;
  `-- master-scan` lists the top lives offering HEVC/AV1; `-- positions <vod-id>…` saves the account's resume
  positions in those VODs, read only, without spending the refresh token)
- Chat panel alone, generated messages and frame timings: `cargo run -p ui --example chat --release` (`PEEBY_RATE`)
- Sign-in window alone (ADR 0017): `cargo run -p peeby -- --login youtube|twitch` writes the session (a
  cookie jar, the auth-token) on stdout: redirect it to a file, never into a log or a shared terminal
- Twitch playback spike: `cargo run -p spike-twitch-play --release -- [login | twitch.tv/videos/<id> | vod:<login>]`
  (`PEEBY_PLAYER_TYPE`, `PEEBY_CODECS`, `PEEBY_VARIANT`, `PEEBY_USHER=v1`, `PEEBY_PROXY=1`, `PEEBY_LOW_LATENCY=0`,
  `PEEBY_FIXTURES=1`, `PEEBY_QUIT_AFTER=<s>`; see its main.rs; it prints `hwdec-current` and, through the proxy, the
  start delay)
- Shell is Windows: PowerShell or Git Bash, MSVC toolchain pinned in rust-toolchain.toml.

## Product constraints (no exceptions)
- Playback starts within about 1–2 s of the click. Streams come from the VISIONOS client as HLS (ADR 0003).
  Never hand DASH `https` URLs to mpv before their `available_at` (403 until then); the yt-dlp fallback forces HLS.
- With cookies, an InnerTube response with `logged_in=0` is an error (`Error::NotLoggedIn`), never an empty feed.
- Cookies are secrets: never in logs, URLs or error messages. They live in `%APPDATA%\peeby\cookies.txt`
  (a whole cookie jar does not fit the Windows Credential Manager), see ADR 0004.
- YouTube rotates session cookies through `Set-Cookie`: every signed-in request goes through `Client::send` /
  `keep_rotated_cookies`, which writes the jar back. Ignoring them invalidates the session within about an hour.
  Never give the user's file to yt-dlp (it rewrites it): give it a copy.
- Stream URLs embed the user's IP and signatures: never log them at info level or above; the mpv log file is opt-in
  (`PEEBY_MPV_LOG`). Fixtures are redacted (IP 203.0.113.7, signatures `REDACTED`) before they are committed.
- Nothing installed on the host: libmpv, yt-dlp and Deno ship with the app. yt-dlp always runs with `--ignore-config`,
  `--cache-dir` in the app's folder and an explicit `--js-runtimes`; versions are pinned with SHA-256 in fetch-tools.ps1.
- Return YouTube Dislike: attribution is required (link to returnyoutubedislike.com), at most 100 requests/min and
  10,000/day per client, so cache results.
- Scope: Windows only, one YouTube account.

## Rust conventions
- Indent with 2 spaces in every file type (.editorconfig, rustfmt.toml).
- No `unwrap()`/`expect()` outside tests unless the invariant is explained in a comment next to it.
- Errors: `thiserror` in library crates, `anyhow` in the `peeby` binary.
- `unsafe` only in FFI modules (`player::sys`, `player::d3d`, `gpui-video`), each block with a `// SAFETY:` comment.
- Every `#[allow(...)]` carries a comment saying why.
- Log with `tracing`. Cookies never appear in logs; stream URLs only at debug level, never in error messages.
- Nothing blocking on the GPUI UI thread: network and disk I/O run in the background.
- Code, identifiers, comments and docs in English. UI strings in English and French, both at the call site:
  `ui::i18n::t("Settings", "Réglages")`, `ui::tf!("{n} videos", "{n} vidéos")` (ADR 0014). Logs stay English;
  tests never call `i18n::set` and expect English.

## InnerTube
- Source of truth for client contexts (VISIONOS values, client name ids, headers): the yt-dlp source at the pinned
  version (`yt_dlp/extractor/youtube/_base.py`), not memory.
- Parse leniently from `serde_json::Value`: a malformed item is skipped, never fails the whole feed.
- Every parser is tested against a real response in `crates/innertube/tests/fixtures/`. A new endpoint gets a saved,
  redacted response before its parser is written. Signed-in responses are trimmed to a few items and stripped of
  account data (`mainAppWebResponseContext.datasyncId`, visitorData, top bar) before they are committed.
- Grids and related videos use `lockupViewModel`: with two or more `metadataRows`, row 0 is the channel and the next
  ones views · date; a channel's own tab has a single row.
- The WEB `clientVersion` is one constant (`WEB_CLIENT_VERSION`), refreshed at startup from the page's ytcfg.
- VISIONOS needs a session `visitorData` (else `LOGIN_REQUIRED`); refresh it once and retry before failing.

## Spaces
- A service is a space under `crates/peeby/src/spaces/<name>/` implementing `shell::space::Space`; the shell and the
  `ui` crate know no service. Its settings are its part of the gear's page (`Space::settings`); player settings shared
  by every space are `[player]` (`settings::Player`). Pages keep a `Nav` (their space and their tab, ADR 0015) to
  navigate: what they open goes to their tab. A tab opened in the background builds nothing until it shows (a `Page` has no
  view until then, ADR 0019): never build a page that plays in a tab nobody looks at.
- The mini-player is a second window (ADR 0008): never update a window from inside its own event (defer), and add
  window mouse listeners only while painting (a `canvas` paint callback), never from `render`.
- Jellyfin (ADR 0009): the token lives in the Credential Manager (blocking calls: before the UI or in the background),
  never in a URL or a log; its images go through `images::route` with the `Authorization` header. Playing anything
  writes to the user's account (reports: last played date, play count, resume position, watched flag): ask the
  user before a playback test.
- Watch progress lives on the account (ADR 0013): playing a YouTube video signed in writes its history and resume
  position (stats pings), a Twitch VOD with the auth-token its `viewingHistory`. Ask the user before a playback test.

## Twitch (docs/twitch-plan.md, ADR 0010)
- The space needs no sign-in: its catalog is anonymous GQL (`twitch::catalog`, ADR 0016). The web `auth-token`
  (`twitch|auth-token`) is the sign-in; Helix is optional, for the followed channels (GQL refuses them) and as
  the catalog's backup: its Device Code Grant uses Peeby's public Client-ID (`twitch::auth::CLIENT_ID`). Refresh
  tokens are single use: only `auth::Session` refreshes, and it stores the new one (Credential Manager,
  `twitch|refresh`) before using it. Approving a code needs
  the user: ask before a sign-in test.
- mpv never gets a usher or CDN playlist URL: live and VOD playlists go through `twitch::Proxy`
  (`Client::play`), which removes ads and fills breaks from the backup `playerType`. Main token
  `embed`, never `site` (pre-roll).
- Playback tokens (`value` holds the user's IP), signatures, and usher's error bodies (they repeat the
  signed URL) never reach a log or an error message: keep only `error_code`.
- Chat: emote ranges count code points (`char`s), never byte or UTF-16 offsets. Chat fixtures are public
  channels' lines with every chatter renamed (`viewerN`, as long as the name it replaces, so that ranges
  still fit), mentions and quoted replies included.
- Channel points (ADR 0012): every spend (reward, bet) waits for a second click and never happens on its
  own. `claimCommunityPoints` fails Twitch's integrity check: never route around it (another Client-ID,
  minted integrity tokens); the bonus is claimed on twitch.tv. `minute-watched` (spade) is sent only for
  the live on screen while it plays, never from a script or for a live nobody watches.
- Fixtures: redact Twitch's session keys by name (`USER-IP`, `SERVING-ID`, `VIDEO-SESSION-ID`, `NODE`,
  `C`/`E`…, and the ads' `*-SESSION-ID`, `RADS-TOKEN`, `*-URL`), never with a blanket IP regex (it breaks
  HEVC codec strings).

## Player
- `ui::player` (crates/ui/src/player/) is the shared player, built from peeby's `player_page.rs`: take controls and
  behavior from there first, keeping service data out of it (spaces give it a `Source`; peeby is at
  `D:\DEV\peeby\crates\peeby\src\player_page.rs`); Jellyfin specifics (reports, track mapping) go to its space.

## Ask before
- Adding, removing or upgrading a dependency, changing the GPUI revision, the libmpv build or the yt-dlp/Deno pins.
- Touching rust-toolchain.toml, CI or .claude/ hooks.
- Introducing `unsafe` outside the FFI modules.
- Changing the copied peeby crates (`player`, `gpui-video`, `vendor/`) beyond small additions: note them in ADR 0001.

## Documentation
- A change in behavior, architecture or decision updates docs/ in the same change.
- A structural decision gets a new ADR in docs/decisions/ (copy 0000-template.md).
- Update docs/roadmap.md when a step is finished.

## Gotchas
- GPUI is `gpui-pre =0.3.10` imported as `gpui` (peeby ADR 0006): docs.rs pages for the `gpui` crate are a different
  version, so read the source under `~/.cargo/registry/src/*/gpui-pre-0.3.10/` instead. gpui-component traits such as
  `Sizable` and `Disableable` must be imported; `InputEvent` lives in `gpui-base-0.8.1`.
- libmpv is never linked: it is loaded at runtime with `libloading` (peeby ADR 0007).
- mpv list options go through `change-list … append` (`http-header-fields`, `audio-files`, `ytdl-raw-options`,
  `script-opts`): a comma in a value would split it otherwise.
- YouTube HLS variants are video only: pass the `AUDIO` group's rendition as `audio-files`, with the VISIONOS User-Agent.
- A YouTube live's playlists are windows whose first segment is mpv's 0: start it with a positive `start` near the
  window's end (`Live::length − LIVE_DELAY`); 0 or a negative `start` stalls about 10 s on the audio rendition (ADR 0011).
- Caption tracks: `baseUrl&fmt=vtt` works; machine translations (`&tlang=`) answer HTTP 429 for VISIONOS sessions.
- Per-frame video images: each `RenderImage` is a new atlas entry, so release the previous one with `Window::drop_image`.
- vendor/gpui-pre-windows is a patched copy (external D3D11 textures, see vendor/README.md). Never edit it without
  regenerating vendor/gpui-pre-windows.patch, and re-apply the patch whenever GPUI is bumped.
- `line_clamp` limits wrapped lines only: cut explicit line breaks before clamping (descriptions).
- Icons that gpui-component widgets draw themselves (spinner: `loader`, toasts: `info`, `circle-check`…) must be in
  `COMPONENT` in `crates/ui/src/assets.rs`: a missing SVG renders as nothing, without an error.
- A window opened at given bounds needs `display_id` set to the display those bounds are on: Windows checks them
  against that display (the primary one when unset) and otherwise falls back to a default size and place.
- shinchiro deletes old libmpv releases: when fetch-mpv.ps1 gets a 404, bump the pin (version, URL, SHA-256).
