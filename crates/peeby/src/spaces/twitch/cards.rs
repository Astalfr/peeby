//! Twitch's data as the shared cards: a live stream (16:9 preview, avatar, channel, category,
//! viewers), a category (its 3:4 box art), a channel (avatar). Image URLs are Helix templates
//! filled at the size drawn.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{AnyElement, App, FontWeight, SharedString, WeakEntity, Window, div, px};
use gpui_component::{h_flex, v_flex};
use twitch::helix::{self, Category, Helix, Stream};
use ui::card::{Hover, MediaCard};
use ui::picture::{self, Picture};
use ui::theme;

use super::{Route, TwitchSpace};

/// Least width of a stream card: the cards widen to fill their rows.
pub const STREAM_WIDTH: f32 = 300.;
/// The pages' side padding (`p_8`), both sides.
const PAGE_PADDING: f32 = 64.;
/// Width of a category's box art.
pub const CATEGORY_WIDTH: f32 = 150.;
/// Twitch's box art is 3:4.
pub const BOX_ART: f32 = 3. / 4.;
/// Space between cards.
pub const GAP: f32 = 20.;

/// Avatars by user id.
pub type Avatars = HashMap<String, String>;

/// The avatars of `ids` (at most 100), by id; an error leaves them out.
pub async fn avatars(helix: &Helix, ids: Vec<String>) -> Avatars {
  match helix.users(&ids).await {
    Ok(users) => users
      .into_iter()
      .map(|u| (u.id, u.profile_image_url))
      .collect(),
    Err(e) => {
      tracing::warn!("Twitch avatars: {e}");
      HashMap::new()
    }
  }
}

/// A picture for a Helix image template.
pub fn template(url: &str) -> Picture {
  let url = url.to_owned();
  Picture::sized(move |w, h| helix::sized(&url, w, h))
}

/// A live preview: Twitch's CDN keeps it for a while, so a five-minute stamp gets a fresh one
/// regularly while the image store still caches it in between.
fn preview(url: &str) -> Picture {
  let stamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.as_secs() / 300);
  let url = url.to_owned();
  Picture::sized(move |w, h| format!("{}?t={stamp}", helix::sized(&url, w, h)))
}

/// `41 770` spectators, short: `41,8 k`.
pub fn compact(n: u64) -> String {
  match n {
    0..1_000 => n.to_string(),
    1_000..1_000_000 => format!("{:.1} k", n as f64 / 1e3).replace('.', ","),
    _ => format!("{:.1} M", n as f64 / 1e6).replace('.', ","),
  }
}

/// The width of a stream card filling the page's rows (the window less the sidebar and the
/// page's padding).
pub fn stream_width(window: &Window, cx: &App) -> f32 {
  let collapsed = crate::settings::Settings::get(cx).sidebar_collapsed;
  let available =
    f32::from(window.viewport_size().width - theme::sidebar_width(collapsed)) - PAGE_PADDING - 8.;
  let columns = ((available + GAP) / (STREAM_WIDTH + GAP)).floor().max(1.);
  ((available - GAP * (columns - 1.)) / columns).max(STREAM_WIDTH)
}

/// A live stream's grid card, `width` wide; a click opens it.
pub fn stream_card(
  stream: &Stream,
  width: f32,
  avatar: Option<&String>,
  nav: &WeakEntity<TwitchSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("twitch-stream-{}", stream.id);
  let hover = Hover::new(&id, window, cx);
  let login = stream.user_login.clone();
  let nav = nav.clone();
  MediaCard::new(
    id,
    hover,
    stream.title.clone(),
    preview(&stream.thumbnail_url),
  )
  .avatar(avatar.map(|url| template(url)))
  .lines([stream.user_name.clone(), stream.game_name.clone()])
  .badge(Some(format!(
    "{} spectateurs",
    compact(stream.viewer_count)
  )))
  .on_click(move |_, cx| {
    nav
      .update(cx, |space, cx| space.open(Route::Watch(login.clone()), cx))
      .ok();
  })
  .grid(width, window, cx)
}

/// A category's poster; a click opens its streams.
pub fn category_card(
  category: &Category,
  nav: &WeakEntity<TwitchSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("twitch-category-{}", category.id);
  let hover = Hover::new(&id, window, cx);
  let route = Route::Category {
    id: category.id.clone(),
    name: category.name.clone(),
  };
  let nav = nav.clone();
  MediaCard::new(
    id,
    hover,
    category.name.clone(),
    template(&category.box_art_url),
  )
  .play(false)
  .on_click(move |_, cx| {
    nav
      .update(cx, |space, cx| space.open(route.clone(), cx))
      .ok();
  })
  .poster(CATEGORY_WIDTH, BOX_ART, window, cx)
}

/// A channel to show as an avatar and its name.
pub struct Chip<'a> {
  /// Login, opened on click.
  pub login: &'a str,
  /// Shown name.
  pub name: &'a str,
  /// A muted line (category).
  pub detail: Option<String>,
  /// Avatar URL.
  pub avatar: Option<&'a String>,
  /// On air now.
  pub live: bool,
}

/// A channel as an avatar and its name (offline followed channels, search results); a click opens
/// its live.
pub fn channel_chip(
  chip: Chip<'_>,
  nav: &WeakEntity<TwitchSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let Chip {
    login,
    name,
    detail,
    avatar,
    live,
  } = chip;
  let login = login.to_owned();
  let nav = nav.clone();
  let initial: SharedString = name
    .chars()
    .next()
    .unwrap_or('?')
    .to_uppercase()
    .to_string()
    .into();
  let face = match avatar {
    Some(url) => picture::avatar(&template(url), 44., window, cx),
    None => div()
      .size(px(44.))
      .rounded_full()
      .bg(theme::accent_soft())
      .flex()
      .items_center()
      .justify_center()
      .child(initial)
      .into_any_element(),
  };
  h_flex()
    .id(SharedString::from(format!("twitch-channel-{login}")))
    .w(px(240.))
    .gap_3()
    .p_2()
    .rounded(theme::RADIUS)
    .cursor_pointer()
    .hover(|el| el.bg(theme::surface_hover()))
    .on_click(move |_, _, cx| {
      nav
        .update(cx, |space, cx| space.open(Route::Watch(login.clone()), cx))
        .ok();
    })
    .child(div().flex_none().child(face))
    .child(
      v_flex()
        .min_w_0()
        .child(
          div()
            .font_weight(FontWeight::MEDIUM)
            .truncate()
            .child(name.to_owned()),
        )
        .children(detail.map(|d| {
          div()
            .text_xs()
            .text_color(theme::muted())
            .truncate()
            .child(d)
        }))
        .when(live, |el| {
          el.child(
            div()
              .text_xs()
              .text_color(theme::accent_hover())
              .child("En direct"),
          )
        }),
    )
    .into_any_element()
}

/// A titled block of the page.
pub fn section(title: &str, content: impl IntoElement) -> impl IntoElement {
  v_flex()
    .gap_4()
    .child(
      div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.to_owned()),
    )
    .child(content)
}

/// Cards wrapping in rows.
pub fn wrap(cards: Vec<AnyElement>) -> impl IntoElement {
  h_flex()
    .flex_wrap()
    .gap(px(GAP))
    .items_start()
    .children(cards)
}

/// A muted message in place of a list.
pub fn note(text: impl Into<SharedString>) -> impl IntoElement {
  div()
    .text_sm()
    .text_color(theme::muted())
    .child(text.into())
}
