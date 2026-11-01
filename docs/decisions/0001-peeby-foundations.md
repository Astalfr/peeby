# 0001: Foundations inherited from peeby

- Status: accepted
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
- `StartOptions::log_file`.

The app's `player_view.rs` adapts peeby's `player_page.rs` (seek bar, volume slider, control buttons, auto-hide, OSD, shortcuts) without its Jellyfin parts.

## Consequences

- Playback works from day one: 1080p VP9/AV1 with hardware decoding, about 0.2 core of CPU.
- A fix to the copied crates or to the GPUI patch has to be carried to peeby by hand, and the other way around. Moving them to a shared repository is the way out once both projects stabilize.
- Upgrading GPUI follows peeby's procedure ([vendor/README.md](../../vendor/README.md)): bump together with `gpui-component`, re-apply the patch, re-run a video spike.
