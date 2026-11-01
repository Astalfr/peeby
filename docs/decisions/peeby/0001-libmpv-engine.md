# 0001: libmpv as the playback engine

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

Direct Play means the client has to:
- read any container and decode any video and audio codec the library contains (H.264, HEVC 10-bit, AV1, TrueHD, DTS-HD…);
- seek over HTTP Range requests;
- keep audio and video in sync;
- render ASS subtitles faithfully, with embedded fonts.

Building this on raw FFmpeg + libass + an audio library would mean re-implementing mpv.

## Decision

Use libmpv:
- FFmpeg for demuxing and decoding, with d3d11va hardware decoding;
- the network cache;
- A/V sync;
- WASAPI audio output with downmix;
- libass for subtitles.

Peeby drives it through the client API and draws frames through the render API. There is no audio passthrough: all audio is decoded and downmixed to the Windows speaker layout.

## Consequences

- Subtitle and codec support come from a mature player, and ASS behaves as in mpv.
- The app depends on a GPL DLL ([0004](0004-gpl-license.md)) and on a third-party Windows build that has to be pinned (build-windows.md (peeby's `docs/build-windows.md`)).
- The display path is constrained by the render APIs mpv offers ([0002](0002-software-video-surface.md)).
