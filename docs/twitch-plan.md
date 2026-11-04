# Twitch space: technical plan

Status: done 2026-11-01 (the user's answers in [Decisions of the user](#decisions-of-the-user)): steps 1–10. It turns [twitch.md](twitch.md) (the research) and [ADR 0010](decisions/0010-twitch-space.md) (the
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
- Writes to the account (following) stay out of scope.
  2026-11-02: except channel points (redeeming a reward, betting on a prediction), which the user
  asked for. See [ADR 0012](decisions/0012-twitch-channel-points.md).
  2026-11-04: and sending chat messages, which the user asked for. See
  [ADR 0023](decisions/0023-twitch-chat-sending.md).
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
  sign-in for the whole space; on 2026-11-03 the catalog moved to anonymous GQL and Helix became
  optional, for the followed channels ([ADR 0016](decisions/0016-twitch-without-helix-sign-in.md)).
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
  src/points.rs                 channel points (ADR 0012): balance, rewards, predictions, spends
  src/spade.rs                  the watch-time report (`minute-watched`), which earns points
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
  points.rs                     a live's channel points in the watch header (ADR 0012)
  following.rs, browse.rs, category.rs, channel.rs, search.rs, cards.rs (Twitch → MediaCard)
  settings_page.rs              sign-in (device code), auth-token field with its tutorial, quality, chat
spikes/twitch-play/             timings: GQL token, usher, proxy, first frame
```

The shell and `ui` learn nothing about Twitch. The shared additions are generic: the live mode of step 4,
`ui::chat` (the chat model, `ChatPanel`, `watch_layout`) of step 9, and animated images in the image store
when they come.

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
- Verified with the user (2026-11-01):
  - they signed in with a code and saved their auth-token, which the GQL check accepted;
  - a restart restored the session (the saved refresh token exchanged, then validated): "Connecté à
    Twitch en tant que astalfr", the sidebar's entries, "Un jeton est enregistré" in the settings.
- Not verified: the hourly validation (an hour of running), sign-out's revoke (it would end the user's
  session), an expired session's card
- Exit: met. Signed in, restarted, still signed in; auth-token saved and checked
### 6. Live playback in the space
- [x] `play.rs`: a live's `Source` from the proxy's variants:
  - the landscape ones with a picture, best first, then "Audio seul";
  - each quality's `url` is the proxy's (`/<key>/<n>.m3u8`), with `Referer` and `Origin` for the CDN's
    segments, the `[player]` hardware setting, and `live: true`;
  - the first quality not taller than `[twitch] max_height` (default 1080) plays.
- [x] `watch.rs`:
  - `Client::play` (both tokens, both masters, the proxy) and the channel's header are asked for
    together;
  - the header is a GQL `Channel` query (`gql::parse_channel`, tested on a live and an offline
    channel): avatar, title, name in the accent, category, viewers, uptime. GQL, not Helix: it is one
    call for a channel, signed in or not, and Helix comes at step 7;
  - the player gets the name as title and `LiveInfo`, refreshed every minute;
  - every second, the proxy's ad state becomes the player's notice ("Publicité retirée · reprise dans
    N s"), only when the backup does not cover the break;
  - `Ended` shows "Le live est terminé.";
  - the chat column (340 px, "Le chat arrive bientôt.") until step 9, and theater mode shared with
    YouTube (`Settings::theater`).

  Dropping the page drops the proxy
- [x] The space keeps a stack: "Suivies" (until step 7, a field taking a name or a twitch.tv link, and
      `following::channel_login`, tested), then lives pushed on top. Back pops, and a covered player
      pauses. `Space::player` gives the shell the live's player for the mini-player
- [x] `peeby twitch.tv/<login>` opens the Twitch tab (`SpaceEvent::Activate`) and that live once signed
      in, after the card if signed out. VOD URLs wait for step 8
- [x] French messages (`play::describe`): offline, unknown channel, subscribers only, integrity, refused
      auth-token, unreachable
- [x] The playback client carries the saved auth-token (`set_auth_token` rebuilds it). The web
      Client-ID is refreshed from twitch.tv's page at startup
- [x] Settings part: "Qualité par défaut" (1440p to 360p)
- Verified in the app (`peeby https://www.twitch.tv/squeezie`, signed in with the user's auth-token,
  screenshots):
  - the live plays in 1.33 s from the click, 6 qualities (1080p60 to "Audio seul");
  - live controls: the badge, uptime 3:27:06 and 83 081 viewers, no seek bar, quality, theater,
    fullscreen;
  - the header, and the chat column.
