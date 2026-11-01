# Twitch: how a Twitch space would work

Research of 2026-11-01 for step 9 of [refactor-plan.md](refactor-plan.md), before any code. The decisions are in
[ADR 0010](decisions/0010-twitch-space.md): ads are removed by a playlist-rewriting layer, and Peeby asks for the
web `auth-token` (optional) with the tutorial below. Each claim is marked **[src]** (read in a source listed at the
end), **[mem]** (from memory, not re-checked) or **[?]** (uncertain). Check the [?] and [mem] ones against
real responses (saved, redacted fixtures, as for InnerTube) before writing a parser.

## What conflicts with Peeby's constraints

1. **Ads are in the stream.**
   - Twitch stitches ads into the live media playlist as real segments **[src]**.
   - Streamlink drops a segment as an ad when any of these holds **[src]**:
     - it falls in an `#EXT-X-DATERANGE` with `CLASS="twitch-stitched-ad"`;
     - the DATERANGE `ID` starts with `stitched-ad-`;
     - its `#EXTINF` title contains `Amazon`.
   - Since Streamlink 7.5.0 (2025-07) that filter is always on. Twitch moved to fragmented MP4, and the
     transitions between ads and content broke players **[src]**.
   - Handing the usher URL to mpv as is gives ads, and those discontinuities.
   - Removing them needs a layer that rewrites the media playlist between Twitch and mpv: a localhost HTTP
     proxy, or a libmpv stream callback (`mpv_stream_cb_add_ro`).
   - The web `auth-token` of a Turbo account, or of a subscriber of the channel, gets a playback token
     without ads.
2. **Client-Integrity needs a browser.**
   - Some token requests have required a Kasada integrity token since 2023-05-31 **[src]**.
   - Streamlink handles it like this **[src]**:
     - it first asks for the token without one;
     - only if that fails, it starts Chromium over CDP to run Kasada's script, then POSTs
       `https://gql.twitch.tv/integrity` with a random `x-device-id`;
     - it caches the answer and sends `Client-Integrity` and `Device-Id` afterwards.
   - A browser on the host breaks "nothing installed on the host" and the 1–2 s start. WebView2, which
     Windows ships, might stand in **[?]**.
3. **GQL persisted-query hashes and the web Client-ID go stale.**
   - Streamlink updated them in 8.0.0 (2025-11) and again in 8.1.0 **[src]**.
   - yt-dlp's hashes differ from Streamlink's **[src]**.
   - So they are settings refreshed from outside, like `WEB_CLIENT_VERSION` for InnerTube, not code
     constants. Raw (non-persisted) queries avoid hashes where Twitch accepts them.
4. **Usher v2.**
   - Streamlink 8.2.1 (2026-03-05) moved to `/api/v2/channel/hls/{login}.m3u8` and `/vod/v2/{id}.m3u8`
     **[src]**.
   - yt-dlp still uses the v1 paths **[src]**. Whether v1 still works is unknown **[?]**.

## Playback

- **GQL basics** **[src]**:
  - endpoint `POST https://gql.twitch.tv/gql`;
  - the web Client-ID is `kimne78kx3ncx6brgo4mv6wki5h1ko`;
  - signed in, calls add `Authorization: OAuth <auth-token>`.
- **The playback token**:
  - Streamlink uses the persisted `PlaybackAccessToken` query, with the variables `isLive`, `login`,
    `isVod`, `vodID`, `playerType` (default `embed`) and `platform` (`site`) **[src]**.
  - The answer is `streamPlaybackAccessToken` or `videoPlaybackAccessToken`: `{value, signature}`
    **[src]**.
  - yt-dlp sends a raw query instead (`params: {platform: "web", playerBackend: "mediaplayer",
    playerType: "site"}`), with another Client-ID, `ue6666qo983tsx6so1t0vnawi233wa`, adopted in 2023
    **[src]**. Whether that Client-ID still works is **[?]**.
  - Another `playerType` can lose the best qualities **[src]**.
- **Live** **[src]**:
  - `https://usher.ttvnw.net/api/v2/channel/hls/{login}.m3u8` with `sig`, `token`, `platform=web`,
    `p=<random>`, `allow_source=true`, `allow_audio_only=true`, `playlist_include_framerate=true`,
    `multigroup_video=true`, `fast_bread=true`, `supported_codecs=h264[,h265,av1]`.
  - mpv must send `Referer` and `Origin: https://player.twitch.tv` (`http-header-fields`).
