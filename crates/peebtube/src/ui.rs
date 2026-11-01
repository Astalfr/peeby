//! Shared UI pieces: remote images and video cards.

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ElementId, FontWeight, ImageSource, ObjectFit, Pixels, WeakEntity, Window, div,
  img, px,
};
use gpui_component::{h_flex, v_flex};
use innertube::{Thumbnail, VideoItem};

use crate::app::{AppView, Route};
use crate::images::Images;
use crate::theme;

/// An image filling its parent, picked from `sizes` for `width` logical pixels; a flat surface
/// until it is loaded.
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
  base
    .children(ready.map(|render| {
      img(ImageSource::Render(render))
        .absolute()
        .inset_0()
        .size_full()
        .rounded(radius)
        .object_fit(ObjectFit::Cover)
    }))
    .into_any_element()
}

fn duration_badge(duration: &Option<String>) -> Option<impl IntoElement + use<>> {
  duration.as_ref().map(|d| {
    div()
      .absolute()
      .bottom(px(6.))
      .right(px(6.))
      .px_1()
      .rounded(px(4.))
      .bg(gpui::rgba(0x000000cc))
      .text_xs()
      .font_weight(FontWeight::MEDIUM)
      .child(d.clone())
  })
}

fn open_on_click(
  id: impl Into<ElementId>,
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
) -> gpui::Stateful<gpui::Div> {
  let nav = nav.clone();
  let route = Route::Watch(video.id.clone());
  let video_id = video.id.clone();
  div()
    .id(id.into())
    .cursor_pointer()
    // Resting on a card resolves its stream, so a click plays at once.
    .on_hover(move |hovered, _, cx| crate::prefetch::hover(&video_id, *hovered, cx))
    .on_click(move |_, window, cx| {
      nav
        .update(cx, |app, cx| app.open(route.clone(), window, cx))
        .ok();
    })
}

/// Grid card: 16:9 thumbnail, then channel avatar, title, channel and metadata.
pub fn video_card(
  video: &VideoItem,
  width: f32,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let thumb = image_fill(&video.thumbnails, width, theme::RADIUS_LG, window, cx);
  let avatar = (!video.avatar.is_empty()).then(|| {
    div()
      .size(px(36.))
      .flex_none()
      .child(image_fill(&video.avatar, 36., px(18.), window, cx))
  });
  open_on_click(format!("card-{}", video.id), video, nav)
    .w(px(width))
    .flex_none()
    .child(
      div()
        .relative()
        .w_full()
        .h(px(width * 9. / 16.))
        .child(thumb)
        .children(duration_badge(&video.duration)),
    )
    .child(
      h_flex()
        .mt_3()
        .gap_3()
        .items_start()
        .children(avatar)
        .child(
          v_flex()
            .min_w_0()
            .gap_1()
            .child(
              div()
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .child(video.title.clone()),
            )
            .child(meta_text(video)),
        ),
    )
    .into_any_element()
}

/// List row (related videos): small thumbnail on the left, text on the right.
pub fn compact_card(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  const WIDTH: f32 = 168.;
  let thumb = image_fill(&video.thumbnails, WIDTH, theme::RADIUS, window, cx);
  open_on_click(format!("related-{}", video.id), video, nav)
    .w_full()
    .child(
      h_flex()
        .gap_2()
        .items_start()
        .child(
          div()
            .relative()
            .w(px(WIDTH))
            .h(px(WIDTH * 9. / 16.))
            .flex_none()
            .child(thumb)
            .children(duration_badge(&video.duration)),
        )
        .child(
          v_flex()
            .min_w_0()
            .gap_1()
            .child(
              div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .child(video.title.clone()),
            )
            .child(meta_text(video)),
        ),
    )
    .into_any_element()
}

/// Channel, then views · date, in muted small text.
fn meta_text(video: &VideoItem) -> impl IntoElement + use<> {
  v_flex()
    .text_sm()
    .text_color(theme::muted())
    .children(video.channel.clone().map(|c| div().line_clamp(1).child(c)))
    .child(div().line_clamp(1).child(video.metadata.join(" · ")))
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
