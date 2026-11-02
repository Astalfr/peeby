# 0010: A Twitch space

- Status: accepted (the two open questions answered by the user on 2026-11-01)
- Date: 2026-11-01

## Context

Peeby has one space per service (ADR 0007): YouTube and Jellyfin are in. The user asked to research Twitch
and prepare for it without building it (refactor plan, step 9). The research, with its sources, is in
[twitch.md](../twitch.md). Twitch differs from the other two services in five ways:

- Its streams are live, and the ads are inside them.
- It needs two credentials for different things: a Helix token, and the web cookie.
- Some playback tokens require a browser-made integrity token.
- Its private API (GQL) changes its query hashes.
- Its pages center on a chat.

## Decision

- **Shape**:
  - A `twitch` crate with no GPUI dependency, parsing leniently from `serde_json::Value` and tested on
    saved, redacted responses, as `innertube`.
  - A `spaces/twitch/` space with its own pages.
  - Nothing in the shell or in `ui` learns about Twitch.
- **Catalog through Helix**:
  - Followed live channels, followed channels, categories, search, VODs, clips and users.
  - A user token from the Device Code Grant, with Peeby registered as a public client (no secret).
  - Recommendations, which only GQL gives, wait.
- **Playback through GQL then usher v2**:
  - The `PlaybackAccessToken` query, then the usher v2 playlist given to mpv with `Referer` and `Origin`
    headers.
  - Clips play as direct MP4.
  - The web Client-ID and the persisted-query hashes are settings refreshed at startup, as
    `WEB_CLIENT_VERSION`, not constants.
- **No browser on the host**:
  - Peeby never starts Chromium for Client-Integrity.
  - When a token needs one, playback fails with a clear message. WebView2 can be tried later behind a
    switch.
- **Secrets**:
  - Twitch's tokens are small, so they go in the Windows Credential Manager, as Jellyfin's (ADR 0009).
  - A refreshed token is saved at once: refresh tokens are single use.
- **Shared code to add when the space is built**:
  - `ui::player::Source::live`: no seek bar, a live badge, uptime and viewers, no seeking keys.
  - A WebSocket client for the chat, as a new dependency to agree on.
  - Animated emotes, or static ones first.
- **Chat**:
  - Read-only first, anonymous IRC over WebSocket (`justinfan`), Twitch emotes from the message tags.
  - Then badges, then 7TV, BTTV and FFZ emotes.

## Answered questions (the user, 2026-11-01)

1. **Ads are removed.** A layer between Twitch and mpv rewrites the live media playlist and leaves out the
   ad segments, with Streamlink's rules: inside an `#EXT-X-DATERANGE` of class `twitch-stitched-ad` or
   with an id starting with `stitched-ad-`, an `#EXTINF` title containing `Amazon`, and the prefetch
   segments after a discontinuity. Whether it is a local HTTP proxy or a libmpv stream callback is decided
   when it is built (a spike first, measured against the 1–2 s start). It has rules of its own, tested on
   saved playlists, since Twitch changes how it marks ads. While an ad runs, the player shows a notice
   rather than a frozen picture.
2. **Peeby asks for the web `auth-token`**, optionally, besides the Helix sign-in, with a short tutorial in
   the space's settings part on how to copy it from the browser ([twitch.md](../twitch.md#the-auth-token-tutorial)).
   It goes in the Credential Manager, never in a log, a URL or an error message. The tutorial says what
   it gives access to and how to revoke it. With it, playback tokens are those of the user's account (no
   ads with Turbo or a subscription, subscriber-only VODs); without it, playback still works, with ads
   removed by the layer above.

3. **Sign-in required** (2026-11-01). A public client cannot get a Helix app token (no client credentials
   flow without a secret), so the catalog needs the user's token anyway. Signed out, the space shows only
   its sign-in card. Peeby's Client-ID on dev.twitch.tv is `zsxaog2l339o1lbld85h17ksscnjwr` (public, not
   a secret). Tokens are validated at startup and every hour, as Twitch requires.
4. **Low latency and HEVC/AV1 are in scope** (2026-11-01): prefetch segments kept by the proxy, and the
   best codec the GPU decodes, h264 as fallback. Plan: [twitch-plan.md](../twitch-plan.md).

5. **Live and chat are generic** (2026-11-01). They are built to serve YouTube lives later, and possibly
   other platforms:
   - the live mode of the shared player (`Source::live`, `LiveInfo`, `set_notice`);
   - a chat model and panel in `ui::chat` that know no service;
   - a watch layout with a chat column, in `ui`.

   Each space converts its own protocol (Twitch IRC now, InnerTube's live chat later). Details:
   [twitch-plan.md](../twitch-plan.md#live-and-chat-are-generic).

## Consequences

- The playback path depends on undocumented endpoints (GQL, usher) that change several times a year:
  refreshable settings and fixtures limit the damage, but a Twitch change can break playback until
  Peeby updates.
- Signing in takes a code typed on twitch.tv (device flow), no password in Peeby.
- What is checked against the code (the table in [twitch.md](../twitch.md#what-peebys-architecture-already-covers)):
  the space trait, sources with headers, qualities, previews, image templates, cards and an embedded
  player beside a chat already fit. The live flag, the chat's WebSocket, animated images and the
  ad layer are the additions.
- The ad layer sits in the playback path of every live stream: it is a likely cause of breakage and of
  delay, so it gets its own tests (saved playlists with ads) and a measurement of the start time.
- The `auth-token` is a full-access secret: losing it means a password change on Twitch. The tutorial
  says so before the field.
- Revisit if Twitch publishes a playback API, or if Client-Integrity becomes required on every token.
- Built (2026-11-01, [twitch-plan.md](../twitch-plan.md) steps 1–10). What the building settled:
  - the ad layer is a local playlist proxy; playback starts in 1.2–1.5 s through it, and with Twitch's
    prefetch segments (low latency, on by default) a live starts about 2.6 s behind its capture instead
    of about 7 s;
  - a public client has no app token, so the whole space needs the user's sign-in (their choice);
  - the chat is read anonymously over IRC, with Twitch, 7TV and BTTV emotes (FFZ dropped by the user; BTTV added on 2026-11-02)
    and Helix badges; the chat model and panel are generic, in `ui::chat`, for YouTube lives later;
  - HEVC is Twitch's codec for the source above 1080p (1440p60 on the channels that send it, with
    H.264 below), decoded by the GPU here; AV1 was offered by no channel tried. A codec the GPU does
    not decode is dropped for H.264.