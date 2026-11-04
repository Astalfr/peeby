# 0022: Cookie jar encrypted with DPAPI

- Status: accepted
- Date: 2026-11-04

## Context

The YouTube session is a whole browser cookie jar ([ADR 0004](0004-cookie-session.md)), kept until
now in clear in `%APPDATA%\peeby\cookies.txt`. Whoever gets a copy of that file (a backup, a
synced folder, another Windows account, a disk read offline) gets the Google session.

Options considered:

- **Credential Manager**, like the Jellyfin and Twitch tokens: a secret holds at most 2,560 bytes
  (`CRED_MAX_CREDENTIAL_BLOB_SIZE`), a jar is several KB. Splitting it across entries loses the
  atomic write that the cookie rotation relies on; filtering it down to a few cookies leaves it at
  the limit. It protects with DPAPI anyway.
- **App-Bound Encryption** (Chrome's: a SYSTEM service checks which executable asks): needs an
  installed service, against "nothing installed on the host".
- **A TPM key** or **Windows Hello**: a TPM key still decrypts for any program of the user on the
  machine; Hello asks for the user's presence, impossible for a jar rewritten in the background.
- **DPAPI on the file** (`CryptProtectData`, current user): the Credential Manager's protection,
  without a size limit, and the write stays a temporary file plus a rename.

No option protects from a program running as the user: Peeby uses and rotates the cookies on its
own, so whatever Peeby decrypts unattended, such a program can too (or read Peeby's memory).

## Decision

- The jar lives at `%APPDATA%\peeby\cookies.dat` (or `PEEBY_COOKIES`), encrypted with DPAPI for
  the current user, with an entropy of Peeby's own (`settings::JAR_ENTROPY`) and
  `CRYPTPROTECT_UI_FORBIDDEN`. The calls are in `desktop::sys` (`protect`, `unprotect`), the
  crate's FFI module, through the `windows` crate's `Win32_Security_Cryptography` feature.
- `innertube::Client::new` takes a `SaveCookies` (a function given the jar) instead of a path:
  the app's is `settings::cookie_saver()`, which encrypts. `innertube::save_to_file` writes a
  plain file, for the examples.
- Migration: at load, a former `cookies.txt` is encrypted into `cookies.dat` then deleted; a
  plain file given with `PEEBY_COOKIES` is encrypted in place.
- yt-dlp reads a plain jar: it gets a plain copy in its cache folder
  (`%LOCALAPPDATA%\peeby\yt-dlp\cookies.txt`), deleted at the next start and on sign-out.
- `peeby --export-cookies` writes the jar in clear on stdout, for the innertube examples:
  redirect it to a file, never into a log or a shared terminal.

## Consequences

- A copy of `cookies.dat` is useless on another machine or account. A generic tool that decrypts
  the user's DPAPI data also needs Peeby's entropy: a constant of its binary, an obstacle, not a
  secret.
- A Windows password reset by an administrator (not a change by the user) loses the DPAPI keys:
  the jar no longer decrypts, Peeby starts signed out and the user signs in again.
- The plain copy for yt-dlp exists between a fallback and the next start or sign-out.
- The examples need an exported copy (`peeby --export-cookies > copy.txt`) instead of the file.
