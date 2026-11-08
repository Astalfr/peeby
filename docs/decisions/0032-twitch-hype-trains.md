# 0032: A live's Hype Train from GQL and PubSub

- Status: accepted
- Date: 2026-11-08

## Context

Twitch pins a channel's Hype Train at the top of its chat: a bar in the channel's color filled
with the level's progress, the level, the share done and the level's countdown; unfolded, the
level's emotes, the conductors (top contributor of subs and of bits) and the channel's record.
Before a train starts, "N more contributions" in the same place; at its end, the level reached.
Peeby should show the same over a live's chat, beside the raid, predictions and polls.

- **GQL gives the state, anonymous.** `user(login) { channel { hypeTrain { approaching
  execution } } }` (raw query, checked on 2026-11-08 against GQL's validation; introspection is
  off): the train's level, the level's progress and goal, the total, the level's end
  (`expiresAt`), the channel's color, the level's emotes, the conductors, the record
  (`allTimeHigh`), the Golden Kappa and Treasure flags; the approach's goal, participants and end.
  Read every few seconds, it would lag behind contributions and cost a request each time.
- **PubSub gives every move, anonymous.** `hype-train-events-v1.<channel id>` takes an anonymous
  `LISTEN` on `wss://pubsub-edge.twitch.tv/v1`, as `raid.<id>` does (ADR 0024). Listening to 500
  lives for 30 minutes (2026-11-08, `raid_probe --topic hype-train-events-v1`) gave 13
  approaches, 12 starts, 42 contributions, 15 level-ups, 26 conductor changes and 10 ends, each
  with the train's progress. It says only what changes: a page opened mid-train needs GQL first.
- PubSub closed the connection (code 1009) on a `LISTEN` of 50 Hype Train topics: frames must stay
  small.

## Decision

- `twitch::pubsub::listen(channel_id)` replaces `raids::listen`: one anonymous connection per live
  watched for both topics, one `LISTEN` each; it says when it is listening (again).
- `twitch::hype_train` reads GQL's state (`parse_hype_train`) and the topic's messages
  (`parse_message`); `State::apply` takes a message in (another train's are dropped, a late
  progress moves nothing back), `State::merge` a GQL reading (the topic's progress kept when
  further along), `State::settle` ends a train 10 s past its level's end when its end message was
  missed and drops an expired approach.
- `points::PointsView` reads GQL each time the connection listens (its start, each reconnection),
  then follows the topic. The train is pinned after the raid, before the predictions
  (`space-twitch/src/hype_train.rs`): `ui::chat::Pinned` gains a `fill`, the bar's progress in a
  tint of its accent, as Twitch's banner; the channel's color (gold for a Golden Kappa Train,
  Twitch's purple without one), its level in a badge of that color, the share and the countdown,
  the level's time along its edge. An ended train stays 30 s with the levels it completed and the
  emotes it unlocked.
- Nothing is sent: Peeby does not subscribe or cheer. The fake chat drives the same view.

## Consequences

- A train shows within a second of each contribution, with Twitch's own countdown.
- No new connection: the raid's carries the train.
- PubSub is closed to third parties: if Twitch stops answering it, the train still shows from the
  page's first GQL reading but no longer moves (and ends 10 s after its first level's end).
  Revisit then with whatever Twitch's web player uses, or a GQL reading every few seconds while a
  train runs.
- Shared trains (`sharedHypeTrainDetails`) and Treasure Trains' discounts are not detailed yet: none
  was seen.
