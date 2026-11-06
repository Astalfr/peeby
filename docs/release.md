# Releasing Peeby

Installed copies update from the repository's latest GitHub Release ([ADR 0027](decisions/0027-installer-and-updates.md)).
A release is built by CI from a version tag.

## Steps

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`), then run `cargo build` so
   that `Cargo.lock` follows: CI builds with `--locked`.
2. Note the release in [roadmap.md](roadmap.md), commit, and push `main`.
3. Tag the commit with the same version and push the tag:
   ```
   git tag v1.0.1
   git push origin v1.0.1
   ```
4. CI (`.github/workflows/ci.yml`) runs lint and tests, builds the installer
   `peeby-setup-1.0.1-x64.exe` and publishes the release `v1.0.1` with generated notes. A tag that
   does not match `Cargo.toml` fails the `release` job.

Within 4 hours (at once with "Check for updates" in the settings), every installed copy downloads the installer, checks its SHA-256 and shows the
green **Update** button in the title bar.

Never publish a release by hand with another asset name or without GitHub's digest: the updater
looks for exactly `peeby-setup-<tag version>-x64.exe` and refuses an installer without a digest.
A pre-release or a draft is never offered.

## Building the installer locally

With [Inno Setup 6](https://jrsoftware.org/isinfo.php):

```
pwsh scripts/fetch-mpv.ps1
pwsh scripts/fetch-tools.ps1
pwsh scripts/fetch-vcruntime.ps1
cargo build --release -p peeby
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /DAppVersion=1.0.0 installer\peeby.iss
```

It writes `target\installer\peeby-setup-1.0.0-x64.exe`. CI builds the installer only for a
version tag, which also publishes it: try a release's installer locally first.

## What an installed copy contains

`%LOCALAPPDATA%\Programs\Peeby\`: `peeby.exe`, `libmpv-2.dll`, the VC++ runtime (`vcruntime140.dll`, `vcruntime140_1.dll`: Windows does not ship it), `tools\yt-dlp.exe` (it updates
itself, and an update of Peeby keeps it), `tools\deno.exe`, `LICENSE.txt`, and the uninstaller.
The user's data stays where the app keeps it (`%APPDATA%\peeby`, `%LOCALAPPDATA%\peeby`, the
Credential Manager), and an uninstall leaves it.
