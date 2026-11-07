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
  - the balance, under the channel's name for its points, read again every minute; when a
    reading finds it higher, the gain ("+50") rises over it and fades out in 1.6 s (only fades
    with reduced motion; since 2026-11-07);
  - its rewards in a popover;
  - its predictions, read again every 20 s, with their odds and a countdown. As on Twitch, each
    is a bar at the top of the chat, drawn over the messages: folded by default, with its state
    (countdown, the viewer's bet, closed). A click unfolds its card (outcomes, bets). The polls
    share the stack (`ui::chat::pinned_stack`), and only one card is unfolded at a time, so a
    prediction and a poll never cover the chat together. An ended one stays 20 s with its result
    (the winner, the viewer's gain or loss, or "canceled"), read from `resolvedPredictionEvents`.
    Polls come from `viewablePoll` (anonymous, every 10 s) and a free vote from `voteInPoll`
    (one click, with the auth-token; paid votes are never sent). An outgoing raid (Twitch's PubSub,
    [ADR 0024](0024-twitch-raids-pubsub.md)) is pinned first with its countdown; when it goes,
    the page follows it to the raided channel, unless the viewer clicked "Quitter le raid" (in the raid by default, as on Twitch);
    a line along each bar's bottom edge shows the time left to bet, then the time left for the
    result to show (a poll's alike).
  Without the auth-token, or on a channel without points, nothing shows (polls excepted, which
  every viewer sees).
- **Spending** takes two clicks: the button turns into "Confirmer". A reward that asks for a text
  takes it in a field. A bet takes a share of the balance (10 %, 25 %, 50 %, everything), at least
  10 points. Twitch's refusals keep their code (`Error::Refused`) and read in French.
- **The bonus chest is not claimed by Peeby's own calls.** It is claimed in the channel's popout
  chat on twitch.tv, in a hidden WebView2 window (not in the taskbar, never focused) signed in
  with the auth-token (`peeby --twitch-chat <login>`; the user's decision, 2026-11-06). A script
  watches the page's own requests and clicks the "Claim Bonus" button, as 7TV's auto-claim does in
  the browser, once the page has fetched its integrity token (about 10 s after it loads: a claim
  sent before that fails with "failed integrity check", measured 2026-11-06). The window closes
  when Twitch answers the claim, or after 25 s without a button: the claim is twitch.tv's own
  request, with its own integrity token. Twitch's page does not claim on its own (the user's
  check). Accepted on 2026-11-06 (`twitch_capture -- chat-page <login> --claim`).
  - **Automatic** with the "Claim the bonus automatically" setting (`[twitch] auto_claim_bonus`,
    on by default): when a points reading shows a bonus, the claim starts after a random 1 to 2 s,
    at most twice per bonus.
  - **The "Bonus" button** comes in with a bump (it grows past its size and settles) and leaves
    fading as it shrinks; it claims the bonus by hand: greyed out with a spinner beside it while the window
    works, then hidden until the points are read again (2 s later), which shows it again if the
    bonus still waits.
- **A watch-time report** (`twitch::spade`), as the web player sends it:
  - one `minute-watched` event after each minute of the live on the watch page, and only while mpv
    plays it (`core-idle` is `no`: neither paused nor buffering);
  - one live at a time per window, as one per browser tab on twitch.tv: only for the player that
    started last in its window (`ui::player::is_now_playing`), never a channel that is not on
    screen (2026-11-07: lives in two windows each count; it was one for the whole app before);
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
  - **The bonus claim**: clicked in twitch.tv's own chat page, in the window the button opens
    (2026-11-06; whether it claims is for the user to check). Making integrity tokens for Peeby's
    own calls (another client's identity, a token taken from the WebView for a GQL call) stays
    out.
  - **Points for watching**: done, the balance grows. Still to see: a bonus chest after about
    15 minutes, and the button that opens twitch.tv's chat for it.
- Revisit if Twitch requires persisted queries, or an integrity token for the spends.
