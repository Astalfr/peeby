//! The user's playlists (a grid, from the sidebar) and a playlist's page: its header, "Play
//! all" and "Shuffle", and its videos, loading the next ones as the end comes into view.
//! Playing starts a [`Queue`] on a watch page. In playlists the user can edit, an "x" on each
//! video removes it.
//!
//! A playlist can be shown and played in reverse order (saved per playlist in the settings).
//! Reversing needs its end, so a reversed playlist loads all its pages first.

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, ElementId, FontWeight, ScrollHandle, SharedString, WeakEntity, Window,
  div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::{Disableable, IconName, Selectable, h_flex, v_flex};
use innertube::{Client, Error, Playlist, PlaylistSummary};

use crate::settings::Settings;
use crate::spaces::youtube::cards::playlist_row;
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::queue::Queue;
use crate::spaces::youtube::save::{self, EntryRemoved};
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::assets::Icon;
use ui::card::WIDE;
use ui::card::skeleton_card;
use ui::i18n::{Lang, lang, t};
use ui::images::Images;
use ui::picture::image_fill;
use ui::widgets::{loader, loading_more, near_scroll_end, notice, page_title};
use ui::{motion, runtime, tf, theme};

/// Width of a playlist card, and of a playlist page's cover.
const CARD_WIDTH: f32 = 320.;
/// Width of a video's thumbnail on a playlist page.
const ROW_THUMB: f32 = 160.;

enum State<T> {
  Loading,
  Loaded(T),
  /// Signed out, or YouTube no longer accepts the session.
  SignedOut,
  Failed(SharedString),
}

impl<T> State<T> {
  fn from_error(error: &Error) -> Self {
    match error {
      Error::SignInRequired | Error::NotLoggedIn => Self::SignedOut,
      e => Self::Failed(error_text(e).into()),
    }
  }
}

/// The notice of a page that needs a session, with a link to the account page.
fn signed_out(nav: &WeakEntity<YouTubeSpace>) -> AnyElement {
  let nav = nav.clone();
  notice(
    Icon::Account,
    t(
      "Sign in to see your playlists",
      "Connectez-vous pour voir vos playlists",
    ),
    t(
      "Import the cookies of a YouTube session from the Account page.",
      "Importez les cookies d'une session YouTube depuis la page Compte.",
    ),
  )
  .child(
    h_flex().pt_2().child(
      Button::new("to-account")
        .primary()
        .label(t("Import cookies", "Importer des cookies"))
        .on_click(move |_, _, cx| {
          nav.update(cx, |app, cx| app.open_settings(cx)).ok();
        }),
    ),
  )
  .into_any_element()
}

fn failed(title: &'static str, error: SharedString) -> gpui::Div {
  notice(IconName::TriangleAlert, title, error)
}

/// "Loading the whole playlist to reverse it… 12 videos", singular for one (and for zero in
/// French).
fn gathering_text(count: usize) -> String {
  match (lang(), count) {
    (Lang::En, 1) => "Loading the whole playlist to reverse it… 1 video".to_owned(),
    (Lang::Fr, 0 | 1) => {
      format!("Chargement de toute la playlist pour l'inverser… {count} vidéo")
    }
    _ => tf!(
      "Loading the whole playlist to reverse it… {count} videos",
      "Chargement de toute la playlist pour l'inverser… {count} vidéos"
    ),
  }
}

// --- The playlists ---------------------------------------------------------------------------

pub struct PlaylistsView {
  client: Client,
  nav: WeakEntity<YouTubeSpace>,
  state: State<Vec<PlaylistSummary>>,
}

impl PlaylistsView {
  pub fn new(client: Client, nav: WeakEntity<YouTubeSpace>, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let mut view = Self {
      client,
      nav,
      state: State::Loading,
    };
    view.load(cx);
    view
  }

  fn load(&mut self, cx: &mut Context<Self>) {
    if !self.client.signed_in() {
      self.state = State::SignedOut;
      return;
    }
    self.state = State::Loading;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.playlists().await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(playlists)) => {
              tracing::info!("{} playlists", playlists.len());
              this.state = State::Loaded(playlists);
            }
            Some(Err(e)) => {
              tracing::warn!("playlists: {e}");
              this.state = State::from_error(&e);
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }
}

