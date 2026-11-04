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
- **What needs which** (ADR 0016, 2026-11-03):
  - Nothing: the catalog through anonymous GQL (streams by category, top categories, search, a
    channel's VODs and clips, users, badges; see "Catalog through GQL instead of Helix"), playback
    with the ads removed, the chat.
  - Helix (A), optional: followed live channels and followed channels (user token), which GQL refuses
    to raw queries; and the catalog's backup when GQL fails. An app token would be enough for the
    catalog, but a public client cannot get one: no client credentials flow without a secret **[src]**.
  - GQL with B, the space's sign-in: the account (`currentUser`), playback without ads (Turbo or
    subscriber), subscriber-only VODs, sometimes the best qualities, channel points (ADR 0012), VOD
    positions (ADR 0013); the recommended streams would need it too (no Helix endpoint; the query
    names are **[?]**).
  - A token issued for Peeby's own Client-ID very likely does not unlock what B does **[likely]**.
  - Chat read anonymously needs neither.

## Chat

- **IRC over WebSocket** **[src]**:
  - `wss://irc-ws.chat.twitch.tv:443` (plain WebSocket stopped on 2025-08-15).
  - `CAP REQ :twitch.tv/tags twitch.tv/commands`, then `PASS`, `NICK`, `JOIN #login`.
  - Answer every `PING` with `PONG :tmi.twitch.tv`.
  - Handle `PRIVMSG`, `CLEARCHAT`, `CLEARMSG`, `USERNOTICE` and `ROOMSTATE`.
- **Sending**, checked on 2026-11-04 (`twitch_capture -- chat-login`, nothing sent): the web
  `auth-token` signs in to IRC as `PASS oauth:<auth-token>`, `NICK <login>` (the login from
  `currentUser`); Twitch answers the welcome and the `JOIN`. A refused token gets
  `NOTICE * :Login authentication failed` (no `msg-id`: tags are not on yet). A message goes as
  `PRIVMSG #login :text`, at most 500 characters, and 20 per 30 s for a viewer (beyond: locked out of
  chat for 30 minutes) **[src]**. Twitch does not send a message back to the connection that wrote it,
  only to the room's other connections **[mem]**: Peeby reads with the anonymous connection and
  writes with a second one ([ADR 0023](decisions/0023-twitch-chat-sending.md)). Refusals come as
  `NOTICE` with a `msg-id` (`msg_slowmode`, `msg_followersonly`, `msg_subsonly`, `msg_duplicate`,
  `msg_ratelimit`, `msg_banned`, `msg_timedout`…) **[src]**.
- **Anonymous reading**: `NICK justinfan<digits>` with any `PASS`. It is a long-standing convention
  (Chatterino), not in Twitch's docs; it worked on 2026-11-01 (16 channels joined at once).
- **Twitch's recommendation** **[src]**: EventSub `channel.chat.message`, which needs a user token with
  `user:read:chat`. Its message fragments: text, emote, mention, cheermote, `gif` (2026-07).
- **Emotes**:
  - The IRC `emotes` tag gives `id:start-end` ranges (end included) that **count Unicode code points**
    (Rust `char`s), checked on 2026-11-01: on 8 captured lines with an emoji before an emote, code points
    give the emote's code and UTF-16 units do not. Never slice a `&str` with them. In a `/me` message the
    ranges count from the text inside `\x01ACTION …\x01`; in a reply, from the whole text, which starts with
    the `@name ` Twitch adds.
  - Twitch emote images: `https://static-cdn.jtvnw.net/emoticons/v2/{id}/{static|animated}/{dark|light}/{1.0|2.0|3.0}`
    **[src]**.
  - Badges come from Helix Global and Channel Chat Badges, with direct image URLs **[src]**.
  - Third-party emotes, matched by word and keyed by the broadcaster's numeric id:
    - 7TV `https://7tv.io/v3/users/twitch/{id}` and `/v3/emote-sets/global`, checked on 2026-11-01:
      anonymous, a set's `emotes[]` entries carry the channel's alias (`name`) over the emote's own
      (`data.name`), files under `https:` + `data.host.url` (1x is 32 px high; WebP, AVIF, GIF, PNG;
      `static_name` is the still first frame of an animated one), zero-width as entry flag 1 (emote flag
      256); a Twitch user without 7TV answers 404 `{"error_code":12000}`;
    - BTTV `https://api.betterttv.net/3/cached/users/twitch/{id}` and `/3/cached/emotes/global`, checked on
      2026-11-02: anonymous; the global set is an array of `{id, code, imageType, animated, modifier}`, a
      channel's answer has `channelEmotes` and `sharedEmotes` (those carry the uploader's `user`); entries
      give `width`/`height` only when not 28 px square; `modifier` entries (`w!`, `h!`…) change the emote after
      them and are no image; files `https://cdn.betterttv.net/emote/{id}/{1x|2x|3x}.webp` (animated ones
      animate, no still variant); overlays are a fixed list of global codes (`cvMask`, `cvHazmat`, `TopHat`…);
      a Twitch user without BTTV answers 404 `{"message":"user not found"}`;
    - FFZ `https://api.frankerfacez.com/v1/room/id/{id}` and `/v1/set/global` **[mem]**.
