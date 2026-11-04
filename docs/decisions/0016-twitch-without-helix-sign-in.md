# 0016: Twitch without the Helix sign-in

- Status: accepted (the user, 2026-11-03)
- Date: 2026-11-03

## Context

ADR 0010 made the Helix sign-in (Device Code Grant) required: a public client has no app token, so
the catalog needed the user's token, and signed out the space showed only its sign-in card. The web
`auth-token`, optional, served playback (Turbo, subscriptions, subscriber-only VODs), and later the
channel points (ADR 0012) and the VOD positions (ADR 0013).

The user asked whether the Helix app was needed at all with the auth-token. Raw GQL queries were
probed on 2026-11-03, anonymously and with the auth-token ([twitch.md](../twitch.md#catalog-through-gql-instead-of-helix)):

- users, searches and chat badges answer in full, anonymously;
- top categories, a category's streams, a channel's VODs and clips answer their first page (up to
  100 items), and their second page (`after`) fails Twitch's integrity check;
- the follows (`currentUser { follows }`, `followedLiveUsers`, the sidebar's followed section)
  answer `service error` with a valid auth-token. Twitch's own pages send a device id Peeby does
  not have; making one up is routing around a bot check, so this was not pursued.

Peeby's pages only ever used one page of each list, but "Parcourir" (two pages, 200 categories).

## Decision

- **The space opens without signing in.** Its catalog goes through anonymous GQL
  (`twitch::catalog::Catalog`, raw queries, the first page of each list), with Helix's types as the
  shared model. Anonymous on purpose: a refused auth-token never stops the browsing.
- **The auth-token is the sign-in** (read from twitch.tv's sign-in in a WebView since
  [ADR 0017](0017-sign-in-webview.md), the tutorial as the manual way). It names the account entry (`currentUser`), plays as the
  user's account and carries the points and VOD positions, as before. The settings page asks for it
  first ("Compte"); "Se déconnecter" in the sidebar forgets it and disconnects Helix.
- **Helix is optional**, for the followed channels only ("Suivies", the sidebar's lives, the bell),
  connected from the "Suivies" page, which shows the Device Code card until then. Since
  2026-11-04 (the user), it comes after the account: signed out (no auth-token, or one Twitch
  rejects), "Suivies" shows the web sign-in card (ADR 0017) instead, the sidebar's lives and the
  bell stay off, and a saved Helix session is restored only once an auth-token is there. Signing
  out of the account disconnects Helix too. When connected,
  it is also the catalog's backup: a GQL failure (integrity check, a GQL error, an unexpected
  answer) asks Helix instead. Its session keeps ADR 0010's rules (single-use refresh tokens,
  validated every hour); its expiry only takes the followed channels away.
- A new tab opens on "Suivies" with Helix (or while its session is restored) and the account
  signed in, else on "Parcourir".

## Consequences

- Nothing is needed to browse and play: a first launch shows Twitch at once.
- "Parcourir" lists 100 categories instead of 200.
- The catalog now depends on GQL's raw queries, which Twitch may change or close like the follows.
  The Helix backup covers that for connected users; the others lose the catalog until Peeby
  updates. Fixtures (`gql_catalog_*.json`) and `smoke` watch for it.
- Signed in with the auth-token only, there is no "Suivies" list. Revisit if a raw GQL query for
  the follows is found that needs no integrity token, or if Twitch closes the catalog's queries
  (probed again on 2026-11-04, still none: [twitch.md](../twitch.md#catalog-through-gql-instead-of-helix)).
- A user who had connected Helix without the auth-token loses "Suivies" until they sign in to
  the account (their Helix session is kept, unspent, and comes back then).
- Supersedes ADR 0010's third answer ("Sign-in required").