/// A playlist: its first video's thumbnail over two thin layers (a stack), the count badge,
/// then the title and details. On hover it rises, a red ring fades in and the title turns red.
fn playlist_card(
  playlist: &PlaylistSummary,
  nav: &WeakEntity<YouTubeSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("playlist-{}", playlist.id);
  let hovered = motion::hover_state(SharedString::from(format!("{id}-hover")), window, cx);
  let is_hovered = *hovered.read(cx);
  let t = motion::tween(
    SharedString::from(format!("{id}-lift")),
    0.,
    if is_hovered { 1. } else { 0. },
    theme::FAST,
    window,
    cx,
  );
  let thumb = image_fill(
    &convert::picture(&playlist.thumbnails),
    CARD_WIDTH,
    WIDE,
    theme::RADIUS_LG,
    window,
    cx,
  );
  let route = Route::Playlist {
    id: playlist.id.clone(),
    title: playlist.title.clone(),
  };
  let nav = nav.clone();
  let height = CARD_WIDTH * 9. / 16.;
  // Layers behind the thumbnail: narrower, peeking out above it.
  let layer = |inset: f32, top: f32, opacity: f32| {
    div()
      .absolute()
      .top(px(top))
      .left(px(inset))
      .right(px(inset))
      .h(px(12.))
      .rounded_t(theme::RADIUS)
      .bg(theme::muted().opacity(opacity))
  };
  div()
    .id(ElementId::Name(id.into()))
    .w(px(CARD_WIDTH))
    .flex_none()
    .cursor_pointer()
    .on_hover(move |hover, window, cx| {
      hovered.update(cx, |h, _| *h = *hover);
      window.refresh();
    })
    .on_click(move |_, window, cx| {
      nav
        .update(cx, |app, cx| app.open(route.clone(), window, cx))
        .ok();
    })
    // The hovered box stays put and only its content rises (a moving box could loop).
    .child(
      v_flex()
        .w_full()
        .relative()
        .top(px(-4. * t))
        .child(
          div()
            .relative()
            .w_full()
            .h(px(height + 8.))
            .child(layer(16., 0., 0.25))
            .child(layer(8., 4., 0.45))
            .child(
              div()
                .absolute()
                .top(px(8.))
                .left_0()
                .right_0()
                .h(px(height))
                .rounded(theme::RADIUS_LG)
                .when(t > 0.01, |el| el.shadow_lg())
                .child(thumb)
                .children(playlist.count.clone().map(|count| {
                  h_flex()
                    .absolute()
                    .bottom(px(8.))
                    .right(px(8.))
                    .px(px(6.))
                    .py(px(1.))
                    .gap_1()
                    .items_center()
                    .rounded(px(4.))
                    .bg(gpui::black().opacity(0.8))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child(gpui_component::Icon::new(Icon::Playlists).size(px(12.)))
                    .child(count)
                }))
                .child(
                  div()
                    .absolute()
                    .inset_0()
                    .rounded(theme::RADIUS_LG)
                    .border_2()
                    .border_color(theme::accent().opacity(t)),
                ),
            ),
        )
        .child(
          v_flex()
            .mt_3()
            .gap_0p5()
            .child(
              div()
                .line_height(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .text_color(if is_hovered {
                  theme::accent_hover()
                } else {
                  theme::foreground()
                })
                .child(playlist.title.clone()),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme::muted())
                .truncate()
                .child(playlist.metadata.join(" · ")),
            ),
        ),
    )
    .into_any_element()
}

