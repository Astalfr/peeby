# Direct Play: never transcode

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

Jellyfin decides whether to transcode from the **DeviceProfile** the client sends with `PlaybackInfo`. The goal is a profile that accepts everything, plus a client that refuses to play anything that is not Direct Play.

## Flow

1. `POST /Items/{itemId}/PlaybackInfo` with:
   - query or body: `UserId`, `MediaSourceId`, `AudioStreamIndex`, `SubtitleStreamIndex`, `StartTimeTicks`;
   - `EnableDirectPlay: true`, `EnableDirectStream: false`, `EnableTranscoding: false`, `AllowVideoStreamCopy: false`, `AllowAudioStreamCopy: false`;
   - `MaxStreamingBitrate`: very high (e.g. 1,000,000,000), so that bitrate limits never trigger a transcode;
   - `DeviceProfile`: see below.
2. Read the chosen `MediaSources[i]` in the response:
   - `SupportsDirectPlay` must be `true`;
   - `TranscodingUrl` must be absent or empty;
   - keep `PlaySessionId` for the stream URL and for reporting.
3. **If either check fails, do not play.** Show an error that lists `TranscodingReasons`. No silent fallback.
4. Stream URL: `GET /Videos/{itemId}/stream?static=true&mediaSourceId=…&PlaySessionId=…`. `static=true` serves the file as-is, with HTTP Range support. The `Authorization` header reaches mpv through `http-header-fields`.

## DeviceProfile

```jsonc
{
  "Name": "Peeby",
  "MaxStreamingBitrate": 1000000000,
  "MaxStaticBitrate": 1000000000,
  "DirectPlayProfiles": [
    { "Type": "Video", "Container": "", "VideoCodec": "", "AudioCodec": "" },
    { "Type": "Audio", "Container": "" }
  ],
  "TranscodingProfiles": [],
  "ContainerProfiles": [],
  "CodecProfiles": [],
  "SubtitleProfiles": [
    { "Format": "ass",     "Method": "Embed" }, { "Format": "ass",     "Method": "External" },
    { "Format": "ssa",     "Method": "Embed" }, { "Format": "ssa",     "Method": "External" },
    { "Format": "srt",     "Method": "Embed" }, { "Format": "srt",     "Method": "External" },
    { "Format": "subrip",  "Method": "Embed" }, { "Format": "subrip",  "Method": "External" },
    { "Format": "vtt",     "Method": "Embed" }, { "Format": "vtt",     "Method": "External" },
    { "Format": "pgssub",  "Method": "Embed" }, { "Format": "pgssub",  "Method": "External" },
    { "Format": "dvdsub",  "Method": "Embed" }, { "Format": "dvdsub",  "Method": "External" }
  ]
}
```

- Containers and codecs are **listed explicitly**: see `CONTAINERS`, `VIDEO_CODECS` and `AUDIO_CODECS` in `crates/jellyfin-api/src/playback.rs`. Empty strings were tried first, but Jellyfin 12.3 answered `NoCompatibleStream` for some items. That error turned out to come from missing episodes, which have no file; the explicit lists are kept because they are unambiguous.
- **Missing (virtual) episodes** always answer `NoCompatibleStream`, because there is no file. Episode lists use `isMissing=false`.
- `SubtitleDeliveryMethod` values in the spec are `Encode`, `Embed`, `External`, `Hls` and `Drop`. **`Encode` means burn-in, which is a transcode.** That is why ASS must be declared with both `Embed` and `External`.
- The serialized profile has a unit test, so that a refactor cannot drop the ASS entries without anyone noticing.

## Checking it

In the server dashboard (Dashboard → Activity / active devices), a Peeby session must show **Direct Play**. Phase 0c and the phase 3 exit criterion both check this.

Verified on Jellyfin 12.3 (`direct_play_negotiation` live test): MKV episodes with embedded ASS and 2–14 font attachments, and MP4 movies with an external SRT, are all offered as Direct Play.