- **VOD chat (replay)**, checked on 2026-11-02, anonymous: the raw query `video(id) { comments(contentOffsetSeconds: N) { edges { node { id contentOffsetSeconds createdAt commenter { id login displayName } message { fragments { text emote { emoteID } } userBadges { setID version } userColor } } } pageInfo { hasNextPage } } }` answers with the web Client-ID, no integrity token. A page holds about 50–60 messages, starting a little before `N` (a few seconds on a busy chat, up to ~20 s on a quiet one); asking `N`, `N+1`, `N+2` can return the same page. **Paging with `after: <cursor>` answers `failed integrity check`** (`IntegrityCheckFailed`): never sent. Past the end, `comments` is null. Subscriptions come as plain messages ("X subscribed at Tier 1…"); `userBadges` often starts with an empty `{setID: ""}`. Emote ids are the IRC ones.
- **VOD game chapters**, checked on 2026-11-03, anonymous: the raw query `video(id) { moments(first: 100,
  momentRequestType: VIDEO_CHAPTER_MARKERS, types: [GAME_CHANGE]) { edges { node { positionMilliseconds
  durationMilliseconds description details { ... on GameChangeMomentDetails { game { id displayName boxArtURL } } } } } } }`
  answers with the web Client-ID, no integrity token (about 40 ms). The first chapter starts at 0; a VOD that
  kept one game answers `edges: []`, an unknown id `video: null`. `boxArtURL` without arguments is a size template
  (`…/ttv-boxart/21779-{width}x{height}.jpg`), as Helix's. Used by `Client::vod_chapters`, given to the
  player as its chapters (only from two games). Several VODs in one query, one aliased `video(id)` each, at most 15
  as for the positions (`Client::vod_chapters_many`, checked 2026-11-03): the channel page's cards.
- **VOD resume position**, checked on 2026-11-02 with the auth-token: `currentUser { viewedVideos(first: N) }` and
  `video(id) { self { viewingHistory { position updatedAt } } }` read it (null signed out);
  `updateUserViewedVideo(input: {userID, videoID, position, videoType: VOD})` writes it, raw, with no integrity
  token (`UpdateUserViewedVideoPayload`, `updatedAt` moved). Helix has nothing like it. Used by
  `spaces::twitch::progress` (ADR 0013). Several VODs in one query: one aliased `video(id)` per
  VOD, at most 15 (`The number of root field aliases 40 exceeds the root field aliases limit
  allowed (15)`), checked the same day; a VOD never watched answers `position: null`.
- **Order of work** (done on 2026-11-01, animation included): anonymous IRC with Twitch emotes from the
  tag, deleted messages, 7TV, badges; FFZ dropped (the user's choice). BTTV added on 2026-11-02 (the user
  asked: its global `D:` is common in chats).
- **Fake chat** (debug builds only, `crates/peeby/src/spaces/twitch/fake_chat/`): "Faux chat" at the
  bottom of the Twitch sidebar, or `cargo run -p peeby -- --fake-chat`. It tests the chat column
  without a channel, an account or the network:
  - Generated lines are raw IRC (tags escaped, `char` emote ranges). They go through
    `twitch::chat::irc::parse` and `chat::event`, as a live's do.
  - Fake 7TV/BTTV words (animated, wide, zero-width), badges and the points icon are drawn in memory
    (`ui::chat::fake::Squares`, under `fake-chat://`). Only Twitch's own emotes, off by default,
    load from Twitch's CDN.
  - Controls on the left:
    - Feed: the interval (one message every 10, 5 or 2 s, up to 300 a second), bursts sent `BATCH` at a
      time, the connection state.
    - Content: emotes, badges, colors (dark ones included), mentions (of the viewer too), links, emoji,
      long lines, `/me`, highlighted messages.
    - Events: subs, gifts, raids, deletions, timeouts, a cleared chat, server notices.
    - The room's modes, sent as `ROOMSTATE` deltas.
    - Writing: the field's state, and what a sent message gets back: an echo, a refusal `msg-id`, or
      "not sent". The real 20 per 30 s limit applies.
  - Points and predictions are `points::PointsView` on `Backend::Fake`. It reads and spends a local
    state that never reports `minute-watched`:
    - The balance and a bonus chest.
    - Every kind of reward: with a text to write, subscribers only, unavailable, cooling down,
      unaffordable.
    - Refusal codes for spends.
    - Predictions with 2 or 5 outcomes, fake bettors, locking, a winner that gets paid, cancelling.

