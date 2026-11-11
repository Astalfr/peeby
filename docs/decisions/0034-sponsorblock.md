# 0034: SponsorBlock segments on YouTube videos

- Status: accepted
- Date: 2026-11-11

## Context

The user asked (2026-11-11, docs/features-plan.md, item 1) to skip a YouTube video's sponsored
parts, as SponsorBlock's browser extension does on youtube.com. [SponsorBlock](https://sponsor.ajay.app)
is a database its users fill: per video, segments in categories (sponsor, self-promotion,
reminder to subscribe, intro, end cards, preview or recap, hook, tangent, non-music part), each
with an action (`skip`, `mute`, `full` for a label of the whole video, `poi` for a highlight).
Its API is public and anonymous; its data is under CC BY-NC-SA 4.0, attribution asked.

Asking `GET /api/skipSegments?videoID=<id>` would tell SponsorBlock every video the user watches.
The API's privacy form, `GET /api/skipSegments/<prefix>`, takes the first hex digits of the
id's SHA-256 and answers every video sharing them (about 130 to 160 videos for 4 digits,
measured 2026-11-11), from which the client keeps its own. A video nobody marked answers 404.

The player already had skippable segments (Jellyfin's media segments, ADR-less, from peeby):
a skip button while one plays, the muted parts of a Twitch VOD on the seek bar.

## Decision

- `innertube::Client::sponsor_segments` asks by a 4-digit prefix (`sha2`, added to `innertube`
  with the user's agreement), every category, `skip` actions only (`mute` would need the player
  to mute a part: left out), and keeps the video's own. Its answer is a fixture
  (`sponsorblock_5f6b.json`, trimmed to three videos).
- The watch page asks once per video when the setting is on, keeps the answer for the session
  (`SponsorCache`), and gives the player its segments (`set_segments`), each in SponsorBlock's
  color on the seek bar (the extension's colors) and named in the bar's hover. A failure leaves
  the video as it is (a warning in the log).
- Per category, the settings (`[sponsorblock]`, YouTube's part of the settings page) say: skipped
  as it starts (`skip`: once, so that seeking back into it plays it, with an on-screen "Sponsor
  skipped"), a button while it plays (`button`), or ignored (`off`). By default: sponsors skipped,
  hooks and tangents ignored, the others a button. The section links to sponsor.ajay.app with
  the license, the attribution SponsorBlock asks for.
- The player's segments gain `auto_skip` and `mark` (a color), and kinds for the categories that
  had none (`Sponsor`, `SelfPromo`, `Interaction`, `EndCards`, `Hook`, `Filler`, `OffTopic`).
  SponsorBlock's outro maps to `EndCards`, not `Outro` (credits), so that it never turns the skip
  button into "Next episode"; its preview maps to `Recap`.

## Consequences

- One more outside service, anonymous, learning only a prefix shared by many videos.
- Shorts and lives ask nothing (their pages do not call it).
- A setting changed applies to the next video opened.
- Submitting or voting segments, and the `mute` and `poi` actions, are not done.
