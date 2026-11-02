# 0013: Watch progress kept on the user's account

- Status: accepted
- Date: 2026-11-02

## Context

Peeby should remember how far a video was watched, on YouTube and on Twitch VODs, so that it
resumes there. The progress can be kept in two places: a local store in `%APPDATA%\peeby`, or the
service's own account data, which youtube.com and twitch.tv use themselves. Jellyfin already
works the second way (progress reports, ADR 0009).

What was checked live on 2026-11-02, with the user's agreement for the writes:

- **YouTube, reading.** Signed-in cards carry the progress. A `lockupViewModel` (home,
  subscriptions, history, related videos) has
  `thumbnailOverlayProgressBarViewModel.startPercent` and, on its click,
  `watchEndpoint.startTimeSeconds`. A `videoRenderer` (search) or a `playlistVideoRenderer`
  (Watch later) has `thumbnailOverlayResumePlaybackRenderer.percentDurationWatched` and the same
  endpoint. Neither the WEB `player` response nor `next` gives the position of the video being
  opened.
- **YouTube, writing.** The stats pings of a signed-in WEB `player` response
  (`playbackTracking.videostatsPlaybackUrl`, then `videostatsWatchtimeUrl`) answer 204. The
  playback ping put the video at the top of the history. The `cmt` of the last watchtime ping
  became its `startTimeSeconds` in the subscriptions feed (3329 s). The protocol comes from yt-dlp
  2026.10.26's `_mark_watched` (`ver=2`, `cpn`, `cmt`, `el=detailpage`, `st`/`et`), plus what
  youtube.com's player adds (`state`, `final`, `c`, `cver`). The pings of the VISIONOS `player`
  response cannot be used: it is requested with an anonymous visitor, and its URLs lack
  `subscribed`/`uga`.
- **Twitch.** Raw GQL with the web auth-token reads `video.self.viewingHistory.position` and
  writes it with `updateUserViewedVideo(input: {userID, videoID, position, videoType: VOD})`.
  Unlike `claimCommunityPoints`, this mutation passed with no integrity token. Helix has no
  equivalent.

## Decision

Progress lives on the account, never in a local store:

- **YouTube** (`spaces::youtube::history`): when the watch page opens, it requests a signed WEB
  `player` in the background, for its stats URLs only. Then a playback ping when playback starts,
  and a watchtime ping of the segment watched every 30 s while playing, on pause, at the end
  (`final`) and when the page goes. A video opened from a card starts at the card's
  `startTimeSeconds`, as on youtube.com. Cards draw the red progress bar. Nothing is sent for
  lives or signed out. Pings go through `Client` so that rotated cookies are kept.
- **Twitch** (`spaces::twitch::progress`): when a VOD opens, `vod_viewing` is requested alongside
  the playback (about 30 ms), and the VOD starts at the saved position unless it is within 30 s of
  the end. The position is saved with `updateUserViewedVideo` every 30 s while playing, on pause,
  at the end and when the page goes. This needs the auth-token; without it, nothing is read or
  saved.
- Only the session keeps positions in memory (`history::Resume`): a video played earlier in the
  session resumes from its last reported position even from an older card, or with no card.

## Consequences

- Progress follows the user on every device, and YouTube's history (and therefore its
  recommendations) includes what is watched in Peeby. If the user has paused their YouTube
  history, YouTube keeps nothing, as for youtube.com.
- Playing a YouTube video now writes to the account (history, resume position). Live tests that
  play a video must ask the user first, as for Jellyfin and Twitch.
- A YouTube video opened without a card (a link, the command line, a notification) starts at 0
  unless it was played in this session: YouTube gives no position for it outside a card.
- Quitting the app can lose up to 30 s of position: the last ping goes when a page closes, and
  the app's exit may not close its pages first.
- One more request per YouTube video (the WEB `player`, about 0.2 s), off the path to the first
  frame.
- Revisit if YouTube stops honouring the pings without a PoToken, or if `updateUserViewedVideo`
  starts asking for an integrity token. The fallback would then be a local store, for that service
  only.
