//! Shared UI pieces: remote images, video cards, their skeletons and page notices. The look
//! and the motion follow docs/design.md.

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ElementId, Entity, FontWeight, ImageSource, ObjectFit, Pixels, SharedString,
  WeakEntity, Window, div, img, px, relative,
};
use gpui_component::skeleton::Skeleton;
use gpui_component::{IconNamed, Sizable, h_flex, v_flex};
use innertube::{Thumbnail, VideoItem};

use crate::app::{AppView, Route};
use crate::assets::Icon;
use crate::images::Images;
use crate::{motion, theme};

/// Side of a channel avatar next to a card's title.
const AVATAR: f32 = 36.;
/// Width of a related video's thumbnail.
const COMPACT_WIDTH: f32 = 168.;

/// An image filling its parent, picked from `sizes` for `width` logical pixels; a flat surface
/// until it is loaded, then the image fades in.
///
/// `radius` rounds the image itself: GPUI clips children to a rectangle, so a rounded,
/// overflow-hidden parent does not round its images.
pub fn image_fill(
  sizes: &[Thumbnail],
  width: f32,
  radius: Pixels,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let base = div()
    .size_full()
    .relative()
    .rounded(radius)
    .bg(theme::surface())
    .overflow_hidden();
  let pixels = (width * window.scale_factor()).round() as u32;
  let (Some(thumbnail), Some(images)) = (
    Thumbnail::pick(sizes, pixels),
    cx.try_global::<Images>().cloned(),
  ) else {
    return base.into_any_element();
  };
  let ready = images
    .0
    .update(cx, |store, cx| store.get(&thumbnail.url, cx));
  let opacity = motion::tween(
    SharedString::from(format!("fade-{}", thumbnail.url)),
    0.,
    if ready.is_some() { 1. } else { 0. },
    theme::NORMAL,
    window,
    cx,
  );
  base
    .children(ready.map(|render| {
      img(ImageSource::Render(render))
        .absolute()
        .inset_0()
        .size_full()
        .rounded(radius)
        .object_fit(ObjectFit::Cover)
        .opacity(opacity)
    }))
    .into_any_element()
}

fn duration_badge(duration: &Option<String>) -> Option<impl IntoElement + use<>> {
  duration.as_ref().map(|d| {
    div()
      .absolute()
      .bottom(px(8.))
      .right(px(8.))
      .px(px(6.))
      .py(px(1.))
      .rounded(px(4.))
      .bg(gpui::black().opacity(0.8))
      .text_xs()
      .font_weight(FontWeight::SEMIBOLD)
      .text_color(gpui::white())
      .child(d.clone())
  })
}

/// Dimmed layer with a round play button over a thumbnail, faded in by `t`.
fn play_overlay(t: f32, size: f32, radius: Pixels) -> impl IntoElement {
  div()
    .absolute()
    .inset_0()
    .flex()
    .items_center()
    .justify_center()
    .opacity(t)
    .rounded(radius)
    .bg(gpui::black().opacity(0.3))
    .child(
      div()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .shadow_lg()
        .child(gpui_component::Icon::new(Icon::Play).size(px(size * 0.45))),
    )
}

