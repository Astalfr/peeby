# 0012: Twitch channel points

- Status: proposed (narrower than the scope the user chose on 2026-11-02: the bonus is not claimed
  automatically and watching earns nothing more than before; waiting for the user's answer)
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
- **Points for watching**: whether Twitch credits time watched through Peeby is unknown. The web
  player reports it with `minute-watched` events (spade). Faking those events is how point farmers
  work, so they wait for a decision.

## Decision

- **`twitch::points`** holds the parsers and requests, tested on saved answers. It uses raw GQL
  queries and mutations with the web Client-ID and the auth-token. There is no hash to maintain.
- **On the watch page of a live**, at the header's right:
  - the balance, under the channel's name for its points, read again every minute;
  - its rewards in a popover;
  - its predictions, read again every 20 s, with their odds and a countdown.
  Without the auth-token, or on a channel without points, nothing shows.
- **Spending** takes two clicks: the button turns into "Confirmer". A reward that asks for a text
  takes it in a field. A bet takes a share of the balance (10 %, 25 %, 50 %, everything), at least
  10 points. Twitch's refusals keep their code (`Error::Refused`) and read in French.
- **The bonus chest is not claimed by Peeby.** When one waits, a "Bonus" button opens the channel's
  chat on twitch.tv, where it is claimed with one click.
- **No spade events.** Peeby only plays the stream, as before.
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
  - **Points for watching**: measure first whether the balance grows while Peeby plays a live. If it
    does not, decide whether to send the web player's `minute-watched` events for the live actually
    on screen.
- Revisit if Twitch requires persisted queries, or an integrity token for the spends.
