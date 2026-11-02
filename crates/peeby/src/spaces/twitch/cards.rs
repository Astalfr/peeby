//! Twitch's data as the shared cards: a live stream (16:9 preview, avatar, channel, category,
//! viewers), a category (its 3:4 box art), a channel (avatar). Image URLs are Helix templates
//! filled at the size drawn.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{AnyElement, App, FontWeight, SharedString, WeakEntity, Window, div, px};
use gpui_component::{h_flex, v_flex};
use twitch::helix::{self, Category, Clip, Helix, Stream, Video};
use ui::card::{Hover, MediaCard};
use ui::i18n::{Lang, lang, t};
use ui::picture::{self, Picture};
use ui::{tf, theme};

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

/// `41770` viewers, short: `41.8K` (French: `41,8 k`).
pub fn compact(n: u64) -> String {
  let (value, en, fr) = match n {
    0..1_000 => return n.to_string(),
    1_000..1_000_000 => (n as f64 / 1e3, "K", " k"),
    _ => (n as f64 / 1e6, "M", " M"),
  };
  let value = format!("{value:.1}");
  match lang() {
    Lang::En => format!("{value}{en}"),
    Lang::Fr => format!("{}{fr}", value.replace('.', ",")),
  }
}

/// Whether `n` takes the plural in the interface's language: in English everything but 1, in
/// French 2 and more (0 and 1 are singular).
pub fn plural(n: u64) -> bool {
  lang().pick(n != 1, n > 1)
}

/// `1 viewer`, `41.8K viewers`: the count shortened.
pub fn compact_viewers(n: u64) -> String {
  let count = compact(n);
  if plural(n) {
    tf!("{count} viewers", "{count} spectateurs")
  } else {
    tf!("{count} viewer", "{count} spectateur")
  }
}

/// `1 viewer`, `80,308 viewers`: the count in full.
pub fn viewers(n: u64) -> String {
  let count = grouped(n);
  if plural(n) {
    tf!("{count} viewers", "{count} spectateurs")
  } else {
    tf!("{count} viewer", "{count} spectateur")
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
  .badge(Some(compact_viewers(stream.viewer_count)))
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
/// its page.
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
        .update(cx, |space, cx| {
          space.open(Route::Channel(login.clone()), cx)
        })
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
              .child(t("Live", "En direct")),
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

/// `80,308` (French grouping: `80 308`, a narrow no-break space).
pub fn grouped(n: u64) -> String {
  let separator = lang().pick(',', '\u{202f}');
  let digits = n.to_string();
  let mut out = String::new();
  for (i, d) in digits.chars().enumerate() {
    if i > 0 && (digits.len() - i).is_multiple_of(3) {
      out.push(separator);
    }
    out.push(d);
  }
  out
}

/// `12,345 views`.
pub fn views(n: u64) -> String {
  let count = grouped(n);
  if plural(n) {
    tf!("{count} views", "{count} vues")
  } else {
    tf!("{count} view", "{count} vue")
  }
}

/// A month's short name, `month` from 1 to 12.
fn month_name(month: i64) -> &'static str {
  match month {
    1 => t("Jan", "janv."),
    2 => t("Feb", "févr."),
    3 => t("Mar", "mars"),
    4 => t("Apr", "avr."),
    5 => t("May", "mai"),
    6 => t("Jun", "juin"),
    7 => t("Jul", "juil."),
    8 => t("Aug", "août"),
    9 => t("Sep", "sept."),
    10 => t("Oct", "oct."),
    11 => t("Nov", "nov."),
    _ => t("Dec", "déc."),
  }
}

/// `Nov 1, 2026` (French: `1 nov. 2026`) from milliseconds since the epoch; empty when unknown.
pub fn date(millis: Option<i64>) -> String {
  let Some(millis) = millis else {
    return String::new();
  };
  // Civil date from days since the epoch (Howard Hinnant's algorithm).
  let days = millis.div_euclid(86_400_000) + 719_468;
  let era = days.div_euclid(146_097);
  let doe = days - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = doy - (153 * mp + 2) / 5 + 1;
  let month = if mp < 10 { mp + 3 } else { mp - 9 };
  let year = yoe + era * 400 + i64::from(month <= 2);
  let month = month_name(month);
  tf!("{month} {day}, {year}", "{day} {month} {year}")
}

/// How far `position` seconds is into a VOD `length` seconds long (0 to 1), for its card.
fn progress(position: u32, length: u64) -> Option<f32> {
  (length > 0 && position > 0).then(|| (f64::from(position) / length as f64).min(1.) as f32)
}

/// A VOD's grid card, `width` wide, with a progress bar at `position` (seconds, where the viewer
/// stopped); a click plays it.
pub fn video_card(
  video: &Video,
  position: Option<u32>,
  width: f32,
  nav: &WeakEntity<TwitchSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("twitch-video-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let route = Route::Vod(video.id.clone());
  let nav = nav.clone();
  MediaCard::new(
    id,
    hover,
    video.title.clone(),
    template(&video.thumbnail_url),
  )
  .lines([format!(
    "{} · {}",
    views(video.view_count),
    date(video.created_at)
  )])
  .badge(Some(ui::player::format_time(video.duration as f64)))
  .progress(position.and_then(|p| progress(p, video.duration)))
  .on_click(move |_, cx| {
    nav
      .update(cx, |space, cx| space.open(route.clone(), cx))
      .ok();
  })
  .grid(width, window, cx)
}

/// A clip's grid card, `width` wide; a click plays it.
pub fn clip_card(
  clip: &Clip,
  width: f32,
  nav: &WeakEntity<TwitchSpace>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("twitch-clip-{}", clip.id);
  let hover = Hover::new(&id, window, cx);
  let route = Route::Clip(clip.id.clone());
  let nav = nav.clone();
  let thumbnail = clip.thumbnail_url.clone();
  MediaCard::new(
    id,
    hover,
    clip.title.clone(),
    Picture::sized(move |_, _| thumbnail.clone()),
  )
  .lines([
    tf!("Clipped by {}", "Clip de {}", clip.creator_name),
    format!("{} · {}", views(clip.view_count), date(clip.created_at)),
  ])
  .badge(Some(ui::player::format_time(clip.duration)))
  .on_click(move |_, cx| {
    nav
      .update(cx, |space, cx| space.open(route.clone(), cx))
      .ok();
  })
  .grid(width, window, cx)
}

#[cfg(test)]
mod tests {
  use super::{compact, date, grouped, progress, viewers, views};

  #[test]
  fn formats() {
    assert_eq!(grouped(80_308), "80,308");
    assert_eq!(grouped(1_234_567), "1,234,567");
    assert_eq!(compact(41_770), "41.8K");
    assert_eq!(compact(2_500_000), "2.5M");
    assert_eq!(compact(999), "999");
    assert_eq!(viewers(1), "1 viewer");
    assert_eq!(viewers(0), "0 viewers");
    assert_eq!(views(1_000), "1,000 views");
    assert_eq!(date(Some(1_793_563_851_000)), "Nov 1, 2026");
    assert_eq!(date(Some(0)), "Jan 1, 1970");
    assert_eq!(date(None), "");
  }

  #[test]
  fn vod_progress() {
    assert_eq!(progress(2940, 29_400), Some(0.1));
    // Past the end (a VOD cut after it was watched): a full bar.
    assert_eq!(progress(500, 400), Some(1.));
    assert_eq!(progress(0, 400), None);
    assert_eq!(progress(10, 0), None);
  }
}
