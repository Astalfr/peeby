//! Embedded SVG icons (Lucide, ISC license: assets/icons/LICENSE), served to GPUI, and the
//! Peeby logo. Adapted from the Jellyfin-only peeby; gpui-component also draws some of the icons
//! inside its widgets (window buttons).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
  AnyElement, App, AssetSource, DevicePixels, Global, ImageSource, IntoElement, RenderImage,
  SharedString, Styled, SvgSize, Window, img, px,
};
use gpui_component::IconNamed;

use crate::images;

macro_rules! icons {
  ($($name:ident => $file:literal,)*) => {
    /// Icons used by Peeby.
    // Each variant is named after what its icon shows; a doc line per variant would repeat it.
    #[allow(missing_docs)]
    #[derive(Clone, Copy, Debug)]
    pub enum Icon {
      $($name,)*
    }

    impl IconNamed for Icon {
      fn path(self) -> SharedString {
        match self {
          $(Self::$name => concat!("icons/", $file, ".svg"),)*
        }
        .into()
      }
    }

    const OWN: &[(&str, &[u8])] = &[
      $((concat!("icons/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg"))),)*
    ];
  };
}

icons! {
  Play => "play",
  Pause => "pause",
  Maximize => "maximize",
  Minimize => "minimize",
  Volume => "volume-2",
  VolumeLow => "volume-1",
  Muted => "volume-x",
  Back10 => "rotate-ccw",
  Forward10 => "rotate-cw",
  ArrowLeft => "arrow-left",
  Home => "house",
  Subscriptions => "users",
  ThumbsUp => "thumbs-up",
  ThumbsDown => "thumbs-down",
  Comments => "message-square",
  Subtitles => "captions",
  Audio => "audio-lines",
  Keyboard => "keyboard",
  Chapters => "list-video",
  Settings => "settings",
  Account => "circle-user",
  Search => "search",
  Grid => "layout-grid",
  List => "list",
  Playlists => "list-video",
  Shuffle => "shuffle",
  Repeat => "repeat",
  Previous => "skip-back",
  Next => "skip-forward",
  More => "ellipsis-vertical",
  WatchLater => "clock",
  Save => "list-plus",
  Check => "check",
  Lock => "lock",
  Reverse => "arrow-down-up",
  Menu => "menu",
  Theater => "rectangle-horizontal",
  Bell => "bell",
  BellRing => "bell-ring",
  BellOff => "bell-off",
  Film => "film",
  Tv => "tv",
  Folder => "folder",
  Star => "star",
  Info => "info",
  ChevronLeft => "chevron-left",
  ChevronRight => "chevron-right",
  SignOut => "log-out",
  Points => "coins",
  Gift => "gift",
}

macro_rules! files {
  ($($file:literal),* $(,)?) => {
    &[$((concat!("icons/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg")))),*]
  };
}

/// Icons gpui-component draws inside its own widgets (same paths as its `IconName`), besides
/// those of [`Icon`] (`info`).
const COMPONENT: &[(&str, &[u8])] = files!(
  "chevron-down",
  "chevron-up",
  "close",
  "loader",
  "loader-circle",
  "eye",
  "eye-off",
  "circle-x",
  "triangle-alert",
  "circle-check",
  "window-close",
  "window-maximize",
  "window-minimize",
  "window-restore",
);

pub(crate) const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

/// The logo, decoded once per device-pixel size.
#[derive(Default)]
struct LogoCache(HashMap<u32, Option<Arc<RenderImage>>>);

impl Global for LogoCache {}

/// The Peeby logo (the portrait over the spaces' three cards), `size` logical pixels wide. It is
/// scaled on the CPU to the exact device-pixel size: GPUI samples without mipmaps, so the 512 px
/// file drawn small would look pixelated. Decoding the small embedded PNG is quick and happens
/// once per size.
pub fn logo(size: f32, window: &Window, cx: &mut App) -> AnyElement {
  let side = (size * window.scale_factor()).round() as u32;
  let image = cx
    .default_global::<LogoCache>()
    .0
    .entry(side)
    .or_insert_with(|| images::decode(LOGO_PNG, Some((side, side))))
    .clone();
  match image {
    Some(image) => img(ImageSource::Render(image))
      .size(px(size))
      .flex_none()
      .into_any_element(),
    None => gpui::div().size(px(size)).flex_none().into_any_element(),
  }
}

/// Colored SVGs, rasterized once per source and device-pixel size (keyed by the bytes' address:
/// they are `'static`).
#[derive(Default)]
struct ColorSvgCache(HashMap<(usize, u32), Option<Arc<RenderImage>>>);

impl Global for ColorSvgCache {}

/// A colored SVG (a service's logo), `size` logical pixels square. Unlike [`gpui::svg`], which
/// draws a one-color mask, it keeps the file's colors; as [`logo`], it is rasterized to the exact
/// device-pixel size. The SVG's view box should be square.
pub fn color_svg(bytes: &'static [u8], size: f32, window: &Window, cx: &mut App) -> AnyElement {
  let side = (size * window.scale_factor()).round().max(1.) as u32;
  let key = (bytes.as_ptr() as usize, side);
  let image = match cx.default_global::<ColorSvgCache>().0.get(&key) {
    Some(image) => image.clone(),
    None => {
      let renderer = cx.svg_renderer();
      let pixels = DevicePixels(side as i32);
      let image = renderer
        .parse_svg(bytes)
        .and_then(|svg| {
          renderer.render_parsed(&svg, SvgSize::ExactSize(gpui::size(pixels, pixels)))
        })
        .inspect_err(|e| tracing::warn!("SVG not rendered: {e}"))
        .ok();
      cx.default_global::<ColorSvgCache>()
        .0
        .insert(key, image.clone());
      image
    }
  };
  match image {
    Some(image) => img(ImageSource::Render(image))
      .size(px(size))
      .flex_none()
      .into_any_element(),
    None => gpui::div().size(px(size)).flex_none().into_any_element(),
  }
}

/// The logo's card, a rounded square that the opening animation tints and turns ([`crate::splash`]).
pub const LOGO_CARD: &str = "logo/card.svg";

/// Shapes of the logo, drawn as one-color masks.
const LOGO: &[(&str, &[u8])] = &[(LOGO_CARD, include_bytes!("../assets/logo/card.svg"))];

fn files() -> impl Iterator<Item = &'static (&'static str, &'static [u8])> {
  OWN.iter().chain(COMPONENT).chain(LOGO)
}

/// Asset source for [`gpui::Application::with_assets`].
pub struct Assets;

impl AssetSource for Assets {
  fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
    Ok(
      files()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| Cow::Borrowed(*bytes)),
    )
  }

  fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
    Ok(
      files()
        .filter(|(name, _)| name.starts_with(path))
        .map(|(name, _)| SharedString::from(*name))
        .collect(),
    )
  }
}
