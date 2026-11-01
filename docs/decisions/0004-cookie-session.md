# 0004: Signed-in session from browser cookies

- Status: accepted
- Date: 2026-11-01

## Context

Anonymous Home has been empty since 2024 (verified: `logged_in=0`, empty grid, "Votre historique YouTube est désactivé"). Personalized feeds need a Google session. OAuth (the TV device flow) was blocked by YouTube in late 2024 and removed from yt-dlp, so cookies are the only working method.

## Decision

- The user exports youtube.com cookies (Netscape `cookies.txt`) from a **private** window, then closes it so the browser does not rotate them.
- The file lives at `%APPDATA%\peebtube\cookies.txt`, or `PEEBTUBE_COOKIES`. A cookie jar is several KB with dozens of values, too large for the Windows Credential Manager that peeby uses for its token.
- WEB requests send `Cookie`, `X-Goog-AuthUser: 0` and `Authorization: SAPISIDHASH … SAPISID1PHASH … SAPISID3PHASH …`.
- With cookies, a response with `logged_in=0` is an error (`Error::NotLoggedIn`), never an empty feed.

## Consequences

- Cookies are secrets: never in logs, URLs or error messages. `Cookies` has a redacting `Debug`.
- **YouTube rotates session cookies** (`SIDCC`, `__Secure-*PSIDTS`…) through `Set-Cookie` on signed-in responses. A client that keeps sending the old values gets the session invalidated: it happened on 2026-11-01, about an hour after the first export, before the updates were handled. Since then `Client` applies the `Set-Cookie` headers of signed-in responses and writes the whole jar back to the file (temporary file + rename). Deletions are ignored. Not yet verified over a long session.
- yt-dlp also writes the jar back when it exits, so the fallback gives it a copy, never the user's file.
- Later: a sign-in WebView (Grayjay-style), if Google allows it.
- Using cookies goes against YouTube's terms of service. The risk to the account is low for personal use, but not zero.
- The signed-in path was verified on 2026-11-01 (`cargo run -p innertube --example smoke -- <cookies.txt>`: Home, subscriptions, 475 channels).