## Catalog through GQL instead of Helix

Probed on 2026-11-03, raw queries with the web Client-ID, anonymous and with the user's auth-token
(the same answers both ways, unless said):

- **Follows**: `user(login) { self { follower { followedAt } } }` reads whether the auth-token's account
  follows a channel (`self` null anonymous, `follower` null when not followed); `currentUser { follows }` answers
  nothing. **`followUser` and `unfollowUser` answer `failed integrity check`** (`IntegrityCheckFailed`), anonymous
  and with the auth-token (one no-op `unfollowUser` of a channel not followed), and Helix has had no follow
  endpoint since 2021: the follow button opens twitch.tv (`spaces::twitch::follow`), never routed around.
- **Answers in full** (no integrity token): `users(ids: [...])` and `user(login)` (`description`,
  `profileImageURL(width)`, `offlineImageURL`, `roles { isPartner }`), `searchUsers(userQuery, first: 20)`
  and `searchCategories(query, first: 20)` (their second page too), `searchFor(userQuery, platform: "web")`,
  `badges { setID version title imageURL(size: NORMAL | DOUBLE | QUADRUPLE) }` (global, 556 versions,
  ~1.4 s) and `user { broadcastBadges { … } }` (a channel's).
- **First page only**: `games(first, options: { sort: VIEWER_COUNT })`, `game(id) { streams(first,
  options: { sort: VIEWER_COUNT }) }`, `user { videos(first, type: ARCHIVE, sort: TIME) }` (with
  `lengthSeconds`, `previewThumbnailURL(width, height)`, `muteInfo { mutedSegmentConnection { nodes {
  offset duration } } }`) and `user { clips(first, criteria: { period: ALL_TIME, sort: VIEWS_DESC }) }`.
  `first` goes up to 100; **the second page (`after: <cursor>`) answers `failed integrity check`**,
  signed in or not, as the VOD chat's.
- **Closed**: `currentUser { follows }` and `currentUser { followedLiveUsers }` answer `service error`
  with a valid auth-token (`currentUser { id }` answers). `personalSections(FOLLOWED_SECTION)` answers
  the popular section instead. Twitch's own pages send headers Peeby does not have (a device id);
  making them up is routing around a bot check, so this stops here: **the follows need Helix**.
- **Probed again on 2026-11-04** (auth-token, web Client-ID): raw `currentUser { follows(first, order) }`
  and `followedLiveUsers(first)` still answer `service error`, even with a single field; another
  user's `user(login) { follows }` answers an empty list (follows are private). Twitch-Channel-Points-Miner's
  persisted `ChannelFollows` (hash `eecf8152…`) answers **`failed integrity check`**: closed, not routed
  around. Its persisted `PersonalSections(FOLLOWED_SECTION)` answers the popular section, as the raw one.
  Anonymous `users(ids:) { stream { id viewersCount game { id } } }` does tell who is on air: with the
  list of follows known, the live readings need no Helix.
- Used by `twitch::catalog::Catalog` (ADR 0016), with Helix as its backup when connected;
  fixtures `gql_catalog_*.json` (`twitch_capture -- catalog`), counts in `smoke`. The same image
  templates as Helix's (`{width}x{height}`), the same badge URLs (`/1`, `/2`, `/3`), and
  `users(ids:)` takes the 97 ids of a category's first page at once.

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
| VOD cards' chapters ("3 chapitres" over the picture, each playing the VOD from there) | `ui::player::chapter_list` in a `Popover`, `Route::Vod { start }` | Covered |
| VOD game chapters (twitch.tv's chapter button), with the game's box art | `set_chapters_with_pictures`: seek bar pieces, hover title and picture, chapters menu, Page Up / Page Down | Covered |
| VOD muted parts (`muted_segments`) red on the seek bar, told on entering, skippable | `set_segments` with `SegmentKind::Muted` | Covered |
| Size-templated images (`{width}x{height}`, `%{width}x%{height}`) | `Picture::sized` (the space fills the template) | Covered |
| Stream, VOD and clip cards; category box art (tall) | `MediaCard` (`badge`, `lines`, `avatar`, `poster` at any ratio) | Covered |
| A watch page with the player and a chat beside it | `ui::chat::watch_layout` (step 9) with `ui::chat::ChatPanel` | Covered |
| Live: no seek bar, a live badge, uptime and viewers instead; no 0–9/Home seeks | `Source::live` (step 4) | Covered |
| Resuming a paused live goes back to its live edge, without delay | `PlayerView::toggle_pause`: after more than 1 s of pause, mpv restarts on the current quality ("Retour au direct") | Covered |
| Ads removed | `twitch::Proxy`, the local playlist proxy (step 3) | Covered |
| Low latency | The proxy adds Twitch's prefetch segments (step 10) | Covered |
| Animated emotes (WebP, GIF) | `ui::images::decode` keeps the frames, `img` with an id plays them | Covered (step 9) |
| Chat WebSocket | `tokio-tungstenite` (agreed), `twitch::chat::connect` | Covered (step 9) |
| Single-use refresh token kept at once | Credential Manager (`keyring`), as Jellyfin's token | Covered (small secrets) |
| GQL hashes and Client-ID refreshed without a release | The Client-ID is read from twitch.tv's page at startup (`Client::refresh_client_id`); raw queries, no hashes | **Partly**: since ADR 0016 the whole catalog depends on GQL too; the Helix backup covers a connected user |

## The auth-token tutorial

Since [ADR 0017](decisions/0017-sign-in-webview.md) this is the manual way: "Se connecter à Twitch…"
opens twitch.tv's sign-in in a WebView2 window (`peeby --login twitch`) and reads the `auth-token`
cookie, checked with `currentUser` like a pasted one. Only the sign-in goes through it: the window
closes right after, never kept for integrity tokens or a device id (ADRs 0012, 0016). The tutorial
stays below the button, and is what a failure points to.

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
> 3. Chrome ou Edge : onglet « Application », puis « Cookies ». Firefox : onglet « Stockage », puis
>    « Cookies ». Choisissez le site twitch.tv.
> 4. Dans la liste, trouvez la ligne `auth-token` et copiez sa valeur (30 lettres et chiffres).
> 5. Collez-la ici.
>
> Ce jeton donne un accès complet à votre compte : ne le partagez pas. Peeby le garde dans le Gestionnaire
> d'identification de Windows. Il reste valable même après une déconnexion du site ; pour l'annuler,
> changez votre mot de passe Twitch ou utilisez « Se déconnecter partout ».

Peeby checks a pasted token (30 alphanumerics, then a GQL call that answers as signed in) before saving it,
as it checks imported YouTube cookies.

## Channel points

Probed on 2026-11-02 with the web Client-ID and the user's auth-token, and nothing else: no extra
headers, no persisted hash. Decisions: [ADR 0012](decisions/0012-twitch-channel-points.md).

- **Reads, raw queries** **[src]** (checked):
  - `user(login) { id channel { self { communityPoints { balance availableClaim { id }
    activeMultipliers { factor } } } communityPointsSettings { isEnabled name image customRewards { … } } } }`
    gives the viewer's balance, the waiting bonus chest and the custom rewards (`cost`, `prompt`,
    `isUserInputRequired`, `isEnabled`, `isPaused`, `isInStock`, `isSubOnly`, `cooldownExpiresAt`,
    `backgroundColor`, `image`, `defaultImage`).
  - `channel(name) { activePredictionEvents lockedPredictionEvents }` gives the predictions
    (`status`, `createdAt`, `predictionWindowSeconds`, `outcomes { totalPoints totalUsers color }`,
    `self { prediction { points outcome } }`).
  - Twitch-Channel-Points-Miner's persisted `ChannelPointsContext` answers too, with more fields:
    automatic rewards, goals, earning rates (`passiveWatchPoints`, `claimPoints`…).
- **Spends, raw mutations** (checked with made-up ids, so nothing was spent):
  - `redeemCommunityPointsCustomReward(input: {channelID, cost, prompt, rewardID, title,
    transactionID, textInput})` and `makePrediction(input: {eventID, outcomeID, points,
    transactionID})` answer `{ error: { code: "NOT_FOUND" } }`. That is an answer, not an integrity
    failure. A real spend was **not** tried, so whether a real reward passes is **[?]**.
  - The miner's smallest bet is 10 points **[src]**.
- **The bonus claim** (checked): `claimCommunityPoints(input: {channelID, claimID})` answers
  `failed integrity check` (`extensions.code: IntegrityCheckFailed`, `challenge.type: integrity`),
  even with a made-up claim id. The miner sends it with the Android TV Client-ID (`ue6666…`) and a
  token from that client's own device sign-in **[src]**. Peeby does not try that route
  (ADR 0012).
- **Points for watching**:
  - The web player reports time watched by POSTing base64 `minute-watched` events (`channel_id`,
    `broadcast_id`, `user_id`, `player: "site"`, `location: "channel"`, `live`). Without `location`,
    spade still answers 204 but credits no points **[src: twitch-miner-go PR #42]**. They go to a
    `spade_url`, from twitch.tv's `settings.*.js` **[src]**, the same on the home page and on a
    channel's page (2026-11-02).
  - Without these events, Twitch credited nothing: the user played a live in Peeby for 20 minutes
    (2026-11-02), and the balance did not move. With the reports (and `location`), it grew by 10
    points (the user, same day).
  - On 2026-11-02, the page linked `https://assets.twitch.tv/config/settings.<hash>.js`, whose
    `spade_url` was `https://spade.twitch.tv/track` (fixtures `twitch_page_settings.html`,
    `twitch_settings.js`, trimmed). The body is `data=<base64 of the JSON list>`, form-encoded.
  - DNS filters (Pi-hole, AdGuard, NextDNS lists) often block `spade.twitch.tv` as telemetry. On the
    user's network it resolved to `0.0.0.0` (2026-11-02), and every report failed with "error
    sending request". Peeby does not get around it: the user allows the domain or earns nothing. The
    first failure in a row logs a warning with its causes.
  - The bonus chest comes after about 15 minutes of credited watching **[mem]**.
- **Live updates**: the miner listens to PubSub (`wss://pubsub-edge.twitch.tv/v1`, topics
  `community-points-user-v1.<id>`, `predictions-channel-v1.<id>`) **[src]**. Whether PubSub still
  serves them is **[?]**. Peeby polls instead.
- Fixtures: `gql_points_context.json` (balance replaced by 12345), `gql_predictions_active.json`,
  `gql_predictions_locked.json`, `gql_redeem_not_found.json`, `gql_predict_not_found.json`.
  Request ids are redacted. Rewards and predictions are the channels' public data.

## Checked live on 2026-11-01

A probe run by the user, anonymously, on a live channel:

- The raw `PlaybackAccessToken` query (yt-dlp's text, no persisted hash) answers a token with both the web
  Client-ID and yt-dlp's `ue6666…`, with `playerType` `site` and `embed`. No integrity token was asked.
- usher v2 (`/api/v2/channel/hls/`) and v1 (`/api/channel/hls/`) both answer 200, in about 150 ms. The v2
  master is larger (17 KB against 10 KB, with `supported_codecs=av1,h265,h264` and `multigroup_video=true`).
- **The v2 master has its own format**:
  - one `#EXT-X-SESSION-DATA:DATA-ID="…",VALUE="…"` line per key, where v1 has a single
    `#EXT-X-TWITCH-INFO` line with the same keys (`USER-IP`, `SERVING-ID`, `VIDEO-SESSION-ID`, `NODE`,
    `C` and `E`, which are base64 segment URLs…);
  - no `EXT-X-MEDIA` groups: each `EXT-X-STREAM-INF` carries `STABLE-VARIANT-ID` (`1080p60`,
    `1080p60-portrait`, `audio_only`), `IVS-NAME`, `IVS-GROUPS` (`landscape`, `portrait`) and
    `IVS-VARIANT-SOURCE`.
  - A channel with `CHANNEL-METADATA="multitrack_video,multigroup_video"` sends both a landscape and a
    portrait ladder. Here the landscape one was H.264 and the 1080p60 portrait one HEVC. Peeby keeps
    `landscape`.
- **Audio is muxed** (`CODECS="avc1.4D401F,mp4a.40.2"`), and an `audio_only` variant exists. Media
  playlists are fragmented MP4 (`EXT-X-MAP`), 2 s segments, `TARGETDURATION:5`.
- **Anonymous playback starts with a pre-roll**: the first media playlist was all ad, 30 s, every segment
  marked three ways, as Streamlink expects:
  - `EXT-X-DATERANGE` with `CLASS="twitch-stitched-ad"`, `ID="stitched-ad-…"`, `DURATION=30.235` and
    `X-TV-TWITCH-AD-ROLL-TYPE="PREROLL"`;
  - a `CLASS="twitch-stream-source"` range with `X-TV-TWITCH-STREAM-SOURCE="Amazon|…"`;
  - `#EXTINF:2.000,Amazon|…` on each segment.

  Dropping the segments alone would leave the screen empty for 30 s at the start. See the plan, step 3:
  the proxy plays a backup stream while an ad runs.
- **Pre-roll by `playerType`** (second run, anonymous, same channel, two runs in a row):
  - only `site` got the pre-roll;
  - `embed`, `popout`, `frontpage` and `channel_home_live` got none and kept 1080p60;
  - `autoplay` and `picture-by-picture` got none but stopped at 360p30, `thunderdome` at 480p30.

  Streamlink's default is `embed`. Mid-rolls were not observed (none ran during the capture), so whether
  `embed` escapes them too is unknown.
- **How a pre-roll is served** (`live_preroll_00`…`24`):
  - The ad playlist starts at `EXT-X-MEDIA-SEQUENCE:0` and grows in real time: one 2 s segment every 2 s,
    16 segments for the 30.235 s ad, the last one 0.235 s. It cannot be skipped by reading ahead.
  - The live then follows in the same playlist after an `EXT-X-DISCONTINUITY`, a new `EXT-X-MAP` and
    `#EXT-X-TWITCH-LIVE-SEQUENCE:4567`. Its segments are titled `live`, and a
    `twitch-stream-source` range has `X-TV-TWITCH-STREAM-SOURCE="live"`.
  - Ad quartile ranges (`twitch-ad-quartile`, 0 to 4) mark the ad's progress.
  - The ad-free `embed` playlist is an ordinary sliding window: `EXT-X-MEDIA-SEQUENCE:4536` equal to
    `EXT-X-TWITCH-LIVE-SEQUENCE:4536`.
  - The live sequence number is what lines up the main and the backup playlists.
- **Prefetch**: two `#EXT-X-TWITCH-PREFETCH:` lines end each live playlist. Given to FFmpeg as ordinary
  segments after the content (step 10), they start a live about 2.6 s behind its capture instead of about
  7 s (measured against the segments' `PROGRAM-DATE-TIME`).
- **Codecs** (the user's auth-token, `supported_codecs=av1,h265,h264`, 2026-11-01): 16 of the top 100 lives
  send their 1440p60 source in HEVC only, every lower quality in H.264; the others are all H.264. No AV1.
  Anonymously, the 1440p source is not offered. A quality is never repeated in two codecs.
- **Redaction** (fixtures and anything printed): redact the keys by name in both formats, plus the ad
  ranges' `AD-SESSION-ID`, `RADS-TOKEN`, `CLICK-BEACON-ID`, `X-TV-TWITCH-SESSIONID` and `*-URL` values.
  A blanket IPv4 pattern corrupts HEVC codec strings (`hev1.1.6.L123.B0`-like): never use one.

## Not verified

- The Helix details marked [mem]: the reference page was truncated when fetched.
- The BTTV and FFZ endpoints.
- The 800 points a minute.
- Whether the playback token needs an integrity token when signed in with the auth-token (anonymous does
  not, see above).
- Ad-free tokens from a Helix token (very likely not).
- Muxed audio.
- WebView2 for Client-Integrity.
- GQL's recommendation queries.
- A real reward redeemed or a real bet placed (only made-up ids were sent); PubSub.

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
- twitch-miner-go, the spade payload's `location` and `player`: https://github.com/Guliveer/twitch-miner-go/pull/42
- Twitch-Channel-Points-Miner-v2 (GQL operations, claim, spade, PubSub topics):
  https://github.com/rdavydov/Twitch-Channel-Points-Miner-v2 (`constants.py`, `classes/Twitch.py`,
  `classes/WebSocketsPool.py`, `classes/entities/Stream.py`)
- 7TV endpoints (third-party README): https://lobehub.com/fr/mcp/daugustin-7tv-mcp
- HLS ad discontinuities: https://github.com/video-dev/hls.js/issues/4416,
  https://github.com/xbmc/inputstream.adaptive/issues/1071
