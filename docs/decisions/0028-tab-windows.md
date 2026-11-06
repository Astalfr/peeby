# 0028: Tabs dragged along the bar, out to a window of their own and back

- Status: accepted (steps 1 and 2 of 5 done: moving a tab along the bar, several windows)
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
2. **Several windows** (done). The main window becomes one of several shells over the same
   spaces (`shell::windows`):
   - A registry of windows replaces the single main window, and the window used last (the
     last activated) takes what has no window of its own: the notification area's icon, a
     launch's arguments, a toast's click, a live of the list, a tab a space opens without a tab
     to follow (`after: None`), the settings a page asks for.
   - The spaces' events go through one router for the app (`windows::route_events`), not one
     subscription per shell: `Activate(tab)` goes to the window holding the tab, which comes
     to the front (shown again if hidden); `Opened { after: Some(tab) }` to the window holding
     `tab`; the rest to the window used last.
   - A space renders, in each window, the tab that window shows (`Tabs::show(tab, window)`,
     `Tabs::shown_in`; a tab shown in a window leaves the one it was in); `Tabs::shown` stays
     the tab shown last in any window, where a notification opens. YouTube's notification
     opens in its tab of the window used last, else its tab shown last. A tab nobody looks at
     still builds nothing and never plays (ADR 0019); `minute-watched` already follows the
     player that started last (`NowPlaying`), wherever it is.
   - **A window is never destroyed while Peeby runs.** Closed while another is open, its tabs
     close (their pages and videos), as a browser's do, and it hides as a spare that the next
     new window reuses. A page keeps what it was built with: subscriptions through its window
     (`subscribe_in`: the YouTube queue's next video, Jellyfin's player events and their
     account reports), gpui-component's inputs and focus callbacks. A tab moved to another
     window would lose them with the window it was built in; hidden, that window keeps them
     going. What they show through that window (a focus, a gpui-component notification) shows
     there, out of sight: a known limit. A page that replaces itself (the YouTube queue's next
     video, Jellyfin's next episode) is built in the window holding its tab at that moment
     (`windows::in_window_of`), since it takes the focus there and may enter fullscreen; and
     Jellyfin's player events, which send the account reports, no longer go through a window.
   - The last window in use behaves as the single one did: closed, it hides in the
     notification area (setting "Stay in the background"), else Peeby quits. Peeby quits when
     any of its windows really closes, since only the last one does.
   - Each shell has its own mini-player, for its own tabs. The accent is the app's (a global
     theme): it follows the window brought to the front.
   - Until step 3, a tab's right-click menu has "Move to a new window" ("Déplacer dans une
     nouvelle fenêtre", disabled on a window's only tab): the tab leaves with its pages and its
     video (not to the mini-player), into a window of the same size 32 px right and down, or a
     spare.
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
