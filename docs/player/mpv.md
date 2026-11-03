# libmpv

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

libmpv does the demuxing (FFmpeg), hardware decoding, network caching, A/V sync, audio output and subtitle rendering (libass). Peeby drives it through the client API (`client.h`) and draws its frames with the render API (`render.h`).

## Bindings

The bindings are hand-written and loaded at runtime with `libloading` ([ADR 0007](../decisions/peeby/0007-libmpv-ffi-libloading.md)), because `libmpv2` only wraps the OpenGL render API. The FFI lives in `player::sys`, and the rest of the crate exposes a safe API. The spike version is `spikes/sw-video/src/mpv.rs`.

## Starting options

These are starting points, to be tuned. The reason for each option is recorded here when it changes.

| Option | Value | Why |
|---|---|---|
| `vo` | `libmpv` | Rendering is done by us through the render API |
| `hwdec` | `auto` (D3D11 surface) / `auto-copy` (software fallback) | d3d11va zero copy on the GPU path. The software path needs frames copied back to RAM |
| `cache` | `yes` | Network file |
| `demuxer-max-bytes` | `300MiB` | Readahead for remote MKV, to tune |
| `demuxer-readahead-secs` | `20` | To tune |
| `cache-secs` | `60` for YouTube and Twitch (`StartOptions::readahead_secs`), else mpv's default (1 h) | Without it, mpv downloads a YouTube video as fast as it can: after about 160 segments in two seconds, YouTube answers HTTP 401 to the rest, and the video or audio stops where its download stopped. A Twitch VOD would otherwise be fetched whole through the proxy |
| `network-timeout` | `30` | Slow servers |
| `http-header-fields` | `Authorization: MediaBrowser …`, added with `change-list http-header-fields append` | Auth without a token in the URL. As a plain option it would be split on its commas; the `-append` option variants do not exist in the client API |
| `embeddedfonts` | `yes` | MKV font attachments (default) |
| `sub-ass-override` | `no` | Never restyle ASS |
| `sub-files` | added with `change-list sub-files append <url>` | External subtitles (Jellyfin `DeliveryUrl`), after the embedded tracks |
| `sub-fonts-dir` | `%LOCALAPPDATA%\peeby\cache\fonts\<item>` | Fonts downloaded for external .ass |
| `audio-channels` | `auto-safe` | Downmix to the Windows speaker layout. No passthrough |
| `audio-device` | `auto`, or the output picked in the top bar (`wasapi/{…}`, `[player] audio_device`) | Changed at runtime on every player when the user picks another output (mpv reopens its audio output). An output that is gone falls back to mpv's default |
| `keep-open` | `yes` | Stay paused on the last frame instead of unloading. End of file is detected with `eof-reached`, since `END_FILE` does not fire |
| `input-default-bindings` | `no` | Keys are handled by the UI |
| `start` | from `PlaybackPositionTicks` | Resume |
| `aid` / `sid` | mapped from Jellyfin indexes | See [subtitles-ass.md](subtitles-ass.md#track-index-mapping) |

## Audio outputs

`player::audio_devices` lists the outputs with an mpv made only to ask (`vo=null`, `video=no`, no render context): it reads `audio-device-list/count` and each `audio-device-list/N/name|description` (sub-properties of a node list, no JSON to parse) and keeps the `wasapi/` ones; `auto` is shown by the caller as "Windows default". It takes about 0.15 s (DLL load included), so it runs in the background, each time the top bar's menu opens (a headset plugged in since shows up), the previous list on screen meanwhile. `ui::player::AudioOutput` is the global the players start with and observe; the shell sets it from `[player] audio_device` and on each pick.

## Security

mpv error messages built by `player::sys` never include option values or command arguments. Those can hold the Authorization header or private URLs, and they would end up in logs. Only the option or command name is reported.

## Lifecycle

1. Create the mpv handle, set the options, call `mpv_initialize`.
2. Create the render context: D3D11 on our device, or SW for the fallback. See [video-surface.md](video-surface.md).
3. Run `loadfile <stream url>`, then wait for `MPV_EVENT_FILE_LOADED`. Then send the `Playing` report and apply the track selection.
4. Observe properties, send a `Progress` report every ~10 s and on user actions. Observed: `time-pos`, `duration`, `pause`, `paused-for-cache`, `track-list`, `chapter-list`, `eof-reached`, `mute`.
5. At the end of the file (`eof-reached`, see `keep-open` above) or on stop, send the `Stopped` report, free the render context **before** the mpv handle, then destroy the handle. What happens at the end of an episode (up next, watched flag) is in [playback-ux.md](playback-ux.md).

## Player API exposed to the UI

```rust
// Shape only: final names land with the code.
pub trait Player {
  fn load(&mut self, req: LoadRequest) -> Result<(), PlayerError>; // url, headers, start, aid, sid, external subs
  fn set_paused(&mut self, paused: bool);
  fn seek(&mut self, to: Seek); // absolute or relative
  fn set_audio_track(&mut self, id: Option<TrackId>);
  fn set_subtitle_track(&mut self, id: Option<TrackId>);
  fn add_external_subtitle(&mut self, url: &str, title: &str, lang: Option<&str>);
  fn set_volume(&mut self, percent: u8);
  fn set_subtitle_delay(&mut self, seconds: f64);
  fn stop(&mut self);
}
```

Events (position, duration, pause, buffering, track list, end of file, errors) come back on a channel.
