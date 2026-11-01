# Roadmap

Status legend: ✅ done · 🚧 in progress · ⏳ not started

| Milestone | Status | Content | Exit criterion |
|---|---|---|---|
| **Study and spikes** | ✅ | Feasibility study, Home via InnerTube, playback spike (yt-dlp, then VISIONOS HLS), bundled tools | [etude-faisabilite.md](etude-faisabilite.md), ADRs 0001–0005 |
| **1: foundations** | ✅ | Workspace, CLAUDE.md, docs; `innertube` (Home, watch page, streams, cookies) with fixture tests; app shell, Home grid, watch page with autoplay and related videos | Open a video from a URL and from a related video, ~1 s to playback; signed-in Home shows recommendations |
| **2: YouTube pages** | ✅ (RSS fallback moved to the backlog) | Subscriptions list and feed (`FEsubscriptions`, RSS fallback), comments (`commentEntityPayload`) and replies, Return YouTube Dislike bar (cache, attribution), channel page, infinite scroll | — |
| **3: player comfort** | ✅ | Quality choice, subtitles on demand (`sub-add`), prefetch on hover, yt-dlp fallback ([ADR 0005](decisions/0005-bundled-yt-dlp.md)) | — |
| **4: session and settings** | 🚧 cookie import, `Set-Cookie` persistence and yt-dlp updates done; settings page left | Cookie import UI, `Set-Cookie` persistence, settings page (language, quality), yt-dlp updater | — |
| **5: distribution** | ⏳ | CI, bundle with libmpv, yt-dlp and Deno in `tools/`, installer (peeby's scripts) | Downloadable build |

## Milestone 4 status

- Done (2026-11-01):
  - **Account page** (sidebar "Compte"): session state, export instructions, "Importer cookies.txt…" (file picker; checked with a Home request before it replaces the session file), "Se déconnecter". The app switches session without restarting. Pages that need a session link to it; an expired session says so.
  - **yt-dlp updates**: single-file build (it can update itself, "onedir" cannot), `yt-dlp -U` at most once a day in the background. Verified: "yt-dlp is up to date (stable@2026.10.26)", then the fallback played an age-restricted video in 5.4 s.
- **Not verified:** the import flow in the UI (file picker, valid file), since no fresh cookies were available; the expired-session state was seen in the logs only.
- Left: settings page (language and region, default quality), saved in a settings file. It needs a serialization dependency (`serde` + `toml`, as peeby): to be agreed first.

## Milestone 3 status

- Done (2026-11-01):
  - **Quality menu**: the whole HLS ladder (2160p to 144p, VP9 preferred). Changing quality restarts mpv on the new variant at the current position: verified with the `player` crate directly, 1080p to 720p, playback again after about 1.0–1.1 s, subtitle track kept.
  - **Subtitles on demand**: the VISIONOS player response lists the caption tracks (`captions.playerCaptionsTracklistRenderer`); mpv fetches one as WebVTT (`&fmt=vtt`) with `sub-add` when it is picked (menu or C). Verified: loaded and selected in about 0.1 s. Machine translations (`&tlang=`) answer HTTP 429 for this client, so they are not offered.
  - **Prefetch on hover**: resting 300 ms on a video card resolves its stream; the watch page then starts mpv without waiting for the resolver (saves the 0.2–0.4 s). Cache of 64 streams, 4 h.
  - yt-dlp fallback (see milestone 2).
- **Not verified:** the menus and the hover prefetch in the UI (not clicked in a test run). The app logs the ladder and tracks it receives: 8 qualities and 6 tracks on a test video.

## Milestone 2 status

- Done and verified (2026-11-01), signed in:
  - `cargo run -p innertube --example smoke -- <cookies.txt>` passes all endpoints live: Home (23 videos), subscriptions (111 videos, more), 475 subscribed channels, channel videos and continuation, watch page, comments (20, total count), stream, RYD votes.
  - App: signed-in Home grid; sidebar with Home, Subscriptions and the subscribed channels; watch page with playback in about 1.6 s, comments and the RYD bar.
  - Player controls adapted from peeby's player page (`player_view.rs`): seek bar with time preview, volume slider, play/pause, ±10 s, fullscreen (only the player is shown), auto-hide, OSD, pause pulse, shortcuts (K/space, J/L, arrows, 0–9, M, F, Esc). Volume is kept for the session.
  - Fixtures from signed-in responses are trimmed (3–4 items) and stripped of account data (`datasyncId`, visitorData, top bar).
- **Not verified:** clicks in the UI (sidebar, channel link, "Plus de vidéos", "Plus de commentaires", player buttons, volume drag, fullscreen) — same code paths as the logged ones, but not clicked in a test run.
- Then (same day):
  - Comment replies ("N réponses", then "Plus de réponses"), tested on a real replies page (`comment_replies.json`).
  - Infinite scroll on feeds and comments: the next page starts within about a screen of the end; a "Réessayer" button only after a failure.
  - yt-dlp fallback when VISIONOS fails (verified on an age-restricted video: 8.1 s, see ADR 0005).
  - Cookie rotation: the session was invalidated after about an hour because `Set-Cookie` updates were ignored. They are now applied and saved (ADR 0004). **The cookies must be exported again**, and the fix is not verified over a long session yet.
- RSS fallback for subscriptions: moved to the backlog (the signed-in feed works).

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

## Backlog

- RSS fallback for the subscriptions feed (`feeds/videos.xml?channel_id=`), for use without a session.
- Shorts shelves (`shortsLockupViewModel`) on Home and subscriptions.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| YouTube blocks the VISIONOS client | No playback | yt-dlp fallback, values tracked against yt-dlp's source |
| YouTube changes renderers | Empty feeds or pages | Lenient parsing, fixtures, `logged_in` checks |
| Cookies expire or rotate | Signed out | `Error::NotLoggedIn` shown to the user; persist `Set-Cookie` (milestone 4) |
| Copied crates drift from peeby | Duplicate fixes | Shared repository once both stabilize |
