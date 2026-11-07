//! YouTube videos as the shared media cards: a click opens the watch page (a middle click, in a
//! new tab), resting on a card resolves its stream (so a click plays at once), and the save
//! buttons sit on the thumbnail. Playlists of a search or of the related list too: a click
//! opens their page.

use gpui::{AnyElement, App, Window};
use innertube::{PlaylistSummary, VideoItem};
use ui::card::{Hover, MediaCard, RowAction};

use crate::{Nav, Route};
use crate::{convert, history, save};

/// Opens the watch page of `video`.
fn open_video(video: &VideoItem, nav: &Nav) -> impl Fn(&mut Window, &mut App) + use<> {
  let nav = nav.clone();
  let route = Route::Watch(video.id.clone());
  let (id, resume) = (video.id.clone(), history::card_resume(video));
  move |window, cx| {
    // Where YouTube resumes it, as a click on youtube.com.
    history::offer(&id, resume, cx);
    nav.open(route.clone(), window, cx);
  }
}

/// Opens the watch page of `video` in a new tab, named after the video.
fn open_video_in_new_tab(video: &VideoItem, nav: &Nav) -> impl Fn(&mut Window, &mut App) + use<> {
  let nav = nav.clone();
  let route = Route::Watch(video.id.clone());
  let title = gpui::SharedString::from(video.title.clone());
  let (id, resume) = (video.id.clone(), history::card_resume(video));
  move |_, cx| {
    history::offer(&id, resume, cx);
    nav.open_in_new_tab(route.clone(), Some(title.clone()), cx);
  }
}

/// The card of `video` under `id` (which tells apart the same video in two lists): title,
/// thumbnail, duration or live badge, members-only badge, channel then views · date, and the stream prefetch on hover.
fn card(id: &str, hover: Hover, video: &VideoItem) -> MediaCard {
  let video_id = video.id.clone();
  MediaCard::new(
    id.to_owned(),
    hover,
    video.title.clone(),
    convert::picture(&video.thumbnails),
  )
  .lines([
    video.channel.clone().unwrap_or_default(),
    video.metadata.join(" · "),
  ])
  .badge(video.duration.clone())
  .badge_left(
    video
      .members_only
      .then(|| ui::i18n::t("Members only", "Réservé aux membres")),
  )
  .badge_left_icon(video.members_only.then_some(ui::assets::Icon::Star))
  .live(video.live)
  .progress(video.progress.map(|p| f32::from(p.percent) / 100.0))
  .on_hover(move |hovered, cx| crate::prefetch::hover(&video_id, hovered, cx))
}

/// Grid card, `width` wide, with the channel avatar and the save buttons.
pub fn video_card(
  video: &VideoItem,
  width: f32,
  nav: &Nav,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("card-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let t = hover.t;
  card(&id, hover, video)
    .avatar(Some(convert::picture(&video.avatar)))
    .overlay(save::watch_later_button(&id, &video.id, t, cx))
    .trailing(save::menu_button(&id, &video.id, t))
    .on_click(open_video(video, nav))
    .on_middle_click(open_video_in_new_tab(video, nav))
    .grid(width, window, cx)
}

/// Compact card (related videos), with the "Watch later" button.
pub fn compact_card(video: &VideoItem, nav: &Nav, window: &mut Window, cx: &mut App) -> AnyElement {
  let id = format!("related-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let t = hover.t;
  card(&id, hover, video)
    .overlay(save::watch_later_button(&id, &video.id, t, cx))
    .on_click(open_video(video, nav))
    .on_middle_click(open_video_in_new_tab(video, nav))
    .compact(window, cx)
}

/// List row (subscriptions in list mode, as YouTube showed them until about 2023), with the
/// channel avatar and the save buttons.
pub fn video_row(video: &VideoItem, nav: &Nav, window: &mut Window, cx: &mut App) -> AnyElement {
  let id = format!("row-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let t = hover.t;
  card(&id, hover, video)
    .avatar(Some(convert::picture(&video.avatar)))
    .overlay(save::watch_later_button(&id, &video.id, t, cx))
    .trailing(save::menu_button(&id, &video.id, t))
    .on_click(open_video(video, nav))
    .on_middle_click(open_video_in_new_tab(video, nav))
    .row(window, cx)
}

/// The card of `playlist` under `id`: its first video's picture on a stack, with its count after
/// the playlist icon, then its channel and details (`Playlist`); a click opens its page.
fn playlist_media_card(id: &str, playlist: &PlaylistSummary, nav: &Nav, hover: Hover) -> MediaCard {
  let route = Route::Playlist {
    id: playlist.id.clone(),
    title: playlist.title.clone(),
  };
  let title = gpui::SharedString::from(playlist.title.clone());
  let open = {
    let (nav, route) = (nav.clone(), route.clone());
    move |window: &mut Window, cx: &mut App| nav.open(route.clone(), window, cx)
  };
  let open_in_new_tab = {
    let nav = nav.clone();
    move |_: &mut Window, cx: &mut App| nav.open_in_new_tab(route.clone(), Some(title.clone()), cx)
  };
  MediaCard::new(
    id.to_owned(),
    hover,
    playlist.title.clone(),
    convert::picture(&playlist.thumbnails),
  )
  .lines(
    playlist
      .channel
      .iter()
      .cloned()
      .chain((!playlist.metadata.is_empty()).then(|| playlist.metadata.join(" · "))),
  )
  .badge(playlist.count.clone())
  .badge_icon(Some(ui::assets::Icon::Playlists))
  .stacked(true)
  // It opens the playlist's page: no play button.
  .play(false)
  .on_click(open)
  .on_middle_click(open_in_new_tab)
}

/// Compact playlist card (related list).
pub fn playlist_compact_card(
  playlist: &PlaylistSummary,
  nav: &Nav,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("related-playlist-{}", playlist.id);
  let hover = Hover::new(&id, window, cx);
  playlist_media_card(&id, playlist, nav, hover).compact(window, cx)
}

/// Playlist row (search results).
pub fn playlist_result_row(
  playlist: &PlaylistSummary,
  nav: &Nav,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("row-playlist-{}", playlist.id);
  let hover = Hover::new(&id, window, cx);
  playlist_media_card(&id, playlist, nav, hover).row(window, cx)
}

/// A video of a playlist (its page, or the queue next to the player). `id` must tell apart two
/// entries of the same video, which playlists allow. `current` marks the video playing.
#[allow(clippy::too_many_arguments)] // A row of a list: each argument is one aspect of it.
pub fn playlist_row(
  id: &str,
  number: usize,
  video: &VideoItem,
  thumb_width: f32,
  current: bool,
  on_click: impl Fn(&mut Window, &mut App) + 'static,
  on_remove: Option<RowAction>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let hover = Hover::new(id, window, cx);
  card(id, hover, video).on_click(on_click).numbered(
    number,
    thumb_width,
    current,
    on_remove,
    window,
    cx,
  )
}