impl Render for PlaylistsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let content = match &self.state {
      // Replaced by the skeleton below.
      State::Loading => div().into_any_element(),
      State::SignedOut => signed_out(&self.nav),
      State::Failed(e) => failed(
        t(
          "Cannot load the playlists",
          "Impossible de charger les playlists",
        ),
        e.clone(),
      )
      .child(
        h_flex().pt_2().child(
          Button::new("retry")
            .primary()
            .label(t("Retry", "Réessayer"))
            .on_click(cx.listener(|this, _, _, cx| {
              this.load(cx);
              cx.notify();
            })),
        ),
      )
      .into_any_element(),
      State::Loaded(playlists) if playlists.is_empty() => div()
        .text_color(theme::muted())
        .child(t("No playlists.", "Aucune playlist."))
        .into_any_element(),
      State::Loaded(playlists) => {
        let cards: Vec<_> = playlists
          .iter()
          .map(|p| playlist_card(p, &self.nav, window, cx))
          .collect();
        h_flex()
          .flex_wrap()
          .items_start()
          .gap_x_4()
          .gap_y_8()
          .children(cards)
          .into_any_element()
      }
    };
    let skeleton = h_flex()
      .flex_wrap()
      .gap_x_4()
      .gap_y_8()
      .children((0..6).map(|_| skeleton_card(CARD_WIDTH)))
      .into_any_element();
    let body = motion::crossfade(
      SharedString::from(format!("playlists-load-{}", cx.entity_id())),
      matches!(self.state, State::Loading),
      skeleton,
      content,
      window,
      cx,
    );
    v_flex()
      .id("playlists")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .px_8()
          .py_6()
          .gap_6()
          .child(page_title(t("Playlists", "Playlists")))
          .child(body),
      )
  }
}

// --- One playlist ----------------------------------------------------------------------------

pub struct PlaylistView {
  client: Client,
  id: String,
  /// Title from the playlists page, shown until the playlist has loaded.
  title: String,
  nav: WeakEntity<YouTubeSpace>,
  state: State<Playlist>,
  loading_more: bool,
  /// The last next-page request failed: no automatic retry, the button retries.
  more_failed: bool,
  scroll: ScrollHandle,
}

impl PlaylistView {
  pub fn new(
    client: Client,
    id: String,
    title: String,
    nav: WeakEntity<YouTubeSpace>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    // An entry removed here or from a queue playing this playlist leaves the list.
    let edits = save::edits(cx);
    cx.subscribe(&edits, |this, _, removed: &EntryRemoved, cx| {
      if removed.playlist_id != this.id {
        return;
      }
      if let State::Loaded(playlist) = &mut this.state {
        playlist
          .videos
          .retain(|v| v.set_video_id.as_deref() != Some(removed.set_video_id.as_str()));
        cx.notify();
      }
    })
    .detach();
    let mut view = Self {
      client,
      id,
      title,
      nav,
      state: State::Loading,
      loading_more: false,
      more_failed: false,
      scroll: ScrollHandle::new(),
    };
    view.load(cx);
    view
  }

