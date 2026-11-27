# 0043: Twitch clips through Helix, with a button only when Helix is connected

- Status: accepted
- Date: 2026-11-27

## Context

twitch.tv's player has a Clip button (Alt+X). Peeby's Twitch space had no way to make one.

- GQL's clip mutation sits behind the integrity check ADR 0012 forbids routing around.
- Helix has two endpoints ([twitch.md](../twitch.md), "Helix"): `POST /helix/clips` (a live, `clips:edit`)
  and `POST /helix/videos/clips` (a VOD, `editor:manage:clips`, reserved to the channel and its
  editors, which a viewer is not: 403 captured on 2026-11-27).
- The Device Code Grant approves a fixed list of scopes. Sessions approved before have only
  `user:read:follows`, and Helix answers a missing scope with a **401**, as for a stale token.

## Decision

- Helix only. `auth::SCOPES` grows to `user:read:follows clips:edit editor:manage:clips`.
- The button (and Alt+X) is a generic page action of the shared player (`ui::player::ControlAction`),
  which knows no service; the Twitch watch page sets it only while the space has a Helix session
  (`TwitchSpace::clip_access()` is not `NoHelix`), and follows the space's notifications to add or
  remove it without reloading the page. Not on a clip, not on a multistream's secondary tile.
- A session without the new scopes keeps the button: a click says it needs a new approval, with a
  "Reconnect" button. "Reconnect" revokes the Helix session (a new approval is the only way to
  add scopes) and opens the approval card. The scopes are compared before the call.
- `Helix::send` tells a 401 about a *scope* (`Error::MissingScope`) from a stale token: only the
  latter refreshes the token.
- A live: create, wait for the clip (1 s apart, 15 tries), copy `clips.twitch.tv/<slug>`, open
  `edit_url`, and a toast offers "Open" (a Peeby tab) and "Edit" (twitch.tv). A VOD: try
  `videos/clips` ending at the playhead (at least 30 s in); on the editor refusal, open the VOD on
  twitch.tv at the playhead with a toast saying why.

## Consequences

- Sessions approved before need one new approval to clip; the settings page says so.
- A test creates a public clip that only the creator dashboard can delete: ask first.
- The VOD success was never seen (no editor rights on a tested channel); the Helix error text
  decides the fallback, so a changed message would turn the fallback into an error toast.
- Alt+X depends on Windows delivering Alt+letter (`WM_SYSKEYDOWN`) to GPUI: not verified.
