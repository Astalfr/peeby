# 0028: Tabs dragged along the bar, out to a window of their own and back

- Status: accepted (step 1 of 5 done: moving a tab along the bar)
- Date: 2026-11-07

## Context

The user wants tabs as in a browser: hold a tab and move it along the bar, pull it out of the
bar to open it in a new window, and drop a tab on another window's bar to merge them.

Peeby has one main window. Its root, `Shell`, keeps the bar's order of tabs (`TabRef`s); the
tabs themselves, their histories and pages belong to the spaces, one entity per service for the
whole app (ADR 0015). That part already fits several windows: a second `Shell` over the same
spaces would show other tabs of them, and the session (the rotated cookies, Twitch's single-use
refresh token) would still exist once. A playing video already moves between windows without
restarting: the mini-player draws the page's own `VideoView` (ADR 0008).

What assumes one window:

- the app quits when the main window closes (`main.rs`); the notification area icon, the
  single-instance launches, closing to the notification area, the mini-player and F5 all reach
  that window;
- `SpaceEvent`s (a tab opened, a tab to show, the settings) go to the one shell;
- a space keeps one "tab it shows", while two windows could each show a tab of the same space.

GPUI's drag and drop (`on_drag`, `on_drop`) works inside a window only, and ends at its edge.
The patched GPUI captures the mouse while a button is held (`SetCapture` in
`vendor/gpui-pre-windows/src/events.rs`), so the window still gets the moves and the release
anywhere on the screen.

## Decision

The drag is Peeby's own: a tab's left button down records it (`shell::tab_drag`), and window
listeners, registered at each paint while the button is held (as the mini-player's move and the
player's seek bar do), follow the pointer and the release over the whole screen. Built in
five steps, each one shippable:

1. **Along the bar** (done). Past 4 px, the tab shows (as a browser does), its slot stays empty
   and a copy of it follows the pointer's x, opaque and over the others, kept over the row of
   tabs. Each canvas measures its tab at layout (`TabLayout`); when the copy's middle passes a
   neighbor's, the tab moves to after it in `Shell::tabs`. The release leaves it there. Nothing
   is saved: the tabs are not restored at startup yet.
2. **Several windows.** The main window becomes one of several shells over the same spaces. A
   registry of windows replaces the single main window: the app quits from the notification
   area only, closing the last window hides it as today, the icon, the launches and the
   notifications go to the window used last. `SpaceEvent`s go to the window holding the tab
   (a tab opened from a tab joins its window). A space keeps the tab it shows per window, so a
   tab nobody looks at still builds nothing and never plays (ADR 0019). The mini-player follows
   the window whose page it shows.
3. **Out to a new window.** A tab pulled out of the bar (vertically past its height) and released
   off every window's bar opens a window at the pointer with that tab, its page and player kept.
   The window it leaves needs a tab left: the last one does not tear out.
4. **Back into a window.** Released over another window's bar, at the slot the pointer is over
   (the screen's coordinates against each window's bar); a window whose last tab leaves closes.
5. **The window follows** (optional): once torn out, the new window moves with the pointer until
   the release, as in Chrome, through `SetWindowPos` in `desktop::sys`.

## Consequences

- Reordering needs no state beyond the bar, and steps 3 and 4 reuse its listeners for the release
  position.
- Step 2 touches the start, the notification area and the mini-player: each later step depends
  on it, and it is the one to test first in the app.
- The tabs passed slide to their new place in 150 ms (ease out), drawn shifted by the distance
  they jumped and easing back (`tab_drag::Slide`); the released tab slides from the copy into
  its slot. None with reduced motion.
- Restoring the tabs at startup (roadmap) will have to save each window's tabs and bounds.
