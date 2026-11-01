# Jellyfin API

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

- **Minimum server version:** 12.0. Jellyfin dropped the `10.` prefix after 10.11, so 12.0 follows 10.11.
- **Reference spec:** [../reference/openapi-12.4.0.json](../reference/openapi-12.4.0.json), downloaded from `https://api.jellyfin.org/openapi/jellyfin-openapi-stable.json` on 2026-10-31.
  - It describes 12.4, which is newer than the 12.0 minimum. Any endpoint used must also exist in 12.0. When unsure, check the `/api-docs/openapi.json` of a 12.0 server.
  - To refresh it, download the stable spec again and rename the file to its `info.version`.

## Authentication

Every request sends this header:

```
Authorization: MediaBrowser Client="Peeby", Device="<hostname>", DeviceId="<uuid>", Version="<crate version>", Token="<token>"
```

`Token` is omitted before login. Never put the token in a URL (`api_key`), since URLs end up in logs.

| Step | Endpoint |
|---|---|
| Check the server and its version | `GET /System/Info/Public` |
| Password login | `POST /Users/AuthenticateByName` |
| Quick Connect | `GET /QuickConnect/Enabled`, `POST /QuickConnect/Initiate`, poll `GET /QuickConnect/Connect?secret=…`, then `POST /Users/AuthenticateWithQuickConnect` |
| Logout | `POST /Sessions/Logout` |

## Endpoints used (MVP)

All of these paths were checked against the 12.4 spec.

| Purpose | Endpoint |
|---|---|
| Signed-in user (name, avatar) | `GET /Users/Me`, avatar `GET /UserImage?userId=…&tag=…` |
| Libraries | `GET /UserViews` |
| Item lists (filter, sort, paging, search) | `GET /Items` (`ParentId`, `IncludeItemTypes`, `SortBy`, `StartIndex`, `Limit`, `searchTerm`, `Fields`) |
| Item detail | `GET /Items/{itemId}` |
| Series | `GET /Shows/{seriesId}/Seasons`, `GET /Shows/{seriesId}/Episodes` |
| Home rows | `GET /UserItems/Resume`, `GET /Shows/NextUp` (`nextUpDateCutoff`, `enableRewatching` from the settings), `GET /Items/Latest` |
| Watched state | `POST` / `DELETE /UserPlayedItems/{itemId}` |
| Favorites | `POST` / `DELETE /UserFavoriteItems/{itemId}` |
| Images | `GET /Items/{itemId}/Images/{imageType}?maxWidth=…&tag=…` |
| Playback negotiation | `POST /Items/{itemId}/PlaybackInfo`, see [direct-play.md](direct-play.md) |
| Stream | `GET /Videos/{itemId}/stream?static=true&mediaSourceId=…&PlaySessionId=…` |
| External subtitles | `GET /Videos/{routeItemId}/{routeMediaSourceId}/Subtitles/{routeIndex}/Stream.{routeFormat}` |
| Attachments (fonts) | `GET /Videos/{videoId}/{mediaSourceId}/Attachments/{index}` |
| Reporting | `POST /Sessions/Playing`, `POST /Sessions/Playing/Progress`, `POST /Sessions/Playing/Stopped` |
| Skippable parts (intro, credits…) | `GET /MediaSegments/{itemId}`. 404 or an empty list means none ([ADR 0009](../decisions/peeby/0009-segments-from-chapters.md)) |
| Previous and next episode | `GET /Shows/{seriesId}/Episodes?adjacentTo={id}&isMissing=false` (previous, current, next) |
| Trickplay tiles | `GET /Videos/{itemId}/Trickplay/{width}/{index}.jpg?mediaSourceId=…`, layout from `BaseItemDto.Trickplay` |

## Playback reporting

Every report carries `ItemId`, `MediaSourceId`, `PlaySessionId`, `PositionTicks`, `IsPaused`, `AudioStreamIndex`, `SubtitleStreamIndex` and `PlayMethod: DirectPlay`. One tick is 100 ns.

- `Playing`: sent once the file is loaded.
- `Progress`: sent every ~10 s, and on pause, seek and track change.
- `Stopped`: sent on stop, end of file or app exit. Without it, resume and Next Up go wrong.

## Models

Models are written by hand for the subset we use. Generated code from the full spec is huge and awkward to use. Unknown fields are ignored, and fields the client relies on are not `Option` unless the spec marks them nullable. Each model is tested against a real JSON fixture captured from the server, in `crates/jellyfin-api/tests/fixtures/`, with tokens removed. Exception: `MediaSegmentDto` and `TrickplayInfoDto` are tested on JSON written from the spec, because the test server has neither (the `playback_extras` live test checks that the calls succeed).
