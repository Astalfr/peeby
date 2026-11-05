# 0024: A live's outgoing raid from Twitch's PubSub

- Status: accepted
- Date: 2026-11-05

## Context

When a channel raids another, Twitch's web player shows "X is raiding Y" with a countdown, then
takes its viewers to Y's live unless they leave. Peeby should do the same on a live's watch page.

- **GQL does not show it to viewers.** `user(login) { raid }` exists (`id`, `viewerCount`,
  `targetChannel`, `sourceChannel`), but it stayed null on about 2,600 lives scanned 52 times over
  two hours, anonymous and with the auth-token, without an error. It is very likely the
  broadcaster's own view. No field gives the countdown either.
- **PubSub does.** `wss://pubsub-edge.twitch.tv/v1` still answers, although Twitch closed PubSub to
  third-party apps in 2025. It takes an anonymous `LISTEN` on `raid.<channel id>` (empty `error`).
  Listening to 500 top lives for 30 minutes (2026-11-05, `twitch` example `raid_probe`) gave 13
  raids, each a `raid_update_v2` at its announcement and again as viewers join it, then
  `raid_go_v2` (several times) when it leaves. They carry the target's login, name and avatar,
  `viewer_count`, `raid_created_at` and `force_raid_now_seconds` (90). No cancel was seen.
- **The other options:** reading GQL every 10 s and guessing the departure from the end of the
  live (it never sees a raid); IRC (the source's chat says nothing; the target's
  `USERNOTICE msg-id=raid` comes after the raid).

## Decision

- `twitch::raids::listen(channel_id)` holds one anonymous PubSub connection for the live on
  screen: `LISTEN raid.<id>`, a `PING` every 4 minutes (15 s for its `PONG`), `RECONNECT`
  followed, reconnection with a growing delay (1 to 30 s). It ends when its receiver is dropped,
  that is when the watch page goes. No token is ever sent on it.
- `points::PointsView` starts it once the live's channel id is known. It pins the raid first over
  the chat (`ui::chat::pinned_stack`), its countdown (`raid_created_at` +
  `force_raid_now_seconds`) along the bar. On `raid_go_v2`, once per raid, the page opens the
  target's live in its tab, unless the viewer clicked "Quitter le raid": the viewer is in the raid by default, as on Twitch, and its card's one button leaves it (then "Rejoindre le raid"). A cancel (`raid_cancel_v2`,
  unconfirmed) removes the bar.
- Only raid topics are listened to: the other PubSub topics (points, predictions) stay on GQL.

## Consequences

- The announcement arrives within a second, with the real countdown and departure.
- One more connection to Twitch per live watched, outside the GQL and IRC ones.
- PubSub is a closed service for third parties: Twitch may stop answering it. Then the bar never
  shows (raids still arrive in the target's chat as notices); revisit with Twitch's EventSub
  (`channel.raid`, which needs a token) or whatever the web player uses then.
- The cancel message's name is a guess: confirm it on the first canceled raid seen.
