# 0019: A browser-like history per tab, the settings included

- Status: accepted
- Date: 2026-11-03

## Context

Each tab held a stack of pages (ADR 0015): a sidebar page or the search cleared it, the other
pages were pushed, "back" popped. So "back" often did not lead where the user expected: from a
video opened from Home, then Subscriptions, "back" stopped at Subscriptions, and Home and the
video were gone. There was no "forward". The settings page was the shell's, shown over any tab
and closed by "back", outside of any history.

The user wants a real history within a tab, a forward arrow, and the settings counted as a page.

Constraints: a page with a player holds an mpv instance, and a Jellyfin player reports the
playback stopped when it is dropped (ADR 0009); a YouTube watch page writes to the account while
it plays (ADR 0013). Nothing may play in a tab nobody looks at (ADR 0015). Opening the settings
moves a playing video to the mini-player (ADR 0008) instead of pausing it.

## Decision

- `shell::space::Tab<R>` holds a history: entries and the one shown. An entry is a page of the
  space (`Page`: route, view while built, title) or the settings page. Opening a page drops the
  entries after the one shown and adds it, whatever the page (sidebar pages no longer clear
  anything). "Back" and "forward" move along the entries. A queue's next video and the next
  Jellyfin episode replace the entry shown, so "back" leads to where playback started.
- A page opened in a tab that has shown (`Tab::live`) is built at once: the next episode or the
  next video of a queue while the mini-player shows it, even with another tab or the settings
  on screen. Otherwise `Page::view` stays `None` and the space builds the page shown at its next
  render. This replaces the pending routes of a tab opened in the background, and rebuilds the
  pages reached with "forward" or kept too far behind. Replacing drops the page left before the
  next one is built (one player at a time). A queue's next video or the next episode while the
  settings show replaces the page under them; the settings stay on top.
- Views kept: the pages behind the one shown keep their view (scroll, search, a paused player),
  up to 10 entries back; older ones keep only their route and title. The page left with "back"
  drops its view, as a pop did: its player stops and reports. A sidebar page, the search or a
  page that plays drops the views of the pages with a player behind it (YouTube watch and Shorts,
  Twitch lives, VODs and clips), so a tab holds one player (since 2026-11-13: a video opened from
  another kept the first one loaded, paused); any page opened in Jellyfin drops a player behind
  it. A history keeps at most 50 entries.
- The sidebar lights the last top-level page (the sidebar's, the search) up to the page shown,
  nothing on the settings.
- The settings page is an entry of a tab's history, drawn by the shell (`Space::settings_shown`).
  Since 2026-11-04 the gear (and a page's `SpaceEvent::OpenSettings`) opens it in a tab of its
  own, right after the tab shown (`Space::settings_tab`, `Tabs::add_settings`: a history of the
  settings alone, so no "back"), or shows the tab that already has them; the tab is named
  "Settings" and has the gear as its icon. Leaving the tab shown moves its playing video to the
  mini-player, as any tab switch does. A page opened from the settings (a sign-in) comes after
  them in their tab, and a session change (`Tab::reset`) keeps a tab on the settings there. The
  settings page is made each time it shows.
- The top bar's refresh button (and F5, Ctrl+R: `observe_keystrokes`, so whatever has the
  focus; not in fullscreen) drops the view of the page shown (`Tab::reload`,
  `Space::reload`): the space builds it again from its route at the next render, as after
  "back" and "forward", so its data is read again; on the settings, the shell makes them again.
- The mouse's side buttons do nothing in fullscreen, which belongs to the player.
- `Space` has `type Route`, `history`/`history_mut` and a `cover` hook; `can_go_back`,
  `can_go_forward`, `back`, `forward`, `settings_tab` and `settings_shown` are provided once
  for every space.
- The top bar has a forward button next to back, and the mouse's side buttons go back and
  forward.

## Consequences

- "Back" after a sidebar click returns to the page before it, as in a browser, and "forward"
  undoes a "back".
- A page reached with "forward" is built again: a video starts over from its resume position on
  the account, a Jellyfin title is played again (and reported). A rebuilt watch page of a queue
  plays its video alone.
- A paused player can stay alive behind a page without one (up to 10 entries back, until a
  sidebar page or another player opens), as on the old stack. "Back" to a video left for another
  builds it again: YouTube resumes at this session's last position.
- Revisit if tabs are restored at startup: the routes of each history are what to save.
