# 0009: Skippable parts from chapter titles when the server has no media segments

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-11-01

## Context

Jellyfin exposes skippable parts (intro, credits…) through `GET /MediaSegments/{itemId}`. The segments come from provider plugins (intro detection by audio fingerprint, EDL files…). A stock server has none. The test server returns an empty list for every item checked, and no trickplay either.

The library is mostly fansub and BD anime MKVs. In a random sample of 60 episodes, a third had named chapters: `Opening`/`OP`, `Part A`/`Part B`, `Ending`/`ED`/`Credits`, `Preview`/`PV`. The naming is not uniform:
- `Intro` is the cold open before `OP` in some releases, and the opening itself in releases without an `OP` chapter;
- `Outro` follows `Ending` in some releases (the scene after the credits);
- `Credits Start` / `Credits End` / `Preview End` are markers rather than parts.

## Decision

- Use the server's media segments when there are any.
- Otherwise, derive the parts from mpv's `chapter-list`, with the rules in [player/playback-ux.md](../../player/playback-ux.md#skip-buttons-opening-recap-credits). The rules are pure functions in `peeby::segments`, with a unit test for each naming style seen on the server.
- "Up next" starts where only credits and previews remain, so a scene after the credits is not skipped.

## Consequences

- Skip intro, skip credits and up next work on this library without any server plugin.
- A misnamed chapter can produce a wrong skip button. The length limits (5 s minimum, 4 min maximum for openings and recaps) filter the obvious cases. A wrong button is only shown, never acted on, except up next, which can be cancelled.
- New naming styles mean a new rule and a new test. If the server gains a segment provider, its segments take over without any change.
