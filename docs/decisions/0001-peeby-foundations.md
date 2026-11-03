# 0001: Foundations inherited from peeby

- Status: accepted; the "copies" consequence is superseded by [0007](0007-multi-space-app.md)
- Date: 2026-11-01

## Context

Peeby (`D:\DEV\peeby`, a Jellyfin client by the same author) already solved video playback in GPUI on Windows, with measurements. Its decisions and their reasons apply unchanged here:

| Topic | Peeby decision | Peeby ADR |
|---|---|---|
| Playback engine | libmpv (FFmpeg, d3d11va, A/V sync, WASAPI) | 0001 |
| Platform | Windows only, MSVC, GPUI's DirectX 11 backend | 0003 |
| License | GPL-3.0-or-later (libmpv builds are GPL) | 0004 |
| GPUI | `gpui-pre =0.3.10`, the snapshot `gpui-component` 0.8.1 pins | 0006 |
| libmpv bindings | Hand-written FFI, DLL loaded at runtime with `libloading` | 0007 |
| Video surface | Zero-copy D3D11 shared textures drawn by a patched `gpui-pre-windows`, software fallback | 0008 |

## Decision

Reuse them as they are. The crates `player` and `gpui-video`, `vendor/gpui-pre-windows` (with its patch) and `scripts/fetch-mpv.ps1` are **copies** of peeby's (2026-11-01), not a shared dependency, for now.

PeebTube additions to `player`:
- `StartOptions::external_audio` (`audio-files`, for HLS audio renditions);
- `StartOptions::ytdl` (mpv's `ytdl_hook`, for the yt-dlp fallback);
- `StartOptions::log_file`;
- `StartOptions::readahead_secs` (`cache-secs`: YouTube refuses segments fetched in a burst; Twitch VODs would be fetched whole);
- `Player::add_subtitle` (`sub-add`, captions fetched on demand);
- `Player::property`, read-only, for the player's statistics panel;
- `Player::set_speed` (`speed`), for the 2× speed while Space or the left button is held;
- `Event::Cached` (`demuxer-cache-time`, observed), for the loaded part drawn on the seek bar;
- `StartOptions::audio_device`, `Player::set_audio_device` and `audio_devices` (an mpv without render context, `sys::RenderBackend::None`), for the top bar's audio output menu;
- `play_wav` (`sys::play_wav`: Windows' `PlaySoundW` from winmm.dll, loaded at runtime like libmpv), for the opening animation's sound;
- the GPU ring is recreated at once after a quiet spell, else once a resize settles (60 ms), and the replaced ring lives until the next one (no black video while the mini-player is resized, ADR 0008).

PeebTube additions to `gpui-video`:
- `VideoView::set_corner_radii` (rounded picture on the watch page: a parent's `overflow_hidden` clips to a rectangle, not to its corners);
- `VideoView` releases replaced frames from every window (`App::drop_image`), since the mini-player draws the same view in a second window;
- re-exports `set_window_bounds` from the patched GPUI.

PeebTube additions to the GPUI patch (`vendor/gpui-pre-windows`):
- `set_window_bounds` (`window.rs`): moves and resizes a window without activating it, for the mini-player's edges (ADR 0008).

Additions to `jellyfin-api` (copied at step 5 of the refactor, ADR 0009):
- live tests read their credentials from the environment (no `dotenvy`);
- `jellyfin_api::Url` re-exports `url::Url`, the type of the client's URLs, so the app needs no `url` dependency.

The app's `player_view.rs` adapts peeby's `player_page.rs` (seek bar, volume slider, control buttons, auto-hide, OSD, shortcuts) without its Jellyfin parts.

## Consequences

- Playback works from day one: 1080p VP9/AV1 with hardware decoding, about 0.2 core of CPU.
- A fix to the copied crates or to the GPUI patch has to be carried to peeby by hand, and the other way around. Moving them to a shared repository is the way out once both projects stabilize.
- Upgrading GPUI follows peeby's procedure ([vendor/README.md](../../vendor/README.md)): bump together with `gpui-component`, re-apply the patch, re-run a video spike.
