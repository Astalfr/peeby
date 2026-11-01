//! What a component shows as an image, whatever the service: a set of fixed sizes to pick from
//! (YouTube thumbnails) or a URL for any size (servers that resize, size templates), and the
//! elements that draw it.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, ImageSource, ObjectFit, Pixels, SharedString, Window, div, img};

use crate::images::Images;
use crate::{motion, theme};

/// One size of an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Size {
  /// Image URL.
  pub url: String,
  /// Width in pixels, 0 when unknown.
  pub width: u32,
}

/// Builds the URL of an image for a width and a height in device pixels.
pub type SizedUrl = Rc<dyn Fn(u32, u32) -> String>;

/// An image of a card, an avatar, a backdrop…
#[derive(Clone)]
pub enum Picture {
  /// Fixed sizes: the smallest at least as wide as drawn is used, else the largest.
  Sizes(Vec<Size>),
  /// A URL for any size, for servers that resize (Jellyfin `fillWidth`, Twitch templates).
  Sized {
    /// The URL for a size.
    url: SizedUrl,
    /// Shown blurred while the image loads (Jellyfin's `ImageBlurHashes`).
    blurhash: Option<SharedString>,
  },
}

impl Picture {
  /// The URL to draw this picture `width`×`height` device pixels, if there is one.
  pub fn url(&self, width: u32, height: u32) -> Option<String> {
    match self {
      Self::Sizes(sizes) => sizes
        .iter()
        .filter(|s| s.width >= width)
        .min_by_key(|s| s.width)
        .or_else(|| sizes.iter().max_by_key(|s| s.width))
        .map(|s| s.url.clone()),
      Self::Sized { url, .. } => Some(url(width, height)),
    }
  }

  /// A picture from a URL for any size, without a placeholder.
  pub fn sized(url: impl Fn(u32, u32) -> String + 'static) -> Self {
    Self::Sized {
      url: Rc::new(url),
      blurhash: None,
    }
  }

  /// Whether there is nothing to show.
  pub fn is_empty(&self) -> bool {
    matches!(self, Self::Sizes(sizes) if sizes.is_empty())
  }
}

/// `picture` filling its parent, drawn `width` logical pixels wide with the `aspect` ratio
/// (width / height); its BlurHash, or a flat surface, until it is loaded, then the image fades
/// in.
///
/// `radius` rounds the image itself: GPUI clips children to a rectangle, so a rounded,
/// overflow-hidden parent does not round its images.
pub fn image_fill(
  picture: &Picture,
  width: f32,
  aspect: f32,
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
  let scale = window.scale_factor();
  let pixels = (width * scale).round() as u32;
  let height = (width / aspect.max(0.01) * scale).round() as u32;
  let (Some(url), Some(images)) = (
    picture.url(pixels, height),
    cx.try_global::<Images>().cloned(),
  ) else {
    return base.into_any_element();
  };
  let ready = images.0.update(cx, |store, cx| store.get(&url, cx));
  let opacity = motion::tween(
    SharedString::from(format!("fade-{url}")),
    0.,
    if ready.is_some() { 1. } else { 0. },
    theme::NORMAL,
    window,
    cx,
  );
  let placeholder = match picture {
    Picture::Sized {
      blurhash: Some(hash),
      ..
    } if opacity < 1. => images.0.update(cx, |store, _| store.blurhash(hash, aspect)),
    _ => None,
  };
  let layer = |render| {
    img(ImageSource::Render(render))
      .absolute()
      .inset_0()
      .size_full()
      .rounded(radius)
      .object_fit(ObjectFit::Cover)
  };
  base
    .children(placeholder.map(layer))
    .children(ready.map(|render| layer(render).opacity(opacity)))
    .into_any_element()
}

/// A transparent picture (a title's logo) scaled to fit `max_width` × `max_height` logical
/// pixels, keeping its ratio. `None` until it has loaded, so the caller shows the title meanwhile.
pub fn logo(
  picture: &Picture,
  max_width: f32,
  max_height: f32,
  window: &mut Window,
  cx: &mut App,
) -> Option<AnyElement> {
  let images = cx.try_global::<Images>().cloned()?;
  let scale = window.scale_factor();
  let url = picture.url(
    (max_width * scale).round() as u32,
    (max_height * scale).round() as u32,
  )?;
  let render = images.0.update(cx, |store, cx| store.get(&url, cx))?;
  let size = render.size(0);
  let (w, h) = (size.width.0 as f32 / scale, size.height.0 as f32 / scale);
  let fit = (max_width / w.max(1.)).min(max_height / h.max(1.)).min(1.);
  Some(
    img(ImageSource::Render(render))
      .w(gpui::px(w * fit))
      .h(gpui::px(h * fit))
      .into_any_element(),
  )
}

/// A round avatar of `size` logical pixels.
pub fn avatar(picture: &Picture, size: f32, window: &mut Window, cx: &mut App) -> AnyElement {
  div()
    .size(gpui::px(size))
    .flex_none()
    .child(image_fill(
      picture,
      size,
      1.,
      gpui::px(size / 2.),
      window,
      cx,
    ))
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn size(width: u32) -> Size {
    Size {
      url: format!("w{width}"),
      width,
    }
  }

  #[test]
  fn sizes_pick_the_smallest_wide_enough_else_the_largest() {
    let picture = Picture::Sizes(vec![size(168), size(336), size(720)]);
    assert_eq!(picture.url(300, 0).as_deref(), Some("w336"));
    assert_eq!(picture.url(336, 0).as_deref(), Some("w336"));
    assert_eq!(picture.url(1000, 0).as_deref(), Some("w720"));
    assert_eq!(Picture::Sizes(Vec::new()).url(100, 0), None);
  }

  #[test]
  fn sized_urls_get_the_drawn_size() {
    let picture = Picture::sized(|w, h| format!("img?w={w}&h={h}"));
    assert_eq!(picture.url(320, 480).as_deref(), Some("img?w=320&h=480"));
    assert!(!picture.is_empty());
  }
}
