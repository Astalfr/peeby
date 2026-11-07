# 0011: YouTube lives with DVR and chat

- Status: accepted; its HLS window superseded by [0025](0025-youtube-lives-from-dash.md) for lives with DVR
- Date: 2026-11-02

## Context

YouTube lives came out of VISIONOS like videos (an HLS master, video-only variants and an audio
rendition), so they played, but mpv started them at the beginning of their window and waited about 10 s
before moving. A live's media playlist is a window of 5 s segments: an hour for a DVR live
(`isLiveDvrEnabled`, `playlist_duration/3600` in the URLs), the broadcast so far while it is younger.
Measured with `spike-yt-play` on a live:
- mpv's timeline starts at the window's first segment; its duration is the edge and grows with the live;
  `seekable` is yes;
- `start=<window − 30>` plays 0.59 s after load; a negative `start` does nothing (no seek, the same stall);
- a seek 2 minutes back plays again in about 1.5 s, a seek back to `duration − 10` in about 2 s, in the
  same mpv: no restart is needed to move in the window or come back to the edge.

The user wants youtube.com's behavior: go back in a live, and back to the live.

Options considered for going further back than the playlist's window: rewriting playlists through a
local proxy (as Twitch's) with older `sq/N` segments, or DASH with `--live-from-start` as yt-dlp does.
Both cost a proxy or a second stream path for little gain: an hour back covers what people do.

## Decision

- `innertube::Stream::live` says a stream is a live happening now (`videoDetails.isLive`), with what its
  media playlist covers when resolved (`length`, one more request of about 60 ms, lives only) and its DVR
  window (`playlist_duration` of the manifest URL when `isLiveDvrEnabled`).
- The space starts a live `LIVE_DELAY` (15 s) before the end of its window. The shared player gets a DVR
  mode (`Source::dvr_window`) beside Twitch's live mode, which does not change: the seek bar over the
  window, pause in place, the live badge as a "back to live" button, quality changes that keep the
  distance to the edge ([playback-ux.md](../player/playback-ux.md#dvr-lives-youtube)).
- The chat is read by polling `live_chat/get_live_chat` (WEB, signed in when cookies are set so the
  rotated ones are kept) from the token in `next`'s `liveChatRenderer`, at most every 5 s (YouTube asks
  for up to 10 s), each page's messages spread over the time to the next as they were sent. It shows on
  `ui::chat`, as Twitch's, in the same layout. It shows the present even when the player is behind.
  Read-only, text messages and removals; Super Chats and memberships are skipped until a real response
  with them is saved. As on Twitch, mentions of the channel's handle (`WatchPage::channel_handle`,
  from the owner's link) are red, and messages mentioning the signed-in account's handle
  (`account/account_menu`, read once per session when a chat first opens) on a red tint (2026-11-07).

## Consequences

- Lives start within the 1–2 s rule and can be sought back an hour, with no proxy.
- A live prefetched on hover is used for 10 s only: its window may grow and the start would fall behind.
- Older than the window is out of reach; a finished live's chat replay is not shown.
- A chat poll every few seconds per open live.
- Revisit for Super Chats and memberships (once a response with them is saved), sending messages (signed
  in), or a longer DVR through a proxy if an hour is not enough.