- Not verified: the ad notice in the app (no ad ran, `embed` gets no pre-roll), the mini-player with a
  live, the offline and unknown channel messages in the app, the theater layout, back from a live
- Seen: signed in with the auth-token, the best variant was still 1080p60 with `supported_codecs=h264`,
  although the channel's source is 1440p60. The 1440p source is probably HEVC/AV1-only (step 10)
- Exit: met. `peeby twitch.tv/<login>` plays a live in the Twitch tab, without ads
### 7. Helix catalog and pages
- [x] Fixtures first: `cargo run -p peeby --example twitch_capture` signs in from the Credential
      Manager (the refresh token spent and its successor saved, so never while Peeby runs) and saves 8
      answers trimmed to 1 to 4 items. Everything about the account is replaced: the user, and the
      followed channels' identities, titles, previews, categories, viewers, start times, follow dates,
      total and cursor. Public lists are only trimmed. VODs and clips come with step 8
- [x] `twitch::helix`:
  - `Helix::get(path, query)`: a 401 refreshes the token once through the single-flight
    `Session::refresh_after`, then retries; a 429 is `RateLimited` (no waiting);
  - `me`, `users` (by id, 100 at most), `followed_streams`, `followed_channels`, `top_categories`,
    `category_streams`, `search_channels`, `search_categories`;
  - lenient parsers: an item without an id is skipped, never the page;
  - 9 tests on the fixtures.
- [x] Image templates: `helix::sized` fills `{width}x{height}` and `%{width}x%{height}`. Live previews get
      a five-minute stamp, fresh regularly while the image store still caches them. `404_processing`
      waits for VODs (step 8). The cards ask for the smallest size the CDN keeps cached that covers the
      drawn one (`cards::cached_size`: 320x180, 640x360, 1920x1080 for previews and VOD thumbnails,
      52x72 to 285x380 for box art): any other size is resized on the spot, about 2 s instead of 50 ms,
      and since the URL changed with the window's width, every resize downloaded the grid again
- [x] Pages (`cards.rs` holds the shared cards):
  - **Suivies**: the followed channels on air as stream cards (preview, avatar, title, channel,
    category, viewers), widened to fill their rows, then "Hors ligne", the others as avatars and
    names in alphabetical order, under a field filtering them by name or login. It reads up to 300
    live and 500 followed channels;
  - partners (Helix `users`, `broadcaster_type`) get Twitch's verified badge on their avatar in
    "Hors ligne" and in the search's channels;
  - **Parcourir**: the top 200 categories as 3:4 box-art posters. A category's page shows its 100
    lives with the most viewers;
  - **Rechercher** from the top bar: up to 20 channels (on air first, with "En direct") and the
    categories.

  A click on a channel opens its live; an offline one says so (its page with VODs and clips: step 8)
- [x] Sidebar: Suivies, Parcourir, then up to 12 followed channels on air (avatar, name, category,
      a red "LIVE" pill, a dot on the avatar when folded), refreshed every 2 minutes; the user and sign-out at the bottom
- [x] A Helix `SessionExpired` from any page signs the space out with its message
- Moved to step 8: the channel page (header, live, VODs, clips)
- Not in `smoke`: Helix needs the user's session; the capture tool is the live check
- Verified in the app, signed in (screenshots, posted clicks and keys):
  - "Suivies" with the user's live channels and their avatars;
  - "Parcourir" with the categories, then "Just Chatting" with its lives on 4 columns;
  - the search "zerator", channels with the live one first;
  - the sidebar's live channels.
- Not verified: the "Hors ligne" section (below the fold), the sidebar's refresh after 2 minutes, a
  429, `SessionExpired` from a page
- Exit: met. Followed channels and a category browse and open, signed in
### 8. VODs and clips
- Fixtures first:
  - GQL, anonymous (scratchpad probe): a clip's playback (`gql_clip_playback.json`, token redacted), a
    VOD's `seekPreviewsURL` and the storyboard JSON it points to;
  - Helix, from `twitch_capture`: a public channel's archives and clips (`helix_videos.json`,
    `helix_clips.json`).

  The storyboard of a VOD still being broadcast answers 403: it has none until the broadcast ends
