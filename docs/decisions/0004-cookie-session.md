# 0004: Signed-in session from browser cookies

- Status: accepted
- Date: 2026-11-01

## Context

Anonymous Home has been empty since 2024 (verified: `logged_in=0`, empty grid, "Votre historique YouTube est désactivé"). Personalized feeds need a Google session. OAuth (the TV device flow) was blocked by YouTube in late 2024 and removed from yt-dlp, so cookies are the only working method.

## Decision

- The user exports youtube.com cookies (Netscape `cookies.txt`) from a **private** window, then closes it so the browser does not rotate them, and imports the file from the app's account page. The import is checked with a Home request (`logged_in=1`) before it replaces the session file; a rejected file changes nothing. "Se déconnecter" deletes the file.
- The file lives at `%APPDATA%\peeby\cookies.txt` (encrypted in `cookies.dat` since [ADR 0022](0022-encrypted-cookie-jar.md)), or `PEEBY_COOKIES` (`%APPDATA%\peebtube\` before the rename to Peeby, ADR 0007; not migrated). A cookie jar is several KB with dozens of values, too large for the Windows Credential Manager that peeby uses for its token.
- WEB requests send `Cookie`, `X-Goog-AuthUser: 0` and `Authorization: SAPISIDHASH … SAPISID1PHASH … SAPISID3PHASH …`.
- With cookies, a response with `logged_in=0` is an error (`Error::NotLoggedIn`), never an empty feed.

## Consequences

- Cookies are secrets: never in logs, URLs or error messages. `Cookies` has a redacting `Debug`.
- **YouTube rotates session cookies** (`SIDCC`, `__Secure-*PSIDTS`…) through `Set-Cookie` on signed-in responses. A client that keeps sending the old values gets the session invalidated: it happened on 2026-11-01, about an hour after the first export, before the updates were handled. Since then `Client` applies the `Set-Cookie` headers of signed-in responses and writes the whole jar back to the file (temporary file + rename). Deletions are ignored. Not yet verified over a long session.
- yt-dlp also writes the jar back when it exits, so the fallback gives it a copy, never the user's file.
- A sign-in WebView now comes first ([ADR 0017](0017-sign-in-webview.md), 2026-11-03): Google allows
  WebView2, and the jar it gives goes through the same Home check. This import stays as the manual way.
- Using cookies goes against YouTube's terms of service. The risk to the account is low for personal use, but not zero.
- The signed-in path was verified on 2026-11-01 (`cargo run -p innertube --example smoke -- <cookies.txt>`: Home, subscriptions, 475 channels).
