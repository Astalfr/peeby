# 0042: Cards from `next`, end screens from the watch page

- Status: accepted
- Date: 2026-11-25

## Context

YouTube's annotations are gone (deleted in January 2019); what replaced them is a video's
**cards** (a teaser over the picture at a moment, a link to a video, a channel or a site) and
its **end screen** (boxes over the last 5 to 20 seconds: videos, playlists, the channel, a
site).

- Cards are in the WEB `next` answer Peeby already asks for each video: their teasers and
  times in `cards.cardCollectionRenderer`, their content in the same order in the description
  panel's `videoDescriptionInfocardsSectionRenderer`.
- The end screen is only in the WEB player answer (`endscreen.endscreenRenderer`). Checked on
  2026-11-25, anonymous: WEB's `player` refuses the video (`UNPLAYABLE`, no end screen),
  MWEB and TVHTML5 too, web_embedded answers an error, VISIONOS plays without one. The watch
  page's HTML (`youtube.com/watch?v=`) holds the WEB player answer
  (`ytInitialPlayerResponse`) with its end screen, anonymously; it weighs about 1.7–2 MB.
- Signed in, Peeby's own WEB `player` request (`Client::tracking`, for the history) may carry
  it too: not checked (it needs the user's session), and signed out it would not exist.

## Decision

- Cards: parsed from `next` (`WatchPage::cards`), their teaser shown at the top right of the
  player for 6 s from their time (the YouTube setting "Cards" / "Fiches", `cards`, on by
  default, turns the teasers off at once), listed under the unfolded description either way.
- End screen: `Client::end_screen` reads the watch page anonymously and parses its
  `ytInitialPlayerResponse`; the watch page asks for it once playback has started (never during
  the start, long before the end), not for a live.
- The shared player knows neither: a space gives it `Teaser`s and `Hotspot`s (placed as shares
  of the picture, mpv's letterbox bars left out) with what a click does
  ([player/hotspots.md](../player/hotspots.md)). A click follows the link as the description's
  would: this video's timestamp seeks, a YouTube page opens in the tab, a site in the browser.

## Consequences

- One 2 MB anonymous request per video played, read whole to find its player answer.
- If signed-in `tracking` turns out to carry the end screen, signed in it could come from there
  at no cost; the watch page stays for signed out.
- A change of the watch page's HTML (the `ytInitialPlayerResponse = ` marker) leaves videos
  without an end screen (logged as a warning), nothing else.
