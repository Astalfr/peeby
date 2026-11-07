# 0030: 7TV emote sets kept current by its EventAPI

- Status: accepted
- Date: 2026-11-07

## Context

A live's chat loads the channel's 7TV set once (docs/twitch.md, "7TV"), so an emote the
streamer adds while the viewer watches stayed text until the gear's "Reload 7TV emotes"
(2026-11-07). 7TV pushes set changes over its EventAPI (`wss://events.7tv.io/v3`), which its
browser extension uses. Checked on 2026-11-07 with the `seventv_events` example: anonymous,
Hello gives a heartbeat of ~48 s and 500 subscriptions a connection, and an added emote arrives
as the very set entry the REST answer has. The EventAPI's repository is archived, so what was not
seen (removals, renames, close codes, resume) comes from its README.

Options: poll the REST sets (a request per channel every few minutes, late, for changes that are
rare); follow the EventAPI and apply its ChangeMaps; follow it and refetch the set on any change.

## Decision

A live's chat opens one EventAPI connection once its extras have loaded, subscribed to the
global set, the channel's set and the channel's 7TV user (`twitch::seventv::events`, on the
existing `tokio-tungstenite`). Added, removed and renamed emotes are applied to the session's copy
of the set, and the chat's words and picker are rebuilt from the copies without a request.
Whatever the connection cannot vouch for asks 7TV's REST again, as the gear does: a new
connection after the first (no resume: its failure is undocumented, and a refetch covers what was
missed), a change it cannot read, a `user.update` (the channel may have switched sets; the new set
is then watched). Close codes that name a client bug stop the connection; maintenance waits at
least 5 minutes. The gear's reload stays.

## Consequences

- One more anonymous WebSocket per open live chat, to a third party that already serves the
  emotes; it closes with the page. Nothing is sent but subscriptions: no account, no token.
- A rename or a removal relies on 7TV's documented ChangeMap until one is seen; a shape that does
  not match turns into a refetch, not a wrong set.
- Several tabs on one channel apply the same change to the same copy: `Set::apply` is idempotent.
- Revisit if 7TV retires the v3 EventAPI, or if many open lives hit its connection limits.