- **VODs** **[src]**:
  - `https://usher.ttvnw.net/vod/v2/{id}.m3u8` with `nauthsig`, `nauth` and the same parameters.
  - Subscriber-only VODs answer HTTP 403 (`vod_manifest_restricted`, `unauthorized_entitlements`): map it
    to an error of its own.
- **Clips** are not HLS **[src]**:
  - GQL `VideoAccessToken_Clip` or `ShareClipRenderStatus` give `videoQualities[].sourceURL` and a token.
  - The clip plays as a direct MP4 from `sourceURL?sig=…&token=…`.
- **Codecs** **[src]**: 1440p and Enhanced Broadcasting send HEVC or AV1 in fragmented MP4, only when
  `supported_codecs` lists them. Check D3D11 hardware decoding of both before offering them.
- **Audio** **[?]**: variants are believed to carry their audio (muxed), unlike YouTube's video-only HLS.
  There is also an `audio_only` variant.
- **Qualities** **[src/mem]**:
  - they come from the master playlist (`NAME="1080p60"`; the source variant is the `chunked` group);
  - each one is a variant URL, as for YouTube.
- **Seek-bar previews** **[src]**: yt-dlp's `VideoPlayer_VODSeekbarPreviewVideo` query returns
  `seekPreviewsURL`, a storyboard for VODs.
- **Latency** **[src/mem]**:
  - Streamlink's low-latency mode keeps the `#EXT-X-TWITCH-PREFETCH` segments. mpv ignores that tag.
  - Without the rewriting layer, expect 5–10 s of latency.

## Credentials

Two credentials, for different things. Both are secrets, under the same rules as the YouTube cookies
(ADR 0004): never in logs, URLs or error messages.

- **A. A Helix user token, from the Device Code Grant** **[src]**:
  - Peeby registers as a **public** client on dev.twitch.tv: no secret, device flow only.
  - `POST https://id.twitch.tv/oauth2/device` with `client_id` and `scopes`. The answer is
    `device_code`, `user_code`, `verification_uri`, `expires_in` (about 1800 s) and `interval` (about 5 s).
  - Then poll `POST https://id.twitch.tv/oauth2/token` (`grant_type=urn:ietf:params:oauth:grant-type:device_code`),
    which answers `authorization_pending` until the user approves on another screen.
  - Access tokens last about 4 h. **Refresh tokens are single use** and expire after 30 days unused, so
    every refresh must save the new one at once, like the cookie rotation.
  - Scopes: `user:read:follows`; `user:read:chat` for EventSub chat.
- **B. The web `auth-token` cookie** **[src]**:
  - 30 characters, sent as `Authorization: OAuth <token>` with the web Client-ID.
  - It gives full access to the account and survives a sign-out. Revoking it takes a password change or
    "sign out everywhere".
- **What needs which**:
  - Helix (A): followed live channels and followed channels (user token); streams by category, top
    categories, search, a channel's VODs and clips, users, emotes, badges (an app token is enough).
  - GQL with B: the recommended streams (no Helix endpoint; the query names are **[?]**), and playback
    without ads (Turbo or subscriber), subscriber-only VODs, sometimes the best qualities.
  - A token issued for Peeby's own Client-ID very likely does not unlock what B does **[likely]**.
  - Chat read anonymously needs neither.

## Chat

- **IRC over WebSocket** **[src]**:
  - `wss://irc-ws.chat.twitch.tv:443` (plain WebSocket stopped on 2025-08-15).
  - `CAP REQ :twitch.tv/tags twitch.tv/commands`, then `PASS`, `NICK`, `JOIN #login`.
  - Answer every `PING` with `PONG :tmi.twitch.tv`.
  - Handle `PRIVMSG`, `CLEARCHAT`, `CLEARMSG`, `USERNOTICE` and `ROOMSTATE`.
- **Anonymous reading** **[mem]**: `NICK justinfan<digits>` with any `PASS`. It is a long-standing
  convention (Chatterino), not in Twitch's docs.
