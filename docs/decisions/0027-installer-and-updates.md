# 0027: Installer and updates from GitHub Releases

- Status: accepted
- Date: 2026-11-05

## Context

Version 1.0 is the first downloadable build. It needs an installer that ships what Peeby loads
beside its executable (libmpv, `tools\yt-dlp.exe`, `tools\deno.exe`: ADR 0005, peeby ADR 0007),
and a way for installed copies to update without the user downloading each release.

Twitty (`D:\DEV\twittyv5`), the author's other GPUI app, already does both: an Inno Setup
installer built by CI on a version tag, published as a GitHub Release, and an updater that reads
`releases/latest`, downloads the installer, checks GitHub's SHA-256 `digest` of the asset and runs
it silently. Peeby takes the same design, with these differences:

- Peeby hides in the notification area when its window closes (ADR 0020), so Restart Manager's
  polite close may not end it, and it is a single instance: a relaunch that finds the old one
  still alive would only hand it its arguments.
- yt-dlp updates itself in place (`yt-dlp -U`, ADR 0005): the install folder must stay writable,
  and an update of Peeby must not put back the older pinned yt-dlp.
- Peeby stays in the notification area for days: one check at startup is not enough.

Options considered for the update itself: an MSIX package (Store-like updates, but a sandboxed
install folder that `yt-dlp -U` cannot write, and a signing certificate); a zip replaced by a
helper process (reimplements what the installer already does: closing the app, replacing files in
use, the uninstall entry). Inno Setup run silently does it all.

## Decision

- **Installer** (`installer/peeby.iss`, Inno Setup 6): per user only
  (`%LOCALAPPDATA%\Programs\Peeby`, no admin rights, no dialog to pick Program Files), with
  `peeby.exe`, `libmpv-2.dll`, the VC++ runtime it links against (`vcruntime140.dll`,
  `vcruntime140_1.dll`, app-local: Windows lacks it), `tools\yt-dlp.exe` (copied only if missing, so a self-updated one
  stays) and `tools\deno.exe`. `CloseApplications=force` (ends a Peeby hidden in the notification
  area and the yt-dlp or Deno it started), `RestartApplications=no`; `/relaunch=1` in silent mode
  starts Peeby again at the end. Uninstalling removes the `Run` value and the toasts' registration,
  and leaves the user's data (settings, cookie jar, Credential Manager entries, caches).
- **CI** (`.github/workflows/ci.yml`): fmt, clippy and tests on every push and pull request; a
  `v<version>` tag (only a tag) also fetches the pinned libmpv, yt-dlp and Deno, builds the release, compiles
  the installer `peeby-setup-<version>-x64.exe` and publishes it as the tag's GitHub Release, once
  the tag is checked against the version in `Cargo.toml`.
- **Updater** (`crates/peeby-core/src/update/`, ADR 0029): only in a release build installed by the installer
  (its `unins000.exe` beside the exe). 20 s after the start, then every 4 hours (or at once from "Check for updates" in the settings' About section, which then says whether Peeby is up to date), an anonymous
  `GET /repos/Astalfr/peeby/releases/latest` (drafts and pre-releases ignored; GitHub allows 60
  anonymous requests an hour). A tag newer than this build has its installer downloaded in the
  background into `%LOCALAPPDATA%\peeby\updates` and checked against the asset's `digest`; a
  corrupt or failed download leaves nothing and is tried again at the next check. While it
  downloads, the title bar shows the **Update** button greyed out with the app's loader, taking no
  click; once checked it turns green (its tooltip names the version). A click writes the
  settings, starts the installer (`/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS
  /relaunch=1`) and quits; the installer replaces the files and starts Peeby again. Old installers
  are deleted at the start.
- **At startup** (`update::startup`, the user asked, 2026-11-11): before any window opens, the
  same check, cut after 5 s. A newer release is installed at once: an installer an earlier
  session downloaded is reused if it still matches its digest, else it is downloaded while a
  small window shows its progress (none at a start with Windows); then the settings are written,
  the installer starts and Peeby quits. Started with Windows, it passes `/background=1`, and the
  installer starts Peeby again with `--background`. Anything failing (no network, GitHub slow or
  down, a corrupt download) opens Peeby as usual, and the periodic checks carry on.

## Consequences

- A release is a version bump in `Cargo.toml` and a pushed tag (docs/release.md); every installed
  copy offers it within 4 hours, and installs it at its next start.
- A start that finds a newer release waits for its download (about 100 MB) before opening; a
  start without network waits up to 5 s.
- `/background=1` needs the new installer's script: it holds from the release after 1.5.1 on.
- The repository must stay public: the check is anonymous. A private one would need a token in
  every installed copy.
- The installer is not signed: SmartScreen warns on the first install (not on silent updates
  started by Peeby). Signing would need a certificate.
- No "later" or "skip this version": the button stays until the update is installed, and it
  takes no room until an installer is ready.
- The update flow itself (download, silent install, relaunch) can only be tried end to end once a
  second release exists.
