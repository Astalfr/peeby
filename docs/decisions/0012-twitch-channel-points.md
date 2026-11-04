# 0012: Twitch channel points

- Status: accepted for the balance, the spends and the watch-time report (2026-11-02, after the
  user's measurement); proposed for the bonus chest, claimed on twitch.tv, until the user confirms
- Date: 2026-11-02

## Context

The user asked for channel points on Twitch. They chose this scope: the balance, the bonus chest
claimed automatically, points gained by watching, and spending (rewards and predictions). Until now
[twitch-plan.md](../twitch-plan.md) kept every write to the account out of scope. These features
write to the account.

Helix only exposes channel points to the broadcaster. A viewer's points live in GQL, with the web
`auth-token` (credential B in [twitch.md](../twitch.md#credentials)). Probed on 2026-11-02 with the
web Client-ID and the user's auth-token (details in twitch.md, "Channel points"):

- **Reads** answer to raw queries, with no persisted hash: the balance, the waiting bonus
  (`availableClaim`), the custom rewards, and the open and locked predictions.
- **Spends** pass. `redeemCommunityPointsCustomReward` and `makePrediction`, sent with made-up ids,
  answer `NOT_FOUND`, not an integrity error.
- **The bonus claim** is refused. `claimCommunityPoints` answers `failed integrity check`
  (`IntegrityCheckFailed`, `challenge: integrity`), even for a made-up claim id. Twitch-Channel-Points-Miner
  sends it under another Client-ID with a token from that client's own sign-in. Using a different
  client's identity to get past an integrity check is circumventing an anti-bot measure: Peeby
  does not do it.
- **Points for watching**: the user played a live in Peeby for 20 minutes (2026-11-02), and the
  balance did not move. Twitch credits time watched from the web player's `minute-watched` events
  (spade), not from the stream being fetched: with them (and `location: "channel"`), the balance
  grew by 10 points (the user, 2026-11-02). Point farmers send those events for lives nobody
  watches. Peeby sends them only for the live on screen, while it plays.

## Decision

- **`twitch::points`** holds the parsers and requests, tested on saved answers. It uses raw GQL
  queries and mutations with the web Client-ID and the auth-token. There is no hash to maintain.
- **On the watch page of a live** (since [ADR 0023](0023-twitch-chat-sending.md), the balance and its
  rewards sit under the chat's field; since 2026-11-04, the predictions are pinned over the chat):
  - the balance, under the channel's name for its points, read again every minute;
  - its rewards in a popover;
  - its predictions, read again every 20 s, with their odds and a countdown. As on Twitch, each
    is a bar at the top of the chat, drawn over the messages: folded by default, with its state
    (countdown, the viewer's bet, closed). A click unfolds its card (outcomes, bets). The polls
    share the stack (`ui::chat::pinned_stack`), and only one card is unfolded at a time, so a
    prediction and a poll never cover the chat together. An ended one stays 20 s with its result
    (the winner, the viewer's gain or loss, or "canceled"), read from `resolvedPredictionEvents`;
    a line along each bar's bottom edge shows the time left to bet.
  Without the auth-token, or on a channel without points, nothing shows (polls excepted, which
  every viewer sees).
- **Spending** takes two clicks: the button turns into "Confirmer". A reward that asks for a text
  takes it in a field. A bet takes a share of the balance (10 %, 25 %, 50 %, everything), at least
  10 points. Twitch's refusals keep their code (`Error::Refused`) and read in French.
- **The bonus chest is not claimed by Peeby.** When one waits, a "Bonus" button opens the channel's
  chat on twitch.tv, where it is claimed with one click.
- **A watch-time report** (`twitch::spade`), as the web player sends it:
  - one `minute-watched` event after each minute of the live on the watch page, and only while mpv
    plays it (`core-idle` is `no`: neither paused nor buffering);
  - one live at a time: only for the player that started last (`ui::player::now_playing`), never a
    channel that is not on screen;
  - it carries the channel id, the live's id and the viewer's id (read with the points), never logged;
  - the endpoint is `spade_url`, read once from twitch.tv's settings script.
- Polling, not PubSub: one context request a minute and one predictions request every 20 s per open
  live. No new socket and no new dependency. Transaction ids come from the `uuid` crate that `peeby`
  already uses.

## Consequences

- The user's points can be spent from Peeby. Two clicks guard every spend, and nothing spends on its
  own.
- If Twitch adds an integrity check to the spends, they fail with "Twitch demande une vérification
  que seul son site peut faire".
- Open, for the user to decide:
  - **The bonus claim**: the user claims it on twitch.tv's own page, which the button opens. Making
    integrity tokens for Peeby's own calls (a hidden WebView2, another client's identity) would be
    the same bypass, so it is not an option.
  - **Points for watching**: done, the balance grows. Still to see: a bonus chest after about
    15 minutes, and the button that opens twitch.tv's chat for it.
- Revisit if Twitch requires persisted queries, or an integrity token for the spends.
