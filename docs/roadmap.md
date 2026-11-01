# Roadmap

Status legend: ✅ done · 🚧 in progress · ⏳ not started

| Milestone | Status | Content | Exit criterion |
|---|---|---|---|
| **Study and spikes** | ✅ | Feasibility study, Home via InnerTube, playback spike (yt-dlp, then VISIONOS HLS), bundled tools | [etude-faisabilite.md](etude-faisabilite.md), ADRs 0001–0005 |
| **1: foundations** | 🚧 | Workspace, CLAUDE.md, docs; `innertube` (Home, watch page, streams, cookies) with fixture tests; app shell, Home grid, watch page with autoplay and related videos | Open a video from a URL and from a related video, ~1 s to playback; signed-in Home shows recommendations |
| **2: YouTube pages** | ⏳ | Subscriptions list and feed (`FEsubscriptions`, RSS fallback), comments (`commentEntityPayload`), Return YouTube Dislike bar (cache, attribution), channel page, infinite scroll | — |
| **3: player comfort** | ⏳ | Seek bar, volume, quality choice, subtitles on demand (`sub-add`), prefetch on hover, fullscreen controls, yt-dlp fallback ([ADR 0005](decisions/0005-bundled-yt-dlp.md)) | — |
| **4: session and settings** | ⏳ | Cookie import UI, `Set-Cookie` persistence, settings page (language, quality), yt-dlp updater | — |
| **5: distribution** | ⏳ | CI, bundle with libmpv, yt-dlp and Deno in `tools/`, installer (peeby's scripts) | Downloadable build |

## Milestone 1 status

- Done and verified (2026-11-01):
  - `innertube`: 11 tests on real fixtures, including a feed continuation (continuations carry `logged_in` too). `cargo run -p innertube --example home` answers `logged_in=false` with YouTube's message.
  - App, signed out:
    - Home shows the empty state with the cookie instructions;
    - `peebtube <url>` plays 1080p VP9 with d3d11va and audio, about 1.4–1.6 s after the page opens;
    - title, channel, description and related videos with thumbnails are shown;
    - a covered watch page pauses its player, even when the player starts after being covered (`peebtube <id1> <id2>`). "Back" does not resume by itself.
  - The VISIONOS resolver uses its own anonymous visitorData, separate from the signed-in session's.
  - InnerTube errors are shown in French (`ui::error_text`).
- **Not verified:**
  - signed-in Home (needs the user's cookies);
  - clicks in the UI: the URL field, a related card, "Plus de vidéos", the back button (wired, same code paths as the command line);
  - Claude Code hooks (fmt, clippy gate): not installed yet, see CLAUDE.md "Ask before".

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| YouTube blocks the VISIONOS client | No playback | yt-dlp fallback, values tracked against yt-dlp's source |
| YouTube changes renderers | Empty feeds or pages | Lenient parsing, fixtures, `logged_in` checks |
| Cookies expire or rotate | Signed out | `Error::NotLoggedIn` shown to the user; persist `Set-Cookie` (milestone 4) |
| Copied crates drift from peeby | Duplicate fixes | Shared repository once both stabilize |
