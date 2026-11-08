# 0033: Twitch multistream

- Status: accepted
- Date: 2026-11-08

## Context

The user wants to watch several Twitch lives at once: a large main live and smaller ones that are
still visible, the chat either as a tab per channel or merged into one. Decided with the user on
2026-11-08: Twitch only for now (the parts in `ui` stay generic, for YouTube lives later); at most
four lives; the secondary ones always muted, in a row under the main one; only the main live
earns points (minutes watched, the bonus); the multistream opens from a sidebar entry of its own,
with a "+" that searches the followed channels on air or the whole of Twitch, and nothing about
it on the other pages.

Measured on 2026-11-08 with four `spike-twitch-play` running at once through the proxy, all four
in 1080p60 H.264 (the spike's variant choice does not apply through the proxy: the worst case):
about 0.4 core in all, about 2 % of the GPU's 3D engine each, every one decoded on the GPU
(d3d11va), about 215 MB per process (window included), each playing 1.5 to 2.1 s after its start
with the four starting together.

## Decision

- **A page of its own**, `Route::Multistream` (a sidebar section, "Multistream"), built from the
  lineup the space keeps while Peeby runs (`TwitchSpace::multistream`): leaving it for another
  sidebar page drops its players, coming back plays the same lives again.
- **Each live is a `WatchView` tile** (`WatchView::tile`): its playback, proxy, chat, writer,
  points and PubSub as on a live's own page, without its layout. The multistream lays out the
  tiles' players and chats, follows their raids in place (`WatchEvent::Raid`) and makes one of
  them the main live (`WatchView::set_main`).
- **One mpv per live, each with its own D3D11 device**, as for any page: the measurement shows no
  need to share one, so the copied `player` crate is unchanged.
- **The main live** has the whole shared player (its controls, qualities, its recording's past, the
  mini-player, picture-in-picture) and the keyboard; its top-right corner removes it, the first of
  the row taking its place. **A secondary live** plays through `PlayerView::set_secondary`: always
  muted (the viewer's mute is kept for when it becomes main), never `NowPlaying` in its window (no
  sound, no mini-player, no watch time), at most 480p (`SECONDARY_HEIGHT`), and drawn as its
  picture only: the multistream draws its name and buttons over it. A click on it swaps it with
  the main live: their players keep running, only their quality and roles change, the old main
  one first, so that the new one finds nobody to pause.
- **Points**: `PointsView::set_earning` is on for the main live only: `minute-watched` and the
  bonus's automatic claim follow it through swaps. A secondary live's points can still be spent
  from its chat (a second click, as anywhere).
- **Chat**: a tab per channel (the shown one, with each other tab's unread lines, red when they
  mention the viewer; the tab follows the main live unless `[twitch] multistream_chat_follows =
  false`), or merged (`[twitch] multistream_merged`). Merged, each tile's panel copies what it
  gets to the merged panel (`ChatPanel::set_mirror`): lines marked with their `ChatSource` (the
  channel's picture, a color by tile), moderation scoped to their source
  (`ChatEvent::ClearUserIn`, `ChatEvent::ClearSource`), the room's state and the connection left
  to the channel's own panel; a channel's eye leaves its new lines out. The merged chat writes to
  the chosen channel: it takes that panel's field state, emotes, points footer and pinned items
  (`ChatPanel::writing`, `write_like`) and hands what is sent to that panel, whose writer sends it.
- **One connection per channel**, anonymous for reading and signed in for writing, as on a live's
  page: four of each is well within what Twitch allows, and keeps every chat feature as it is.
  Twitch's rate limit (20 messages per 30 s) is the account's, whatever the channel, so every
  writer of the process now counts in one window (`twitch::chat::writer`'s `SENT`), which also
  covers two live pages open in two tabs.

- **The swap is animated** (2026-11-08, the user's request): the two players leave their places
  for a layer over the page and glide to each other's (position, size and corners eased in and
  out over 420 ms), their places left black meanwhile; the sound and the roles change at once,
  the qualities at the end. Each player keeps the previous mpv's picture until the new one has a
  frame (`PlayerView`'s held picture, `VideoView::has_frame`), so the quality change shows no
  black either; this also holds for any quality change and restart at a live's edge. Not with
  reduced motion.

## Consequences

- Swapping lives looks seamless: frames stretch while a player is resized each frame (mpv's
  ring settles 60 ms after the last size), then the quality changes behind the held picture.
- Four lives cost four times a live's network and memory; the row's 480p keeps their decoding
  small.
- The merged chat holds its own copy of the lines (at most `MAX_MESSAGES`): two busy chats fill it
  twice as fast, the eye is there to quiet one.
- Not done: a cross-service multistream (a YouTube live beside Twitch ones) would need a tile
  trait in `peeby-core`, since a space never depends on another; a grid layout; dragging tiles.
