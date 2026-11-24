# Plan: a more responsive feel (2026-11-24)

From a review of the app's motion and loading states against current practice (Fluent 2 and
Windows 11 motion, Apple's "fluid interfaces", stale-while-revalidate and optimistic updates).
Nothing is started: each item is done in its own change, with its docs (the "Motion" table of
[design.md](design.md)), its tests and its English and French strings, in the order the user
chooses. Items are grouped in lots by value for the effort.

What the app already does well, and stays: `motion::tween` (interruptible, per element id),
`motion::crossfade` from skeletons to content, card hover, the accent's transition between
spaces, tabs sliding while dragged, the chat gliding to new lines, poll bars, the tab content's
fade and slide, the list of lives' staggered rows, the splash. The rating and the subscribe
button are already optimistic (they change at once and go back if YouTube refuses).

Rules that apply throughout:
- Motion stays "discreet and short" (design.md): nothing loops but loaders, and every new
  animation is skipped when `cx.reduce_motion()` is set.
- Durations follow Fluent: 83 ms (micro feedback), 167 ms (controls, hover, menus), 250 ms
  (navigation, elements entering). Entering: decelerate; leaving: accelerate and shorter.
- Nothing may delay playback: the 1–2 s budget from the click to the first frame comes first, and
  no animation waits for the network or holds back a frame of video.
- Anything that sends more requests to YouTube (prefetch) is weighed against the bot check
  ([youtube-bot-check.md](youtube-bot-check.md), ADR 0038/0040) and agreed with the user first.

## Lot 1: the most felt for the least code

### 1. A watch page seeded from its card

Done for YouTube (2026-11-24), not yet checked in the app: the seed is not on the route but in
a global keyed by video id (`space-youtube/src/seed.rs`), as `history::offer` hands the resume
position, so history and restored tabs are untouched. Left: Twitch and Jellyfin.

Today `Route::Watch(id)` carries the id only, so the watch page shows `skeleton_details()`
([watch.rs](../crates/space-youtube/src/watch.rs), `details`) until `next` answers, though the
card knew the title, the channel, its avatar and the thumbnail.

- Add an optional seed to the route (title, channel name and id, avatar, thumbnail, duration,
  live flag), filled by the cards, list rows, related videos, queue, notifications and search
  results; links from outside (command line, notification toasts) open without one.
  The seed is not part of the route's identity (history, restored tabs, ADR 0035 save the id).
- While `next` runs, the page shows the seeded title and channel line (the rest as skeletons),
  and the thumbnail as the player's poster, under the video surface, until mpv's first frame;
  then the poster fades out (167 ms). `next`'s answer replaces the seed without a jump: same
  layout, same sizes.
- Twitch (a live's card: title, channel, category, preview) and Jellyfin (an item's card:
  backdrop, title) get the same where their pages wait for a request.
- Tests: a route with and without a seed; the seed never written to `tabs` restore data.

### 2. Optimistic "Watch later" and playlist removals

Done (2026-11-24), not yet checked in the app. A removed entry is hidden at once (no height
fold yet) and comes back if YouTube refuses.

`save.rs` (`Pending`) shows a spinner on the "Watch later" and "x" buttons while YouTube
answers. Make them change at once, as the rating does: the check (or the row's removal, folding
its height in 167 ms) shows at the click, goes back with an error toast if YouTube refuses; a
second click while pending still sends nothing. The spinner shows only when the answer takes
more than 400 ms. The subscribe button's loader ([subscribe.rs](../crates/space-youtube/src/subscribe.rs))
follows the same 400 ms rule.

### 3. Pressed states