/// A clickable card for `video`, with its hover flag and the hover animation (0 to 1).
/// `id` tells apart the same video in two lists. Resting on it resolves its stream, so a click
/// plays at once.
fn hover_card(
  id: &str,
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> (gpui::Stateful<gpui::Div>, bool, f32) {
  let hovered: Entity<bool> =
    motion::hover_state(SharedString::from(format!("{id}-hover")), window, cx);
  let is_hovered = *hovered.read(cx);
  let t = motion::tween(
    SharedString::from(format!("{id}-lift")),
    0.,
    if is_hovered { 1. } else { 0. },
    theme::FAST,
    window,
    cx,
  );
  let nav = nav.clone();
  let route = Route::Watch(video.id.clone());
  let video_id = video.id.clone();
  let card = div()
    .id(ElementId::Name(id.to_owned().into()))
    .cursor_pointer()
    // GPUI takes a single hover listener per element: the prefetch and the hover flag share it.
    .on_hover(move |hover, window, cx| {
      crate::prefetch::hover(&video_id, *hover, cx);
      hovered.update(cx, |h, _| *h = *hover);
      window.refresh();
    })
    .on_click(move |_, window, cx| {
      nav
        .update(cx, |app, cx| app.open(route.clone(), window, cx))
        .ok();
    });
  (card, is_hovered, t)
}

/// Grid card: 16:9 thumbnail, then channel avatar, title, channel and metadata. On hover it
/// rises a little, a red ring and a play button fade in, and the title turns red.
pub fn video_card(
  video: &VideoItem,
  width: f32,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let (card, hovered, t) = hover_card(&format!("card-{}", video.id), video, nav, window, cx);
  let thumb = image_fill(&video.thumbnails, width, theme::RADIUS_LG, window, cx);
  let avatar = (!video.avatar.is_empty()).then(|| {
    div().size(px(AVATAR)).flex_none().child(image_fill(
      &video.avatar,
      AVATAR,
      px(AVATAR / 2.),
      window,
      cx,
    ))
  });
  // The hovered box stays in place and only its content rises: a box that moved itself could
  // slip out from under the pointer, lose the hover, fall back and loop.
  card
    .w(px(width))
    .flex_none()
    .child(
      v_flex()
        .w_full()
        .relative()
        .top(px(-4. * t))
        .child(
          div()
            .relative()
            .w_full()
            .h(px(width * 9. / 16.))
            .rounded(theme::RADIUS_LG)
            .when(t > 0.01, |el| el.shadow_lg())
            .child(thumb)
            .child(play_overlay(t, 52., theme::RADIUS_LG))
            .children(duration_badge(&video.duration))
            .child(
              div()
                .absolute()
                .inset_0()
                .rounded(theme::RADIUS_LG)
                .border_2()
                .border_color(theme::accent().opacity(t)),
            ),
        )
        .child(
          h_flex()
            .mt_3()
            .gap_3()
            .items_start()
            .children(avatar)
            .child(
              v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                  div()
                    .line_height(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_clamp(2)
                    .text_color(if hovered {
                      theme::accent_hover()
                    } else {
                      theme::foreground()
                    })
                    .child(video.title.clone()),
                )
                .child(meta_text(video).text_sm().line_height(px(18.))),
            ),
        ),
    )
    .into_any_element()
}

/// List row (related videos): small thumbnail on the left, text on the right, on a soft
/// surface while hovered.
pub fn compact_card(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let (card, hovered, t) = hover_card(&format!("related-{}", video.id), video, nav, window, cx);
  let thumb = image_fill(&video.thumbnails, COMPACT_WIDTH, theme::RADIUS, window, cx);
  card
    .w_full()
    .p_1()
    .rounded(theme::RADIUS_LG)
    .bg(theme::surface().opacity(t))
    .child(
      h_flex()
        .gap_3()
        .items_start()
        .child(
          div()
            .relative()
            .w(px(COMPACT_WIDTH))
            .h(px(COMPACT_WIDTH * 9. / 16.))
            .flex_none()
            .child(thumb)
            .child(play_overlay(t, 36., theme::RADIUS))
            .children(duration_badge(&video.duration)),
        )
        .child(
          v_flex()
            .flex_1()
            .min_w_0()
            .pt_0p5()
            .gap_0p5()
            .child(
              div()
                .text_sm()
                .line_height(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .text_color(if hovered {
                  theme::accent_hover()
                } else {
                  theme::foreground()
                })
                .child(video.title.clone()),
            )
            .child(meta_text(video).text_xs().line_height(px(16.))),
        ),
    )
    .into_any_element()
}

/// Width of a list row's thumbnail.
const ROW_WIDTH: f32 = 246.;

/// List row (subscriptions in list mode, as YouTube showed them until about 2023): thumbnail
/// on the left, then the title, the channel with its avatar and views · date; a soft surface,
/// a play button and a red title on hover.
pub fn video_row(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let (card, hovered, t) = hover_card(&format!("row-{}", video.id), video, nav, window, cx);
  let thumb = image_fill(&video.thumbnails, ROW_WIDTH, theme::RADIUS, window, cx);
  let channel = video.channel.clone().map(|name| {
    h_flex()
      .gap_2()
      .items_center()
      .when(!video.avatar.is_empty(), |el| {
        el.child(div().size(px(20.)).flex_none().child(image_fill(
          &video.avatar,
          20.,
          px(10.),
          window,
          cx,
        )))
      })
      .child(div().min_w_0().truncate().child(name))
  });
  card
    .w_full()
    .p_2()
    .rounded(theme::RADIUS_LG)
    .bg(theme::surface().opacity(t))
    .child(
      h_flex()
        .gap_4()
        .items_start()
        .child(
          div()
            .relative()
            .w(px(ROW_WIDTH))
            .h(px(ROW_WIDTH * 9. / 16.))
            .flex_none()
            .child(thumb)
            .child(play_overlay(t, 44., theme::RADIUS))
            .children(duration_badge(&video.duration)),
        )
        .child(
          v_flex()
            .flex_1()
            .min_w_0()
            .pt_1()
            .gap_2()
            .text_sm()
            .text_color(theme::muted())
            .child(
              div()
                .text_lg()
                .line_height(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .text_color(if hovered {
                  theme::accent_hover()
                } else {
                  theme::foreground()
                })
                .child(video.title.clone()),
            )
            .children(channel)
            .child(div().truncate().child(video.metadata.join(" · "))),
        ),
    )
    .into_any_element()
}

/// Placeholder shaped like a [`video_row`].
pub fn skeleton_row() -> impl IntoElement {
  h_flex()
    .p_2()
    .gap_4()
    .items_start()
    .child(
      Skeleton::new()
        .w(px(ROW_WIDTH))
        .h(px(ROW_WIDTH * 9. / 16.))
        .flex_none()
        .rounded(theme::RADIUS),
    )
    .child(
      v_flex()
        .flex_1()
        .pt_1()
        .gap_3()
        .child(Skeleton::new().w(relative(0.6)).h(px(20.)))
        .child(Skeleton::new().w(px(160.)).h(px(14.)))
        .child(Skeleton::new().w(px(200.)).h(px(14.))),
    )
}

/// Channel, then views · date, in muted text.
fn meta_text(video: &VideoItem) -> gpui::Div {
  let row = |text: String| div().truncate().child(text);
  v_flex()
    .text_color(theme::muted())
    .children(video.channel.clone().map(row))
    .child(row(video.metadata.join(" · ")))
}

/// Placeholder shaped like a [`video_card`].
pub fn skeleton_card(width: f32) -> impl IntoElement {
  v_flex()
    .w(px(width))
    .flex_none()
    .child(
      Skeleton::new()
        .w_full()
        .h(px(width * 9. / 16.))
        .rounded(theme::RADIUS_LG),
    )
    .child(
      h_flex()
        .mt_3()
        .gap_3()
        .items_start()
        .child(Skeleton::new().size(px(AVATAR)).flex_none().rounded_full())
        .child(
          v_flex()
            .flex_1()
            .gap_2()
            .child(Skeleton::new().w_full().h(px(16.)))
            .child(Skeleton::new().w(px(width * 0.6)).h(px(14.)))
            .child(Skeleton::new().w(px(width * 0.4)).h(px(14.))),
        ),
    )
}

/// Placeholder shaped like a [`compact_card`].
pub fn skeleton_compact() -> impl IntoElement {
  h_flex()
    .p_1()
    .gap_3()
    .items_start()
    .child(
      Skeleton::new()
        .w(px(COMPACT_WIDTH))
        .h(px(COMPACT_WIDTH * 9. / 16.))
        .flex_none()
        .rounded(theme::RADIUS),
    )
    .child(
      v_flex()
        .flex_1()
        .pt_0p5()
        .gap_2()
        .child(Skeleton::new().w_full().h(px(14.)))
        .child(Skeleton::new().w(px(120.)).h(px(12.)))
        .child(Skeleton::new().w(px(90.)).h(px(12.))),
    )
}

/// Page title.
pub fn page_title(text: impl Into<SharedString>) -> impl IntoElement {
  div()
    .text_2xl()
    .font_weight(FontWeight::BOLD)
    .child(text.into())
}

/// Centered notice for an empty or failed page: an icon in a soft disc, a title and a body.
/// Callers add their buttons as children.
pub fn notice(
  icon: impl IconNamed,
  title: impl Into<SharedString>,
  body: impl Into<SharedString>,
) -> gpui::Div {
  let body: SharedString = body.into();
  v_flex()
    .max_w(px(520.))
    .mx_auto()
    .mt(px(72.))
    .gap_3()
    .items_center()
    .text_center()
    .child(
      div()
        .size(px(64.))
        .mb_2()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent_soft())
        .text_color(theme::accent())
        .child(gpui_component::Icon::new(icon).size(px(28.))),
    )
    .child(
      div()
        .text_xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into()),
    )
    .when(!body.is_empty(), |el| {
      el.child(div().text_color(theme::muted()).child(body))
    })
}

/// A small muted "loading" line with a spinner, for the end of a list.
pub fn loading_more() -> impl IntoElement {
  h_flex()
    .gap_2()
    .items_center()
    .text_sm()
    .text_color(theme::muted())
    .child(gpui_component::spinner::Spinner::new().small())
    .child("Chargement…")
}

/// French text for an InnerTube error, as shown in the UI (the `Display` text is English).
pub fn error_text(error: &innertube::Error) -> String {
  use innertube::Error;
  match error {
    Error::NotLoggedIn => "YouTube ne reconnaît plus la session : les cookies ont expiré ou ont \
                           changé. Exportez-les à nouveau."
      .into(),
    Error::SignInRequired => "Cette page demande d'être connecté.".into(),
    Error::Cookies(detail) => format!("Fichier de cookies invalide : {detail}"),
    Error::Unplayable { status, reason } if status == "LOGIN_REQUIRED" => {
      format!("Cette vidéo demande d'être connecté. {reason}")
    }
    Error::Unplayable { reason, .. } if !reason.is_empty() => {
      format!("Vidéo indisponible : {reason}")
    }
    Error::Unplayable { status, .. } => format!("Vidéo indisponible ({status})"),
    Error::Http(_) => "Impossible de joindre YouTube. Vérifiez la connexion.".into(),
    Error::Status { status, .. } => format!("YouTube a répondu par une erreur (HTTP {status})."),
    Error::Parse { .. } => "Réponse de YouTube inattendue : son format a peut-être changé.".into(),
  }
}

/// Whether a scroll area is within about a screen of its end, to load the next page early.
pub fn near_scroll_end(handle: &gpui::ScrollHandle) -> bool {
  let max = f32::from(handle.max_offset().y);
  let scrolled = -f32::from(handle.offset().y);
  max - scrolled < 1200.
}
