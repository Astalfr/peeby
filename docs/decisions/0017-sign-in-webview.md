# 0017: Sign-in through a WebView

- Status: accepted (the user, 2026-11-03)
- Date: 2026-11-03

## Context

Both signed-in sessions were set up by hand:

- **YouTube** (ADR 0004): export youtube.com's cookies from a private window with a browser
  extension, close the window, import the file.
- **Twitch** (ADR 0016): open the developer tools on twitch.tv and copy the `auth-token` cookie
  (the tutorial of [twitch.md](../twitch.md#the-auth-token-tutorial)).

ADR 0004 left "a sign-in WebView, if Google allows it" for later: Google sometimes refuses
embedded browsers ("This browser or app may not be secure"). A spike checked it on 2026-11-03,
with WebView2 (Edge's engine, part of Windows) through `wry`, InPrivate, its cookies read from
WebView2's cookie manager (HttpOnly ones included):

- **Google** let the user sign in with no rejection (an address prompt on the way). The
  youtube.com cookies it set (19, the whole `SAPISID`/`PSID`/`PSIDTS` family) passed
  `innertube --example smoke`: `logged_in=true`, every signed-in endpoint.
- **Twitch**: twitch.tv's sign-in went through, and its `auth-token` cookie answered GQL
  `currentUser`.

## Decision

- **The sign-in window is the first way, the manual one stays.** YouTube's account and Twitch's
  "Compte" settings show "Se connecter à …" first; the cookie import and the auth-token tutorial
  stay below it, always available, and are what a failure points to (WebView2 missing, the
  window failing, Google refusing someday).
- **A separate process.** Peeby starts itself again as `peeby --login youtube|twitch`
  (`web_sign_in`), before any GPUI code runs: tao's event loop and GPUI's cannot share the main
  thread. The window shows the service's own sign-in page (the password goes to Google or Twitch
  only), polls the profile's cookies every second, and once the session is there writes it on
  stdout and exits: YouTube's youtube.com cookies as a Netscape file, Twitch's `auth-token`. Exit
  codes: 0 signed in, 2 closed by the user, 3 unavailable.
- **Checked like a manual import.** The session goes through the same check as before (YouTube:
  a Home request with `logged_in=1`, then the cookie jar, encrypted since [ADR 0022](0022-encrypted-cookie-jar.md); Twitch: the token's shape and
  `currentUser`, then the Credential Manager): a refused session changes nothing.
- **Read once, then closed.** InPrivate, with its profile in a temporary folder deleted on
  exit, so nothing keeps rotating the cookies after Peeby took them (the private window of
  ADR 0004, closed). Peeby keeps the child's stdin open; closing it (the settings page dropped,
  Peeby quit) closes the window.
- **Only the sign-in.** The window is never kept, hidden or reused for integrity tokens, a device
  id or anything Twitch's or Google's checks would otherwise refuse (ADRs 0012, 0016). One
  exception, since 2026-11-06 (ADR 0012): `peeby --twitch-chat <login>` shows a channel's popout
  chat on twitch.tv, hidden and out of the taskbar (`PEEBY_CLAIM_VISIBLE` shows it, to debug),
  InPrivate in a temporary profile, the auth-token given on stdin and set as its cookie; a script
  clicks its "Claim Bonus" button, and the window closes. Nothing is read back from it but
  whether the button was there (its exit code).
- Secrets never reach a log: the child logs pages at debug level, host and path only (sign-in
  URLs carry state in their query), and Peeby discards its stderr.

## Consequences

- Signing in takes a password and a click instead of an extension or the developer tools.
- New dependencies in `peeby`: `wry` (WebView2) and `tao` (its window). No `unsafe` in Peeby:
  `wry` holds the COM calls.
- WebView2 must be on the host: it is part of Windows 11 and of current Windows 10; without it,
  the manual ways remain.
- The `cookie` crate drops the leading dot of a domain, so every cookie is written as a domain
  cookie (`.youtube.com`, subdomains included). The signed-in smoke passed that way; reading the
  raw domain would mean COM calls in Peeby.
- Google may start refusing WebView2: the cookie import stays for that day. Revisit if it does.
- Not verified: a long session started this way (it holds the same cookies as an export), a
  Windows 10 host, an account with stricter two-step verification.