- [x] VODs: `Client::play(Target::Vod)` through the same proxy (no ads to filter, the token stays out
      of mpv's URLs), seekable, with the qualities of the master. Resume: the account's position
      (`viewingHistory`, saved with `updateUserViewedVideo`, ADR 0013)
- [x] Seek-bar previews:
  - `Client::storyboard` → `gql::StoryboardLevel`s (largest frames first; `frame_at` gives the sheet,
    column and row);
  - `play::Storyboard` is the shared `Previews`;
  - tested on the saved storyboard (first, middle and last frames).
- [x] Subscriber-only VODs: the `SubscriberOnly` message, and a hint to add the auth-token when none is
      saved
- [x] Clips: `Client::clip` (raw GQL query, `gql::parse_clip_playback`) gives the MP4s signed with the
      token, one quality each (1080p60 to 360p). They play as direct files
- [x] Channel page (from step 7), `channel.rs`:
  - the header from Helix `users` (avatar, name, description);
  - the live from GQL `Channel` ("EN DIRECT", title, category, viewers, "Regarder");
  - "Vidéos" (40 archives) and "Clips" (40 most viewed) tabs, as grid cards widened to the row (VODs:
    duration, views, date; clips: author, views, date).

  Channels in the offline list and the search open it; the channel's name under a live, a VOD or a
  clip opens it too
- [x] `helix::Video`, `Clip`, `parse_duration` (`3h8m33s`), `user` (by login), `videos`, `video`,
      `clips`, `clip`; 3 new tests. `peeby` takes `twitch.tv/videos/<id>`, `clips.twitch.tv/<slug>` and
      `twitch.tv/<login>/clip/<slug>` too (`Route::from_url`, tested)
- [x] Muted segments (2026-11-02): `Video::muted_segments`, merged when they touch (`play::muted`), given to
      the player as `SegmentKind::Muted` segments: red on the seek bar, "Son coupé" in the hover preview, an
      OSD "Son coupé (droits d'auteur)" on entering one, and the skip button "Passer la partie muette" (S)
- [x] Game chapters (2026-11-03): `Client::vod_chapters` (GQL `moments`, `GAME_CHANGE`), given to the player
      with their box art (`set_chapters_with_pictures`) when a VOD has two games or more; the channel page's
      VOD cards list them too (`Client::vod_chapters_many`), a click plays the VOD from that chapter
      (`Route::Vod { start }`, which links' `?t=` fill as well) (`play::chapters`), without delaying playback
- Verified in the app (screenshots, posted clicks and moves):
  - a public VOD plays in 0.99 s (7 qualities, seek bar, 8:09:09), with a storyboard frame shown when
    hovering the seek bar;
  - a clip plays in 1.34 s (4 MP4 qualities);
  - the channel page opens from the clip's channel name, with the live banner and the VODs.
- Not verified: the Clips tab by click, a subscriber-only VOD's message in the app, a VOD with the
  user's auth-token, `404_processing` thumbnails (they show empty)
- Exit: met. A VOD plays and seeks with previews, a clip plays
### 9. Chat (read-only), generic in `ui`
- Built as [Live and chat are generic](#live-and-chat-are-generic) says:
  - the model (`ChatMessage`, `Fragment`, `ChatEvent`) and the `ChatPanel` go in `ui::chat`, with no
    Twitch type;
  - the Twitch space converts IRC into them;
  - the panel's own tests and an example (`cargo run -p ui --example chat`, generated messages) do not
    involve Twitch, so that a YouTube live chat can plug in later without changing `ui`.
- [x] Fixtures first: `tests/fixtures/chat_lines.txt`, 21 lines from busy public channels (anonymous
      connection, 10 minutes, 12,700 lines captured): a full `ROOMSTATE`, plain, highlighted and first
      messages, emotes (`emotesv2_` ids too), emoji before emotes, two `/me`, two replies, a cheer, a
      `CLEARMSG`, a timeout and a ban, `sub`, `resub`, `subgift`, `submysterygift`, `viewermilestone`,
      `announcement`. Chatters are renamed `viewerN` (ids `100000N`) with aliases as long as the names they
      replace, mentions and quoted replies included, so that emote ranges still fit; client nonces dropped.
      **Settled**: the ranges count code points (twitch.md, "Chat")
- [x] UI spike (`cargo run -p ui --example chat --release`): words as flex-wrapped runs with emotes in fixed
      boxes (`StyledText` has no inline boxes in this GPUI), `gpui::list` with `FollowMode::Tail`. At 20 and
      50 generated messages a second with 500 kept: 120 frames a second, the slowest frame 16 ms at 20/s and
      24 ms at 50/s. An `img` keeps its own ratio over a set height: emotes set `aspect_ratio`
- [x] `chat::irc`: pure parser (tags unescaped, prefix, parameters), events (`Message`, `Notice`, `Delete`,
      `ClearUser`, `Clear`, `RoomState` merged by the page, `Ping`, `Reconnect`, `Joined`, `ServerNotice`),
      `ChatLine::pieces` cutting the text at its emotes on `char` indexes (ranges that do not fit dropped).
      5 unit tests, 9 fixture tests
- [x] `chat::connect`: `wss://irc-ws.chat.twitch.tv:443`, `CAP REQ`, `PASS`/`NICK justinfan<n>`,
      `JOIN #login`, PONG, `RECONNECT` at once, other losses with 1, 2, 4… 30 s (back to 1 s after a
      connection that joined), 6 minutes without a line taken as dead, stops when its receiver is dropped.
      Updates on a bounded tokio channel (1,024), awaited from the UI thread
- [x] `ui::chat` (no Twitch type): the model (`ChatMessage`, `Author`, `Badge`, `Fragment`, `Emote`,
      `MessageKind`, `ChatEvent`), `split_text` (mentions, links), `name_color` (the user's color lightened
      to a luminance of 0.35 on the dark theme, else one of 12 by name), `ChatPanel`:
  - the name in its color, then the fragments, emotes 28 px high;
  - at most 500 messages, dropped from the top only while following;
  - scrolled up, new messages are held (500 at most) and "Reprendre le défilement · N nouveaux" resumes;
    reaching the bottom again resumes by itself at the next batch;
  - deleted messages greyed (`Delete`, `ClearUser`), `Clear` empties it;
  - the room's modes in a hover card on an "i" icon at the right of its header, the connection's state
    under it;
  - notices (subs, raids…) in a box with an accent edge, highlighted messages tinted, `/me` in the
    author's color.

  `ui::chat::watch_layout` is the watch page with a chat column (the chat on the page's height on the
  right; in theater mode, as on Twitch, the player fills the rest of the page and the chat runs on its
  whole height, flush against it, the header hidden). `spaces/twitch/chat.rs` turns `twitch::chat`
  updates into `ChatEvent`s, 200 at most per batch, the room's modes as French labels ("Followers depuis
  1 j", "Mode lent : 30 s"…); Twitch emotes as `Picture::sized` at scale 1, 2 or 3 for the drawn size
- Verified (2026-11-01): in the app, signed in, on the secondary screen: jynxzi's, zackrawrr's and
  hasanabi's chats next to the live (about 25,000–43,000 viewers), names in their colors, mentions,
  links, Twitch emotes inline, the "Followers depuis…" label; in the example, the timings above and the
  pause on scrolling up (the list stays, "Reprendre le défilement · 125 nouveaux" after 6 s, now worded "… nouveaux messages").
  **Not verified**: a reconnection after a real loss or a `RECONNECT`, the resume button's click, a
  deletion or a ban seen in the app, a VOD's or clip's theater mode now that it goes
  through `watch_layout`, the mini-player while the chat runs
- [x] 7TV emotes (the user asked for 7TV alone, 2026-11-01): `twitch::seventv` parses the global set and a
      channel's (`7tv.io/v3/users/twitch/<id>`, 404 without a 7TV account: no emotes), keeping the set
      entry's name (the channel's alias), the WebP files and their still first frame, the zero-width flag.
      Tested on 3 saved responses (trimmed, without the emote creators' accounts). The chat source asks for
      both sets once `ROOMSTATE`'s `room-id` arrives (`RoomState::room_id`), keeps them for the session,
      and turns the words they name into emotes (the channel's names over the global ones, case kept).
      Animated emotes are drawn still (`static_name`) and zero-width ones as ordinary emotes until the
      animated-emotes change. Verified in the app on zackrawrr's chat: 7TV emotes inline, wide ones at
      their ratio, in theater mode (the chat beside the player). **Not verified**: a channel without 7TV
      in the app
- [x] Badges: `helix::parse_badges` (`/chat/badges/global`, `/chat/badges`: every version of every set,
      18/36/72 px images), tested on 2 fixtures saved by `twitch_capture -- badges` (6 global sets, a public
      channel's subscriber versions, trimmed). The chat source loads them with the 7TV sets once the room's
      id arrives, keeps them for the session (`ChatSets`), the channel's versions over the global ones, and
      gives each author the badges of their `badges` tag that exist. Verified in the app on hasanabi's chat:
      subscriber and global badges before the names. **Not verified**: a channel without its own badges
      in the app
- FFZ: dropped, the user's choice (2026-11-01). BTTV, first dropped too, was added on 2026-11-02 (see below)
- [x] Animated emotes and zero-width overlays: `ui::images::decode` keeps the frames and delays of animated
      GIFs and WebPs (at most 300; `image`'s `gif` feature added, agreed), the store counts every frame in
      its budget, and the panel plays them (not with reduced motion). It picks each frame from a clock
      and paints it itself (`paint_image` in a `canvas`, 2026-11-02): GPUI's `img` stops animating
      when the window loses the focus, and the chat runs beside a video watched from another window;
      copies of an emote play in step, delays of 10 ms or less count as 100 ms as in browsers. `Emote::overlay` draws an emote over the one before it (blank
      text between allowed), else as an ordinary one: 7TV's zero-width emotes. 7TV emotes now ask for the
      animated WebP; Twitch's `default` URLs were already animated GIFs. 2 tests (a GIF built in memory);
      a real 7TV animated WebP decoded to its 55 frames (checked once, not kept as a fixture: third-party
      art). The example blinks some emotes and draws rings over others: 120 frames a second at 20 messages
      a second. **Not verified**: the motion in the app (screenshots are still), a zero-width 7TV emote in
      the app
- [x] VODs: the chat replayed (2026-11-02). GQL's `video.comments(contentOffsetSeconds:)`, raw
      query, anonymous (`gql::comments_request`, `parse_comments`: each comment as a `ChatLine`, its
      fragments' emotes as `char` ranges, Twitch's empty badge dropped; `gql_vod_comments.json`, 2 tests).
      The `after` cursor fails the integrity check, so pages go by offset only: a page holds about 50
      messages from a little before the offset (5 s on a busy chat, ~100 s on a quiet one), the next
      is asked from the last offset fetched and the overlap dropped by id; a second fuller than a page
      loses its tail. `spaces::twitch::replay` reads the playhead every 250 ms, shows the messages due,
      asks the next page 15 s ahead, and starts over on a seek (back more than 1.5 s, forward more than
      10 s), showing the page's messages before the new position; the panel is titled "Rediffusion du
      chat". 7TV emotes and badges as the live's (the VOD's `owner.id`). 5 tests. Verified in the app on
      a busy public VOD (Twitch and 7TV emotes, badges, the chat following playback). **Not verified**:
      a seek in the app, a VOD whose chat is unavailable
- [x] BTTV emotes (2026-11-02, the user asked: BTTV's global `D:` stayed text): `twitch::bttv` parses the
      global set and a channel's own and shared emotes (404 without a BTTV account: no emotes), skipping
      modifiers (`w!`, `h!`…) and marking the global hats and masks as overlays. The chat source loads
      them with the 7TV sets, BTTV global, 7TV global, BTTV channel, 7TV channel, each winning over the
      ones before; the replay gets them too. 3 tests on 3 saved responses (and the chat's word test extended) (forsen's set, trimmed, without
      the uploaders). **Not verified**: in the app
- Exit: met: the chat with Twitch and 7TV emotes (animated, overlays), badges, next to the player
- Dependencies: `tokio-tungstenite` with `native-tls`, `futures-util` (its stream and sink), tokio's
  `time` (the reconnection delay), agreed

### 10. Polish and docs
- [x] Bell: `spaces/twitch/bell.rs`, Twitch's section of the panel (`Space::unseen`, `Space::notifications`).
      It reads the sidebar's reading of the followed lives (every 2 minutes, now every live of the first
      page): the first reading learns who is on air and announces nothing (so that opening Peeby does
      not announce every live), later ones announce the channels not on air at the previous one, newest
      first (30 kept, a channel back on air replacing its entry). Opening the panel sees them; a click
      opens the live. Nothing is kept across restarts. 4 tests. Verified in the app: the section under
      YouTube's, with its empty text. **Not verified**: a real announcement (a followed channel going
      live while Peeby runs), the unseen count on the bell
- [x] **Low latency**: `Splicer::new(low_latency)` follows the content with the main playlist's prefetch
      segments, numbered after its newest segment, never during an ad break nor when the newest kept
      segment is not the main content's; `Client::play` takes the switch, VODs never. No mpv option was
      needed: FFmpeg starts a live three segments from the end, so two more segments at the end start it
      about two segments later. `[twitch] low_latency` with "Faible latence" in the settings part, on by
      default. Measured with the spike on a live (`PEEBY_PROXY=1`, `PEEBY_LOW_LATENCY=0|1`, the segment
      FFmpeg starts at against its `PROGRAM-DATE-TIME`): about **2.6 s** behind with it against about
      **7.0 s** without, start time unchanged (1.31 s), no stall in 45 s. 2 tests on the saved playlists.
      **Not verified**: the delay by eye against the chat, a mid-roll with it on
- [x] **HEVC and AV1**: checked first, on this PC: the spike (`PEEBY_VARIANT`, now printing `video-codec`
      and `hwdec-current`) played a 1080p60 HEVC variant with `d3d11va`. Then `twitch_capture -- master
      <login>` and `-- master-scan` (the user's auth-token, every codec): 16 of the top 100 lives offer
      **1440p60 in HEVC only**, with H.264 at 1080p60 and below; no AV1 anywhere. Saved as
      `usher_master_codecs.m3u8` (session values redacted by allowlist). So Twitch does not repeat a
      quality across codecs: lives and VODs ask for `av1,h265,h264` minus the codecs found decoded on the
      CPU (`[twitch] software_codecs`), and the watch page checks every 4 s that an AV1 or HEVC quality
      plays with `hwdec-current` other than `no` (two readings in a row, never with hardware acceleration
      off, so that a quality still loading or a forced software decoder saves nothing; removing the codec
      from `settings.toml` asks for it again); if not, the codec is saved there and playback moves to
      the best H.264 quality not taller (`play::fallback`). The quality menu shows the codec only when two
      variants share a name (none seen). New in `ui::player`: `property`, `quality`, `set_quality`.
      4 tests. Verified in the app: squeezie offers 7 qualities, 1440p60 first; the default (1080p)
      plays H.264; a VOD still plays (0.91 s, 7 qualities); the low-latency live plays in the app.
      **Not verified**: 1440p HEVC played in the app (needs picking it in the menu), the
      fallback (this GPU decodes HEVC), AV1
- [x] docs: architecture.md, design.md (Twitch pages, the chat, the bell), twitch.md, ADR 0010
      consequences, README, CLAUDE.md, roadmap
- Exit: met. Live and VOD playback without ads, followed channels, chat (the roadmap's criterion)

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Twitch changes how it marks ads | Ads come back, or content is cut | Filter rules tested on saved playlists; a new capture becomes a fixture |
| Integrity token required on every playback token | No playback | Clear error; WebView2 behind a switch (ADR 0010), revisit |
| GQL Client-ID or hashes go stale | No playback | Raw queries, startup refresh, `settings.toml` override |
| The proxy hop slows the start | Over the 1–2 s budget | Measured at steps 1 and 3; the master can be fetched while the token resolves on hover (prefetch, as YouTube) |
| Refresh token burned by a race | Signed out | Single-flight refresh, keyring written first |
| Chat throughput on busy channels | UI stutter | Virtual list, cap of 500, batch appends per frame |
| Prefetch segments announced just before a mid-roll turn out to be ads | A few seconds of ad slip through (Streamlink has the same caveat) | Prefetch dropped as soon as a reload shows the break; "Faible latence" can be switched off |

## Still to ask

- Any write to the account (following a channel, sending a chat message): not planned, each would need its
  own go-ahead and test.
- A playback test of a subscriber-only VOD or with the auth-token: it plays as the user's account.
