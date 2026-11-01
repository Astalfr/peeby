//! Embedded SVG icons (Lucide, ISC license: assets/icons/LICENSE), served to GPUI, and the
//! Peeby logo. Adapted from the Jellyfin-only peeby; gpui-component also draws some of the icons
//! inside its widgets (window buttons).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
  AnyElement, App, AssetSource, Global, ImageSource, IntoElement, RenderImage, SharedString,
  Styled, Window, img, px,
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
}

macro_rules! files {
  ($($file:literal),* $(,)?) => {
    &[$((concat!("icons/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg")))),*]
  };
}

/// Icons gpui-component draws inside its own widgets (same paths as its `IconName`).
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
  "info",
  "window-close",
  "window-maximize",
  "window-minimize",
  "window-restore",
);

const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

/// The logo, decoded once per device-pixel size.
#[derive(Default)]
struct LogoCache(HashMap<u32, Option<Arc<RenderImage>>>);

impl Global for LogoCache {}

/// The round Peeby logo (with the red play badge), `size` logical pixels wide. It is
/// scaled on the CPU to the exact device-pixel size: GPUI samples without mipmaps, so the 256 px
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

fn files() -> impl Iterator<Item = &'static (&'static str, &'static [u8])> {
  OWN.iter().chain(COMPONENT)
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
