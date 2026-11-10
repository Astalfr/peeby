# Playback page

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

What the player page does beyond play/pause. The code is in `crates/peeby/src/player_page.rs`. The pure logic lives in `segments.rs` (skippable parts) and `tracks.rs` (track mapping and carry-over), both covered by unit tests.

## Skip buttons: opening, recap, credits

A **Skip intro / Skip recap / Skip credits / Skip preview** button appears bottom right while one of those parts plays. It stays visible when the other controls hide. `S` does the same.

Where the parts come from ([ADR 0009](../decisions/peeby/0009-segments-from-chapters.md)):

1. **Server media segments** (`GET /MediaSegments/{itemId}`), when the server has any. They need a segment provider plugin; the test server has none.
2. Otherwise, **chapter titles** from mpv's `chapter-list` (never chapters a page gave, see the seek bar below). Fansub and BD releases name them:

| Title (case-insensitive, first word) | Part |
|---|---|
| `Opening`, `OP`, `OP1`, `OP 2` | Opening |
| `Intro`, only when no `OP`/`Opening` chapter exists (otherwise it is the cold open) | Opening |
| `Recap`, `Previously` | Recap |
| `Ending`, `ED`, `ED2`, `Credits`, `Credits Start`, `End Credits` | Credits |
| `Outro`, only when no `ED`/`Ending`/`Credits` chapter exists (otherwise it is the scene after the credits) | Credits |
| `Preview`, `PV`, `Next`, `Yokoku` | Preview |
| Anything ending in ` End` (`Credits End`, `Preview End`) | Nothing: it marks what follows |

A chapter runs until the next one starts. Parts shorter than 5 s are ignored, and so are openings or recaps longer than 4 min, which are probably mislabeled.

Some releases carry UTF-8 chapter titles that were decoded as Windows-1252 upstream (`ã€Œ` instead of `「`). The server returns them that way too. `ui::repair_mojibake` undoes it when the text round-trips cleanly.

## Previous and next episode

For an episode, its neighbours come from `GET /Shows/{seriesId}/Episodes?adjacentTo={id}&isMissing=false`, which crosses season boundaries. The server returns up to three items (previous, current, next), so the sides are found by the current item's position: the first episode has no previous one.

The **previous** and **next** buttons frame the seek and play buttons. For an episode, both are always there and disabled at either end of the series, so the other buttons do not move; a movie has neither. Going back (button or `P`) never marks the episode watched, and also replaces the player in the navigation stack.

## Up next

- The **up next** card appears where only credits and previews remain: the start of the last run of credits and preview parts that reaches the end. Credits followed by a story scene do not count. Without segment information, the card appears 20 s before the end.
- With autoplay (the default), the card counts down 10 s and then plays the next episode. **Cancel** hides it until the user seeks back before that point. Without autoplay, the card offers **Play now** only.
- At the end of the file, the next episode starts unless the card was cancelled. Otherwise the page goes back.
- Moving on sends `Stopped`. When the episode is already in its final credits (autoplay, end of file, **Next episode** over the credits), it also sends `POST /UserPlayedItems/{id}`, because the server only marks an item watched past ~90 % and credits often start earlier. `N` or the next-episode button earlier in the episode does not mark it watched.
- The next player **replaces** the current one in the navigation stack, so Back returns to the series page.

Settings (`%APPDATA%\peeby\settings.toml`):

```toml
[playback]
autoplay_next = true
autoplay_countdown = 10
```

## End of file

mpv runs with `keep-open=yes`, so at the end it pauses on the last frame and never unloads the file: `END_FILE` does not fire. The player observes `eof-reached` and sends `Event::EndReached`. The page then sends `Stopped` at the full duration and marks the item watched.

