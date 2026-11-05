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

## Decision

- The recording is asked for on its own, anonymously (`Client::live_archive`, query `LiveArchive`:
  `user.stream.archiveVideo { id createdAt }`), with the live's playback and channel. With an
  auth-token, Twitch answers `server error` on that field (2026-11-05, every live tried), and a GQL
  error fails the whole answer. Asked inside the channel's query, it broke the signed-in viewer's
  header and found no recording on any live, as the user saw. Once the live plays and its recording
  is known, the watch page prepares it in the background (`WatchView::prepare_rewind`): a second
  proxy on the VOD and the viewer's position in it. It is asked again every minute until there is
  one (a live just started may have none yet).
- The shared player gets `Rewind { started_at, recording: Source }` (`PlayerView::set_rewind`), for a
  live without DVR:
  - **The bar.** The seek bar covers 0 to `now − started_at`; on the live, the playhead is at the
    edge. The timeline is computed, not mpv's.
  - **Going back.** A seek back plays the recording from there (a new mpv, about 0.7 s, at the
    live's quality label), at least `REWIND_EDGE` (30 s) behind.
  - **Coming back.** A seek within 30 s of the edge, or a click on the live badge, goes back to the
    live at its edge.
  - **Pause.** A pause longer than a second resumes in the recording where it stopped, as a DVR live
    does; within 30 s of the edge, at the live.
  - **The page** follows each switch through `PlayerEvent::Rewound { rewound, at }`.
- While the recording plays, it is VOD playback (user's choice, 2026-11-05): its position is saved to
  the account like any VOD's (`progress::Saver`, ADR 0013), including where it was left on going back
  to the live. `minute-watched` is not sent: that is not the live on screen (ADR 0012).
- The chat stays the live's.
- No recording (VODs off, or none yet), or one Twitch refuses (subscribers only): the live keeps
  today's behavior, without a seek bar.

## Consequences

- One more token, usher request and proxy per live opened, unused unless the viewer goes back.
- The recording trails the live by a few seconds to a segment, so the last 30 s before the edge play
  from the live, not from the recording.
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