Done (2026-11-24): `card::Hover::pressed` (`motion::press`, `Hover::track` for hand-made
frames such as the playlists' cards) on every `MediaCard` layout, with the picture darkened;
`.active` with `theme::surface_pressed` on the rows, entries and buttons whose hover changes
their surface. The first pass was too faint on related videos (`surface` to `surface_hover`)
and missed the sidebar's entries: fixed after the user's check.

Only five elements have an `.active(` style: cards, rows, sidebar entries, tiles and most
buttons answer the hover but not the press. GPUI cannot scale a `div`, so:
- cards: the 4 px hover rise drops back to 2 px and the ring dims while the mouse is down;
- rows, tiles, sidebar entries: the surface one step darker (`surface` under `surface_hover`);
- gpui-component buttons: check what `Button` already does when pressed, and set the theme's
  active colors through `theme::apply` if it uses none.
Instant on press (no transition), 83 ms back on release.

### 4. Windows' animation setting

Done (2026-11-24): `desktop::follow_animation_setting`, read again on each window activation
(simpler than listening for `WM_SETTINGCHANGE` in GPUI's window procedure); `motion::tween`
itself honors `reduce_motion`. Spinners keep turning (`ui::loader` does not check it).

Read "Animation effects" (`SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`) into GPUI's
`reduce_motion` at startup and on `WM_SETTINGCHANGE`, in `peeby_core::desktop::sys` (already an
FFI module). design.md notes this is not done yet. Test: the splash and every tween are skipped
with the setting off.

## Lot 2: loading ahead (needs the user's go-ahead on requests)

### 5. Hover prefetch with web_embedded first

Done (2026-11-24), not yet checked in the app: `prefetch::fetch_page` / `claim_page` (with the
session's client, whatever the player; 5 min, 30 s for a live; 16 pages) and
`ui::picture::preload` for the poster (640 px) and the avatar. The web_embedded `player`
prefetch is not done.

With web_embedded first (the default signed in, ADR 0040), [prefetch.rs](../crates/space-youtube/src/prefetch.rs)
is idle, so most signed-in users get no head start. On a 300 ms hover, prefetch what costs no
`player` request: the `next` answer (title, details, related videos, cached for a few minutes)
and the large thumbnail and avatar for the poster (item 1). Prefetching web_embedded's
`player` answer itself, with its `n` solved in the running Deno, is a separate decision: measure
its gain and its request count against the bot check first.

### 6. Feeds from the last session (stale-while-revalidate)

No feed is cached between runs. Save the last Home, subscriptions and Twitch catalog first pages
(items only: ids, titles, thumbnails, counts; no account data) in `%LOCALAPPDATA%\peeby`, show them
at once on startup (behind the splash), refresh in the background, and merge the answer:
items already shown stay in place, new ones come in at the top with a "N new videos" pill
(a click scrolls up) rather than moving what the user is looking at. Not for a feed older than a
day, and the cache is cleared at sign-out. Needs a short ADR (what is stored, where, for how long).

### 7. Thumbnails ahead of the scroll

Done (2026-11-24) for the YouTube feeds, not measured: the next page is asked for when a row
two rows before the last one is built (`LOAD_AHEAD_ROWS`), and each row built loads the
thumbnails of the row three below it (`PRELOAD_AHEAD_ROWS`, `ui::picture::preload`). The
overdraw stays 800 px. Search, playlists, Twitch and Jellyfin lists are left as they are.

The feed lists already build 800 px beyond the view (`OVERDRAW` in
[feed.rs](../crates/space-youtube/src/feed.rs)), so the next page starts about a screen early.
Measure first whether the end-of-list spinner still shows while scrolling at a normal pace; if it
does, raise the overdraw or start `load_more` from the row count. Separately, ask the image store
for the thumbnails of the next rows (downloaded and decoded, not uploaded to the GPU) so they are
ready when they scroll in.

## Lot 3: feedback and new motion

### 8. Player feedback

Done (2026-11-24), not yet checked in the app: `SeekPulse` and the volume ring
(`ui::widgets::gauge`, drawn with the loader's `Ring`); timed in the render rather than
`with_animation`, so reduced motion still shows them (still, then gone).

- Play/pause: already there (`pause_pulse`: the new state's icon grows and fades out).
- Seek by keys: `+10 s` / `−10 s` on the side that moved, adding up while the key is pressed again
  within a second (`+30 s`), then fading out.
- Volume by wheel or keys: a small ring with the percentage, next to the OSD.
These are overlays in [overlays.rs](../crates/ui/src/player/overlays.rs), not new OSD texts.

### 9. Back and forward directions

Done (2026-11-24), not yet checked in the app: `Tab::came_back` and `space::page_entrance`,
shared by the three spaces (Twitch had no page entrance until now). A new page and "forward"
slide in from the right rather than rise (the user's choice). The entrance plays once per view built
(`Page::entrance` holds the view it played for), so the pages that play take it too: leaving
fullscreen does not replay it.

Pages fade in rising 8 px whatever the direction. Going back (ADR 0019's history, the mouse's side
button), the page slides in 24 px from the left instead; forward and new pages keep rising. The
direction is known where the shell moves in the tab's history.

### 10. Sidebar folding

Done (2026-11-24), not yet checked in the app, in 150 ms (`theme::FAST`) rather than 167 ms.
The chat's column in theater mode folds the same way (the user's request). The headings
switch to their separators at the end, so a space with headings (Twitch's
"Live", Jellyfin's libraries) shifts the entries under them once the fold ends.

The sidebar switches from 232 px to its icons at once ([shell/mod.rs](../crates/peeby-core/src/shell/mod.rs),
the menu button). Tween its width (167 ms, decelerate), the labels fading out in the first half
on folding and in the second half on unfolding; the content panel follows its width.

### 11. Menus and popovers opening

The "⋮" menus, the bell's panel, the audio output menu and the search suggestions open in a
single frame. Fade them in moving 4 px from their anchor (120 ms), and out without movement (83 ms),
as the sidebar's hover cards already do. Check first what gpui-component's `Popover` and
`PopupMenu` allow from outside.

### 12. A feed's first appearance

The first screen of cards fades in rising 8 px, 25 ms apart (the first 12), as the list of lives'
rows. Only when a feed loads for the first time in its tab: not on going back, refreshing or
loading the next page.

### 13. Hover scrubbing on thumbnails

Moving the mouse across a hovered thumbnail shows the storyboard's frames at that point (the
storyboard parsing already serves the seek bar's preview), with a thin progress line at the
bottom. Only where the storyboard is known without a new request (the prefetched `player`
answer); otherwise the thumbnail stays still. Depends on item 5.

## Lot 4: foundations and larger work

### 14. Springs in `ui::motion`

`motion::tween` eases every value with `ease_out_cubic` and restarts from rest when its target
changes: a hover reversed halfway stops, then starts back. Add `motion::spring(id, target,
response, damping)` that keeps position and velocity across target changes (critically damped
by default, so no bounce), and use it where values are redirected often: tabs while dragged,
the Shorts' slide, the sidebar's width, card hover. A test for continuity of position and
velocity on a target change. Needs an ADR (the motion model changes) and the design.md table.

### 15. Connected animation from card to player

Builds on item 1: on a click, the card's thumbnail grows from its place in the grid to the
player's place (250 ms, decelerate), the page's other parts fading in around it; the video
surface takes over at its first frame. Needs the card's bounds at the click (a `canvas`
prepaint) and an overlay over the content panel; never in fullscreen, and never holding back the
first frame.

### 16. Smooth wheel scrolling

GPUI scrolls by whole steps with a mouse wheel, where WinUI's ScrollViewer animates each notch
(about 150 ms). Animate line-based wheel deltas (`ScrollDelta::Lines`) toward a target offset,
added up while the wheel turns; pixel deltas (touchpads, precise wheels) stay as they are.
For `ListState` lists (feeds, chat) and `ScrollHandle` pages, behind one helper in `ui`. Test
with the chat's following and the feed's lazy rows.

## Suggested order

Lot 1 (items 1 to 4), then lot 2 once the requests are agreed (5, 6, 7), then lot 3 in any order,
then 14 before 15 and 16.