An `eof-reached` more than 10 s before the duration (`EARLY_END_MARGIN`) is not the end: one of the streams stopped (a YouTube variant's video, or its separate audio rendition), so the other one played on alone, or a seek went past the point where it stopped. The player then restarts mpv on the same quality from the current position. If it ends early again within a minute of that restart, playback stops with an error. It never moves on to the next video or marks the item watched.

A seekable video that keeps buffering (`paused-for-cache`) for 10 s while it plays (`STALL_AFTER`) is stuck, not slow. This was seen after a pause: the server had closed the connection, the next transfer failed, and from then on mpv's video demuxer read ahead without end while nothing more was decoded. The player restarts mpv on the same quality from the current position. If it stalls again within a minute of a restart, the player leaves it buffering. Lives are not watched: they wait on their edge.

A YouTube variant's picture can also stop while its separate audio rendition plays on, without an end or buffering. This was seen on corrupt VP9 data that every decoder refused (d3d11va, the copy modes, then software), after which mpv's video stayed at its end and seeks inside the cache did not bring it back. mpv cannot play past what the file's own demuxer read otherwise. So a playhead more than 2 s past `demuxer-cache-time` (`VIDEO_BEHIND`) for 2 s (`VIDEO_STOPPED_AFTER`), while it plays and well before the end, means the video stopped. The player then restarts mpv on the same quality from the current position, which is past the bad data. If it happens again within a minute of a restart, playback stops with an error rather than staying frozen. Lives are not watched.

## Audio and subtitles carried across episodes

When the user changes tracks, the page records the choice for the series, for the rest of the session (`Browser::remember_tracks`). The next episode of that series starts with the stream in the same language, preferably with the same title and forced flag. This keeps `Full` subtitles from turning back into `Signs & Songs`. When the file has no match, the server defaults apply. Those follow the user's language settings on the server.

## Seek bar

- **Shape.** A rounded track (5 px, 8 px on hover). The accent progress and the lighter hover fill carry the same radius and are never narrower than the track is tall, because GPUI clips children to rectangles and square fill ends would stick out of the rounded track. On hover, a 14 px accent knob with a white ring marks the playhead.

- **Chapters** split the track into pieces 2 px apart, as on YouTube; the hovered chapter's piece grows 4 px taller. Both the track's thickening on hover and a piece's growth ease over 150 ms (`motion::tween`). The chapters menu (`ui::player::chapter_list`, which a space's cards can show too) lists them with their start time (in a column as wide as the longest one, so that the titles line up), the title 220 px wide at most and wrapped on two lines at most (its entries, as the quality, audio and subtitle menus', aligned left; the quality menu is a little narrower, 200 px at least instead of 240), and Page Up / Page Down jump between them ("previous" more than 5 s into a chapter goes back to its start, as mpv's `chapter-seek-threshold`).
- **Where chapters come from.** The file's (mpv's `chapter-list`), unless the page gives its own with `PlayerView::set_chapters`: YouTube's watch page does, from the `next` response's player bar markers (`DESCRIPTION_CHAPTERS`, else `AUTO_CHAPTERS`), since its HLS streams carry none. Given chapters are never replaced by mpv's list (sent again, empty, by each new mpv after a quality change), Page Up / Page Down seek to them directly, and no skippable segment is guessed from their titles: a creator's `Intro` or `Outro` is not a release's opening or credits. `set_chapters_with_pictures` gives each one a picture too (a `Picture`, drawn at Twitch's box art ratio 52:72): the Twitch space does, with the game's box art, shown in the chapters menu and beside the chapter's name in the hover preview.
- **Hover** shows the time and chapter under the mouse, centered on it (the preview is a thumbnail wide whenever a chapter is named). When the server has trickplay thumbnails (`BaseItemDto.Trickplay`), the matching thumbnail appears too: the tile image (`GET /Videos/{id}/Trickplay/{width}/{tile}.jpg`) comes through the image store, and the thumbnail is shown by offsetting the tile inside a clipped box. **Untested against a server**: the test server generates no trickplay, so only the cell arithmetic has tests.

## Title bar

