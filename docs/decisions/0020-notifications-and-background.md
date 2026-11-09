# 0020: Windows notifications, the notification area and a single instance

- Status: accepted
- Date: 2026-11-03

## Context

The bell only knew what happened while Peeby was open and on screen: Twitch's followed channels
going live (read every two minutes from Helix) and YouTube's inbox (its unseen count every 30
minutes). The user wants:

- Twitch: a followed channel goes live, a followed live changes category, with the streamer's
  picture in the Windows notification;
- YouTube: a new video and a live started, from what YouTube's own bell sends;
- for each kind, the choice of the bell, Windows' notifications, both or neither;
- Peeby kept running in the notification area when its window closes, quit from the icon's
  menu; an option to make the "×" quit (off by default) and to start with Windows.

What GPUI 0.3.10 offers on Windows: toasts (`App::show_system_notification`) without an image,
no icon in the notification area, and `Platform::hide` is a no-op (there is no per-window hide).
Windows calls a toast's `Activated` only while the app holds the toast object:
`tauri-winrt-notification` (in the lock file already) drops it as soon as it is shown, so a
click would be lost.

YouTube, checked on 2026-11-03 with the user's go-ahead: the inbox (`FEnotifications_inbox`)
tells uploads and lives apart by the label its click is tracked with (base64 bytes of
`navigationEndpoint.clickTrackingParams`: `ni-push-u-sub` for an upload or a Short,
`ni-push-lbss` for a live), the same whatever the language; a notification's id is its time in
microseconds. Reading the inbox sees the notifications on youtube.com: the unseen count went
from 1 to 0 a few seconds after the list was read.

A Peeby kept in the background makes a second launch likely (a shortcut, `peeby <url>`, the
start with Windows). Two processes would both rewrite the cookie jar YouTube rotates (ADR 0004)
and spend the single-use Twitch refresh token (ADR 0010): one of the sessions would die.

## Decision

- **Kinds and settings.** `[notifications]` holds a `Delivery { in_app, windows }` per kind
  (`twitch_live`, `twitch_category`, `twitch_inbox`, `youtube_video`, `youtube_live`), all on
  by default. Each
  space shows its kinds in its part of the settings (`shell::delivery_row`: "Peeby" and
  "Windows" switches). The spaces decide what to announce; `desktop::toast` only shows.
- **Twitch** (`spaces::twitch::bell`): the bell remembers each live's category id. A channel
  that was not on air is `Change::Live`; one whose category id changed to another non-empty one
  is `Change::Category`. The first reading still only learns. `Bell::update` keeps in the bell
  the kinds whose `in_app` is on and returns every event; `bell::toasts` shows those whose
  `windows` is on, with the channel's avatar.
- **YouTube** (`spaces::youtube::inbox`): `NotificationKind` (`Upload`, `Live`, `Other`) is
  parsed from the tracking label. The unseen count is read every 3 minutes; Peeby's badge starts
  at YouTube's count and then counts the new notifications itself. When YouTube's count grows,
  the first page of the inbox is read and the notifications newer than the session's start
  (minus a minute) not counted yet are the new ones; reading it clears youtube.com's count, so
  the list is read only then. The panel leaves out the kinds whose `in_app` is off; `Other`
  (replies, mentions) always shows in the bell and never in Windows'. A toast's click marks the
  notification read and opens the video.
- **Twitch's notification center** (`spaces::twitch::inbox`, 2026-11-09): the account's own
  notifications on twitch.tv (its bell: watch streaks, drops, Twitch's news), read with the
  auth-token every 3 minutes through raw GQL (`twitch::notifications`: `currentUser {
  notifications(first: 20, displayType: VIEWER) { summary { unseenCount lastSeenAt } edges {
  node } } }`, fields found from GQL's validation messages, introspection being off; fixture
  `gql_notifications.json`). They need only the auth-token, not Helix. Reading writes nothing:
  no mutation marking them seen was found among the guessed names, so Peeby's "seen" is its own
  (the panel opened), after twitch.tv's `lastSeenAt`, and twitch.tv's count stays as it is. A
  notification made after the last look is new. The first reading only learns; later, one not
  read before and made since Peeby started (minus a minute) is announced. Its kind is
  `twitch_inbox` (a Windows notification with its picture, titled "Twitch"). They show in
  Twitch's tab with the followed lives' events, newest first: a channel's avatar round,
  Twitch's art square, bold runs as Twitch writes them (`**reward**`). A click opens a
  channel's live, a VOD or a clip in Peeby; anything else (a drop campaign, a streak to save,
  Twitch's blog) opens in the browser.
- **The bell's panel** (`shell::bell`, 2026-11-09): a tab per space whose `Space::unseen` is
  `Some`, with its unread count, the tab bar of the list of lives (`shell::space_tabs`, shared
  by both). It opens on the tab shown last while it has unread notifications, else on the first
  that has some. Only the tab shown builds its space's section, so a space sees its
  notifications (its badge cleared, YouTube's marked read) only when its tab shows.
- **Floods**: beyond 3 Windows notifications from one reading (after a sleep, an outage), one
  sums them up.
- **Toasts** (`desktop::toast`): built with WinRT (`windows` 0.62, the version GPUI's Windows
  platform already builds) with the image as `appLogoOverride`, cropped round, and an optional
  preview as an inline image under the text (a YouTube upload's or live's thumbnail). Windows
  reads neither URLs nor WebP for an unpackaged app: `ui::images::local_png` writes PNGs into
  the image cache (96 px avatars, 640×360 previews). Each kind's row in the settings has a
  "Test" button: a sample of the kind shown through the same code as a real one, whatever its
  switches say, made as indistinguishable as can be. YouTube's announces a real video (YouTube's
  first, "Me at the zoo") in YouTube's wording, with its channel's name and avatar from its
  watch page (`next`, which writes nothing to the account). Twitch's announces a followed channel
  on air now, with its avatar, title and category; without one, the user's own account. Its
  click only shows Peeby, and opens no video, which would write the account's history. The last 32 toasts are kept so that their click arrives; it goes through a
  channel to the UI thread, shows the main window and runs the toast's action there. The app id
  `Peeby.Peeby` is registered under `HKCU\Software\Classes\AppUserModelId` with the name and
  the icon (written to `%LOCALAPPDATA%\peeby\peeby.ico`). The `ToastNotifier` made at startup
  gets disconnected from Windows' notification service after a while (`Show` fails with
  0x800401FD, seen 25 to 70 minutes after launch, without any restart of the service or the
  shell): on a failure, a new notifier shows the toast and replaces it.
