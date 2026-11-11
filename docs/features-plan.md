# Plan: the next features (2026-11-11)

The user picked these from a review of the app (2026-11-11). Nothing is started: each item is
done in its own change, with its docs, fixtures and tests, in the order the user chooses. Each
item is numbered as in the user's list (items 2, 14, 15, 16 and 18 to 21 were left out).

Rules that apply throughout (CLAUDE.md): a new endpoint gets a saved, redacted response before its
parser; anything that writes to an account (a comment, a subscription) is tested only with the
user's go-ahead; UI strings in English and French; a setting goes to its space's part of the
settings page, or to General.

## YouTube

### 1. SponsorBlock

Done (2026-11-11, [ADR 0034](decisions/0034-sponsorblock.md)), checked by the user in the app.

Skip, or offer to skip, a video's sponsored parts, intros, outros, self-promotion and subscription
reminders, from [SponsorBlock](https://sponsor.ajay.app)'s public API (anonymous, by video id or
its hashed prefix for privacy: `/api/skipSegments/<sha256 prefix>`).

- The player already shows Jellyfin's skippable segments ("Skip intro", "Skip recap", "Skip
  credits"): feed it the YouTube segments the same way (`Source`), with a "Skip sponsor" button and
  the segments marked on the seek bar.
- Settings (`[youtube]`): on or off (on by default); per category, skip automatically, show the
  button, or ignore. The segments are cached per video; a failure leaves the video untouched.
- Attribution as the API asks; an ADR (a new external service), and a fixture of its answer.

### 3. A channel page with its tabs, and 17. its header

Done (2026-11-11), checked by the user in the app.

Today the channel page is its "Videos" grid (sort and search).

- Tabs as on youtube.com: Videos, Shorts, Lives (`channel_streams` is already parsed), Playlists;
  each tab's first page saved as a fixture first (`browse` with each tab's `params`).
- Header (17): the banner, the large avatar, the name, the handle, the subscriber and video
  counts, the short description (opening the full "About" in a dialog), and the subscribe button
  with its bell (the roadmap's "Not done": it needs a signed-in capture of a channel page's
  header).

### 4. Search suggestions

Done (2026-11-11), checked by the user in the app.

