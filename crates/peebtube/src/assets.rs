//! Embedded SVG icons (Lucide, ISC license: assets/icons/LICENSE), served to GPUI. Adapted
//! from peeby; gpui-component also draws some of them inside its widgets (window buttons).

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};
use gpui_component::IconNamed;

macro_rules! icons {
  ($($name:ident => $file:literal,)*) => {
    /// Icons used by PeebTube.
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