- **Twitch's recommendation** **[src]**: EventSub `channel.chat.message`, which needs a user token with
  `user:read:chat`. Its message fragments: text, emote, mention, cheermote, `gif` (2026-07).
- **Emotes**:
  - The IRC `emotes` tag gives `id:start-end` ranges, believed to count Unicode code points **[?]**: do not
    slice a `&str` with them.
  - Twitch emote images: `https://static-cdn.jtvnw.net/emoticons/v2/{id}/{static|animated}/{dark|light}/{1.0|2.0|3.0}`
    **[src]**.
  - Badges come from Helix Global and Channel Chat Badges, with direct image URLs **[src]**.
  - Third-party emotes, matched by word and keyed by the broadcaster's numeric id:
    - 7TV `https://7tv.io/v3/users/twitch/{id}` and `/v3/emote-sets/global` **[weak src]**;
    - BTTV `https://api.betterttv.net/3/cached/users/twitch/{id}` and `/3/cached/emotes/global` **[mem]**;
    - FFZ `https://api.frankerfacez.com/v1/room/id/{id}` and `/v1/set/global` **[mem]**.
- **Order of work**:
  1. anonymous IRC with Twitch emotes from the tag;
  2. badges;
  3. 7TV, BTTV and FFZ;
  4. deleted messages.

## Helix

- Headers `Client-Id` and `Authorization: Bearer <token>`. Pages: `first` (at most 100), `after`,
  `pagination.cursor` **[src]**.
- Rate limit **[src for the mechanism]**:
  - a token bucket per client id and per user (`Ratelimit-Limit`, `-Remaining`, `-Reset`), HTTP 429 when
    empty;
  - about 800 points a minute **[forum]**.
- Endpoints:
  - `GET /helix/streams/followed` and `/helix/channels/followed` (`user:read:follows`);
  - `GET /helix/streams` by user, category or language: `viewer_count`, `started_at`,
    `thumbnail_url` with `{width}x{height}`, `tags` **[src/mem]**;
  - `GET /helix/games/top`, `/helix/games`: `box_art_url` with `{width}x{height}` **[src]**;
  - `GET /helix/search/categories`, `/helix/search/channels` (`is_live`) **[mem]**;
  - `GET /helix/videos`: `duration` as `3h8m33s`, `muted_segments`, `thumbnail_url` with
    `%{width}x%{height}` **[mem]**;
  - `GET /helix/clips`: `duration` in seconds (float), `vod_offset`, a direct `thumbnail_url`; no playable
    URL (playback goes through GQL) **[src]**;
  - `GET /helix/users`: `profile_image_url`, `offline_image_url` **[mem]**.
- Images:
  - Live previews are cached by Twitch's CDN: add a cache-buster.
  - `/404_processing_…` means a VOD not processed yet.
  - yt-dlp replaces the size by `0x0` for the full size **[src]**.

## What Peeby's architecture already covers

Checked against the code on 2026-11-01:

| Need | Where | Status |
|---|---|---|
| A tab with its accent, sidebar entries, back, search, settings part, mini-player | `shell::space::Space` | Covered: Twitch is one more `Space` |
| HLS with per-source HTTP headers (`Referer`, `Origin`) | `StartOptions::http_headers` | Covered |
| Muxed audio (no `audio-files`) | `StartOptions::external_audio` is optional | Covered |
| Quality menu over the master's variants | `Source::qualities` (one `StartOptions` each) | Covered |
| Clips as direct MP4 | `Source::options` with a plain URL | Covered |
| VOD storyboard on the seek bar | `ui::player::Previews` | Covered |
| Size-templated images (`{width}x{height}`, `%{width}x%{height}`) | `Picture::sized` (the space fills the template) | Covered |
| Stream, VOD and clip cards; category box art (tall) | `MediaCard` (`badge`, `lines`, `avatar`, `poster` at any ratio) | Covered |
| A watch page with the player and a chat beside it | `PlayerView` in `Layout::Embedded`, laid out by the space's own page (as YouTube's watch page) | Covered |
| Live: no seek bar, a live badge, uptime and viewers instead; no 0–9/Home seeks | `PlayerView` shows a seek bar whenever mpv gives a duration (a live window has one) | **Missing**: a `Source::live` flag |
| Ads removed | Nothing between Twitch and mpv | **Missing**: a playlist-rewriting layer (decided: build it) |
| Low latency | Same layer (prefetch segments) | **Missing**, optional |
| Animated emotes (WebP, GIF) | `ui::images::decode` keeps the first frame | **Missing**: frames in the image store, or static emotes |
| Chat WebSocket | No WebSocket client in the workspace | **Missing**: a dependency (e.g. `tokio-tungstenite`), to agree on |
| Single-use refresh token kept at once | Credential Manager (`keyring`), as Jellyfin's token | Covered (small secrets) |
| GQL hashes and Client-ID refreshed without a release | Nothing yet | **Missing**: settings refreshed at startup, as `WEB_CLIENT_VERSION` |

