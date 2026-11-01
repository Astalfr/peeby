//! PeebTube's look: a neutral dark palette with a red accent, Fluent-style shapes (Windows 11),
//! applied to gpui-component's theme so its widgets follow. Adapted from peeby's theme; see
//! docs/design.md.

use std::time::Duration;

use gpui::{App, Hsla, Pixels, Rgba, WindowAppearance, WindowBackgroundAppearance, px, rgb};
use gpui_component::{Theme, ThemeMode};

/// Fast transitions: hover, fades of small elements.
pub const FAST: Duration = Duration::from_millis(150);
/// Normal transitions: image fade-in, page change.
pub const NORMAL: Duration = Duration::from_millis(200);
/// Slow transitions: skeleton to content cross-fade.
pub const SLOW: Duration = Duration::from_millis(300);

/// Corner radius of controls and cards.
pub const RADIUS: Pixels = px(8.);
/// Corner radius of large surfaces (content panel, dialogs, hero).
pub const RADIUS_LG: Pixels = px(12.);

/// Width of the sidebar.
pub const SIDEBAR_WIDTH: Pixels = px(232.);

fn color(hex: u32) -> Hsla {
  rgb(hex).into()
}

fn color_alpha(hex: u32, alpha: f32) -> Hsla {
  let mut c: Rgba = rgb(hex);
  c.a = alpha;
  c.into()
}

/// Window background behind the sidebar and the title bar, when there is no Mica.
pub fn base() -> Hsla {
  color(0x0b0b0c)
}
/// Content panel and page background.
pub fn background() -> Hsla {
  color(0x121214)
}
/// Cards, popovers, inputs.
pub fn surface() -> Hsla {
  color(0x1c1c1f)
}
/// Hovered surfaces.
pub fn surface_hover() -> Hsla {
  color(0x27272b)
}
pub fn border() -> Hsla {
  color(0x2f2f34)
}
pub fn foreground() -> Hsla {
  color(0xf2f2f3)
}
pub fn muted() -> Hsla {
  color(0xa1a1aa)
}
/// Section labels, tertiary text.
pub fn subtle() -> Hsla {
  color(0x71717a)
}
/// Red: primary actions, progress, selection, focus.
pub fn accent() -> Hsla {
  color(0xff4e45)
}
pub fn accent_hover() -> Hsla {
  color(0xff6b63)
}
pub fn accent_active() -> Hsla {
  color(0xe63e36)
}
/// Text on the accent.
pub fn on_accent() -> Hsla {
  color(0xffffff)
}
/// Tinted accent for selected rows and soft highlights.
pub fn accent_soft() -> Hsla {
  color_alpha(0xff4e45, 0.14)
}
/// Veil over Mica so it stays dark.
fn mica_tint() -> Hsla {
  color_alpha(0x0b0b0c, 0.55)
}

/// Whether the window uses Mica: only on a dark system theme, since Mica follows it and a
/// light Mica clashes with the dark UI.
pub fn window_background(cx: &mut App) -> WindowBackgroundAppearance {
  let mica = matches!(
    cx.window_appearance(),
    WindowAppearance::Dark | WindowAppearance::VibrantDark
  );
  cx.set_global(Chrome { mica });
  if mica {
    WindowBackgroundAppearance::MicaBackdrop
  } else {
    WindowBackgroundAppearance::Opaque
  }
}

struct Chrome {
  mica: bool,
}

impl gpui::Global for Chrome {}

/// Background of the window chrome (sidebar, title bar): Mica seen through a dark veil, or
/// a solid color without Mica.
pub fn chrome(cx: &App) -> Hsla {
  if cx.try_global::<Chrome>().is_some_and(|c| c.mica) {
    mica_tint()
  } else {
    base()
  }
}

/// Applies the palette, font and shapes to gpui-component's theme.
pub fn apply(cx: &mut App) {
  Theme::change(ThemeMode::Dark, None, cx);
  let font = ui_font(cx);
  Theme::update(cx, |t| {
    t.font_family = font.into();
    t.font_size = px(15.);
    t.radius = RADIUS;
    t.radius_lg = RADIUS_LG;
    t.shadow = true;

    t.background = background();
    t.foreground = foreground();
    t.border = border();
    t.input = border();
    t.ring = accent();
    t.caret = accent();
    t.selection = color_alpha(0xff4e45, 0.3);
    t.muted = surface();
    t.muted_foreground = muted();
    t.popover = surface();
    t.popover_foreground = foreground();
    t.overlay = color_alpha(0x000000, 0.6);
    t.skeleton = surface_hover();
    t.window_border = border();

    t.accent = surface_hover();
    t.accent_foreground = foreground();
    t.primary = accent();
    t.primary_hover = accent_hover();
    t.primary_active = accent_active();
    t.primary_foreground = on_accent();
    t.secondary = surface();
    t.secondary_hover = surface_hover();
    t.secondary_active = border();
    t.secondary_foreground = foreground();
    t.button_primary = accent();
    t.button_primary_hover = accent_hover();
    t.button_primary_active = accent_active();
    t.button_primary_foreground = on_accent();
    t.button = surface();
    t.button_hover = surface_hover();
    t.button_active = border();
    t.button_foreground = foreground();
    t.button_secondary = surface();
    t.button_secondary_hover = surface_hover();
    t.button_secondary_active = border();
    t.button_secondary_foreground = foreground();

    t.link = accent();
    t.link_hover = accent_hover();
    t.link_active = accent_active();
    t.progress_bar = accent();
    t.slider_bar = accent();
    t.slider_thumb = foreground();
    t.switch = color(0x3a3a40);
    t.switch_thumb = foreground();

    t.colors.list = background();
    t.list_hover = surface_hover();
    t.list_active = accent_soft();
    t.list_active_border = accent();
    t.tab = gpui::transparent_black();
    t.tab_bar = gpui::transparent_black();
    t.tab_active = surface_hover();
    t.tab_foreground = muted();
    t.tab_active_foreground = foreground();

    t.sidebar = gpui::transparent_black();
    t.sidebar_border = gpui::transparent_black();
    t.sidebar_foreground = foreground();
    t.sidebar_accent = surface_hover();
    t.sidebar_accent_foreground = foreground();
    t.sidebar_primary = accent();
    t.sidebar_primary_foreground = on_accent();
    t.title_bar = gpui::transparent_black();
    t.title_bar_border = gpui::transparent_black();

    // Toasts at the bottom right, out of the way of the title bar and the page header.
    t.notification.placement = gpui::Anchor::BottomRight;

    t.scrollbar = gpui::transparent_black();
    t.scrollbar_thumb = color_alpha(0xf2f2f3, 0.22);
    t.scrollbar_thumb_hover = color_alpha(0xf2f2f3, 0.4);
  });
}

/// Segoe UI Variable (Windows 11), else Segoe UI.
fn ui_font(cx: &App) -> &'static str {
  let names = cx.text_system().all_font_names();
  ["Segoe UI Variable Text", "Segoe UI"]
    .into_iter()
    .find(|name| names.iter().any(|n| n == name))
    .unwrap_or(".SystemUIFont")
}