- **Notification area** (`desktop::tray`, `tray-icon` 0.26): Peeby's icon (resource 1); a left
  click opens the list of lives, a double click shows the window, its menu has "Open Peeby" and
  "Quit". Its hidden window gets its
  messages from GPUI's loop on the same thread. `close_to_tray` (on by default) makes the "×"
  hide the main window (`on_window_should_close`, `ShowWindow(SW_HIDE)`); off, or without the
  icon, it quits as before. With several windows (ADR 0028), this is the last one's "×"; the
  others close with their tabs, and the "main window" here is the window used last.
- **List of lives** (`shell::lives`, 2026-11-03, the layout of the Gumbo extension for Twitch):
  a `PopUp` window above the icon (on the icon's screen, `sys::monitor_at` giving its
  `DisplayId` and scale) with a tab per space whose `Space::lives` is `Some` (the shell still
  knows no service): Twitch's followed channels on air from the sidebar's reading (every one of
  its page, the sidebar keeps showing 12), YouTube's from the top of the subscriptions feed (live
  badge, no duration), read when the list opens and at most once a minute, never in the
  background. A row opens its live in the space's last tab, in the main window shown. Windows
  sends a double click as click, double click, release: a click waits for the double click's
  time (`GetDoubleClickTime`, at most 100 ms: the default 500 ms made the list slow) before opening the list, and a double click cancels it. The list
  closes when it loses the focus; the click on the icon that took it does not open it again.
- **Nothing plays where nobody looks** (ADR 0019): hiding the window pauses the video of the
  page shown (`Shell::go_background`); a mini-player stays on screen and goes on. Twitch's
  `minute-watched` follows the player, so it stops with it.
- **Single instance** (`desktop::sys`): a named pipe per user (`\\.\pipe\peeby-<user name>`:
  pipe names are machine-wide). The first process creates it with
  `FILE_FLAG_FIRST_PIPE_INSTANCE`; a next launch finds it taken, writes its arguments (one a
  line) into it and ends. When the pipe exists but nobody answers within a second, the launch
  ends too: running alone would bring back the race on the cookies and the refresh token. The
  first one shows its window and opens what it was given (YouTube videos, a Twitch page). The
  next end of the pipe is made before the last one is read, so the pipe never disappears.
- **A debug build is another profile** (`settings::PROFILE`, 2026-11-03), so that `cargo run`
  works while the release build runs: `peeby-dev` names its pipe (`\\.\pipe\peeby-dev-<user
  name>`), its settings and cookies folder (`%APPDATA%\peeby-dev`) and its Credential Manager
  service, so it never shares the cookie jar or the single-use refresh token and signs in on its
  own. Its app id is `Peeby.PeebyDev`, its name "Peeby (dev)" (window, notification area, toasts,
  `Run` value). The caches (`%LOCALAPPDATA%\peeby`) stay shared.
- **Start with Windows** (`start_with_windows`, off by default): a `Peeby` value under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` with `"<exe>" --background`; it is
  rewritten at startup (the executable may have moved). `--background` opens the main window
  hidden.
- The Win32 calls (`ShowWindow`, the pipe) are `desktop::sys`, an FFI module like `player::sys`.

## Consequences

- Peeby is the user's notification center while it runs: YouTube's badge on youtube.com and the
  phone clears when Peeby reads a new notification (as if the bell had been opened).
- A test run (`cargo run -p peeby`) while Peeby runs only hands its arguments to it: quit it
  first (icon → Quit).
- A notification that arrives while Peeby is closed is never shown: there is no COM activator
  or background task, so nothing runs without Peeby. A click on an old toast in the Action
  Center after Peeby restarted does nothing.
- A YouTube label other than the two seen counts as `Other`: if YouTube renames them, uploads
  and lives stop reaching Windows' notifications and the parser's fixture tests do not catch it
  (they pin the labels of 2026-11-03).
- Revisit if GPUI gains images in its toasts, a tray or a window hide: `desktop` would shrink.