  fn load(&mut self, cx: &mut Context<Self>) {
    self.state = State::Loading;
    let (client, id) = (self.client.clone(), self.id.clone());
    let fetch = runtime::spawn(cx, async move { client.playlist(&id).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(playlist)) => {
              tracing::info!(
                "playlist: {} videos · more: {}",
                playlist.videos.len(),
                playlist.continuation.is_some()
              );
              this.state = State::Loaded(playlist);
            }
            Some(Err(e)) => {
              tracing::warn!("playlist: {e}");
              this.state = State::from_error(&e);
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Whether this playlist is shown last video first.
  fn reversed(&self, cx: &App) -> bool {
    Settings::get(cx).reversed_playlists.contains(&self.id)
  }

  fn toggle_reversed(&mut self, cx: &mut Context<Self>) {
    let id = self.id.clone();
    Settings::update(cx, |s| {
      if let Some(i) = s.reversed_playlists.iter().position(|p| *p == id) {
        s.reversed_playlists.remove(i);
      } else {
        s.reversed_playlists.push(id);
      }
    });
    self.scroll.set_offset(gpui::point(px(0.), px(0.)));
    cx.notify();
  }

  /// Next videos once the end of the list is within about a screen, or all of them, one page
  /// after the other, when the playlist is reversed.
  fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
    let State::Loaded(playlist) = &self.state else {
      return;
    };
    let wanted = self.reversed(cx) || near_scroll_end(&self.scroll);
    if self.loading_more || self.more_failed || !wanted {
      return;
    }
    let Some(token) = playlist.continuation.clone() else {
      return;
    };
    self.loading_more = true;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.playlist_more(&token).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.loading_more = false;
          let State::Loaded(playlist) = &mut this.state else {
            return;
          };
          match result {
            Some(Ok(page)) => {
              playlist.videos.extend(page.items);
              playlist.continuation = page.continuation;
            }
            Some(Err(e)) => {
              tracing::warn!("playlist, next videos: {e}");
              this.more_failed = true;
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Plays the playlist on a watch page from `start`, a place in the order shown (a random
  /// video when shuffled without one). The queue takes the videos loaded so far and loads the
  /// next ones itself; a reversed playlist is complete already.
  fn play(&self, start: Option<usize>, shuffle: bool, window: &mut Window, cx: &mut App) {
    let State::Loaded(playlist) = &self.state else {
      return;
    };
    if playlist.videos.is_empty() {
      return;
    }
    let (videos, continuation) = if self.reversed(cx) {
      (playlist.videos.iter().rev().cloned().collect(), None)
    } else {
      (playlist.videos.clone(), playlist.continuation.clone())
    };
    let queue = cx.new(|cx| {
      Queue::new(
        self.client.clone(),
        playlist.id.clone(),
        playlist.title.clone(),
        videos,
        continuation,
        start,
        shuffle,
        cx,
      )
    });
    self
      .nav
      .update(cx, |app, cx| app.play_queue(queue, false, window, cx))
      .ok();
  }

  fn header(&self, playlist: &Playlist, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let cover = image_fill(
      &convert::picture(&playlist.thumbnails),
      CARD_WIDTH,
      WIDE,
      theme::RADIUS_LG,
      window,
      cx,
    );
    let details = [playlist.owner.clone(), playlist.count.clone()]
      .into_iter()
      .flatten()
      .collect::<Vec<_>>()
      .join(" · ");
    let reversed = self.reversed(cx);
    // Reversed and still loading: nothing to play in that order yet.
    let empty = playlist.videos.is_empty() || (reversed && playlist.continuation.is_some());
    v_flex()
      .w(px(CARD_WIDTH))
      .flex_none()
      .gap_4()
      .child(div().w_full().h(px(CARD_WIDTH * 9. / 16.)).child(cover))
      .child(
        div()
          .text_2xl()
          .font_weight(FontWeight::BOLD)
          .line_clamp(3)
          .child(playlist.title.clone()),
      )
      .when(!details.is_empty(), |el| {
        el.child(div().text_sm().text_color(theme::muted()).child(details))
      })
      .child(
        h_flex()
          .gap_2()
          .child(
            Button::new("play-all")
              .primary()
              .icon(Icon::Play)
              .label(t("Play all", "Tout lire"))
              .disabled(empty)
              .on_click(cx.listener(|this, _, window, cx| this.play(Some(0), false, window, cx))),
          )
          .child(
            Button::new("shuffle")
              .icon(Icon::Shuffle)
              .label(t("Shuffle", "Aléatoire"))
              .disabled(empty)
              .on_click(cx.listener(|this, _, window, cx| this.play(None, true, window, cx))),
          )
          .child(
            Button::new("reverse")
              .ghost()
              .icon(Icon::Reverse)
              .selected(reversed)
              .tooltip(if reversed {
                t("Restore the order", "Remettre dans l'ordre")
              } else {
                t("Reverse the order", "Inverser l'ordre")
              })
              .on_click(cx.listener(|this, _, _, cx| this.toggle_reversed(cx))),
          ),
      )
      .into_any_element()
  }
}

fn skeleton_page() -> impl IntoElement {
  h_flex()
    .gap_8()
    .items_start()
    .child(
      v_flex()
        .w(px(CARD_WIDTH))
        .flex_none()
        .gap_4()
        .child(
          Skeleton::new()
            .w_full()
            .h(px(CARD_WIDTH * 9. / 16.))
            .rounded(theme::RADIUS_LG),
        )
        .child(Skeleton::new().w(px(240.)).h(px(28.)))
        .child(Skeleton::new().w(px(160.)).h(px(14.))),
    )
    .child(v_flex().flex_1().gap_2().children((0..8).map(|_| {
      h_flex()
        .px_2()
        .py_1p5()
        .gap_3()
        .items_center()
        .child(Skeleton::new().w(px(28.)).h(px(12.)))
        .child(
          Skeleton::new()
            .w(px(ROW_THUMB))
            .h(px(ROW_THUMB * 9. / 16.))
            .rounded(theme::RADIUS),
        )
        .child(
          v_flex()
            .flex_1()
            .gap_2()
            .child(Skeleton::new().w(px(320.)).h(px(16.)))
            .child(Skeleton::new().w(px(180.)).h(px(12.))),
        )
    })))
}

impl Render for PlaylistView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Offsets from the last layout: enough to start the next page before the end is reached.
    self.load_more_if_near_end(cx);
    let content = match &self.state {
      // Replaced by the skeleton below.
      State::Loading => div().into_any_element(),
      State::SignedOut => signed_out(&self.nav),
      State::Failed(e) => failed(
        t(
          "Cannot load the playlist",
          "Impossible de charger la playlist",
        ),
        e.clone(),
      )
      .child(
        h_flex().pt_2().child(
          Button::new("retry")
            .primary()
            .label(t("Retry", "Réessayer"))
            .on_click(cx.listener(|this, _, _, cx| {
              this.load(cx);
              cx.notify();
            })),
        ),
      )
      .into_any_element(),
      State::Loaded(playlist) => {
        let header = self.header(playlist, window, cx);
        let view = cx.weak_entity();
        let reversed = self.reversed(cx);
        // Reversed, the list waits for the last page: its end comes first.
        let gathering = reversed && playlist.continuation.is_some();
        let count = playlist.videos.len();
        // Places in the playlist, in the order shown.
        let order: Vec<usize> = if gathering {
          Vec::new()
        } else if reversed {
          (0..count).rev().collect()
        } else {
          (0..count).collect()
        };
        // Ids carry the place: a playlist may hold the same video twice. Numbers stay the
        // playlist's own, so a reversed list counts down.
        let rows: Vec<_> = order
          .into_iter()
          .enumerate()
          .map(|(shown, i)| {
            let video = &playlist.videos[i];
            let view = view.clone();
            let remove = video
              .set_video_id
              .as_deref()
              .map(|entry| save::removal(&playlist.id, &playlist.title, entry, &video.id, cx));
            playlist_row(
              &format!("pl-{i}-{}", video.id),
              i + 1,
              video,
              ROW_THUMB,
              false,
              move |window, cx| {
                view
                  .update(cx, |this, cx| this.play(Some(shown), false, window, cx))
                  .ok();
              },
              remove,
              window,
              cx,
            )
          })
          .collect();
        let more = if self.more_failed && playlist.continuation.is_some() {
          Some(
            Button::new("more")
              .primary()
              .label(t("Retry", "Réessayer"))
              .on_click(cx.listener(|this, _, _, cx| {
                this.more_failed = false;
                this.load_more_if_near_end(cx);
                cx.notify();
              }))
              .into_any_element(),
          )
        } else if gathering {
          Some(
            h_flex()
              .gap_3()
              .items_center()
              .text_sm()
              .text_color(theme::muted())
              .child(loader(16., theme::accent()))
              .child(gathering_text(count))
              .into_any_element(),
          )
        } else {
          self.loading_more.then(|| loading_more().into_any_element())
        };
        let empty = rows.is_empty() && !gathering;
        h_flex()
          .gap_8()
          .items_start()
          .child(header)
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .max_w(px(960.))
              .gap_1()
              .when(empty, |el| {
                el.child(div().text_color(theme::muted()).child(t(
                  "This playlist has no playable videos.",
                  "Cette playlist ne contient aucune vidéo lisible.",
                )))
              })
              .children(rows)
              .children(more.map(|b| h_flex().pt_4().justify_center().child(b))),
          )
          .into_any_element()
      }
    };
    let body = motion::crossfade(
      SharedString::from(format!("playlist-load-{}", cx.entity_id())),
      matches!(self.state, State::Loading),
      skeleton_page().into_any_element(),
      content,
      window,
      cx,
    );
    let title = match &self.state {
      State::Loaded(_) => None,
      _ => Some(page_title(self.title.clone())),
    };
    div()
      .size_full()
      .relative()
      .vertical_scrollbar(&self.scroll)
      .child(
        v_flex()
          .id("playlist")
          .size_full()
          .overflow_y_scroll()
          .track_scroll(&self.scroll)
          // Wheel scrolling does not re-render by itself: check the end on every scroll.
          .on_scroll(cx.listener(|this, _, _, cx| {
            this.load_more_if_near_end(cx);
            cx.notify();
          }))
          .child(v_flex().px_8().py_6().gap_6().children(title).child(body)),
      )
  }
}
