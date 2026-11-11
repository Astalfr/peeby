# 0035: Windows and tabs restored at startup

- Status: accepted
- Date: 2026-11-11

## Context

The user asked (2026-11-11, docs/features-plan.md, item 10) that Peeby start again with the
windows and tabs it quit with, as a browser does, with a setting to turn it off. Until then each
space opened its startup tab (the setting "Startup tabs") and every other tab was lost.

What a tab is lives in its space: a history of routes, each space's own `Route` type, built
into a page only when the tab shows (ADR 0019). The shell knows neither the routes nor how to
save them. A watch page that builds starts playing and reports to the account (ADR 0013): a
restored session must not play anything before a click.

## Decision

- Each space turns its routes into text keys and back (`Space::route_key`,
  `Space::route_from_key`, joined with a control character, `space::route_key`): no new
  dependency, and a key a newer Peeby no longer knows is dropped, not an error. The debug-only
  fake chat is not kept.
- A tab is kept as its space's id, the keys of its pages up to the one shown (the forward ones
  are dropped), the title shown and whether that page plays (`SavedTab`); the settings' own tab
  is not kept. A window is kept as its client bounds when not maximized, whether it is maximized,
  the index of its tab shown and its tabs (`session::SavedWindow`), the windows in use (shown or
  hidden in the notification area) in the order they opened, the first being the main window.
- `session.json` beside the settings (`%APPDATA%\peeby\`, `peeby-dev` for a debug build) is
  written every 30 seconds and when Peeby quits (`session::keep_saving`), atomically. The last
  window's close button saves first (`session::save_closing`, the window read from its own
  update): once it is gone, the quit's save finds no window, and a save without windows keeps
  the last session.
- At startup, with "Restore tabs" on (General, on by default) and no page asked on the command
  line, the main window opens at its kept place (on the display holding its center; else at the
  default place) with its tabs, the spaces' startup tabs closed once one came back; the other
  windows open after it. Each tab's pages are unbuilt until the tab shows. The tab shown is the
  one that was, unless its page plays and another does not: then the nearest one before it,
  else after (`shown_first`), so that a video waits for a click on its tab.
- Off, nothing changes: the startup tabs open. The session is written either way, so that
  turning it on brings back the last one.

## Consequences

- Scroll positions, a playlist's queue, a player's position (YouTube keeps it on the account)
  and a page's own state (a search's text) are not kept: only routes.
- A route gains a key when it is added; a test checks YouTube's keys come back.
- When every tab of a window plays, its tab shown still plays at startup.