## The auth-token tutorial

The text Peeby shows above the `auth-token` field, in the Twitch part of the settings (UI text in French, as
the rest of the app):

> **Jeton du site Twitch (facultatif)**
>
> Avec lui, Peeby lit les vidéos comme votre compte sur twitch.tv : sans publicité si vous avez Turbo ou un
> abonnement à la chaîne, et les VOD réservées aux abonnés. Sans lui, la lecture marche aussi : Peeby
> retire lui-même les publicités.
>
> 1. Dans votre navigateur, ouvrez twitch.tv et connectez-vous.
> 2. Appuyez sur F12 pour ouvrir les outils de développement.
> 3. Chrome ou Edge : onglet « Application », puis « Cookies » › `https://www.twitch.tv`.
>    Firefox : onglet « Stockage », puis « Cookies » › `https://www.twitch.tv`.
> 4. Dans la liste, trouvez la ligne `auth-token` et copiez sa valeur (30 lettres et chiffres).
> 5. Collez-la ici.
>
> Ce jeton donne un accès complet à votre compte : ne le partagez pas. Peeby le garde dans le Gestionnaire
> d'identification de Windows. Il reste valable même après une déconnexion du site ; pour l'annuler,
> changez votre mot de passe Twitch ou utilisez « Se déconnecter partout ».

Peeby checks a pasted token (30 alphanumerics, then a GQL call that answers as signed in) before saving it,
as it checks imported YouTube cookies.

## Not verified

- The Helix details marked [mem]: the reference page was truncated when fetched.
- The BTTV and FFZ endpoints; the 7TV host.
- The 800 points a minute.
- Usher v1, and yt-dlp's other Client-ID.
- Ad-free tokens from a Helix token (very likely not).
- Muxed audio.
- The IRC emote indexes.
- WebView2 for Client-Integrity.
- GQL's recommendation queries.

## Sources

- Streamlink's Twitch plugin: https://raw.githubusercontent.com/streamlink/streamlink/master/src/streamlink/plugins/twitch.py
- Streamlink's Twitch docs (auth, ads, integrity, low latency, codecs): https://streamlink.github.io/cli/plugins/twitch.html
- Streamlink changelog (8.6.0, 8.2.1 usher v2, 8.1.0, 8.0.0, 7.5.0): https://streamlink.github.io/changelog.html
- yt-dlp's Twitch extractor: https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/yt_dlp/extractor/twitch.py
- yt-dlp's Client-ID change (2023): https://gitea.smoothnet.org/nhyatt/yt-dlp/commit/01231feb142e80828985aabdec04ac608e3d43e2.diff
- Twitch OAuth and the Device Code Grant: https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/
- Helix guide (rate limits, pages): https://dev.twitch.tv/docs/api/guide/
- Helix reference: https://dev.twitch.tv/docs/api/reference/
- Twitch developer changelog (2025–2026): https://dev.twitch.tv/docs/change-log/
- Twitch IRC: https://dev.twitch.tv/docs/chat/irc/
- Rate-limit threads: https://discuss.dev.twitch.com/t/request-limit-not-sufficient-for-helix-api/64433,
  https://discuss.dev.twitch.com/t/error-429-limitation-helix-api/25216
- ttv-lol-pro, usher v2: https://newreleases.io/project/github/younesaassila/ttv-lol-pro/release/v2.5.1
- 7TV endpoints (third-party README): https://lobehub.com/fr/mcp/daugustin-7tv-mcp
- HLS ad discontinuities: https://github.com/video-dev/hls.js/issues/4416,
  https://github.com/xbmc/inputstream.adaptive/issues/1071
