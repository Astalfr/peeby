# 0018: YouTube Shorts page

- Status: accepted
- Date: 2026-11-03

## Context

The YouTube space had no way to watch Shorts other than opening one as a regular video. The user
asked for a "Shorts" page in the YouTube sidebar that works like youtube.com/shorts: one Short at
a time on the page's height, the like and comments buttons on its right.

Options for the list of Shorts:

- The Shorts shelves of Home and of the subscriptions feed (`shortsLockupViewModel`), which the
  parsers skip today: a dozen Shorts, then nothing, and nothing signed out (anonymous Home is
  empty).
- The sequence youtube.com plays from its own "Shorts" entry: `reel/reel_item_watch` without a
  video ("seedless": `params` `CA8%3D`, `inputType` `REEL_WATCH_INPUT_TYPE_SEEDLESS`) gives a
  first Short and a `sequenceContinuation`; `reel/reel_watch_sequence` with it gives about ten
  more and the next token, endlessly. Checked live on 2026-11-03: it answers signed out too, and
  personalizes signed in.

For each Short, the reel responses carry an overlay and a WEB player response, but the `next`
endpoint answers a Short like any video (`videoPrimaryInfoRenderer`, like buttons, comments
token), and VISIONOS serves its HLS like any video (1080×1920 VP9 in the smoke check).

## Decision

- The Shorts page (`Route::Shorts`, `spaces/youtube/shorts.rs`) plays YouTube's reel sequence:
  `Client::shorts` then `Client::shorts_more`, parsed to video ids only (`ShortsPage`). Everything
  else about a Short comes from the endpoints the watch page already uses: `Client::watch`
  (title, channel, rating, comments token, and now the like button's count), `Client::stream`,
  `Client::comments`, the history pings (ADR 0013).
- One `ShortView` per Short, with its own `PlayerView`, built when it shows and dropped (with its
  last history ping) when the next one shows, as the queue does for playlists. The next Short's
  stream is resolved ahead (`prefetch::warm`); more of the sequence loads when 3 are left.
- A Short loops at its end (`PlayerView::replay`), never plays "up next"; neither theater mode
  (`PlayerView::offer_theater`) nor speed changes (`PlayerView::offer_speed`: no speed menu, no
  2× while held) are offered. The like count is YouTube's own (the like button's
  title, untoggled and toggled): Return YouTube Dislike is not asked for Shorts, which a user
  swipes through faster than its 100 requests a minute allow.
- The wheel (one Short per gesture: a new one starts after 250 ms without events), the up and
  down arrows (caught before the player, where they set the volume) and two arrow buttons move
  through the sequence, with a slide as on YouTube (both Shorts move together by the page's
  height for 300 ms; the one leaving is paused and kept until the slide ends). The comments open in a panel beside the Short and stay open from one
  Short to the next.
- The code the watch page and the Shorts page share moved out of `watch.rs`: the comments
  (`comments.rs`, `CommentSection`, with the links of comments and descriptions), the rating
  (`rating.rs`), the stream start (`watch::load_stream`) and the history ping loop
  (`history::report`).
- HLS variants are ranked by their shorter side, so that a vertical 1080×1920 variant is "1080p"
  and fits a 1080 cap, as YouTube labels it.

## Consequences

- Every Short shown while signed in is added to the account's history, as on youtube.com.
- The reel endpoints are undocumented and their `params` are the web client's: if YouTube changes
  them, the page shows the error and its retry button; the fixtures (`reel_seedless.json`,
  `reel_sequence.json`) and the smoke example catch a change of shape.
- The Shorts shelves of Home and the subscriptions feed are still left out (backlog); opening one
  from there into this page would need a seeded `reel_item_watch` (with its video id).