Suggestions under the search field while typing, as on youtube.com (YouTube's completion
endpoint, the client's language), with the arrow keys and Enter; recent searches are not kept.
Debounced, and the request never logs the query at info level.

### 5. Comment sort

Done (2026-11-11), checked by the user in the app.

"Top comments" / "Newest first" over the comments, as on youtube.com: the comments header's sort
menu carries a continuation per order (fixture of the header first).

### 6. Writing a comment or a reply

Done (2026-11-11), checked by the user in the app (deleting one's own comments came with it).

A field over the comments ("Add a comment…") and under a comment ("Reply"), signed in, from the
comments page's composer (`createCommentEndpoint`, `createCommentReplyEndpoint`). A public write
from the account: tested only with the user's go-ahead, on a video they choose; the new comment
shows at once on top (its answer's renderer).

### 7. Shorts shelves on Home and the subscriptions

Done (2026-11-11), checked by the user in the app.

The `shortsLockupViewModel` shelves YouTube puts between the videos (the backlog's item): a row of
vertical cards opening the Shorts page on that Short. A setting turns them off ("Show Shorts in the
feeds", `[youtube]`, on by default).

### 8. Subscribing to a collaboration's channels

Done (2026-11-11), checked by the user in the app.

On a collaboration's watch page (`WatchPage::collaborators`), the subscribe button opens a menu
listing the channels, each with its own subscribe state, as youtube.com does (each list item's
`subscribeButtonViewModel`: a signed-in fixture first).

## Tabs and windows

### 9. Tab shortcuts

Done (2026-11-11), checked by the user in the app.

Ctrl+T (a new tab of the shown space), Ctrl+W (close the tab; the window's last one closes the
window, as the middle click), Ctrl+Tab and Ctrl+Shift+Tab (next and previous tab), Ctrl+1 to 8
(that tab), Ctrl+9 (the last one). Not while a text field has the focus where the key means
something there.

### 10. Restoring the tabs at startup

Done (2026-11-11, [ADR 0035](decisions/0035-restore-tabs.md)), checked by the user in the app.

At quit, each window's tabs (their space, their history's routes, the shown one) and its bounds
(and maximized state, its display) are saved; at startup they come back, built only when shown
(ADR 0019), no video playing until clicked. A setting turns it off ("Restore tabs at startup",
General, on by default), when the startup tabs setting applies instead. Probably an ADR (what a
route saves, how a space parses it back).

### 11. Dragging a maximized window's only tab

Done (2026-11-11), checked by the user in the app.

As with a title bar: pulling the only tab of a maximized window restores it, under the pointer
where the tab was held, and carries it (Windows' move loop, ADR 0028 step 6), Snap included.
Today its tab only detaches as a copy over the window.

### 12. Middle click on the tab bar's empty part

Done (2026-11-11), checked by the user in the app.

Opens a new tab of the shown space, as in Chrome (a double click there keeps maximizing the
window, as a title bar does).

## Player

### 13. Sleep timer

Done (2026-11-11), checked by the user in the app; behind a player setting, off by default.

In the player's settings menu: pause after 15, 30, 45, 60 or 90 minutes, or at the end of the
video (or of the episode, the playlist); the time left shows in the menu, and a notice a few
seconds before the pause. For every space (`ui::player`); not kept across launches.

## Twitch

Picked by the user (2026-11-11), numbered T1 to T5 as in the list they were given (T4, 7TV's
personal emotes, T6, followed categories, and T7, highlighted words, were left out).

### T1. The chat's history on joining

Done (2026-11-11, [ADR 0036](decisions/0036-twitch-chat-history.md)), checked by the user in the app.

A live's chat opens with its last messages instead of empty, as twitch.tv's does. IRC sends no
history: the usual source is a third-party service, robotty's
[recent-messages](https://recent-messages.robotty.de) (anonymous, by channel login, IRC lines
with their tags). Before anything, the user agrees to that service (an outside request naming the
channel watched), and an ADR records it; its answer saved as a fixture (chatters renamed, as every
chat fixture). The history goes through the same parser as the live lines (emotes, badges, 7TV),
marked as history (dimmed until the first live line), and the moderation events it carries
(`CLEARCHAT`, `CLEARMSG`) apply. A failure leaves the chat as today.

### T2. Completion in the chat's field

Done (2026-11-11), checked by the user in the app (`:` and a single letter list the emotes).

In `ui::chat`'s composer, generic (YouTube's live chat gets it too): Tab completes the word before
the cursor with the emotes (the channel's, 7TV, BTTV, FFZ, the user's own) and the chatters seen,
Tab again cycles; `@` opens a list of the chatters seen (most recent first), `:` a list of the
emotes matching (with their picture), arrows and Enter or Tab to pick, Escape to close. The words
the panel already knows (its emote sets, the recent authors) feed it: nothing is fetched.

### T3. A channel's schedule

Done (2026-11-11), checked by the user in the app (each row with its category's box art).

On a Twitch channel's page, its upcoming streams (title, category, start and length, recurring or
not; a vacation shown as such), from GQL's channel schedule (`channel.schedule.segments`,
anonymous): its answer saved as a fixture first (`twitch_capture -- gql-probe` to find the
fields). Nothing shows when the channel has no schedule.

### T5. Shared Chat

Done (2026-11-11), checked by the user in the app. Left out: the session's participants over the chat (IRC does not tell them, GQL needs a sign-in) and the duplicates of a merged multistream chat joined to two participants.

When several streamers merge their chats (Twitch's Shared Chat), each line from another channel
carries that channel's avatar (as twitch.tv shows it) and its badges are the other channel's; the
session's participants show over the chat. IRC tags the lines (`source-room-id`,
`source-badges`…): a captured Shared Chat session is saved as a fixture first (chatters renamed);
duplicates of a line (sent once per room) are dropped. Moderation applies to the line's own room.

## Added after the plan

The user's requests once the plan was done (2026-11-11), on `dev` as well.

### P1. A playlist's next video in the player

Done (2026-11-11), checked by the user in the app (smaller, with a chevron, the "×" in its corner, sliding in, moving with the control bar). In a playlist, 10 s before the video's end, the
player's bottom right shows the next video (thumbnail, "Vidéo suivante dans X s", title,
channel); a click plays it at once (docs/player/playback-ux.md, "Up next").

### P2. An autoplay switch on the player

Done (2026-11-11), checked by the user in the app (with autoplay off, a playlist's video now ends there). The shared player's control bar has an autoplay
switch wherever something comes next (YouTube's playlists, Jellyfin's episodes); it keeps the
player setting "Lire la suite automatiquement".

### P3. A Twitch channel's clips by period

Done (2026-11-11), checked by the user in the app. "24 h", "7 jours", "30 jours", "Depuis toujours"
over a channel's clips, as twitch.tv's filter (GQL's `ClipsPeriod`), the week by default.

### P4. A Jellyfin server behind WireGuard

On `feat/jellyfin-vpn` (2026-11-11), not checked by the user yet. A WireGuard configuration
(`.conf`) imported in the Jellyfin settings (also when signed out): while it is on, every request
and every video of the space go through a userspace tunnel (no driver, no administrator rights),
and the rest of the PC does not. The VPN connects only when Jellyfin needs it (not at launch)
and disconnects after 5 minutes unused. The spike measured 300 to 400 Mb/s and round trips of 1 to 2 ms
on the user's LAN ([ADR 0037](decisions/0037-jellyfin-wireguard.md)).
