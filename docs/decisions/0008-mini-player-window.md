# 0008: The mini-player is a window of its own, above the others

- Status: accepted
- Date: 2026-11-01

## Context

With one tab per space, switching tabs while a video plays has to do something with that video. The user
chose a mini-player, and asked for it as in Chrome's picture-in-picture: outside the main window and above
every other window. Restarting mpv in another view would cost 1–2 s and the position; the video surface is
an external D3D11 texture drawn by the patched GPUI.

## Decision

- The shell opens a second GPUI window of kind `WindowKind::PopUp` (on Windows `WS_EX_TOPMOST |
  WS_EX_TOOLWINDOW`, no frame, no taskbar entry), 400×225 at the bottom right of the main window's screen,
  without taking the focus. It draws the **same** `PlayerView`'s `VideoView`: the player is not restarted and
  the picture stays zero-copy (checked first with `PEEBY_PIP=1 spike-yt-play`).
- It opens when the page that plays is left for another tab or the settings, and only if the video plays. It
  closes when that page shows again (its tab, or its "Revenir à la page" button, which also brings the main
  window to the front); its "×" pauses the video and closes it.
- It holds no player: it asks its space for the current one at each render, so a queue moving to its next
  video carries on in it. A `NowPlaying` global (in `ui::player`) keeps one sound at a time: a player that
  starts pauses the previous one, and the mini-player closes if its player is no longer the one playing.
- Dragging the video moves the window, and its edges and corners (invisible, 6 px; 12 px corners) resize it
  at 16:9, the opposite side staying in place (the middle, along the axis no edge holds). Both follow the mouse
  in screen coordinates and set the bounds with `gpui_windows::set_window_bounds` (a small addition to the
  patched GPUI, ADR 0001), run as a task: from inside an update, even a `cx.defer`, the app is still borrowed
  when `SetWindowPos` sends `WM_SIZE`. The move is not a caption area
  (`WindowControlArea::Drag`): crossing between caption and client areas resets the window's hover.
- On hover: play/pause in the middle, the two buttons at the top right. Whether the controls show is the
  window's hover state, not an element's: the buttons occlude the element and its hover would flicker.

## Consequences

- The app quits with its main window only (`on_window_closed` checks the window id). Since ADR
  0028, with any of its shells' windows (only the last one ever closes), and each window has its
  own mini-player for its own tabs.
- `gpui-video` releases replaced frames from every window, since a view can move between windows (ADR 0001).
- A bounds given to a new window must come with its `display_id`, or Windows moves it to the primary screen
  at a default size (CLAUDE.md, Gotchas).
- Closing the window from its own button is deferred (`cx.defer`): a window cannot be updated from inside
  its own event. Moving or resizing it needs more: a task (see above).
- `gpui-pre` 0.3.10 cannot set a window's position (`Window::resize` keeps the top left corner and
  `start_window_resize` does nothing on Windows), hence `set_window_bounds`. No Aero snap for the mini-player,
  which a picture-in-picture window does not need.
- Jellyfin's player, which hid the whole shell in peeby, plays in the content panel like YouTube's (agreed with
  the user on 2026-11-01): the tabs and the mini-player stay reachable, and F still gives fullscreen.