While the controls show (the mouse moved over the player, they hide with it), the embedded player shows, over a gradient along its top: the channel's avatar (36 px, 44 px in fullscreen), then the title on one line and the channel's name under it, both cut with an ellipsis. The page gives them with `PlayerView::set_title` and `set_author` (`ui::player::Author`: a name and an avatar `Picture`): YouTube's watch page and Shorts from the `next` response's channel, Twitch from its page header (a live shows its broadcast's title and the streamer, a VOD or a clip its title and channel). The bar takes no mouse events: a click on it plays or pauses. The OSD drops below it while it shows. The full-window and page layouts (Jellyfin) keep their own top bar, with the back button.

## Control bar

Left to right: previous episode, back, play/pause, forward, next episode, then the time (elapsed / total). The remaining time (`-12:34`, once the duration is known) sits on the right, just left of the volume, chapters, audio, subtitles, shortcuts and fullscreen buttons.

Player buttons (`control_button`, and the window buttons) are 40 px with 20 px white icons, larger than the 32 px and 16 px used elsewhere, since the screen is watched from further away. On hover the icon turns to the accent (primary) over a light background (white at 18 %, a little stronger than the app's ghost buttons); the window Close button turns red with a white icon. gpui-component's buttons keep one foreground for every state, so the page tracks the hovered button (`hovered_control`) to color the icon.

The volume shows no number. Hovering the mute button opens a horizontal slider right beside it, in the control row, as on YouTube (click or drag; left 0 %, right 100 %, or 200 % with the boost; the wheel over it moves 2 % a notch, in every layout, and goes no further: neither the picture's 5 % step nor the page's scroll): it widens from the button and pushes the rest of the row. It stays open while the mouse is over the button or the slider, and keeps the controls from hiding meanwhile. A drag started on the slider keeps going until the button is released, even when the mouse leaves the slider: it stays open and keeps following the mouse's horizontal position. It opens with a 150 ms fade and widening (ease-out) and closes with the same animation played backwards; going straight from the button to the slider does not replay it. A change shows the value briefly at the top.

After the track, the slider has a small "Boost" button: on, it lets the volume go up to 200 % for quiet videos (mpv amplifies the sound: its `volume-max` is 200, `player::VOLUME_MAX`; mpv's volume is cubic, so 200 % is about +18 dB, and loud sources clip). The track then doubles in length so that 100 % stays where it was, a mark shows 100 %, and the boosted part of the fill and the knob take the warning color; the keys and the wheel go up to 200 % too. Off again, a louder volume comes down to 100 %. The boost is never remembered: each player starts without it, and the volume kept for the next videos is 100 % at most.

## Mouse and keyboard

- Click the picture to play or pause. Double-click toggles fullscreen and undoes the pause from the first click.
- Each play/pause change, whatever triggered it (click, Space, button), shows the new state's icon in the middle of the picture: a dark disc that grows and fades out in 600 ms. The state the file opens in shows nothing.
- The wheel over the picture changes the volume.
- After 1 s without mouse movement the controls fade out (200 ms, eased) and the mouse cursor hides; the next move fades the controls back in and brings the cursor back (`gpui_video::hide_cursor_until_mouse_moves`, from the vendored GPUI patch). They stay visible while paused, while a menu or the help is open, or while the mouse is over them.

| Keys | Action |
|---|---|
| Space, K | Play / pause |
| ← → (J L) | −/+ 10 s; with Shift, 1 min |
| Page Up / Page Down | Previous / next chapter |
| 0–9 | Jump to 0–90 % |
| Home | Back to the start |
| ↑ ↓ | Volume ±5 |
| M | Mute |
| A | Next audio track |
| C | Next subtitle track, then off |
| Z / X | Subtitle delay −/+ 0.1 s |
| S | Skip the current part |
| P / N | Previous / next episode |
| F | Fullscreen |
| Esc | Close menu or help, else leave fullscreen, else go back |
| ? | Shortcut help |

Feedback for keys with no visible control (volume, delay, track, chapter) shows as a short message at the top.

## Live streams

A source with `Source::live` (Twitch, [twitch-plan.md](../twitch-plan.md) step 4) plays as a live:
- **No seeking.** There is no seek bar, no ±10 s buttons and no remaining time. The seeking keys (←/→,
  J/L, Page Up/Down, Home, 0–9) are left to the page, and their lines leave the help.
- **The badge.** "EN DIRECT" on the accent stands in for the times, followed by the time since the
  broadcast started and the viewers, when the page gives them (`PlayerView::set_live_info(LiveInfo)`).
  The uptime is redrawn every second while the controls show.
- **mpv's duration is ignored.** It is only the playlist's window, about 25 s. With no duration there
  is no "up next", and `finished()` stays false.
- **Quality change.** Changing quality restarts at the live edge rather than at the current position.
- **End.** The end of the stream (`PlayerEvent::Ended`) is left to the page: the channel went offline.
- **Where the edge is.** `Source::options.start_seconds` is the live edge in a new mpv's timeline, used
  when mpv restarts (resume after a pause, quality change). Twitch's is 0: mpv starts there by itself.
  YouTube's playlists are windows (an hour of 5 s segments) whose first segment is mpv's 0: started at 0,
  mpv waits about 10 s for the audio rendition, so a YouTube live starts `LIVE_DELAY` (15 s) before the
  window's end ([ADR 0011](../decisions/0011-youtube-live-dvr.md)).

### DVR lives (YouTube)

A live with `Source::dvr_window` (YouTube with `isLiveDvrEnabled`) keeps its past, as on youtube.com.
Since [ADR 0025](../decisions/0025-youtube-lives-from-dash.md) its playlists come from a local proxy and
cover the whole broadcast, up to 12 h; the source keeps the proxy alive (`Source::keep_alive`), and the
space shortens the window once the proxy knows how far back the live really goes
(`PlayerView::set_dvr_window`):
- **Seek bar.** It covers the window up to the edge: mpv's duration is the edge and grows with the live,
  the window's start is `duration − dvr_window` (or 0 while the window still grows from the broadcast's
  start). Hovering it shows how far behind the edge a point is (`-5:23`). The ±10 s buttons, the seeking
  keys and their help lines come back. A seek never goes closer than `LIVE_DELAY` to the edge.
- **The badge.** "EN DIRECT" on the accent at the edge (less than `LIVE_DELAY` + 10 s behind); behind it,
  greyed, followed by how far behind (`-2:04`). A click on it goes back to the edge (a seek, mpv keeps
  reloading the playlist; about 2 s).
- **Pause** keeps the position, as on youtube.com (Twitch's live resumes at its edge).
- **Quality change** keeps the distance to the edge: the new mpv's timeline starts at the window's
  current first segment, so it starts at `min(duration, dvr_window) − behind`.
- **No "up next"**, `finished()` stays false, and there are no seek previews (a live's storyboard is
  another kind, not parsed).

Measured on a live (Lofi Girl, 1 h window, 2026-11-02, `spike-yt-play`): playback 0.59 s after load at
`length − 30`; 2 minutes back, playing again in about 1.5 s; back to the edge in about 2 s.

### Recorded lives (Twitch)

A live without DVR whose page gives it a `Rewind` (`PlayerView::set_rewind`: its recording as a `Source`
and when it started; Twitch's archive VOD, [ADR 0026](../decisions/0026-twitch-live-rewind-from-its-recording.md))
gets the DVR controls over a timeline the player computes:
- **Seek bar.** From 0 to `now − started_at`, redrawn every second; on the live, the playhead is as far
  behind that end as mpv is behind the live's edge, and a seek within the live's own window
  (`Source::dvr_window`, Twitch's last 2 minutes, less 5 s) is mpv's own seek, no restart.
- **Back.** A seek older than the live's window plays the recording from there (a new mpv on the quality of the same label, about
  0.7 s), at least `REWIND_EDGE` (30 s) behind the edge: the recording trails the live by a few seconds to a
  segment. Within the recording, seeks are mpv's own.
- **Forward.** From the recording, a seek within `REWIND_EDGE` of the edge goes back to the live (at its
  edge, then that far behind once its duration is known); the badge goes to the edge.
- **Pause.** A pause resumes in place while the live's window still holds where it stopped, else in the
  recording from there.

A live that starts at its edge by itself (Twitch: `start` 0) and keeps a window (`dvr_window`) without a
recording uses the DVR controls, except that going back to the live, and a seek within `EDGE_SNAP` (3 s) of
the edge, restart mpv at its edge (a seek would lose the low latency); a quality change goes to the edge (the
new quality's window starts then); a pause longer than the window resumes at the edge.
- **Events.** Each switch emits `PlayerEvent::Rewound { rewound, at }` (the recording's position); the page
  reads `PlayerView::rewound()`. On the live, `PlayerView::position()` is on the recording's timeline too
  (Twitch saves it as the recording's position).

`PlayerView::set_notice(Option<SharedString>)` puts the page's message over the picture, in place of
"Chargement…". It is generic: Twitch uses it while an ad is removed and no backup stream covers it, with
the countdown it updates itself.

`cargo run -p ui --example player` with `PEEBY_LIVE=1` shows the live controls and a notice counting down;
`PEEBY_LIVE=dvr` shows a DVR live's (its generated source reports what it has read as its duration, so it
stays at its edge); `PEEBY_LIVE=rewind` a recorded live's, which seeks back into its recording at 4 s and
to the edge at 10 s on its own.

## Icons

Lucide SVGs (ISC) are embedded by `crates/peeby/src/assets.rs` and registered with `Application::with_assets`. They include the few icons gpui-component draws itself (chevrons, close, check…), which had no asset source before. Adding an icon means copying the SVG to `crates/peeby/assets/icons/` and listing it.
