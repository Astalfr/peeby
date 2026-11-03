# 0021: YouTube original titles

- Status: accepted
- Date: 2026-11-03

## Context

YouTube translates video titles into the request's language (`hl`): from the creator's own
translations or automatically. With the content language in French, an English video's card reads
"Bats Haaland, Gagne 100 000 $" instead of "Beat Haaland, Win $100,000". The user wants the titles
as their channels wrote them, by default.

YouTube offers no setting to turn this off. Checked on 2026-11-03 (fr/FR, anonymous):

- `search`, `browse` feeds and `next` (the watch page's title and related videos) carry only the
  translated title, nowhere else in the response;
- an `hl` other than a language (`zxx`, `und`, empty) is refused with HTTP 400, and any language
  translates into that language;
- the WEB `player` response (`videoDetails.title`, `microformat`) and oEmbed
  (`/oembed?url=…`) give the original.

`player` is a large signed request per video; oEmbed is a small anonymous GET, the one browser
extensions that undo the translation rely on.

## Decision

`innertube::Client::with_original_titles(true)` replaces the titles of every list of videos it
returns (feeds, history, channels, searches, playlists, the watch page and its related videos)
with oEmbed's, one anonymous request per video, never with the cookies. Lookups run 32 at once
with a 3 s timeout each, and are cached by video id for the client's life (up to 5,000 ids). A
failed lookup (network, 429, 5xx) keeps the translated title and is tried again next time; a
video oEmbed refuses (401 when embedding is disabled, 403, 404) is cached as having none.

The YouTube setting "Original titles" (`original_titles`, on by default) picks it; toggling it
rebuilds the clients like a language change. The Watch later ids, loaded for the "saved" state
only, skip the lookups.

## Consequences

- A page waits for its titles: about 40 ms more for a search of 20 videos on a warm connection,
  more for a cold cache on a slow link. Playback does not wait (the stream is resolved apart).
- Each new video costs one more request to youtube.com; a rate limit would only bring back the
  translated titles.
- Videos with embedding disabled keep their translated title. The WEB `player` would cover them,
  at a much higher cost per video.
- The notifications inbox and Windows toasts still show YouTube's text: their titles are inside
  sentences, not video items.
