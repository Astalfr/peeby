# Roadmap

Status legend: ✅ done · 🚧 in progress · ⏳ not started

| Milestone | Status | Content | Exit criterion |
|---|---|---|---|
| **Study and spikes** | ✅ | Feasibility study, Home via InnerTube, playback spike (yt-dlp, then VISIONOS HLS), bundled tools | [etude-faisabilite.md](etude-faisabilite.md), ADRs 0001–0005 |
| **1: foundations** | ✅ | Workspace, CLAUDE.md, docs; `innertube` (Home, watch page, streams, cookies) with fixture tests; app shell, Home grid, watch page with autoplay and related videos | Open a video from a URL and from a related video, ~1 s to playback; signed-in Home shows recommendations |
| **2: YouTube pages** | ✅ (RSS fallback moved to the backlog) | Subscriptions list and feed (`FEsubscriptions`, RSS fallback), comments (`commentEntityPayload`) and replies, Return YouTube Dislike bar (cache, attribution), channel page, infinite scroll | — |
| **3: player comfort** | ✅ | Quality choice, subtitles on demand (`sub-add`), prefetch on hover, yt-dlp fallback ([ADR 0005](decisions/0005-bundled-yt-dlp.md)) | — |
| **4: session and settings** | ✅ | Cookie import UI, `Set-Cookie` persistence, settings page (language, quality), yt-dlp updater | — |
| **Peeby refactor** | ✅ | One space per service (YouTube, Jellyfin), shared `ui` crate and player, tabs, mini-player, YouTube search, unified settings ([refactor-plan.md](refactor-plan.md), ADRs 0007–0010) | All ten steps of the plan; peeby's Jellyfin features in the Jellyfin space |
| **Twitch space** | ✅ | Helix catalog, GQL/usher playback with the ad-removing layer, `auth-token` with its tutorial, read-only chat ([twitch.md](twitch.md), [ADR 0010](decisions/0010-twitch-space.md), plan: [twitch-plan.md](twitch-plan.md)) | Live and VOD playback without ads, followed channels, chat |
| **5: distribution (1.0)** | 🚧 (done once v1.0.0 is published and installed) | CI, installer with libmpv, yt-dlp and Deno in `tools/` (Inno Setup, after Twitty's), releases from version tags, updates from GitHub Releases with the title bar's button ([ADR 0027](decisions/0027-installer-and-updates.md), [release.md](release.md)) | Downloadable build |


## Volume boost

- Done (2026-11-07): a "Boost" button after the volume slider (shown on hover) lets the player's volume go up to 200 % ([playback-ux.md](player/playback-ux.md)): mpv's `volume-max` is 200 (`player::VOLUME_MAX`), the track doubles, marks 100 % and draws the boost in the warning color; the keys and the wheel go up to it. Never remembered: each player starts without it, the kept volume is 100 % at most. Checked: build, clippy, tests. **Not verified:** in the app.

## BTTV emote modifiers

- Done (2026-11-07, the user's request for "7TV's modifiers": 7TV has none of its own, these are BTTV's, which BTTV also applies to 7TV emotes): `w! h! v! z! c! l! r! p! s!` before a 7TV or BTTV emote, and FFZ's `ffzW/X/Y/Cursed` after one, in the live and VOD chat ([twitch.md](twitch.md), "BTTV's modifiers"). Flips, quarter turns, cursed and party colors are done to the pixels when decoding (`ui::images::Effects`, `ImageStore::get_with`), wide and shake when drawing; the preview lists them. The fake chat writes some, its squares have a white corner so a flip shows; "Vraies 7TV / BTTV" (fake chat) loads the real global sets. 3 new tests. Checked: clippy, tests. **Not verified:** in the app.

## Picture-in-picture button

- Done (2026-11-07, [ADR 0008](decisions/0008-mini-player-window.md)): a picture-in-picture button in the player's bar (the first of its right-hand buttons, and the I key) puts the video or live in the mini-player while its page stays, showing a placeholder; the mini-player then stays while that page shows, until the button again, "Revenir à la page" or "×". A Short's player has no picture-in-picture button, no seek buttons and no time (`offer_picture_in_picture`, `show_seek_buttons`, `show_time`; ← and → still seek). Checked: clippy. **Not verified:** in the app.

## 1.0: installer, CI and updates

- Done (2026-11-05, [ADR 0027](decisions/0027-installer-and-updates.md)): version 1.0.0. `installer/peeby.iss` (Inno Setup, per user, libmpv and the VC++ runtime beside the exe, yt-dlp and Deno in `tools\`, yt-dlp kept across updates, `CloseApplications=force`, the `Run` value and the toasts' registration removed on uninstall); `.github/workflows/ci.yml` (fmt, clippy, tests on every push; a `v*` tag builds the installer and publishes the release after checking it against `Cargo.toml`); `update` in the binary: an installed release build checks `releases/latest` 20 s after the start then every 4 hours, downloads a newer installer into `%LOCALAPPDATA%\peeby\updates`, checks GitHub's SHA-256 digest, and only then shows the green "Update" / "Mettre à jour" button in the title bar; a click writes the settings, starts the installer silently with `/relaunch=1` and quits. 12 tests (versions, digests, the installer's asset, the download decision, a real `releases/latest` answer as a fixture). Checked: clippy, tests. **Not verified:** the installer (Inno Setup is not on this machine: CI builds it), CI itself (it runs on the first push), and the update end to end (download, silent install, relaunch), which needs a second release (1.0.1).

## Encrypted cookie jar

- Done (2026-11-04): the YouTube jar is encrypted with DPAPI for the Windows user in `cookies.dat`; the former plain `cookies.txt` is encrypted then deleted at the first start, yt-dlp gets a plain copy deleted at the next start and on sign-out, and `peeby --export-cookies` gives the examples a plain copy ([ADR 0022](decisions/0022-encrypted-cookie-jar.md)). Checked: DPAPI round trip (unit test), a plain jar given with `PEEBY_COOKIES` encrypted in place then exported unchanged. **Not verified:** the migration of the real `cookies.txt` and a rotation in the app (the next run of Peeby).

## Session cookie rotation

- Done (2026-11-09, [ADR 0004](decisions/0004-cookie-session.md)): the session ended after about
  five days (2026-11-08) although Peeby kept the `Set-Cookie` of every signed-in request:
  InnerTube responses rotate `SIDCC` but never `__Secure-1PSIDTS`/`3PSIDTS`. Peeby now posts to
  `accounts.youtube.com/RotateCookies` as youtube.com's hidden `RotateCookiesPage` frame does
  (`Client::rotate_cookies`), at startup then at the delay the answer gives (`identity.hfcr`,
  600 s), while it runs (`space-youtube` `rotation`). Checked live on a fresh session: 200 with
  new `PSIDTS`/`PSIDCC` values, the rotated jar accepted (`account` example), and in the app the
  rotation at startup then Home signed in; the delay's parser is tested on the saved answer.
  **Not verified:** a session lasting beyond five days this way; a session left unused while
  Peeby is quit for days.

## Log file

- Done (2026-11-03): Peeby writes its log to `%LOCALAPPDATA%\peeby\logs\<profile>-<ms>.log` as well as stderr: a release build has no console, so its warnings were lost (a Twitch live's Windows notification went missing with no trace; `build` or `Show` failed in `desktop::toast`). One file per launch, created at its first line by a thread of its own, a panic logged and flushed, deleted after 7 days with mpv's logs. Checked with a debug run: the file holds the stderr lines.
- Done (2026-11-03): the log told why: `ToastNotifier::Show` failed with 0x800401FD (the notifier kept since startup disconnected, 25 to 70 minutes after launch). A failure now makes a new notifier and shows the toast again ([ADR 0020](decisions/0020-notifications-and-background.md)). **Not verified:** the retry in the app (it needs the notifier to go stale).

## Subscribe and follow buttons

- Done (2026-11-03): YouTube's watch page has a subscribe button left of the bell (a small one under a Short, from the same `WatchPage`): "S'abonner" (red) or "Abonné", whose menu has "Se désabonner de <chaîne>" (the second click YouTube's confirmation dialog asks for). `Client::set_subscribed` posts `subscription/subscribe` or `subscription/unsubscribe` with the button's `params`; the state is kept by channel id for the session (reset on a session change), shows at once, and goes back with a toast if YouTube refuses. The bell, which the button carries even before a subscription, shows while subscribed, so it appears right after a subscription. Parsed and tested on `next.json` (not subscribed) and `next_signed_in.json` (subscribed, unsubscribe params in the dialog).
- Twitch: a "Suivre" / "Suivi" button on the watch page (live, VOD, clip) and the channel page, the state read from `user.self.follower` with the auth-token (new fixtures `gql_channel_followed.json`, redacted, and `gql_channel_not_followed.json`). `followUser` / `unfollowUser` fail Twitch's integrity check even with the auth-token (checked 2026-11-03) and Helix has no follow endpoint, so the button opens the channel on twitch.tv, where the follow happens.
- Checked live (2026-11-03, with the user's agreement, on Rick Astley's channel, not subscribed before): `subscription/subscribe` then `subscription/unsubscribe` with the button's params both answer the state asked for (`updateSubscribeButtonAction`, now checked by `Client::set_subscribed`; answers saved as fixtures without `responseContext`); the subscribed page's bell has the same params as the one the unsubscribed page carried; right after the unsubscription the watch page still said subscribed, and no longer a moment later. The channel was left unsubscribed. The unsubscribe menu wraps a long channel name (two lines at most).
- **Not verified:** in the app: the button's clicks, the toasts and the revert. **Not done:** a YouTube channel page's button (`channel_videos.json` carries no subscribe button: it needs a signed-in capture of a channel page's header).

## Windows notifications and the notification area

- Done (2026-11-03, [ADR 0020](decisions/0020-notifications-and-background.md)): Windows notifications (WinRT toasts with the channel's round picture; a click shows the window and opens the live or the video) and the bell, per kind in each space's settings ("Peeby" and "Windows" switches): Twitch, a followed channel goes live and a followed live changes category (new, with its own bell rows); YouTube, a new video and a live started, told apart by the inbox's tracking label (`NotificationKind`, the same in every language). YouTube's unseen count is read every 3 minutes and the inbox only when it grows (reading it clears youtube.com's count, checked live); Peeby's badge then counts the new ones itself. More than 3 at once: one summary toast. Closing the window keeps Peeby in the notification area (icon: click to show, menu "Ouvrir Peeby" / "Fermer"), pausing the page's video (the mini-player goes on); "Rester en arrière-plan" off makes the "×" quit. "Démarrer avec Windows" (`Run` key, `--background`). Single instance through a named pipe: a next launch hands its arguments (videos, a Twitch page) to the first. 10 new tests (bell changes, kinds on the existing fixtures, base64, new notifications since the session, toast texts). **Not verified:** in the app (Peeby was running; a second instance would have spent the Twitch refresh token): the toasts, their click, the tray icon and menu, hiding and showing the window, the single instance, the start with Windows.
- Done (2026-11-03): the notification area's icon opens a list of lives on a click (Gumbo's layout: a tab per service, Twitch's followed channels on air and YouTube's subscribed ones, avatar, viewers, category, uptime, title; a click opens the live in Peeby); a double click shows the window. YouTube's lives come from the subscriptions feed, read when the list opens (at most once a minute). **Not verified by Claude:** the clicks on the icon (no access to the desktop).

- Done (2026-11-09): the bell's panel has a tab per space with its unread count, as the list of lives has them (the tab bar is now shared, `shell::space_tabs`); only the tab shown builds its section. Twitch's tab adds the account's notification center on twitch.tv (watch streaks, drops, Twitch's news: `twitch::notifications`, raw GQL found from its validation messages, read every 3 minutes with the auth-token, without Helix) to the followed lives' events, newest first, with its own kind in the settings ("Twitch's notifications": bell, Windows, test). 8 new tests on a new fixture (`gql_notifications.json`, read signed in with `twitch_capture -- notifications`, trimmed, channels renamed). **Not verified:** in the app (the panel's tabs, a Twitch notification's toast and click); twitch.tv's own count is not cleared by Peeby (no mutation found).

## Sign-in WebView

- Done (2026-11-03, [ADR 0017](decisions/0017-sign-in-webview.md)): "Se connecter à YouTube…" and "Se connecter à Twitch…" open the service's own sign-in page in a WebView2 window (`peeby --login`, a separate process: `wry` and `tao`), which hands Peeby the session once signed in (youtube.com's cookies, the `auth-token`) and closes; it is checked like a manual import. The cookie import and the auth-token tutorial stay below the buttons. Checked first with a spike: Google and Twitch both let the user sign in, and the session passed `innertube --example smoke` and GQL `currentUser`. 4 new tests (`web_sign_in`). **Not verified:** the buttons in the app, WebView2 missing, a Windows 10 host, a long session started this way.

## Tabs

- Done (2026-11-03, [ADR 0015](decisions/0015-tabs.md)): tabs of any space instead of one fixed tab per space. "+" opens one (a menu of the spaces), "×" or a middle click closes one (not the last), and a middle click on a link (video, channel, playlist, live, category, VOD, clip, Jellyfin title or episode, sidebar entry) opens it in a new tab in the background, built only when it shows. Tabs are named after their page (a video's title once loaded). A route opened in a tab whose pages are still waiting (a notification into a tab never shown, or after a session change) joins them instead of being cleared by them; a card's resume position is kept per video until its page opens. 3 new tests (`shell::space`). Verified in the app with posted clicks: the three startup tabs, a middle click on a video card opening a background tab named after it, a middle click on that tab closing it, "+" → YouTube opening a new tab on its start page. **Not verified:** playback in a tab opened by a middle click, the mini-player between two tabs of the same space, Twitch and Jellyfin middle clicks, a notification into a tab never shown or with no tab of its space open. No middle click yet on the rows of a playlist page or of the queue panel.
- Done (2026-11-04): the gear opens the settings in a tab of their own (or shows it), and the setting "Startup tabs" (`startup_tabs`, General section) picks the services that open a tab at startup, YouTube alone by default. A refresh button next to back and forward (F5, Ctrl+R) builds the page shown again. **Not verified:** in the running app.
- Done (2026-11-07, [ADR 0028](decisions/0028-tab-windows.md)): a tab held and moved past 4 px follows the pointer along the bar (shown, its slot left empty, an opaque copy over the others) and takes its neighbors' places as the copy's middle passes theirs; the tabs passed slide to their new place, and the released tab into its slot (150 ms, none with reduced motion). The drag follows window listeners rather than GPUI's `on_drag`, for the steps that tear a tab out. 2 tests (`shell::tab_drag`). Verified: clippy, tests. **Not verified:** in the app.
- Done (2026-11-07, [ADR 0028](decisions/0028-tab-windows.md), step 2): several windows over the same spaces. A registry (`shell::windows`) replaces the main window: the window used last takes the icon, launches, toasts, lives and the spaces' requests without a tab; the spaces' events go through one router to the window holding their tab; a space renders each window's own tab. A window closed while another is open closes its tabs and stays hidden as a spare (its pages may live on in another window); the last one hides or quits as before. Until the drag-out, a tab's right-click menu has "Move to a new window". 3 tests (`shell::space`, `shell::windows`). Verified: clippy, tests. **Not verified:** in the app.
- Done (2026-11-07, [ADR 0028](decisions/0028-tab-windows.md), steps 3 and 4): a tab pulled 32 px off the bar (or out of the window) detaches, its slot closing and its copy following the pointer over the window; released elsewhere, it opens in a window of its own under the pointer, released over another window's bar, it joins that window at the place under the pointer (a window emptied this way hides as a spare). 1 test (`shell::tab_drag`). Verified: clippy, tests. **Not verified:** in the app.
- Done (2026-11-07, [ADR 0028](decisions/0028-tab-windows.md), step 5): the tab tears out as soon as it leaves the bar, into a window that follows the pointer (without the focus, so the source keeps the mouse's capture) until the release; released over a bar (its first window's too), it joins it and its window goes. Verified: clippy, tests. **Not verified:** in the app (above all that the capture holds while the new window shows).
- Done (2026-11-07): a window's only tab carries its window, as in Chrome (not a maximized one), and dropped over another window's bar joins it. **Not verified:** in the app.
- Done (2026-11-07): as in Chrome, a carried tab slides into the bar of the window it hovers (its own window hiding) and tears out again when pulled off it; the window holding the tab drives the drag with the mouse's capture (`SetCapture` handed over). **Not verified:** in the app.
- Done (2026-11-11, docs/features-plan.md item 12, branch `dev`): a middle click on the tab bar's empty part opens a tab of the shown tab's space (a window listener painted over that part, which stays the window's drag area). Checked by the user in the app (2026-11-11).
- Done (2026-11-11, docs/features-plan.md item 11, branch `dev`): a maximized window's only tab restores and carries its window (Windows' move loop, as its title bar). Checked by the user in the app (2026-11-11).
- Done (2026-11-11, docs/features-plan.md item 10, [ADR 0035](decisions/0035-restore-tabs.md), branch `dev`): the windows and their tabs come back at startup (setting "Restaurer les onglets", General, on by default): `session.json` written every 30 s and at quit; each space's routes as keys (`Space::route_key`); the pages built when their tab shows, a tab that plays not shown first; the last window's close saves first (the quit's save would find no window), and an empty save keeps the last session. 3 new tests. Checked by the user in the app (2026-11-11).
- Done (2026-11-11, docs/features-plan.md item 9, branch `dev`): a browser's tab keys, in the window that has the focus (not in fullscreen): Ctrl+T a tab of the shown tab's space, Ctrl+W closes the tab (the window's only one closes the window), Ctrl+Tab and Ctrl+Shift+Tab the next and previous tab, Ctrl+1 to 8 that tab, Ctrl+9 the last one (`Shell`'s keystroke observer, beside F5 and Ctrl+R). 1 new test. Checked by the user in the app (2026-11-11).

## Tab history

- Done (2026-11-03, [ADR 0019](decisions/0019-tab-history.md)): each tab keeps a browser-like history instead of a stack that sidebar pages cleared, so "back" returns to the page before, sidebar pages included. A forward button next to back (`arrow-right`), and the mouse's side buttons. The settings page is an entry of the tab's history (the tab reads "Settings" meanwhile; a playing video still goes to the mini-player). Pages are built when they show (this replaced the pending routes of background tabs); the page left with "back" goes with its player, the 10 pages behind the one shown keep their view, a sidebar page drops the players behind it; 50 entries at most. 6 tests (`shell::space`). Verified: clippy and tests. **Not verified:** in the app (back and forward through sidebar pages, the settings and a video in the mini-player, the side buttons, a Jellyfin player left behind, a queue's next video then "back").

## Interface language

- English and French interface (2026-11-02, [ADR 0014](decisions/0014-interface-language.md)): `ui::i18n` (`t`, `tf!`, `Lang`), the setting `interface_language` (default `en`) in the gear's Général section, applied at once to every window. Every UI string of `ui` and of the spaces has its English text next to the French one; YouTube's language and region are relabelled as the content language.

## YouTube channel sort and search

- Done (2026-11-03): a channel's page sorts its videos by YouTube's own chips ("Les plus récentes", "Populaires", "Les plus anciennes", named in YouTube's language: `Feed::chips`, from the grid's `chipBarViewModel`, each a `continuationCommand` that replaces the grid) and has a search box (Enter searches, emptying it goes back to the videos in the sort chosen): `Client::channel_search` (`browse` with the channel's "Search" tab `params` and `query`, `videoRenderer`s) and `Client::channel_search_more`. The sort is hidden while a search shows; an older sort's or search's answer is dropped. 3 new tests, 3 fixtures (`channel_videos_popular.json`, `channel_search.json`, `channel_search_continuation.json`, anonymous, trimmed). The public IP left in 10 committed fixtures (the previews' `initplayback` URLs) is now redacted. Verified: clippy, tests, and live with `smoke` signed out (sort: 30 videos, search: 28 then 30). **Not verified:** in the app (the chips' clicks, the search box, its clear button, the next pages of a search).

## YouTube original titles

- Original titles (2026-11-03, [ADR 0021](decisions/0021-youtube-original-titles.md)): YouTube translates titles into the content language (`hl`), from the creator's translations or automatically, and no InnerTube response carries the original. The YouTube setting "Original titles" (`original_titles`, on by default) puts them back with `Client::with_original_titles`: one anonymous oEmbed request per video (`/oembed`, `parse_oembed_title`, 32 at once, 3 s timeout, cached by id), for feeds, history, channels, searches, playlists and the watch page (its title and related videos). A failed lookup keeps the translated title; oEmbed refuses videos whose embedding is disabled, which stay translated. Toggling it rebuilds the clients like a language change. Verified live (fr/FR search "MrBeast"): "Bats Haaland, Gagne 100 000 $" became "Beat Haaland, Win $100,000", the search about 40 ms longer. The settings switch was not clicked in a test run.

## YouTube Shorts

- Shorts page (2026-11-03, [ADR 0018](decisions/0018-youtube-shorts-page.md)): "Shorts" in the YouTube sidebar, under Abonnements. YouTube's endless Shorts sequence (`Client::shorts`, `Client::shorts_more`: `reel/reel_item_watch` seedless, then `reel/reel_watch_sequence`), one Short at a time on the page's height in a 9:16 player, its channel and title under it; on its right, like (YouTube's count, `WatchPage::likes`), dislike, comments (with their count), "⋮" save menu and "Watch as a video"; previous and next arrows at the right edge; the wheel and ↑/↓ move through the Shorts, with a 300 ms slide (both Shorts move together, the one leaving paused). The comments open in a panel beside the Short and stay open from one Short to the next. A Short loops, at normal speed (no speed menu nor 2× hold), the next one's stream is resolved ahead. Sidebar icon: the Shorts logo, outlined (`shorts.svg`). The comments, the rating, the stream start and the history pings moved out of `watch.rs` to be shared (`comments.rs`, `rating.rs`, `watch::load_stream`, `history::report`); HLS variants are ranked by their shorter side (a 1080×1920 Short is 1080p). 3 new tests, 2 fixtures (`reel_seedless.json`, `reel_sequence.json`, anonymous, trimmed); the smoke example reads the sequence and a Short's stream (checked live signed out and signed in: 1 + 10 Shorts, `logged_in=1` signed in, 1080x1920 VP9). Verified: clippy and tests. **Not verified:** in the app (the page, its layout at several window sizes, the wheel and arrows, the comments panel, a timestamp in a Short's comments, the loop, the mini-player with a Short), and the watch page after the move of its comments, rating, stream start and history pings.

## YouTube history

- History page (2026-11-03): "Historique" in the YouTube sidebar, between Abonnements and À regarder plus tard. `Client::history` (`browse` `FEhistory`, signed in only), its next pages through `Client::continuation`; shown by `FeedView` (`Source::History`) as list rows with the progress bars, the history-off message as its empty state. Parsed by `parse_feed`, already tested on `history.json`; added to the smoke example. Verified: clippy and tests. **Not verified:** in the app or live (the page, its next pages, the history turned off).

## YouTube links

- Links in descriptions and comments (2026-11-03): `WatchPage::description_links` and `Comment::links` from the attributed strings' `commandRuns` (UTF-16 offsets turned into byte ranges, a run off the text skipped), as `LinkTarget`s: a site (out of YouTube's `redirect?q=`), a video (a timestamp of this one seeks and scrolls back to the player, another opens in the tab), a channel, a playlist, another YouTube page (hashtags, in the browser); only http(s) URLs are kept, since Windows would run any other scheme's handler. Drawn in the accent, underlined while hovered, clicked through `InteractiveText`. The description folds to 4 lines with "Show more" / "Show less" ("Afficher plus" / "Afficher moins"). 3 new tests, 1 fixture (`comments_links.json`, anonymous, 3 comments). Verified: clippy and tests. **Not verified:** in the app (the clicks, the hover underline, a seek from a timestamp, the folding). On a live with its chat (`watch_layout`), a timestamp seeks but does not scroll back to the player.

## YouTube comment rating and translation

- Comment thumbs (2026-11-09): signed in, a comment's like and dislike thumbs rate it, as on YouTube: the comment's toolbar entities give its rating (`engagementToolbarStateEntityPayload.likeState`) and the four `performCommentActionEndpoint` actions (`engagementToolbarSurfaceEntityPayload`: like, unlike, dislike, undislike), `Client::rate_comment` posts the one going from the current rating to the clicked one to `comment/perform_comment_action` (one action from a like to a dislike, as YouTube's buttons). The thumb changes at once, goes back with a toast if YouTube refuses; a liked comment shows `likeCountLiked`. Signed out, the thumbs show and a click says to sign in. New example `comments` (`--save`; `--rate` likes, dislikes then un-dislikes the first comment from the account and saves each answer), 2 new tests, 3 fixtures (`comments_signed_in.json`: 3 threads read after a like, the first one liked, account data and reply composer removed; `comment_like.json`, `comment_undislike.json`: the answers, `responseContext` removed). Checked live on dQw4w9WgXcQ (2026-11-09): like, dislike and undislike answer `actionResults[0].status` `STATUS_SUCCEEDED` (like and dislike with a `feedback` and a `runAttestationCommand`), the page read after the like shows `TOOLBAR_LIKE_STATE_LIKED`, and small counts give `likeCountLiked` = `likeCountNotliked` + 1. Verified: clippy and tests. **Not verified:** the `TOOLBAR_LIKE_STATE_DISLIKED` name (the page read after the dislike came in another order, without the comment), the thumbs in the app.
- Comment translation (2026-11-09): "Traduire" ("Translate") beside a comment's thumbs shows it translated by Google into the client's language (`hl`), then "Voir l'original (traduit par Google)" switches back (the translation is kept); "Traduction…" while it waits, a toast if it fails. YouTube's answers give no translate command (only a `translateButtonEntityKey` toggle state and the shared strings), so `Client::translate_comment` builds the web client's action itself (`innertube::proto`: `comment/perform_comment_action`, type 22, `translateCommentParams` with the comment's id, its text and the target language, fields as YouTube.js sends them) and reads the comment entity's `translatedContent` (`parse_comment_translation`). It needs no sign-in and writes nothing to the account; the text is required (400 without it); emoji and line breaks pass as they are. The button shows on every comment with text: no answer says a comment's language. The translation is plain text, without the original's links. 4 new tests, 1 fixture (`comment_translate.json`, anonymous, `responseContext` reduced). Checked live (2026-11-09, anonymous, ZRtdQ81jPUQ): English comments into French and Japanese. Verified: clippy and tests. **Not verified:** the button in the app.

## SponsorBlock

- Done (2026-11-11, the user's request, [ADR 0034](decisions/0034-sponsorblock.md), branch `dev`): a YouTube video's SponsorBlock segments (asked by a 4-digit prefix of the id's SHA-256, kept for the session) are marked on the seek bar in their category's color and named in its hover; per category (YouTube's settings, "SponsorBlock") they are skipped as they start (once, with "Sponsor passé" on screen), offered with a skip button, or ignored; sponsors skipped by default. 1 new test, 1 fixture (`sponsorblock_5f6b.json`); the request checked live (anonymous). Checked by the user in the app (2026-11-11): the marks, the intro button and the sponsor skipped.

## YouTube channel page

- Done (2026-11-11, the user's request, docs/features-plan.md items 3 and 17, branch `dev`): a channel's page has its header (banner, avatar, name, handle, subscriber and video counts, description unfolding, the subscribe button with its bell signed in: the "Not done" of the subscribe buttons) and its tabs: Videos, Shorts (9:16 cards opening the Shorts page on that Short, then the channel's sequence: `Route::Short`), Lives, Playlists, each with its sort chips; the header scrolls with the list. `Feed::header`, `Feed::shorts`, `Client::channel_tab`. 3 new tests, 4 fixtures (`channel_header.json`, `channel_header_signed_in.json`, `channel_shorts.json`, `channel_playlists.json`). Then (the user's requests): the channel line of the video cards links to the channel (white on hover); the tabs and sort orders marked in the accent, without a grey background; the header the list's first item while a tab loads (no shift). Checked by the user in the app (2026-11-11).

## YouTube search suggestions

- Done (2026-11-11, the user's request, docs/features-plan.md item 4, branch `dev`): the search page's field shows YouTube's suggestions while typing (`Client::search_suggestions`: `suggestqueries-clients6.youtube.com/complete/search`, anonymous, `oe=utf-8` or the answer is Latin-1; a script whose argument is the JSON), a channel's with its avatar opening its page; arrows, Enter, Escape, click (a keystroke interceptor, since the field binds the arrows). The query never reaches the log. 1 new test, 1 fixture (`search_suggestions.txt`). Checked by the user in the app (2026-11-11).

## YouTube comment sort

- Done (2026-11-11, the user's request, docs/features-plan.md item 5, branch `dev`): the comments' header offers YouTube's orders (`CommentPage::sorts`, from the header's `sortFilterSubMenuRenderer`, each with its first page's token), right of the total, in the accent style shared with the channel page's sort (`ui::widgets::sort_choices`); a click reloads the comments in that order (watch page and Shorts panel). 1 new test (`comments.json` already had the menu). Checked by the user in the app (2026-11-11).

## YouTube comments written

- Done (2026-11-11, the user's request, docs/features-plan.md item 6, branch `dev`): signed in, a field over a video's comments posts one (`comment/create_comment` with the header's `createCommentParams`), "Répondre" under a comment opens a field posting a reply (`comment/create_comment_reply` with its `createReplyParams`; a reply's reply goes to its thread), and the user's own comments have "Supprimer", confirmed by a second click (`perform_comment_action` with the menu's confirm dialog action). The list follows YouTube's answers (`parse_created_comment`, `parse_comment_removed`). Captured with the user's agreement on their own video: a comment and its reply posted, then both deleted (`comments` example's `--post`/`--reply`); 2 new tests, 4 fixtures (`comments_composer.json`, `comment_create.json`, `comment_create_reply.json`, `comment_delete.json`, the account's handle, avatar and channel replaced). Checked by the user in the app (2026-11-11).

## YouTube Shorts shelves

- Done (2026-11-11, the user's request, docs/features-plan.md item 7, branch `dev`; the backlog's item): Home's and the subscriptions' Shorts shelves show between their rows where YouTube puts them (`Feed::short_shelves`, each after the videos before it, as the mixes), a row of 9:16 cards (the channel page's Shorts cards, opening the Shorts page on the Short, then its sequence) with "Afficher plus" unfolding the rest; the list's items are now laid out as slots (header, rows, shelves, end). The setting "Shorts dans les flux" (YouTube's part, on by default) hides them. 1 new test, 1 fixture (`home_shorts_shelves.json`), `home --save`. Checked by the user in the app (2026-11-11).

## YouTube collaborations' subscriptions

- Done (2026-11-11, the user's request, docs/features-plan.md item 8, branch `dev`): a collaboration's watch page, signed in, has one subscribe button ("S'abonner", "Abonné", "Abonné 1/2") opening the list of its channels, each with its own subscribe button and bell (`Collaborator::subscription`, `notifications`: each channel's `subscribeButtonViewModel` in the owner's dialog, its state from the page's `subscriptionStateEntity`; the channel header's parser now shares it). 1 new test, 1 fixture (`next_collaboration_signed_in.json`, read-only capture), `Client::watch_response`. Checked by the user in the app (2026-11-11).

## YouTube chapters

- Chapters of YouTube videos (2026-11-02): `WatchPage::chapters` from the `next` response's player bar markers (the description's, else YouTube's generated ones), given to the player with `set_chapters`. The shared seek bar now splits into one piece per chapter with gaps, the hovered one thicker (Jellyfin's chapters too); the hover title, the chapters menu and Page Up / Page Down work on them. 1 new test on a trimmed fixture (`next_chapters.json`). Verified: clippy, tests, and the seek bar's pieces on a screenshot of `cargo run -p ui --example player` (which now has chapters). Seen in the app, with the user's agreement: `rfscVS0vtbw` plays with its 35 chapters as pieces of the seek bar and the chapters button. The thickening on hover and the hovered piece's growth are eased (150 ms). **Not verified:** the hovered piece growing and its animation, the chapters menu clicked, Page Up / Page Down pressed.

## Twitch muted audio

- Muted parts of Twitch VODs (2026-11-02): Helix's `muted_segments` (`Video::muted_segments`), merged when they touch, reach the shared player as `SegmentKind::Muted` segments. The seek bar paints them red over the track (`theme::muted_audio`, full where played, dim ahead, whatever the accent), the hover preview adds "Son coupé", entering one shows the OSD "Son coupé (droits d'auteur)", and the skip button offers "Passer la partie muette" (S; never skipped on its own). 2 new tests and the fixture's assertions. Verified: clippy, tests, and on screenshots of `cargo run -p ui --example player` (which now has two muted parts) with Twitch's accent: the bands, the OSD and the skip button. **Not verified:** the hover label (the mouse was not moved), a real VOD in the app.

## Twitch chapters

- Game chapters of Twitch VODs (2026-11-03), as twitch.tv's chapter button: GQL `video.moments` (`VIDEO_CHAPTER_MARKERS`, `GAME_CHANGE`), raw and anonymous (`Client::vod_chapters`, `gql::parse_chapters`), asked beside the playback and given to the player with `set_chapters` when they come, from two games: the seek bar's pieces named after the games, the chapters menu, Page Up / Page Down. 2 new tests, 3 fixtures (`gql_vod_chapters*.json`). Verified: clippy, tests, the query live on public VODs. **Not verified:** in the app.
- Then (2026-11-03): the seek bar shows how far the video is loaded (`Event::Cached`, mpv's `demuxer-cache-time`), lighter than the track, ahead of the playhead. **Not verified** in the app yet.
- Then (2026-11-03): playback speed: a speed menu next to the quality (0.5× to 2×, not on a live, `Player::set_speed`), and Space or the left button held on the picture plays at 2× until the release ("2× speed" on top). **Not verified** in the app yet.
- Then (2026-11-03): chapters can carry a picture (`PlayerView::set_chapters_with_pictures`), Twitch's being the game's box art (`gql::Chapter::box_art`), shown in the chapters menu and in the seek bar's hover preview; the player's menus align their entries left; the hover preview centers the chapter's name (a thumbnail wide even without one). Verified on screenshots of `cargo run -p ui --example player` with `PEEBY_PICTURES=1` (empty picture boxes, the example has no image store): the hover preview, the chapters and audio menus. Chapter titles in the menu are then capped at 220 px and wrap on two lines at most (the example's third chapter has a long title, checked on a screenshot), and the quality menu is a little narrower (200 px at least). **Not verified:** real box art in the app, the quality menu (the example has no qualities; same entries as the audio menu).

- Then (2026-11-03): the channel page's VOD cards show "N chapitres" at the picture's bottom left (from two games), a popover with the same list as the player's menu (now `ui::player::chapter_list`, shared); a click plays the VOD from that chapter. `Route::Vod` carries an optional `start`, which wins over the resume position (the saver still saves); links' `?t=1h2m3s` fill it too. Chapters of the whole list in one batched query per 15 VODs (`Client::vod_chapters_many`), after the positions. 2 new tests, 1 fixture (`gql_vod_chapters_many.json`). Verified: clippy, tests. **Not verified:** in the app (the button, the popover, a click on a chapter).

## Watch progress

- Resume where a video stopped (2026-11-02, [ADR 0013](decisions/0013-watch-progress-on-the-account.md)), kept on the account rather than locally:
  - YouTube: cards carry YouTube's progress (`innertube::Progress`: the red bar, and where a click resumes). The watch page reports to the account like youtube.com's player: a signed WEB `player` request in the background for the stats URLs (`Client::tracking`), a playback ping at the start, then watchtime pings every 30 s, on pause, at the end and when the page goes. Not for lives, nothing signed out.
  - Twitch VODs: the saved position is read with the playback (`Client::vod_viewing`) and the VOD starts there; it is saved with `updateUserViewedVideo` every 30 s, on pause and when the page goes (auth-token only). A channel's VOD cards show it as a progress bar (`Client::vod_positions`, 15 VODs per GQL query; fixture `gql_vod_positions.json`).
  - 8 new tests, 6 fixtures (`history.json`, `playlist_watch_later_progress.json`, `player_web_signed_in.json`, `gql_vod_viewing*.json`, `gql_viewed_video.json`).
  - Checked live outside the app, with the user's agreement: the YouTube pings put a video at the top of the history and set its resume position (seen in the subscriptions feed); the Twitch mutation is accepted and changes `updatedAt`.
  - **Not verified:** in the app (a resume from a card, the pings during a real playback, a VOD resuming and its position seen on twitch.tv).

## Twitch space

- Multistream (2026-11-08, the user's request, [ADR 0033](decisions/0033-twitch-multistream.md),
  docs/twitch.md "Multistream"): the sidebar's "Multistream" plays up to four lives, the main one
  with the whole player, three muted in a row under it (480p, `PlayerView::set_secondary`); a click
  swaps a tile with the main live; "Add a live" offers the followed channels on air or a search of
  Twitch's lives. Only the main live earns points (`PointsView::set_earning`). Chat as a tab per
  channel (unread dots) or merged (`ChatPanel::set_mirror`: lines marked with their channel,
  moderation scoped to it, an eye per channel, writing to the chosen one through its writer). Every
  chat writer now shares Twitch's 20-per-30 s window. Measured first: four 1080p60 lives at once
  cost about 0.4 core and 2 % of the GPU each, decoded on the GPU. Lint and tests pass (2 new
  tests). **Not verified:** in the app (the dev profile is signed in: a run writes watch time).
  - 2026-11-08 (the user's request): the swap is animated, the two players gliding to each other's
    place in a layer over the page; the qualities change at its end, and the player now keeps the
    previous mpv's picture until a restarted one has a frame (any quality change, back to a live's
    edge). **Not verified:** in the app.
  - 2026-11-08 (the user's request): up to six lives; the row's tiles keep 16:9 at the widest the
    row allows; a bar between the main live and the row drags its height (kept in the settings).
    **Not verified:** in the app.
- Chat user cards (2026-11-07, the user's request), YouTube and Twitch: a click on an author's name or
  on a mention opens their card over the chat, with their picture, their name, their last messages in
  this chat and a button opening their channel in a new tab (docs/design.md, `Chat panel`). Lint and tests
  pass. **Not verified:** in the app (the card, a Twitch picture's lookup, a YouTube mention's search).
- Badge hover (2026-11-07, the user's request): a hovered chat badge (Twitch's, 7TV's) opens a
  hover card with it at 72 px (Twitch's 4x) above its title, as on twitch.tv, with the emotes'
  delay. Lint passes. **Not verified:** the hover in the app.
- 7TV paints and badges (2026-11-07, the user's request, [ADR 0031](decisions/0031-seventv-cosmetics.md)):
  the chat's EventAPI connection also follows the channel's `cosmetic.*` and `entitlement.*`. A
  chatter's gradient paint colors each letter of their name (image paints: each letter the
  average color of its slice of the image, stretched over the name, 2026-11-07, the user's request),
  their 7TV badge follows the Twitch ones, on their lines already shown too. The fake chat shows
  four of them. `twitch::seventv::cosmetics`, `ui::chat::Paint`, the probe's `--cosmetics`. 12 new
  tests, 1 fixture (`seventv_cosmetics.jsonl`). Personal emotes are left for later.
  **Not verified:** in the app.
- 7TV live updates (2026-11-07, the user's request, [ADR 0030](decisions/0030-seventv-event-api.md)): a live's
  chat follows 7TV's EventAPI (`wss://events.7tv.io/v3`, anonymous) for the global set, the channel's set and
  the channel's 7TV user, so an emote the streamer adds, removes or renames shows in the words and the picker
  without a reload, each with a notice line in the chat (added, removed, renamed, for the channel's set and the
  global one; the channel switching to another set). `twitch::seventv::events` (heartbeat, close codes,
  reconnects with a resync), `seventv::Set` keeps the set's and the user's ids, `Set::apply`; the space applies
  changes to the session's copies and rebuilds the extras from them. New example `seventv_events`. 8 new tests,
  1 fixture (`seventv_event_pushed.json`). Verified: clippy, tests; the protocol against the live service (Hello,
  Acks, real pushes). **Not verified:** in the app on a live whose set changes; a removal, a rename or a channel
  switching its set (`user.update`) as 7TV sends them (not seen while probing, read from 7TV's README; only
  the subscriptions' Acks were seen for `user.update`).

- Chat options and the 7TV reload (2026-11-07, the user's request): a gear left of "Send" opens a menu with
  "Reload 7TV emotes", which asks 7TV for the global and the channel's sets again and replaces the ones kept
  for the session only once both have answered (a failed half leaves the chat's emotes alone), then rebuilds
  the words and the picker; the option is greyed while it runs and says so when it brought nothing. Generic in
  `ui::chat` (`ChatOption`, `ChatPanel::set_options`, `ChatPanelEvent::Option`): the panel still knows no
  service. The fake chat has the same gear. 2 new tests. Verified in the app (debug build, fake chat): the gear
  draws left of "Send", its menu opens upward, the option fires, and a reload with the real global sets on
  keeps the emotes drawn. **Not verified:** a live's channel reload (needs a real live).
  Researched: 7TV's REST answers carry no cache header, so asking again is enough; automatic updates came
  next, over its EventAPI (above).

- Clickable chat links (2026-11-07, the user's request): a link in a chat message (live, VOD replay, fake chat; any `ui::chat` panel) opens in the browser on a click, with a pointer cursor and the plain accent on hover; a deleted message's link stays inert. Only `http(s)://` words become links (`split_text`), so no other scheme's handler runs. Checked: clippy. **Not verified:** in the app.

- Live catch-up (2026-11-06, the user's report: about 4 s behind the streamer, 2–3 s behind the browser): with low latency on, a live at its edge plays at 1.05× while mpv's buffer stays above 0.6 s and 0.97× under it (`Source::catch_up`), so it no longer keeps the ~2 s it started with and catches up after a stall. Measured with the spike on a live of 1 s segments: 4.2 s to 3.3 s on average, no stall in 3 min; the statistics show the speed ("Low-latency speed"); "Catch up" in the Twitch settings (under "Low latency", on by default) turns it off. 3 new tests. **Not verified:** in the app, and against the browser side by side.

- Writing in a live's chat (2026-11-04, [ADR 0023](decisions/0023-twitch-chat-sending.md), the user's request): a field under the chat with an emote and emoji picker (the channel's then the global 7TV and BTTV sets, with a search; a built-in emoji list), and under it the channel points on the left (balance, rewards and bonus, moved from the header; the predictions stay there) and "Envoyer" on the right. The messages leave on a second IRC connection signed in with the auth-token: cleaned (no line break, 500 characters), at most 20 per 30 s, Twitch's refusals shown as readable lines. Generic in `ui::chat` (`ChatPanel::with_composer`, `ChatPanelEvent::Send`, `Composing`, `EmoteGroup`), also in the panel example. 7 new tests. Verified: clippy and tests; the auth-token's IRC sign-in accepted (`twitch_capture -- chat-login`, nothing sent); the panel example runs with the composer (at 50 messages a second: about 27 frames a second, slowest 43 ms, the same without the composer on this run; the 2026-11-01 measurement was 120 frames a second). **Not verified:** a message sent from the app (a public write from the account: with the user's agreement), the reader showing it, the picker and the emoji rendering on screen, the points under the field. Not yet: Twitch's own emotes in the picker (a new GQL query, fixture first). The balance shows the channel's own points icon when it has one (`Points::icon`, already read), Twitch's coins until it loads or without one.

- No Helix sign-in needed (2026-11-03, [ADR 0016](decisions/0016-twitch-without-helix-sign-in.md), the user's choice): the catalog (top categories, a category's lives, channels, VODs, clips, searches, chat badges) through anonymous GQL, `twitch::catalog::Catalog`, Helix as its backup when connected. The auth-token is the sign-in (account entry from `currentUser`, settings page "Compte" first); Helix only for "Suivies", the sidebar's lives and the bell, connected from the "Suivies" page. "Parcourir" lists 100 categories (was 200: GQL's second page fails the integrity check). 11 new tests on 13 new fixtures (`twitch_capture -- catalog`), `smoke` counts the catalog. **Not verified:** in the app (Peeby was running: a second instance would have spent the Helix refresh token).

- BTTV emotes in the chat (2026-11-02): `twitch::bttv` (global set and a channel's own and shared emotes, modifiers not images of their own (drawn as modifiers since 2026-11-07), the hat and mask overlays), loaded with the 7TV sets, live and VOD chat; order BTTV global, 7TV global, BTTV channel, 7TV channel, each winning over the ones before. Asked by the user for the global `D:`. 3 new tests on 3 saved responses, the chat's word test extended. **Not verified:** in the app.
- Chat hover (2026-11-02): a hovered message is tinted; a hovered emote opens a hover card with it at 112 px (overlays included) and its code. Lint and tests pass; the panel example runs (slowest frame 14 ms). **Not verified:** the hover itself, in the example or the app; on a busy live the messages scroll under a still cursor (pausing the scroll while hovered, as Twitch does, would be the follow-up).
- Animated chat emotes keep playing when Peeby's window loses the focus (2026-11-02): the chat panel picks and paints the frames itself, GPUI's `img` stopping when the window is inactive. 1 new test. Verified in the app with VS Code focused.

- VOD chat replay (2026-11-02): a VOD's chat beside it, "Rediffusion du chat", each message shown when the playhead reaches it, started over on a seek, with the live chat's emotes (Twitch, 7TV) and badges. GQL comments by offset only: the cursor fails Twitch's integrity check. 7 new tests, 1 fixture (`gql_vod_comments.json`). Verified in the app on a busy public VOD. **Not verified:** a seek in the app.

- Channel points (2026-11-02, [ADR 0012](decisions/0012-twitch-channel-points.md); the bonus is claimed on twitch.tv until the user decides), with the web auth-token:
  - `twitch::points`: raw GQL reads and spends. 7 tests on 5 new fixtures.
  - On a live's header: the balance, refreshed every minute; the channel's rewards in a popover; its predictions with odds and a countdown, refreshed every 20 s.
  - 2026-11-04: an ended prediction stays pinned 20 s with its winner and the viewer's result (`resolvedPredictionEvents`, a new fixture and test).
  - 2026-11-04 (the user's request): the predictions moved over the chat, as on Twitch: folded bars at the top of the messages, one card unfolded at a time, shared with the polls.
  - 2026-11-04 (the user's request): outgoing raids from Twitch's PubSub (`twitch::raids`, ADR 0024; GQL's `user { raid }` shows nothing to viewers) pinned over the chat with their countdown and the target's avatar; the viewer is in the raid by default ("Quitter le raid" to stay); the page follows the raid when it goes (1 fixture, 2 tests).
  - 2026-11-08 (the user's request): a live's Hype Train (ADR 0032): `twitch::hype_train` from anonymous GQL at the page's start and after each PubSub reconnection, then PubSub's `hype-train-events-v1` topic on the raid's connection (`twitch::pubsub`); pinned over the chat as on Twitch (the bar filled in the channel's color, the level in a badge, share and countdown; unfolded, the level's emotes, conductors, record), its approach ("N more contributions"), and its end for 30 s; in the fake chat too (3 fixtures, 7 tests). **Not verified:** in the app, on a real train.
  - 2026-11-04 (the user's request): a live's polls from Twitch (`twitch::polls`: `viewablePoll` read every 10 s anonymous, `voteInPoll` with the auth-token; 3 fixtures, 4 tests). A real vote is still to be tried with the user.
  - Spending takes a second click. A reward can ask for a text; a bet takes a share of the balance.
  - The bonus chest is shown, but claimed on twitch.tv: its mutation fails Twitch's integrity check.
  - Watching earns points: the balance did not move in 20 minutes without it (the user's measurement), so Peeby now reports each minute the live plays, as the web player does (`twitch::spade`; 3 more tests, 2 trimmed fixtures).
  - Seen by the user in the app: the balance on a live's header.
  - Seen by the user: the balance grows while a live plays (+10 points), once `spade.twitch.tv` was allowed in their DNS filter.
  - 2026-11-06 (the user's request): the "Bonus" button opens the channel's popout chat on twitch.tv in a small WebView2 window (`peeby --twitch-chat <login>`, InPrivate, the auth-token set as its cookie on www.twitch.tv, handed over stdin); after 3 s a script clicks its "Claim Bonus" button (as 7TV does), and the window closes 1.5 s later; the button hides until the next points reading, which shows it again if the bonus still waits. Verified: clippy and tests. **Not verified:** the claim itself (the user checks).
  - 2026-11-06 (the user's request): the claim window is hidden and out of the taskbar (the chat renders there as in a shown one, checked anonymously), looks for the button every 500 ms from 3 s to 12 s; the "Bonus" button greys out with a spinner while it works; the "Claim the bonus automatically" setting (on by default) claims a bonus a random 1–2 s after a reading shows it, at most twice per bonus. Verified: clippy, tests, the hidden window anonymously. **Not verified:** a real claim.
  - 2026-11-06 (the user's report: no points): the click worked but Twitch answered "failed integrity check": at 3 s the page had not fetched its integrity token yet (it does about 10 s after loading). The click now waits for the page's own `gql.twitch.tv/integrity` answer, watched by an initialization script that also reads Twitch's answer to the claim (logged at info, errors only). `twitch_capture -- chat-page <login> [--claim]` runs the window from the terminal. Verified: a claim accepted on a live (shown window), the hidden window gets its token too (dry run).
  - **Not verified:** a bonus chest and its button; a real redemption or bet; the rewards popover.

- Step 10 done (2026-11-01), the Twitch space is complete: the bell announces the followed channels that go live while Peeby runs; low latency (Twitch's prefetch segments through the proxy, "Faible latence" on by default) starts a live about 2.6 s behind its capture instead of about 7 s, measured with the spike; HEVC: the 1440p60 source that 16 of the top 100 lives send only in HEVC is now offered, decoded by the GPU here, with a fall back to H.264 when a codec decodes on the CPU (kept in the settings). 10 new tests, a multi-codec master fixture (`twitch_capture -- master`). **Not verified:** a real bell announcement, 1440p HEVC picked in the app, the fallback, AV1 (no channel offered it).

- Animated emotes (2026-11-01, step 9, which ends with it): the image store keeps the frames of animated GIFs and WebPs, the chat panel plays them, and 7TV's zero-width emotes are drawn over the emote before them (2026-11-07: the stack is as wide as its widest layer, so a wide overlay no longer spills over the words beside it). 2 new tests. Checked with a real animated 7TV emote (55 frames) and in the example (120 frames a second). **Not verified:** the motion in the app.

- Chat badges (2026-11-01, step 9): Helix global and channel badges, loaded with the 7TV sets and kept for the session, shown before the names. 2 new tests on 2 fixtures (`twitch_capture -- badges`). Verified in the app. BTTV and FFZ dropped: 7TV is enough (the user's choice).

- 7TV emotes in the chat (2026-11-01, step 9): `twitch::seventv` (global and channel sets, the channel's aliases, still frames of animated emotes), loaded once per channel and session from the room's id, words replaced by emotes. 5 new tests on 3 saved responses. Verified in the app on a busy channel.

- Step 9 done (2026-11-01): the read-only chat beside a live. Generic in `ui::chat` (model, `ChatPanel`: a virtual list of 500 messages following the newest, held while scrolled up, emotes inline, deletions greyed, the room's modes; `watch_layout`: the watch page with a chat column), so that YouTube lives can plug in later; `twitch::chat` (IRC parser, anonymous WebSocket with reconnection) and `spaces::twitch::chat` in between. Emote ranges proved to count code points. 19 new tests, 9 of them on a fixture of 21 anonymized IRC lines. Verified in the app on three busy channels; the panel example holds 120 frames a second at 50 messages a second. **Not verified:** a reconnection, the resume button's click, a deletion seen in the app. Badges, third-party and animated emotes come next.

- Step 8 done (2026-11-01): VODs (through the proxy, seekable, storyboard previews on the seek bar), clips (direct MP4s from GQL) and a channel's page (header, live banner, VODs and clips tabs); `peeby` opens `twitch.tv/videos/<id>` and clip links too. 7 new tests on 5 new fixtures. Verified in the app: a VOD in 0.99 s with a preview on hover, a clip in 1.34 s, the channel page. **Not verified:** the clips tab by click, a subscriber-only VOD in the app. Muted segments came later (see Twitch muted audio).

- Step 7 done (2026-11-01): the Helix catalog. `twitch::helix` (followed lives and channels, users, top categories, a category's lives, search; a refused token refreshed once) tested on 8 fixtures saved by `cargo run -p peeby --example twitch_capture` (trimmed, every trace of the account replaced). Pages: "Suivies" (lives as cards, then the offline channels), "Parcourir" (categories) and a category's lives, the search from the top bar; the sidebar lists the followed channels on air, refreshed every 2 minutes. Verified in the app signed in, from screenshots. **Not verified:** the offline section, the sidebar's refresh, a 429.

- Step 6 done (2026-11-01): lives play in the Twitch tab. `peeby twitch.tv/<login>` (or a name typed on "Suivies", a placeholder until step 7) opens a watch page: the shared player in live mode through the ad-removing proxy, the channel's header from a GQL `Channel` query (avatar, title, category, viewers, uptime), a column kept for the chat, theater mode, the ad notice when the backup cannot cover a break, a default quality setting. Verified signed in with the user's auth-token: playback after 1.33 s, 6 qualities, the live badge with uptime and viewers. **Not verified:** the ad notice in the app, the mini-player with a live, the offline and unknown-channel messages in the app.

- Step 5 done (2026-11-01): the Twitch tab (violet, `TW`) with its sign-in card (Device Code Grant with Peeby's Client-ID: the code shown large, twitch.tv/activate opened, polling), `twitch::auth` (device flow, single-flight refresh storing each single-use refresh token before use, validation at startup and hourly, revoke), the secrets in the Credential Manager (`twitch|refresh`, `twitch|auth-token`), and its settings part (account, the optional auth-token with its tutorial, checked through GQL `currentUser`). 6 new tests on 5 new fixtures; `smoke` gets a device code live. Checked on screenshots: the tab, the card, the settings part; then with the user: a real sign-in and auth-token, and the session restored after a restart. **Not verified:** the hourly validation, sign-out (it would end the user's session).

- Step 4 done (2026-11-01): the shared player plays lives (`Source::live`: no seek bar, seeking keys or remaining time, an "EN DIRECT" badge with uptime and viewers from `set_live_info`, quality changes at the live edge, mpv's window duration ignored) and shows a page's message over the picture (`set_notice`, for a removed ad). `PEEBY_LIVE=1 cargo run -p ui --example player` shows both; checked on a screenshot, with the normal mode unchanged. The app still opens and plays a YouTube video (screenshot). **Not verified:** YouTube's control bar in the app (it had hidden before the screenshot, no mouse moved), Jellyfin, the live keys pressed.

- Step 3 done (2026-11-01): ads removed. `twitch::ads` reads media playlists (Amazon titles, stitched-ad date ranges, live sequence), `twitch::splice` fills an ad break with an ad-free backup playlist (`playerType=popout`) aligned on Twitch's live sequence, and `twitch::Proxy` serves the result to mpv from `127.0.0.1` (random key, playlists only, segments straight from the CDN). `Client::play` asks for both tokens together. Measured in the spike: 1.35 s to playback through the proxy; a forced pre-roll (`site`) covered by the backup, 1.51 s, no stall. 8 new tests on the saved reload sequence (98 in the workspace). **Not verified:** a mid-roll, the no-backup notice, the switch by eye.

- Step 2 done (2026-11-01): the `twitch` crate's GQL playback token (raw query, `playerType=embed`, Client-ID read from twitch.tv's page), usher v2 then v1, master variants (v2 `IVS-*` and v1 formats, landscape/portrait, codecs, audio only), and errors (`NotLoggedIn`, `NotFound`, `Offline`, `SubscriberOnly`, `IntegrityRequired`). 13 tests on 6 new fixtures; `cargo run -p twitch --example smoke` passes live. **Not verified:** `IntegrityRequired` against a real answer, the auth-token path, clips and storyboards (step 8).

- Step 1 of [twitch-plan.md](twitch-plan.md) done (2026-11-01): `spikes/twitch-play` plays a live in 0.98–1.29 s and a public VOD in 1.29 s, anonymously: the raw GQL token query, `playerType=embed` (no pre-roll, unlike `site`), usher v2, 1080p60 H.264 on the GPU. Redacted fixtures in `crates/twitch/tests/fixtures/` (masters v1 and v2, 25 reloads across a pre-roll, a backup playlist, GQL tokens, a subscriber-only VOD's 403). The `twitch` crate exists, empty. **Not verified:** a mid-roll, HEVC/AV1 (capped at 1080p signed out), anything signed in.

## Peeby refactor

- Step 10 done (2026-11-01): the docs reread and brought up to date (architecture, design, docs/README.md, CLAUDE.md). The refactor plan is complete; what was not verified is listed under each step of [refactor-plan.md](refactor-plan.md).
- Step 9 done (2026-11-01): [twitch.md](twitch.md) (the research with its sources, and what the architecture covers) and [ADR 0010](decisions/0010-twitch-space.md): ads removed by a playlist-rewriting layer, the web `auth-token` asked for with a tutorial (the user's choices). No code.
- Step 8 done (2026-11-01): one settings page behind the gear, with the app's sections, a shared `[player]` section used by both spaces, and a part per space (`Space::settings`); YouTube's account moved there from its sidebar. Verified on screenshots; see the plan.
- Step 7 done (2026-11-01): YouTube search (`Client::search`, `search_more`, `parse_search` tested on two new redacted fixtures, a results page behind the top bar's search button with next pages on scroll). Verified signed out; see the plan.
- Step 6 done (2026-11-01): the Jellyfin space (`spaces/jellyfin/`): login with Quick Connect, token in the Credential Manager, Home with its banner and rows, libraries as a virtual grid, a title's page, search from the top bar, playback in the content panel (`Layout::Page`) with reports, track mapping, segments, trickplay and adjacent episodes, its settings page. Shared pieces added to `ui`: BlurHash placeholders and per-server image routes, logos, posters with progress and badges, scrolling rows, the grid layout, `plain_text`. Verified on the user's server and on screenshots; see the plan for what was not.
- Step 5 done (2026-11-01): `jellyfin-api` copied from peeby with its fixture tests (17 pass), its live tests reading credentials from the environment; peeby's Jellyfin and player docs and the OpenAPI spec copied, its ADRs archived in `decisions/peeby/`, [ADR 0009](decisions/0009-jellyfin-12-single-account.md). **Not verified:** the live tests (no Jellyfin server given yet).
- Step 4 done (2026-11-01): the shell (top bar with tabs, search, aggregated notifications and gear; per-space sidebar and page stacks; accent per space), YouTube under `spaces/youtube/`, a placeholder Jellyfin tab, the mini-player window above the others ([ADR 0008](decisions/0008-mini-player-window.md)). Verified on screenshots with posted clicks; see the plan for what was not.
- Step 3 done (2026-11-01): peeby's player features in `ui::player` (tracks, subtitle delay, chapters, segments and skip, "up next", help, seek steps, full-window layout, events for reports); `cargo run -p ui --example player` shows them on a generated video. Verified: clippy, tests (segments, text), the example on screenshots, YouTube playback 1.48 s. **Not verified:** menus and keys by hand. Arrows now seek 10 s on YouTube too.
- Step 3 started (2026-11-01): the player moved to `ui::player` and plays a generic `Source`; the mini-player approach was checked with `PEEBY_PIP=1 spike-yt-play` (zero-copy video in an always-on-top `PopUp` window and back). Verified: clippy, tests, playback 1.44 s with 8 qualities and 6 subtitle tracks. **Not verified:** the quality and subtitle menus and the storyboard by hand (same code, not clicked).
- Step 2 done (2026-11-01): `ui` crate (theme with a switchable accent, motion, icons, image store behind a `Fetch`, `Picture`, `MediaCard` in four layouts, skeletons, notices, settings rows); YouTube pages use it, no visual change. Verified: clippy, tests (2 new), Home and watch page on screenshots, playback 1.47 s.
- Step 1 done (2026-11-01): PeebTube renamed Peeby (crate, exe, window, `%APPDATA%\peeby`, `PEEBY_*`), version 0.2.0, [ADR 0007](decisions/0007-multi-space-app.md); `PEEBY_DISPLAY` picks the screen the window opens on. Verified: build, clippy, tests, and `peeby <url>` playing after about 1.5 s. **Not verified:** which screen the window lands on with `PEEBY_DISPLAY=secondary` (no warning, so a second display was found). Next steps and checklists: [refactor-plan.md](refactor-plan.md).

## Notifications

- Done (2026-11-01): the bell of subscribed channels, on the watch page (right of the channel) and on the channels page's tiles: "Toutes", "Personnalisées" or "Aucune" (those the channel offers), the icon showing the current level. Parsed and tested on `subscriptions_channels.json` and a new trimmed, redacted signed-in watch page (`next_signed_in.json`); the signed-out `next.json` carries a bell too but is not subscribed, so it shows none.
- Verified by driving the app: the watch page of a subscribed channel shows its bell ("Toutes"), the menu opens with the current level checked; the channels page shows the 475 bells (all three icons seen), a tile's bell opens its menu without opening the channel, and the grid still scrolls. `smoke` reads 475 bells out of 475 channels live.
- **Not verified:** no level was changed: `notification/modify_channel_preference` was never sent (writing to the account needs the user's go-ahead), so its response has no fixture yet, and the toasts and the revert on failure were not seen. Scrolling smoothness of the channels page with 475 bells was not measured.
- Then: the **inbox**, a bell at the right of the title bar with the unseen count, the notifications in sections as on youtube.com, next pages on scroll, a click opening the video and marking it read. youtube.com reads it with `browse` `FEnotifications_inbox` (found from the browser's own request): `notification/get_notification_menu` answers an empty inbox. Parsers tested on 3 new redacted fixtures (inbox, its next page, the unseen count); `smoke` reads the inbox, its next page and the count live. Fix on the way: the channel bells' levels were not forgotten on a session change (`bell::reset` ran at startup only).
- Verified by driving the app: the bell in the title bar, the panel (Important, Autres notifications, unread dots, avatars, thumbnails), next pages while scrolling (down to notifications 12 days old), no warning in the log.
- **Not verified:** the badge with a count above 0 (the count was 0 during the test), whether YouTube resets its count when the panel opens, clicking a notification: `notification/record_interactions` was never sent (writing to the account needs the user's go-ahead) and its body field name (`serializedRecordNotificationInteractionsRequest`) is not confirmed by a capture. The notifications' "⋮" menu (hide, fewer, turn off) is not offered.

## Smooth feeds

- Done (2026-11-01): the feeds (Home, subscriptions in grid and list mode, channel) are a virtual list, so a scroll builds only the rows in view instead of every loaded card; their header stays at the top. List rows take the whole width; grid cards are at least 416 px and widen to fill the row.
- Verified: the app starts and Home loads (22 videos). **Not verified:** scrolling smoothness, the next page loading at the end, resizing and folding the sidebar (cards per row), the grid/list switch, by hand.

## Like and dislike

- Done (2026-11-01): the watch page's like and dislike buttons rate the video (`like/like`, `like/dislike`, `like/removelike` with the page's `params`); a click on the active one clears the rating. They show the user's rating (`likeStatusEntity`, parsed and tested on `next.json`), change at once and go back with a toast if YouTube refuses; signed out, a toast asks for cookies. The two halves sit close together on a pill as wide as its content; the RYD link moved to the attribution line.
- **Not verified:** not driven in the app, and no rating was sent (writing to the account needs the user's go-ahead), so the `like/*` responses have no fixture yet and the signed-in `likeStatus` is untested.

## Theater mode and folding sidebar

- Done (2026-11-01): theater mode on the watch page ("Mode cinéma" button and T key; the player across the page above the details and related videos), a burger button at the left of the title bar that folds the sidebar to its icons (labels as tooltips). Both are saved in `settings.toml`.
- **Not verified:** not driven in the app yet (build, clippy and tests only): the theater layout and its height, the button and the T key, the folded sidebar and its tooltips.

## Save to playlist

- Done (2026-11-01): "⋮" menu and "Watch later" shortcut on grid cards and list rows (hover only), "Enregistrer" on the watch page (since then a "⋮" menu with "Enregistrer dans une playlist"), the list of playlists with checks, toasts. `playlist/get_add_to_playlist` is parsed and tested on a redacted fixture.
- Then: toasts at the bottom right; an "x" on the videos of editable playlists (page and queue panel) removes the entry (`ACTION_REMOVE_VIDEO` by `setVideoId`, parsed and tested on the "Watch later" fixture).
- Then: a spinner replaces the "x" and the "Watch later" button while their request runs; "À regarder plus tard" in the sidebar, above Playlists; a custom loader (ring and stretching arc); the "Watch later" shortcut shows a check on videos already in it (ids loaded at startup) and then removes them; a reverse-order button on playlist pages (saved per playlist); the URL field of the title bar is gone.
- Then (2026-11-03): the playing video of a queue gets its "x" too; it plays on, out of the queue, and Next or its end plays the video that followed it (shuffle keeps that one next). **Not verified:** build, clippy and tests only, not driven in the app (removing writes to the account).
- Verified: the card buttons appear on hover. **Not verified:** the popover and its list in the app (the test run was stopped), the "x" buttons and the toasts' position on screen, and any actual add or remove: the `browse/edit_playlist` request was never sent (writing to the account needs the user's go-ahead), so its response has no fixture yet.

## Playlists

- Done (2026-11-01): "Playlists" in the sidebar: the user's playlists, a playlist's page (header, numbered videos, next pages on scroll), "Tout lire" and "Aléatoire". Playing a playlist shows its queue next to the player: a video that ends plays the next one, previous/next buttons and P/N keys, shuffle, repeat, next pages loaded as the end comes near, next stream resolved ahead. Parsers tested on 4 new redacted fixtures (library, "Watch later", a public playlist and its continuation); `smoke` checks the library, "Liked videos" and its next page live.
- Verified by driving the app: the playlists page (15 playlists), "Watch later" (17 videos), "Tout lire" (playback after 1.7 s), N, then the end of a video playing the next one (1.3 s, prefetched), the same in fullscreen (it stays fullscreen, Escape still works), playing from row 4, shuffle, back to the playlist page.
- **Not verified:** reaching the end of a loaded page during playback (the queue loads the next one first; only its request was checked live), repeat at the end of a playlist.
- Done (2026-11-07, the user's request): playlists and courses in search results and in the related list, as stacked cards; "Mixes" in the sidebar ("My Mix", the home feed's and the library's mixes) and a mix's page from `next`. Parsers tested on 3 new fixtures (a related list with a playlist, a mix, a signed-in home page with a mix); the requests were checked live once (5 home pages: 4 mixes; "My Mix": 24 videos). Verified in the app: the Mixes page (6 mixes). Then: mixes on Home (as grid cards, in their places), in related lists and in search results, tested on 2 more fixtures (Despacito's related list and a search, anonymous). **Not verified:** the mix cards on Home, the related list and search in the app.

## Going back in a live

- Done (2026-11-05, the user's request): back to the start of a live, and back to the live, on YouTube and
  Twitch.
  - **YouTube** ([ADR 0025](decisions/0025-youtube-lives-from-dash.md)): lives with DVR play from VISIONOS's DASH segments
    through `innertube::LiveProxy`, the whole broadcast up to 12 h (the HLS window was an hour, and on most
    ordinary broadcasts nothing before it was served). MP4 formats only: VP9 (WebM) lost its video at the
    first seek, found by the user in the app on 2026-11-05.
  - **Twitch** ([ADR 0026](decisions/0026-twitch-live-rewind-from-its-recording.md)): a live whose
    channel keeps its VODs gets the shared player's `Rewind`, its recording through a second proxy, with
    the recording's position saved to the account as a VOD's (from the live too); `minute-watched` pauses meanwhile.
  - Tests: 8 new (formats on the live fixture; the proxy's paths, ladder, segment length, playlist, edge
    and search for the first segment kept; a live's recording on 2 new fixtures).
  - Verified anonymously:
    - `live_probe`: a 2 h 38 live listed from segment 0, both ends served through the redirects,
      resolved in 0.16 s; a 24/7 live's 12 h.
    - `spike-yt-play` through the real proxy: first frame at the edge 1.1 s, 30 s into the broadcast
      0.8 s; 3 minutes at the edge of a 29.97 fps live with no 404 and no buffering.
    - The search's request: one byte of a gone segment answers 404, of a kept one 206 (a `HEAD`
      answers 200 for both).
    - A Twitch recording in progress through `spike-twitch-play` and `twitch::Proxy`
      (`PEEBY_PROXY=1`) from 1 h in and from 90 s before its end (2.5 s, token included), the playlist
      reloaded from the proxy near the end.
    - The player example's `PEEBY_LIVE=rewind`: the recording at 60 s, then back to the live.
  - Fixed the same day after the user's test:
    - YouTube: MP4 formats only (VP9's video ended at the first seek), and the edge read on each
      reload (a seek back took 5 to 6 s, now 1.9 s).
    - Twitch: the recording asked anonymously on its own, since `archiveVideo` fails with an
      auth-token. Inside the channel's query it had broken signed-in headers and found no recording.
  - Then (2026-11-05, the user's request): a Twitch live keeps its last 2 minutes in the proxy's
    playlist (`twitch::LIVE_KEPT`, 60 segments; Twitch's CDN serves a segment 3 to 5 minutes after it
    leaves Twitch's own playlist, measured with `segment_ttl`). Seeks within them stay in the live
    (no restart), and the recording takes over beyond, so its delay leaves no gap; a live with no
    recording gets those 2 minutes. Verified with `spike-twitch-play` through the proxy: 1 to 3 s
    behind the edge as before, a seek 60 s back playing again in about 1 s. **Not verified:** in the
    app, and a seek across a replaced ad break (none came during the runs).
  - **Not verified:** in the app (a playback writes the account's history on both services: with the
    user's agreement), and the search trimming a live's front: both 24/7 lives served their whole
    12 h that day. That covers the seek bar's drag over 12 h, the badge's click, a quality change in
    the past, the window shortened after the search on a 24/7 live, the URL refresh after 5.5 h, the end
    of a live through the proxy, and a Twitch recording's position saved and the pause in
    `minute-watched`.

## YouTube lives

- Done (2026-11-02): lives start at their edge in about 0.6 s (they waited about 10 s at the start of their
  window), can be sought back over their DVR window (an hour) with the seek bar, ±10 s and the keys, pause in
  place, and come back to the edge with the "EN DIRECT" badge, greyed with the delay while behind
  ([ADR 0011](decisions/0011-youtube-live-dvr.md)). Their chat shows in a column, as on Twitch: polled from
  `live_chat/get_live_chat`, messages paced as they were sent, moderators and owner in YouTube's colors,
  members' badges, the channel's emojis. Cards of lives show an "EN DIRECT" tag on the thumbnail
  (grid, list and compact). 3 new tests on 4 new fixtures (live player and media playlist, live `next`, a
  chat page with renamed chatters). `smoke` checks a live's window and chat. Verified in the app: a live's first frame about 1 s after
  the click (stream 0.31 s, then mpv's seek to `length − 15` and first frame 0.68 s later, from the mpv log),
  its chat filling, and after three ← keys the greyed badge with `-0:41`, the seek bar and the ±10 s buttons
  (screenshots). **Not verified:** the badge's click back to the edge and the seek bar's drag in the app, a
  quality change in a live, a live without DVR, a live opened in its first seconds, Super Chats and
  memberships (none in the saved responses); the comments of a live page do not load more on scroll (its
  layout scrolls on its own).

## Members-only videos

- Done (2026-11-06): cards of members-only videos show a star and "Réservé aux membres" at the thumbnail's
  bottom left (the star alone under 200 px: queue, related videos, playlist rows) (`VideoItem::members_only`, from the lockup's `BADGE_MEMBERS_ONLY` metadata row, which no longer passes for
  the channel's row on a channel's own tab). When VISIONOS refuses one, a WEB `player` check with the session
  tells a member (yt-dlp plays it with the cookies) from anyone else, to whom the player says the video is for
  the channel's members ([ADR 0005](decisions/0005-bundled-yt-dlp.md)). 2 tests on 2 new anonymous fixtures
  (Okayu Ch.'s live tab, a members-only `player`). **Not verified:** the badge and the message in the running
  app, a member's playback through yt-dlp; search results (`videoRenderer`) carry no badge (anonymous
  searches, global or in a channel, leave these videos out: no response to parse).

## Build times

- Done (2026-11-07, [ADR 0029](decisions/0029-crates-per-space.md)): the dev profile keeps line tables only
  (the PDB was 537 MB), and the binary is split into `peeby-core` (shell, settings, desktop, updates, sign-in
  window) and one crate per space. A one-line change in a space now rebuilds in 8–12 s instead of 17–36 s; in
  `peeby-core` it still takes 18–32 s (every space is rebuilt after it). The 97 tests of the former `peeby`
  crate all pass in their new crates. **Not verified:** the app run after the split (it builds and links).

## Visual refresh

- Done (2026-11-01): peeby's visual system ([design.md](design.md), [ADR 0006](decisions/0006-peeby-visual-system.md)): card hover (lift, red ring, play button), image fade-in, page fade, skeletons on the feeds and the watch page, Fluent sidebar, settings rows, account card, centered empty and error states.
- Then: the subscribed channels moved from the sidebar to a **Channels page** (grid with a filter, opened from "Toutes les chaînes" on the subscriptions page); grid rows align cards at the top (titles at most two lines); feeds drop repeated videos, which made hovered cards jump (shared hover state).
- Subscriptions page: grid or list mode, saved in `settings.toml` (`subscriptions_layout`). Verified: the switch, the rows, hover, and the saved value.
- Done (2026-11-02): the subscriptions feed is in date order again: YouTube's "Les plus pertinentes" shelf, at the top of the grid, is left out (`parse_subscriptions`, tested on a new redacted fixture; checked live with `cargo run -p innertube --example home -- <cookies> subscriptions`). YouTube setting "Page d'accueil" (`start_page`): Home or the subscriptions at startup. **Not verified:** the setting in the running app.
- Verified by driving the mouse: hover on Home and on the subscriptions page (one hover event in, one out, even at the bottom edge of a card), rows aligned at the top with one- and two-line titles, the channels page (475 channels, filter, hover, a click opens the channel). Playback: 1.7 s on a test video. **Not verified:** page fades by eye.

## Milestone 4 status

- Done (2026-11-01):
  - **Account page** (sidebar "Compte"): session state, export instructions, "Importer cookies.txt…" (file picker; checked with a Home request before it replaces the session file), "Se déconnecter". The app switches session without restarting. Pages that need a session link to it; an expired session says so.
  - **yt-dlp updates**: single-file build (it can update itself, "onedir" cannot), `yt-dlp -U` at most once a day in the background. Verified: "yt-dlp is up to date (stable@2026.10.26)", then the fallback played an age-restricted video in 5.4 s.
- **Not verified:** the import flow in the UI (file picker, valid file), since no fresh cookies were available; the expired-session state was seen in the logs only.
- **Settings page** (sidebar "Réglages"), saved in `%APPDATA%\peebtube\settings.toml` (`serde` + `toml`, agreed with the user):
  - language and region of YouTube (11 presets): the clients are rebuilt at once, Home reloads;
  - default quality (2160p to 360p), used by the watch page and the hover prefetch;
  - volume and mute, kept across launches.
  Writes are delayed 0.5 s and replace each other (a volume drag makes dozens of changes). Unknown or missing keys take their defaults.
  Verified: a file with `max_height = 720` gives a 1280x720 stream at launch. The page itself was not clicked in a test run.

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
  - Player controls adapted from peeby's player page (`player_view.rs`): seek bar with a storyboard thumbnail and the time over the hovered point (YouTube's `playerStoryboardSpecRenderer` sprite sheets, largest level) (click and drag: the playhead follows the mouse until the release), volume slider, play/pause, ±10 s, fullscreen (only the player is shown), auto-hide, OSD, pause pulse, shortcuts (K/space, J/L, arrows, 0–9, M, F, Esc). Volume is kept for the session.
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

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| YouTube blocks the VISIONOS client | No playback | yt-dlp fallback, values tracked against yt-dlp's source |
| YouTube changes renderers | Empty feeds or pages | Lenient parsing, fixtures, `logged_in` checks |
| Cookies expire or rotate | Signed out | `Error::NotLoggedIn` shown to the user; persist `Set-Cookie` (milestone 4); `RotateCookies` every 10 min (ADR 0004) |
| Copied crates drift from peeby | Duplicate fixes | Shared repository once both stabilize |
