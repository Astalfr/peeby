# ASS subtitles

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

This is where most clients fail. The rules below are not optional.

## Rendering rules

1. **libass renders the subtitles**, through mpv. Never convert ASS to a simpler format.
2. **MKV font attachments are always loaded** (`embeddedfonts=yes`, the default). Without them, typeset fansubs are unreadable.
3. **External .ass files:**
   - URL: `GET /Videos/{routeItemId}/{routeMediaSourceId}/Subtitles/{routeIndex}/Stream.ass`, or the `DeliveryUrl` that `PlaybackInfo` returns for `DeliveryMethod = External`;
   - added with mpv `sub-add`, with title and language taken from the Jellyfin `MediaStream`;
   - their fonts are not in the stream. Download every entry of `MediaSource.MediaAttachments` that is a font (`MimeType` such as `application/x-truetype-font`, `font/ttf`, `font/otf`, `application/vnd.ms-opentype`) from `GET /Videos/{videoId}/{mediaSourceId}/Attachments/{index}` into `%LOCALAPPDATA%\peeby\cache\fonts\<itemId>\`, and point `sub-fonts-dir` at that folder before loading. **⚠ verify** which attachments Jellyfin lists for a file whose subtitles are external (fonts next to the file, or a fallback-fonts folder on the server).
4. **Render at display resolution.** The software buffer matches the element's physical size, so libass lays out text at that size and nothing is upscaled.
5. **VSFilter compatibility:**
   - honor the `YCbCr Matrix` header, so subtitle colors match the typesets burned into the video;
   - honor `ScaledBorderAndShadow` and `LayoutResX/Y`;
   - `sub-ass-override=no`. The user can only change styles through an explicit setting.
6. **Missing fonts** fall back to DirectWrite, libass's native font provider on Windows. Missing fonts are logged (`tracing::warn`) with their names.
7. **Performance:** measure the libass cost on a heavy karaoke script in phase 0d.
8. **Also supported:**
   - PGS and VobSub (bitmap subtitles) through mpv;
   - track switching without reloading the file;
   - adjustable subtitle delay.

## Track index mapping

Jellyfin identifies streams by their index in `MediaSource.MediaStreams`, which counts all streams together and appends external subtitles. mpv identifies tracks by a per-type `id` (`aid`, `sid`, starting at 1) and reports them in `track-list`.

The mapping (`peeby::tracks::TrackMap`) is computed from the `MediaSource` before mpv starts:
- **embedded tracks:** mpv id N is the N-th embedded stream of that type in Jellyfin `Index` order, which is container order;
- **external subtitles:** they are given to mpv (`change-list sub-files append`) in `MediaStreams` order, so they come after the embedded subtitles, in that order.

The initial `aid`/`sid` come from the choice last made in the same series, when the file has a stream with the same language (see [playback-ux.md](playback-ux.md#audio-and-subtitles-carried-across-episodes)). Otherwise they come from the server's `DefaultAudioStreamIndex` / `DefaultSubtitleStreamIndex`, which follow the user's language preferences. When mpv reports a track change, the selected ids are mapped back to Jellyfin indexes for the progress reports.

Unit tests cover multi-audio, embedded plus external subtitles, and "off". A wrong mapping shows up as "the wrong language plays". **Still to verify on real multi-track files.**

## Test files

See peeby's `docs/testing.md` (test media set, `D:\DEV\peeby`).
