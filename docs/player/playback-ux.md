# Playback page

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

What the player page does beyond play/pause. The code is in `crates/peeby/src/player_page.rs`. The pure logic lives in `segments.rs` (skippable parts) and `tracks.rs` (track mapping and carry-over), both covered by unit tests.

## Skip buttons: opening, recap, credits

A **Skip intro / Skip recap / Skip credits / Skip preview** button appears bottom right while one of those parts plays. It stays visible when the other controls hide. `S` does the same.

Where the parts come from ([ADR 0009](../decisions/peeby/0009-segments-from-chapters.md)):

1. **Server media segments** (`GET /MediaSegments/{itemId}`), when the server has any. They need a segment provider plugin; the test server has none.
2. Otherwise, **chapter titles** from mpv's `chapter-list`. Fansub and BD releases name them:

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

## Audio and subtitles carried across episodes

When the user changes tracks, the page records the choice for the series, for the rest of the session (`Browser::remember_tracks`). The next episode of that series starts with the stream in the same language, preferably with the same title and forced flag. This keeps `Full` subtitles from turning back into `Signs & Songs`. When the file has no match, the server defaults apply. Those follow the user's language settings on the server.

## Seek bar

- **Shape.** A rounded track (5 px, 8 px on hover). The amber progress and the lighter hover fill carry the same radius and are never narrower than the track is tall, because GPUI clips children to rectangles and square fill ends would stick out of the rounded track. On hover, a 14 px amber knob with a white ring marks the playhead.

- **Chapter marks** are drawn as gaps in the bar. The chapters menu lists them with their start time, and Page Up / Page Down jump between them.
- **Hover** shows the time and chapter under the mouse. When the server has trickplay thumbnails (`BaseItemDto.Trickplay`), the matching thumbnail appears too: the tile image (`GET /Videos/{id}/Trickplay/{width}/{tile}.jpg`) comes through the image store, and the thumbnail is shown by offsetting the tile inside a clipped box. **Untested against a server**: the test server generates no trickplay, so only the cell arithmetic has tests.

## Control bar

Left to right: previous episode, back, play/pause, forward, next episode, then the time (elapsed / total). The remaining time (`-12:34`, once the duration is known) sits on the right, just left of the volume, chapters, audio, subtitles, shortcuts and fullscreen buttons.

Player buttons (`control_button`, and the window buttons) are 40 px with 20 px white icons, larger than the 32 px and 16 px used elsewhere, since the screen is watched from further away. On hover the icon turns amber (primary) over a light background (white at 18 %, a little stronger than the app's ghost buttons); the window Close button turns red with a white icon. gpui-component's buttons keep one foreground for every state, so the page tracks the hovered button (`hovered_control`) to color the icon.

The volume shows no number. Hovering the mute button opens a vertical slider right above it (click or drag; bottom 0 %, top 100 %), which stays open while the mouse is over the button or the slider, and keeps the controls from hiding meanwhile. A drag started on the slider keeps going until the button is released, even when the mouse leaves the slider: it stays open and keeps following the mouse height. It opens with a 150 ms fade and 8 px rise (ease-out) and closes with the same animation played backwards; going straight from the button to the slider does not replay it. A change shows the value briefly at the top.

## Mouse and keyboard

- Click the picture to play or pause. Double-click toggles fullscreen and undoes the pause from the first click.
- Each play/pause change, whatever triggered it (click, Space, button), shows the new state's icon in the middle of the picture: a dark disc that grows and fades out in 600 ms. The state the file opens in shows nothing.
- The wheel over the picture changes the volume.
- The controls and the mouse cursor hide after 3 s without mouse movement, and the cursor comes back on the next move (`gpui_video::hide_cursor_until_mouse_moves`, from the vendored GPUI patch). They stay visible while paused, while a menu or the help is open, or while the mouse is over them.

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

## Icons

Lucide SVGs (ISC) are embedded by `crates/peeby/src/assets.rs` and registered with `Application::with_assets`. They include the few icons gpui-component draws itself (chevrons, close, check…), which had no asset source before. Adding an icon means copying the SVG to `crates/peeby/assets/icons/` and listing it.
