# Twitch space: technical plan

Status: agreed 2026-11-01 (the user's answers in [Decisions of the user](#decisions-of-the-user)), not
started. It turns [twitch.md](twitch.md) (the research) and [ADR 0010](decisions/0010-twitch-space.md) (the
decisions) into steps. Each step ends with `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and a run of the app, and YouTube and Jellyfin keep working after every step. Tick
the boxes as steps land, and list under each step what was verified and what was not.

## Decisions of the user

2026-11-01:

- **Client-ID** of Peeby's application on dev.twitch.tv (public client): `zsxaog2l339o1lbld85h17ksscnjwr`.
  A Client-ID is not a secret: it is a constant in the `twitch` crate, overridable in `settings.toml`.
- **Dependencies agreed**: `tokio-tungstenite` with `native-tls` (chat), tokio's `net` feature for the proxy
  (hand-written HTTP/1.1 GET handler, no HTTP server crate), `image`'s animation features (GIF, animated
  WebP) for animated emotes.
- **Nothing without signing in**: signed out, the space shows only its sign-in card.
- **Low latency and HEVC/AV1 are in scope** (step 10).
- Writes to the account (following, sending chat messages) stay out of scope.
- **Live and chat are generic** (2026-11-01): they must also serve YouTube lives later, and possibly other
  platforms. See the next section.

## Live and chat are generic

The user's requirement (2026-11-01): the live mode and the chat must also serve **YouTube lives** later,
and possibly other platforms. As with the player, `ui` holds the shared models and components, which know
no service. Each space only converts its own data:

- **Live playback**: done this way at step 4.
  - `ui::player::Source::live`, `LiveInfo { viewers, started_at }` and `set_notice` hold nothing
    Twitch-specific.
  - A YouTube live is the same `Source` with `live: true`. Its stream is VISIONOS HLS with a sliding
    window, so it needs no proxy and has no ads to remove.
  - What stays in the Twitch space: the proxy, the ad state behind the notice's text.
- **Chat model** in `ui::chat`, with no service type:

  | Type | Content |
  |---|---|
  | `ChatMessage` | `id`, `author { name, color, badges: Vec<Picture> }`, `fragments: Vec<Fragment>`, `time`, `kind` (normal, highlighted, system notice such as a sub or a raid), `deleted` |
  | `Fragment` | `Text`, `Emote { name, picture }` (static or animated), `Mention`, `Link` |
  | `ChatEvent` | `Message`, `Delete(id)`, `ClearUser(name)`, `Clear`, `State` (slow mode, sub-only…) |

  - Twitch maps IRC (`PRIVMSG`, `CLEARMSG`, `CLEARCHAT`, `USERNOTICE`, `ROOMSTATE`) and its
    third-party emotes onto it.
  - YouTube will map InnerTube's live chat: `live_chat/get_live_chat` continuations, with
    `liveChatTextMessageRenderer` runs made of text and emoji, and paid messages and memberships as
    highlighted messages. Its fixture comes before its parser, as always. **[?]** until then.
- **Chat component** `ui::chat::ChatPanel`:
  - an entity fed with `ChatEvent`s by its page, which owns the connection;
  - a variable-height virtual list, at most 500 messages;
  - wrapping text with inline emote images;
  - auto-scroll paused when the user scrolls up;
  - deleted messages greyed;
  - author colors made readable on the theme.

  It knows neither IRC nor InnerTube. Sending messages (later, a write to the account) would be a
  callback the page gives it.
- **Connection**: a per-service source in each space (`spaces/twitch/chat.rs` over `twitch::chat`, later
  `spaces/youtube/chat.rs` over `innertube`), which turns its protocol into `ChatEvent`s on a channel.
  The panel never sees a socket or a continuation.
- **Watch layout with a chat column**: the player and the chat beside it, with theater mode putting the
  chat under or beside the player. It is a `ui` layout helper both spaces' watch pages use, not a Twitch
  page detail.
- **Animated emotes** (frames in the image store) are a `ui::images` feature, for any space.

## Facts checked for this plan

Re-read on 2026-11-01, in addition to twitch.md:

- **A public client cannot get an app token.** Twitch's OAuth page: public clients "cannot use any of the
  other flows like client credentials", which is the only way to an app token. They "can refresh an access
  token without passing a client secret". Refresh tokens from the device flow "are for one time use only".
  So **every Helix call needs the user's token**: signed out, Peeby has no Helix catalog. This corrects
  twitch.md, which said an app token was enough for several endpoints. The user chose to require the
  sign-in for the whole space.
- **Validation is required.** "Your app must validate the OAuth token when it starts and on an hourly basis
  thereafter" (`GET https://id.twitch.tv/oauth2/validate`).
- **The ad layer is a local HTTP proxy, not a libmpv stream callback** **[mem]**. mpv opens HLS through
  FFmpeg's demuxer. FFmpeg fetches the nested media playlists and segments with its own I/O (mpv's
  `demux_lavf` hands nested opens to FFmpeg's default `io_open`), not through mpv streams. A protocol added
  with `mpv_stream_cb_add_ro` would only serve the master, never the media playlist FFmpeg reloads every few
  seconds. The proxy also leaves the copied `player` crate alone (no ADR 0001 note). The spike (step 1)
  confirms this; it does not choose between the two.

## Signed out

The space shows its sign-in card and nothing else (the user's choice): no catalog, no playback, no chat.
`peeby twitch.tv/<login>` opens the Twitch tab on that card and plays the channel once signed in. The GQL
playback calls still use the web Client-ID (and the auth-token when given), never the Helix token: the two
are separate credentials (twitch.md, "Credentials"). The spike (step 1) runs anonymously; it is a tool, not
the app.

## Target layout

```
crates/twitch/                  NEW, no GPUI; thiserror; lenient parsing from serde_json::Value
  src/lib.rs                    Client (reqwest), re-exports
  src/error.rs                  Error: Http, Status, RateLimited{reset}, NotLoggedIn, Offline,
                                IntegrityRequired, SubscriberOnly, Parse
  src/gql.rs                    POST gql.twitch.tv/gql: playback tokens (live, VOD), clip token, storyboard
  src/gql_config.rs             GqlConfig {client_id, hashes}: defaults, settings override, refresh
  src/usher.rs                  usher URLs (v1 or v2, decided by the spike), master parsing → Variant
  src/ads.rs                    pure filter: media playlist in → playlist without ads + AdState out
  src/proxy.rs                  127.0.0.1 server serving rewritten master and media playlists
  src/auth.rs                   device flow, refresh (single-flight), validate
  src/helix.rs                  Helix calls, pagination, rate-limit headers
  src/models.rs                 Stream, Channel, User, Category, Video, Clip, Page<T>
  src/chat/irc.rs               pure IRC parser (tags, PRIVMSG, CLEARCHAT, CLEARMSG, USERNOTICE, ROOMSTATE)
  src/chat/connection.rs        WebSocket connection, PING/PONG, reconnect with backoff
  src/chat/emotes.rs            Twitch emote ranges, 7TV/BTTV/FFZ sets (later)
  tests/fixtures/               redacted responses, saved playlists, IRC lines
  examples/smoke.rs             live check of every endpoint, counts only
crates/ui/src/player/           live mode and a generic notice (step 4), for any space
crates/ui/src/chat/             NEW, generic: ChatMessage / Fragment / ChatEvent, ChatPanel, watch layout
                                with a chat column (step 9), for Twitch now and YouTube lives later
crates/peeby/src/spaces/twitch/
  mod.rs                        TwitchSpace: Space impl, page stack, sidebar
  session.rs                    keyring entries, restore before the UI, save in the background
  play.rs                       resolve a live/VOD/clip into a ui::player::Source (starts the proxy)
  watch.rs                      watch page on ui's layout with a chat column: title, channel, viewers, uptime
  chat.rs                       chat source: twitch::chat lines → ui::chat::ChatEvent (no UI of its own)
  following.rs, browse.rs, category.rs, channel.rs, search.rs, cards.rs (Twitch → MediaCard)
  settings_page.rs              sign-in (device code), auth-token field with its tutorial, quality, chat
spikes/twitch-play/             timings: GQL token, usher, proxy, first frame
```

The shell and `ui` learn nothing about Twitch. The only shared changes are those of step 4, plus
animated images at step 9 if agreed.

## Secrets and redaction

Same rules as ADR 0004 and 0009, plus what Twitch adds:

| Secret | Where it lives | Never |
|---|---|---|
| Helix access token (≈4 h) | memory only | logs, URLs, errors |
| Helix refresh token (single use, 30 days) | Credential Manager `peeby` / `twitch|refresh` | same |
| Web `auth-token` (full account access) | Credential Manager `peeby` / `twitch|auth-token` | same |
| Playback token `value` (JSON with the user's IP and user id) and `signature` | memory, inside the proxy | logs above debug, errors |
| usher and CDN URLs (token, sig in the query) | inside the proxy | logs above debug, errors |

- mpv only ever sees `http://127.0.0.1:<port>/<random>/…` for playlists, and absolute CDN URLs for segments.
- **Fixture redactions**:
  - the playback token's `value` becomes `{"redacted":true}`, `signature` becomes `REDACTED`;
  - the query strings of usher and CDN URLs become `REDACTED`, IPs `203.0.113.7`;
  - the `user_id`, login and display name of the signed-in user become `peeby_test`;
  - `Authorization` and `Client-Integrity` headers are never saved.
- Credential Manager reads block: the refresh token and the auth-token are read before the UI starts (as
  Jellyfin's). Writes go to the background executor.

## Steps

### 1. Playback spike (`spikes/twitch-play`)
- [x] `cargo run -p spike-twitch-play --release -- <login|twitch.tv/videos/id|vod:login>`, timing each
      stage as `spike-yt-play`: GQL `PlaybackAccessToken` → usher master → mpv first frame
- [x] Try usher v2 then v1 (`PEEBY_USHER=v1`), the raw query, both Client-IDs, `playerType`s
      (`PEEBY_PLAYER_TYPE`), codecs (`PEEBY_CODECS`). The persisted query was not needed
- Moved to step 3: the proxy hop, measured with the real proxy rather than a throwaway one
- [x] Fixtures, redacted: GQL answers (live and VOD), masters, media playlists across a pre-roll (start,
      middle, end, with `EXT-X-DATERANGE` and discontinuities), a subscriber-only VOD's 403 body
- [x] The [?] of twitch.md: muxed audio yes, `audio_only` yes, `supported_codecs=h264` gives 1080p60
- Exit: a live and a VOD play from the spike, with measured times, and the fixtures are committed
- Done (2026-11-01), anonymously. Probes with PowerShell scripts (in the scratchpad, not the repo), then
  `spikes/twitch-play`:
  - the raw query answers with both Client-IDs, and usher v2 and v1 both answer. Peeby uses v2 with
    `playerType=embed`;
  - **times from the click to playback**:

    | Run | Total | Token | Master |
    |---|---|---|---|
    | Live, best of 5 | 0.98 s | 0.29–0.39 s | 0.46–0.58 s |
    | Live, worst of 5 | 1.29 s | | |
    | Public VOD | 1.29 s | | |

    All in 1080p60 H.264 with D3D11 (GPU backend), without a pre-roll. The proxy hop is measured at
    step 3 with the real proxy;
  - fixtures saved, their redaction checked, in `crates/twitch/tests/fixtures/`:
    - both masters;
    - 25 reloads across a pre-roll;
    - an `embed` backup playlist;
    - the GQL token answers (live, VOD), the token's keys kept and their values replaced;
    - a subscriber-only VOD's 403 (`usher_vod_restricted.json`);
  - muxed audio confirmed. mpv reports a duration for a live (its 24–28 s window): `Source::live`
    (step 4) is needed to hide the seek bar.
- Learned on the way:
  - **usher's error body repeats the request URL, token and signature included**: only its
    `error_code` and `error` may be shown or logged;
  - many channels keep their VODs for subscribers (`vod_manifest_restricted`). The VOD token says so
    beforehand: `chansub.restricted_bitrates` lists every quality. So the space can mark such VODs
    without asking usher;
  - **signed out, the token caps the resolution**: `maximum_resolution_reasons` refuses `QUAD_HD` and
    `ULTRA_HD` with `AUTHZ_NOT_LOGGED_IN`. A channel whose source is 1440p60 served 1080p60 H.264 at
    most, even with `supported_codecs=av1,h265,h264`. HEVC/AV1 in landscape was not seen. Its test needs
    the auth-token (step 10).
- Not done: a subscriber VOD with the auth-token; a mid-roll

### 2. `twitch` crate: GQL, usher, errors
- [x] `Client` over `reqwest` (same features as `innertube`); the web Client-ID is a constant
      (`gql::WEB_CLIENT_ID`) that `Client::new` can override. The `[twitch]` settings override comes with
      the space (step 5). There are no hashes: the raw query needs none
- [x] Refresh: `Client::refresh_client_id` reads `clientId="…"` from twitch.tv's page (checked:
      `twitch_page_client_id.html`, a 640-character extract). A failed refresh keeps the current one
- [x] `playback_token(Target::Live | Target::Vod, player_type)`, with the auth-token when there is one
      (`Authorization: OAuth …`). `PlaybackToken::restricted` and `subscriber_only()` read
      `chansub.restricted_bitrates`. `Debug` hides the value and signature
- Moved to step 8, where their fixtures will be captured: `clip(slug)`, `vod_storyboard(id)`
- [x] Errors mapped from the answers, each tested on a fixture:
  - `NotLoggedIn` (GQL 401, `gql_invalid_auth_token.json`);
  - `NotFound` (a `null` token, `gql_playback_token_not_found.json`);
  - `Offline` (usher 404 `Can not find channel`, `usher_live_offline.json`);
  - `SubscriberOnly` (usher 403 `vod_manifest_restricted`).

  `IntegrityRequired` matches `integrity` in the GQL message: tested on Streamlink's shape only, since
  no real answer has asked for one. Other usher errors keep only their `error_code`: the body repeats
  the signed URL
- [x] `usher::master_url`, `usher::variants(master)`:
  - v2: `STABLE-VARIANT-ID` as id, `IVS-NAME` as name, `IVS-GROUPS` (`portrait` when not landscape);
  - v1: the `VIDEO` group, named by its `EXT-X-MEDIA`;
  - `Codec::of(CODECS)`, and `audio_only()` (no resolution).

  `IVS-VARIANT-SOURCE` is `source` on every variant, so it says nothing and is not read. `Client::master`
  tries v2, then v1 on an unmapped error
- [x] 13 tests on the fixtures; `examples/smoke.rs` checks live the Client-ID refresh, a live's token
      and master, an offline channel and an unknown one (counts only)
- Exit: `cargo test -p twitch` green on real responses

### 3. Ad filter and local proxy
- [x] `ads::parse(text, base) -> MediaPlaylist`, pure. It marks a segment as an ad when its `#EXTINF`
      title contains `Amazon` or its `PROGRAM-DATE-TIME` falls in a `twitch-stitched-ad` /
      `stitched-ad-…` range. On the 25 saved reloads, both rules mark the same segments. Content segments
      get Twitch's live sequence (`EXT-X-TWITCH-LIVE-SEQUENCE`, from the header, or from the tag after a
      pre-roll). `in_ad()`, `ad_elapsed()` and `ad_duration` describe the break. Relative URIs are made
      absolute (`ads::absolutize` for VODs)
- [x] **Backup during ads**: `splice::Splicer` keeps the content segments by live sequence. While the
      main playlist is in an ad, it adds the backup's newer segments. Its output for mpv:
  - `MEDIA-SEQUENCE` is the live sequence, so it never goes back;
  - a discontinuity on a change of source, a gap or a new init segment, with `EXT-X-MAP` repeated;
  - `EXT-X-DISCONTINUITY-SEQUENCE` counts those that left the 15-segment window;
  - the ad's `DATERANGE`s are dropped.

  `AdState { active, elapsed, duration, covered }` tells the space whether a notice is needed: only when
  not `covered`. Prefetch lines are dropped for now (low latency, step 10)
- Measured 2026-11-01 (twitch.md, "Checked live"): `site` gets a 30 s pre-roll, while `embed`, `popout`,
  `frontpage` and `channel_home_live` got none at 1080p60. So the main token uses `embed`, as Streamlink
  does (`gql::PLAYER_TYPE`), and the backup uses `popout` (`gql::BACKUP_PLAYER_TYPE`). Whether `embed`
  also escapes mid-rolls was not observed; the filter and the backup stay for that case
- [x] 8 tests on the saved playlists as a reload sequence:
  - each rule, and every reload;
  - the backup covering the pre-roll at once;
  - the main playlist taking over after a gap;
  - no backup: the ad only skipped, output empty until the live;
  - a plain live passing through.
- [x] `Proxy::start`, then `Client::play(target, codecs)`: the main and backup tokens and masters are
      asked for together (`tokio::join!`); a failing backup is only logged. The proxy:
  - listens on `127.0.0.1:0`, with a random 128-bit key in the path; HTTP/1.1 GET only, anything else
    404 or 405, an upstream failure 502;
  - serves `/<key>/<variant>.m3u8`: each reload fetches the upstream media playlist, and the backup
    only during an ad, then answers the `Splicer`'s playlist (live) or the absolutized one (VOD);
  - leaves out the master: mpv gets a variant's URL directly (`Proxy::variants`), so FFmpeg does not
    open every variant at start;
  - keeps the upstream URLs in its memory, its logs say no URL, and it stops when the `Proxy` drops.

  A subscriber-only token (`chansub`) fails as `SubscriberOnly` before usher is asked
- [x] `Proxy::ad_state()` instead of a channel: the space reads it on a timer (step 6), with no channel
      crossing the tokio and GPUI executors
- [x] The spike plays through the real proxy (`PEEBY_PROXY=1`):
  - a live in 1.35 s, against 1.21–1.29 s without the proxy;
  - with `PEEBY_PLAYER_TYPE=site` (forced pre-roll), 1.51 s, the 30 s pre-roll covered by the backup;
    the main playlist took over at its end, with no buffering event over 50 s.
- Not verified: a mid-roll, a backup that fails during an ad (the notice path), the picture at the switch
  by eye (the spike logs no stall; a screenshot was not taken)
- Exit: met. A live plays through the proxy, an ad break was skipped, and the start stays within 1–2 s
### 4. `ui::player`: live mode and notice
- [x] `Source::live: bool`. When live:
  - no seek bar, no ±10 s buttons, no remaining time: an "EN DIRECT" badge on the accent stands in for
    the times;
  - no seeking: ←/→, J/L, 0–9, Home and Page Up/Down are left to the page, and their help lines hide;
  - mpv's duration (the playlist's window) is ignored, so there is no "up next" and `finished()` stays
    false;
  - a quality change restarts at the live edge (`start_seconds` 0), not at the current position;
  - `PlayerEvent::Ended` goes to the page as before: the space says the live ended (step 6).
- [x] `PlayerView::set_live_info(Option<LiveInfo { viewers, started_at }>)`: shown after the badge
      (`12 345 spectateurs`, French grouping), the uptime redrawn every second while the controls
      show. The space refreshes the viewers (step 6)
- [x] `PlayerView::set_notice(Option<SharedString>)`, generic, in place of the loading message. The page
      updates the text itself for a countdown, so there is no notice model to keep in sync
- [x] Mini-player: nothing to change, it has no seek bar (picture, play/pause, back, close)
- [x] `PEEBY_LIVE=1 cargo run -p ui --example player`: live mode, 12 345 viewers, 1:23:47 of uptime,
      and an 8 s notice countdown
- [x] docs/player/playback-ux.md, "Live streams"
- Verified on a screenshot of the example:
  - with `PEEBY_LIVE=1`: no seek bar, no ±10 s, no previous/next, the badge, the uptime, the viewers
    and the notice;
  - without it: seek bar, previous/next, times and remaining time, as before.
  - Clippy, and tests: `group_thousands`.
- The app still opens and plays a YouTube video (`peeby <id>`, screenshot).
- Not verified: YouTube's control bar in the app (hidden before the screenshot), and Jellyfin. Their path
  is unchanged: `live` defaults to false, and only live branches were added. The live keys were not
  pressed
- Exit: met for the example; YouTube plays
### 5. The space and the sign-in (Helix device flow, auth-token)
- [x] `spaces/twitch/`: `TwitchSpace` with its accent (Twitch violet `#9146FF`) and badge `TW`, added in
      `main.rs` after YouTube and Jellyfin. Signed out, its only page is the sign-in card. While a saved
      session is restored, it shows "Connexion à Twitch…"
- [x] `twitch::auth`:
  - `Auth::start_device`, `poll` (`Pending`, `SlowDown`, `DeviceExpired`, `DeviceDenied`), `refresh`,
    `validate`, `revoke`;
  - pure parsers tested on 4 new fixtures: a device code (redacted, never approved), the pending
    poll, a refused refresh, a refused validation. The success shapes are tested on Twitch's
    documented ones, since none can be captured without the user;
  - `CLIENT_ID` is Peeby's, overridable in `[twitch] client_id`. The form is encoded by hand: reqwest's
    `form` feature is off.
- [x] **Single-flight refresh** in `auth::Session`:
  - `refresh_after(stale)` holds an async mutex, and returns the current token when another caller
    already refreshed;
  - the new refresh token goes to the `Store` callback (the Credential Manager, on a blocking thread)
    **before** it is used. A failed write is logged and the session kept in memory;
  - `Session::begin` stores the first one, `Session::restore` exchanges the saved one at startup.

  Helix's own 401 retry joins this at step 7, with the Helix client
- [x] Validation at startup (restore and sign-in) and every hour (a GPUI timer). `SessionExpired` signs
      out with "Votre session Twitch a expiré". A network error keeps the saved refresh token (it was
      not spent) and shows the card with the error
- [x] The sign-in card:
  - "Se connecter avec Twitch" shows the code in large type and opens `verification_uri`, which has
    the code filled in;
  - it polls at Twitch's interval, plus 5 s on `slow_down`; "Rouvrir la page" and "Annuler" stay
    available;
  - once approved, the session is stored and validated.

  The sidebar then shows "Suivies" (a placeholder page until step 7), the user (initial; the avatar
  needs Helix `users`, step 7) opening the settings, and "Se déconnecter", which revokes, deletes the
  entry and clears `[twitch] login`
- [x] Settings part:
  - the account ("Connecté en tant que …", "Se déconnecter"; signed out, a pointer to the tab);
  - the auth-token with the tutorial of twitch.md and a masked input. It is checked before saving:
    30 alphanumerics, then GQL `currentUser` answering a login (`gql::parse_current_user`, tested on
    an anonymous answer). It is saved in the Credential Manager. "Supprimer le jeton" is offered once
    a token is saved;
  - the storage row of the shell's page now says the Twitch tokens are in the Credential Manager too.
- [x] Scopes: `user:read:follows`
- Secrets: `peeby` / `twitch|refresh` and `peeby` / `twitch|auth-token` in the Credential Manager, read
  before the UI starts (`session::restore`)
- Verified:
  - `cargo run -p twitch --example smoke` live: a device code issued and polled as pending, an invalid
    token refused by `validate`;
  - in the app, from screenshots with posted clicks: the Twitch tab with its violet and the sign-in
    card, and the settings part (account and the tutorial, wrapped after a fix).
- **Not verified, needs the user**: approving a code, the session surviving a restart, the hourly
  validation, sign-out's revoke, a real auth-token being accepted
- Exit: pending the user's sign-in test
### 6. Live playback in the space
- [ ] `play.rs`: resolve a live into a `Source` (proxy master as `options.url`, `Referer` and
      `Origin: https://player.twitch.tv` in `http_headers`, one `Quality` per variant, `live: true`), the
      default quality from `[twitch]` settings
- [ ] Watch page (embedded layout as YouTube's): player, title, channel and avatar, category, viewers,
      uptime; a right column reserved for the chat (empty until step 9); theater mode as on YouTube
- [ ] `peeby twitch.tv/<login>` and `twitch.tv/videos/<id>` on the command line open the space on that
      channel or VOD (after the sign-in card when signed out)
- [ ] French messages for each error of step 2 (integrity: "Twitch demande une vérification que Peeby ne
      peut pas faire sans navigateur. Réessayez plus tard." — wording to review)
- [ ] The settings part gains the default quality
- Exit: `peeby twitch.tv/<login>` plays a live in the Twitch tab without ads, in the mini-player too

### 7. Helix catalog and pages
- Fixtures first, from signed-in responses, trimmed and redacted: `streams_followed.json`,
  `channels_followed.json`, `streams.json` (by category), `games_top.json`, `games.json`,
  `search_categories.json`, `search_channels.json`, `videos.json`, `clips.json`, `users.json`
- [ ] `helix.rs`: one `get<T>(path, query)`, pages (`first=100`, `after`), `Ratelimit-*` headers read;
      a 429 waits until `Ratelimit-Reset` once, then fails with `RateLimited`
- [ ] Image templates filled by the space: `{width}x{height}` and `%{width}x%{height}`, a cache-buster on
      live previews (rounded to 5 minutes, so the image store still caches), `404_processing` as "VOD en
      cours de traitement"
- [ ] Pages:
  - **Suivies** (sidebar's first entry): live followed channels as stream cards (preview, title,
    channel, category, viewers badge), then the offline followed channels as avatars;
  - **Parcourir**: top categories as tall box-art posters, next pages on scroll; a category's page:
    its live streams;
  - **Chaîne**: header (avatar, name, description), the live if on, VODs and clips tabs;
  - **Recherche** from the top bar: categories and channels (live first).
- [ ] Sidebar: Suivies, Parcourir, then the live followed channels (avatar, name, category as `detail`),
      refreshed every 2 minutes in the background; the user at the bottom (opens the settings)
- [ ] `examples/smoke.rs` covers every endpoint
- Exit: the followed channels and a category browse, open and play, signed in

### 8. VODs and clips
- [ ] VODs: usher VOD master through the same proxy (no ads to filter, but the token stays out of mpv's
      URLs), seekable, resume is not tracked (Twitch has no resume API through Helix **[?]**)
- [ ] Seek-bar previews: the storyboard query → a `Previews` impl (sprite sheets, as YouTube's)
- [ ] Subscriber-only VODs: the `SubscriberOnly` message, and a hint to add the auth-token when none is set
- [ ] Clips: direct MP4 from the clip token, `Source` without qualities when there is a single one
- [ ] Muted segments (`muted_segments`) shown on the seek bar if the shared seek bar can mark ranges
      (it marks chapters; otherwise skip)
- Exit: a VOD plays and seeks with previews, a clip plays

### 9. Chat (read-only), generic in `ui`
- Built as [Live and chat are generic](#live-and-chat-are-generic) says:
  - the model (`ChatMessage`, `Fragment`, `ChatEvent`) and the `ChatPanel` go in `ui::chat`, with no
    Twitch type;
  - the Twitch space converts IRC into them;
  - the panel's own tests and an example (`cargo run -p ui --example chat`, generated messages) do not
    involve Twitch, so that a YouTube live chat can plug in later without changing `ui`.
- Fixtures first: a file of raw IRC lines from a busy channel, with emotes, an emoji before an emote (to
  settle whether `emotes` ranges count code points), a `CLEARMSG`, a `CLEARCHAT`, a sub `USERNOTICE`
- [ ] UI spike, small, before the panel: a message with inline emote images that wraps in GPUI
      (`h_flex().flex_wrap()` of word runs and images, or `StyledText` with inline boxes **[?]**), and a
      variable-height virtual list (`gpui::list` with `ListState`) appending 20 messages a second without
      re-laying out the whole list. Measure at 500 messages kept
- [ ] `chat::irc`: pure parser, tested on the fixture; emote ranges mapped on `char` indexes, never byte
      slices
- [ ] `chat::connection`: `wss://irc-ws.chat.twitch.tv:443`, `CAP REQ`, `PASS`/`NICK justinfan<n>`,
      `JOIN #login`, PONG, reconnect with backoff (1, 2, 4… 30 s), stops when the page drops
- [ ] `ui::chat::ChatPanel` in the watch page's chat column (the shared layout helper):
  - the name in its color, lightened on the dark theme when too dark;
  - the message's fragments with emote pictures (static first);
  - at most 500 messages;
  - auto-scroll paused when the user scrolls up ("Reprendre le défilement");
  - deleted messages greyed.

  `spaces/twitch/chat.rs` turns `twitch::chat` lines into `ChatEvent`s
- [ ] Then, each as its own change: badges (Helix global and channel badges, needs sign-in), 7TV/BTTV/FFZ
      emotes by word (cached per channel), animated emotes (needs `image` animation features and frames in
      the image store, agreed)
- [ ] VODs: no chat replay at first (GQL comments, later)
- Exit: a busy channel's chat scrolls smoothly next to the player, with emotes
- Dependency: `tokio-tungstenite` with `native-tls` (agreed)

### 10. Polish and docs
- [ ] Bell: "en direct" notifications for followed channels that went live since the last check (a
      section of the aggregated panel, `Space::notifications`)
- [ ] **Low latency** (agreed): the proxy turns `#EXT-X-TWITCH-PREFETCH:<url>` lines into ordinary
      segments (Streamlink's approach), except after an ad discontinuity; mpv gets a small read-ahead
      for live sources (`cache-secs`, `demuxer-readahead-secs` around 2–4 s, to tune) through the space's
      `StartOptions`, so a new option in `player` only if no existing one fits (ADR 0001 note then). A
      "Faible latence" switch in the settings part, on by default. Measure the delay to the broadcast
      (the chat is the reference) with and without
- [ ] **HEVC and AV1** (agreed): check D3D11 hardware decoding of both on this PC first (mpv
      `hwdec-current` on a sample). Then `supported_codecs=av1,h265,h264`, and `usher::variants` keeps one
      variant per name and codec. The space picks the best codec the GPU decodes (a per-codec result kept
      in the settings after the first check). When mpv falls back to software decoding on a variant, the
      space moves to the h264 variant of the same quality. The quality menu shows the codec only when two
      codecs share a name
- [ ] docs: architecture.md (the `twitch` crate, the proxy in the playback path), design.md (Twitch pages,
      chat), twitch.md (the [?] settled by the steps), ADR 0010 consequences, README, CLAUDE.md (commands,
      Twitch rules: the proxy, the secrets), roadmap
- Exit: the roadmap's criterion: live and VOD playback without ads, followed channels, chat

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Twitch changes how it marks ads | Ads come back, or content is cut | Filter rules tested on saved playlists; a new capture becomes a fixture |
| Integrity token required on every playback token | No playback | Clear error; WebView2 behind a switch (ADR 0010), revisit |
| GQL Client-ID or hashes go stale | No playback | Raw queries, startup refresh, `settings.toml` override |
| The proxy hop slows the start | Over the 1–2 s budget | Measured at steps 1 and 3; the master can be fetched while the token resolves on hover (prefetch, as YouTube) |
| Refresh token burned by a race | Signed out | Single-flight refresh, keyring written first |
| Chat throughput on busy channels | UI stutter | Virtual list, cap of 500, batch appends per frame |

## Still to ask

- Any write to the account (following a channel, sending a chat message): not planned, each would need its
  own go-ahead and test.
- A playback test of a subscriber-only VOD or with the auth-token: it plays as the user's account.
