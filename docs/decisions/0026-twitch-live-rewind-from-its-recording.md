# 0026: A Twitch live's past from its recording

- Status: accepted
- Date: 2026-11-05

## Context

The user wants to go back in a Twitch live and come back to it, as on YouTube
([ADR 0025](0025-youtube-lives-from-dash.md)). A Twitch live's playlists cover about 25 s, so the past
has to come from somewhere else. Measured on 2026-11-05, anonymous (`archive_probe`,
`spike-twitch-play`):

- While a channel that keeps its VODs is live, Twitch records the broadcast as a VOD. GQL names it
  with `user.stream.archiveVideo { id createdAt }`, status `RECORDING`. Seven of eight top lives had
  one; the eighth channel keeps no VODs.
- The recording's media playlist is `#EXT-X-PLAYLIST-TYPE:EVENT` from segment 0, without an end tag,
  and grows. Its length is the time since its `createdAt`, within −2 to 9 s. That `createdAt` is 4 to
  14 s after the live's.
- Through `twitch::Proxy`'s VOD path (`spike-twitch-play` with `PEEBY_PROXY=1`), mpv plays it from any
  second: first frame 2.5 s after the click, token, usher and proxy included, from 1 h in and from 90 s
  before its end. There it keeps reloading the playlist from the proxy (5 times in 40 s): mpv's
  duration grows and no end comes.

- The recording trails the live by up to 25 s, so its last 30 s could not be reached from it, and
  each move between the live and the recording restarts mpv. The live's own playlist lists about
  28 s (14 segments of 2 s), but Twitch's CDN still serves a segment at least 3 minutes after it
  left that playlist, and no longer 5 minutes after (`segment_ttl`, two lives, 7 minutes each,
  1,200 requests). Through the proxy with a 2-minute window, mpv still plays 1 to 3 s behind the
  edge, its duration grows with the live, and a seek 60 s back plays again in about 1 s.

## Decision

- **The live keeps its last 2 minutes** (`twitch::LIVE_KEPT`, user's request, 2026-11-05): the
  proxy's playlist keeps 60 segments instead of 15, their URLs only (mpv still fetches them from
  the CDN). Only the quality playing keeps them: another quality's window starts when it is picked
  (the user's choice, rather than reading every quality's playlist). The live's `Source` gets
  `dvr_window` = 2 min, so a live with no recording still gets the DVR controls over them. Going
  back to the live, or a seek within 3 s of its edge, restarts mpv at the edge (`EDGE_SNAP`): a seek
  would stop short of its low latency. A quality change goes to the edge. `minute-watched` goes on
  while the live plays behind its edge (the user's choice): it is still the live.
- The recording is asked for on its own, anonymously (`Client::live_archive`, query `LiveArchive`:
  `user.stream.archiveVideo { id createdAt }`), with the live's playback and channel. With an
  auth-token, Twitch answers `server error` on that field (2026-11-05, every live tried), and a GQL
  error fails the whole answer. Asked inside the channel's query, it broke the signed-in viewer's
  header and found no recording on any live, as the user saw. Once the live plays and its recording
  is known, the watch page prepares it in the background (`WatchView::prepare_rewind`): a second
  proxy on the VOD and the viewer's position in it. It is asked again every minute until there is
  one (a live just started may have none yet).
- The shared player gets `Rewind { started_at, recording: Source }` (`PlayerView::set_rewind`), for a
  live:
  - **The bar.** The seek bar covers 0 to `now − started_at`. On the live, the playhead is as far
    behind that end as mpv is behind the live's edge (it slides back while paused). The timeline is
    computed, not mpv's.
  - **The live's window.** A seek within the live's kept minutes (what its mpv has read, less 5 s)
    is mpv's own seek in the live: no restart.
  - **Going back further.** A seek older than that plays the recording from there (a new mpv, about
    0.7 s, at the live's quality label), at least `REWIND_EDGE` (30 s) behind.
  - **Coming back.** From the recording, a seek within 30 s of the edge goes back to the live at its
    edge, then seeks that far behind once its duration is known; the live badge goes to the edge.
  - **Pause.** A pause resumes in place while the live's window still holds where it stopped, else
    in the recording from there.
  - **The page** follows each switch through `PlayerEvent::Rewound { rewound, at }`.
- While the recording plays, it is VOD playback (user's choice, 2026-11-05): its position is saved to
  the account like any VOD's (`progress::Saver`, ADR 0013), including where it was left on going back
  to the live. `minute-watched` is not sent: that is not the live on screen (ADR 0012).
- While the live plays, its position is saved to the recording as well (user's report, 2026-11-08:
  a live watched to the end left nothing on its VOD): the live's playhead is on the recording's
  timeline (`now − started_at`, less how far it is behind the edge), so opening the VOD later
  resumes where the viewer left the live, as on twitch.tv. Same saver, same moments (every 30 s
  while playing, on pause, at the end, when the page goes).
- The chat stays the live's.
- No recording (VODs off, or none yet), or one Twitch refuses (subscribers only): the live still
  has its last 2 minutes.

## Consequences

- One more token, usher request and proxy per live opened, unused unless the viewer goes back.
- The last 2 minutes play from the live itself, so the recording's delay no longer leaves a gap,
  and short seeks back (← 10 s) need no restart.
- Going back resumes the account's `viewingHistory` for that VOD. If the viewer later opens the VOD,
  it starts where they left the recording.
- Revisit if Twitch stops naming the recording on `stream` (anonymously) or makes in-progress VODs
  unplayable.
- Checked signed in (with the auth-token, read only): the recording found anonymously, its playback
  through `Client::play` (6 variants, `EVENT` playlist, no end tag).
- Checks: `cargo run -p twitch --example archive_probe -- (<login>… | top) [--save <dir>]` (each
  live's recording and how far its playlist trails), `PEEBY_START` for `spike-twitch-play` on a
  recording, `PEEBY_LIVE=rewind` for the player example (a generated recorded live that seeks back
  and to the edge on its own).
