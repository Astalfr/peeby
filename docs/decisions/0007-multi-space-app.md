# 0007: Peeby, one app with a space per service

- Status: accepted
- Date: 2026-11-01

## Context

The author keeps two GPUI apps with the same playback stack and visual system: PeebTube (YouTube, this
repository) and peeby (Jellyfin, `D:\DEV\peeby`). ADR 0001 made the playback crates **copies**, so every
fix had to be carried by hand. A Twitch client is wanted next. Three apps with three copies of the player,
the theme and the cards would not stay maintainable.

## Decision

- This repository becomes **Peeby**, a media app with one tab ("space") per service: YouTube and Jellyfin
  now, Twitch later. The crate and the exe are `peeby`; the icon stays PeebTube's (the round logo with the
  red play badge). The workspace version starts at 0.2.0, above both former apps.
- The old peeby repository is abandoned: the code the Jellyfin space needs (`jellyfin-api`, its pages, its
  player features) is copied here, nothing else (no CI, packaging or Claude hooks). `player`, `gpui-video`
  and `vendor/` here are already a superset of peeby's, so they are kept as they are and no longer
  "copies": this supersedes the copy consequence of ADR 0001.
- No migration of user data: the app uses fresh `%APPDATA%\peeby\` and `%LOCALAPPDATA%\peeby\` folders,
  YouTube cookies are imported again and Jellyfin is signed in again. Environment variables are `PEEBY_*`.
- The shell knows no service and a shared `ui` crate knows no service; a space brings its client, its
  pages and the conversion of its data into the shared components' models. The steps, the layout
  decisions and the Jellyfin parity checklist are in [refactor-plan.md](../refactor-plan.md).

## Consequences

- One player, one theme, one set of cards to fix; a new service is a new space module.
- The former peeby writes `%APPDATA%\peeby\settings.toml` too, in another format. The two cannot be used
  side by side: Peeby reads unknown keys as defaults and overwrites the file.
- A large refactor before new features; YouTube must keep working after every step of the plan.
