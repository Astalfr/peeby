# PeebTube

Native YouTube client for Windows, written in Rust with GPUI. Playback through libmpv, streams from YouTube's InnerTube API.
Project docs: docs/README.md (start there). Decisions: docs/decisions/. Progress: docs/roadmap.md.
The playback stack (`player`, `gpui-video`, `vendor/gpui-pre-windows`) is copied from peeby (`D:\DEV\peeby`), see ADR 0001.

## Commands
- Build: `cargo build` · run: `cargo run -p peebtube` (optionally `-- <youtube-url-or-id>…` to open videos, the last on top)
- Lint (must pass): `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all` (2-space indent, see rustfmt.toml)
- Tests: `cargo test --workspace` · single test: `cargo test -p <crate> <test_name>`
- Home feed from the terminal: `cargo run -p innertube --example home -- [cookies.txt]`
- Live check of every endpoint (counts only): `cargo run -p innertube --example smoke -- [cookies.txt]`
  (PowerShell: `"$env:APPDATA\peebtube\cookies.txt"`; `%APPDATA%` only works in cmd)
- libmpv: `pwsh scripts/fetch-mpv.ps1` fills third_party/mpv/ (git-ignored, required to run the player)
- yt-dlp + Deno: `pwsh scripts/fetch-tools.ps1` fills third_party/yt-dlp/ (git-ignored, for the fallback and the spike).
  Single-file `yt-dlp.exe` on purpose: the app updates it with `yt-dlp -U`, which "onedir" builds cannot do.
- Playback spike with timings: `cargo run -p spike-yt-play --release -- <url>` (`PEEBTUBE_RESOLVER=ytdl` for yt-dlp)
- Shell is Windows: PowerShell or Git Bash, MSVC toolchain pinned in rust-toolchain.toml.

## Product constraints (no exceptions)
- Playback starts within about 1–2 s of the click. Streams come from the VISIONOS client as HLS (ADR 0003).
  Never hand DASH `https` URLs to mpv before their `available_at` (403 until then); the yt-dlp fallback forces HLS.
- With cookies, an InnerTube response with `logged_in=0` is an error (`Error::NotLoggedIn`), never an empty feed.
- Cookies are secrets: never in logs, URLs or error messages. They live in `%APPDATA%\peebtube\cookies.txt`
  (a whole cookie jar does not fit the Windows Credential Manager), see ADR 0004.
- YouTube rotates session cookies through `Set-Cookie`: every signed-in request goes through `Client::send` /
  `keep_rotated_cookies`, which writes the jar back. Ignoring them invalidates the session within about an hour.
  Never give the user's file to yt-dlp (it rewrites it): give it a copy.
- Stream URLs embed the user's IP and signatures: never log them at info level or above; the mpv log file is opt-in
  (`PEEBTUBE_MPV_LOG`). Fixtures are redacted (IP 203.0.113.7, signatures `REDACTED`) before they are committed.
- Nothing installed on the host: libmpv, yt-dlp and Deno ship with the app. yt-dlp always runs with `--ignore-config`,
  `--cache-dir` in the app's folder and an explicit `--js-runtimes`; versions are pinned with SHA-256 in fetch-tools.ps1.
- Return YouTube Dislike: attribution is required (link to returnyoutubedislike.com), at most 100 requests/min and
  10,000/day per client, so cache results.
- Scope: Windows only, one YouTube account.

## Rust conventions
- Indent with 2 spaces in every file type (.editorconfig, rustfmt.toml).
- No `unwrap()`/`expect()` outside tests unless the invariant is explained in a comment next to it.
- Errors: `thiserror` in library crates, `anyhow` in the `peebtube` binary.
- `unsafe` only in FFI modules (`player::sys`, `player::d3d`, `gpui-video`), each block with a `// SAFETY:` comment.
- Every `#[allow(...)]` carries a comment saying why.
- Log with `tracing`. Cookies never appear in logs; stream URLs only at debug level, never in error messages.
- Nothing blocking on the GPUI UI thread: network and disk I/O run in the background.
- Code, identifiers, comments and docs in English. UI strings in French.

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

## Player
- `player_view.rs` adapts peeby's `player_page.rs`: take controls and behavior from there first (peeby is at
  `D:\DEV\peeby\crates\peeby\src\player_page.rs`), leaving out its Jellyfin parts.

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
- Caption tracks: `baseUrl&fmt=vtt` works; machine translations (`&tlang=`) answer HTTP 429 for VISIONOS sessions.
- Per-frame video images: each `RenderImage` is a new atlas entry, so release the previous one with `Window::drop_image`.
- vendor/gpui-pre-windows is a patched copy (external D3D11 textures, see vendor/README.md). Never edit it without
  regenerating vendor/gpui-pre-windows.patch, and re-apply the patch whenever GPUI is bumped.
- `line_clamp` limits wrapped lines only: cut explicit line breaks before clamping (descriptions).
- Icons that gpui-component widgets draw themselves (spinner: `loader`, toasts: `info`, `circle-check`…) must be in
  `COMPONENT` in `assets.rs`: a missing SVG renders as nothing, without an error.
- shinchiro deletes old libmpv releases: when fetch-mpv.ps1 gets a 404, bump the pin (version, URL, SHA-256).
