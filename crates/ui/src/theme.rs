//! Peeby's look: a neutral dark palette with the accent of the open space, Fluent-style shapes
//! (Windows 11), applied to gpui-component's theme so its widgets follow. See docs/design.md.

use std::cell::Cell;
use std::panic::Location;
use std::time::Duration;

use gpui::{
  AnyElement, App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
  IntoElement, LayoutId, Pixels, Rgba, Window, WindowAppearance, WindowBackgroundAppearance, px,
  rgb,
};
use gpui_component::{Theme, ThemeMode, ThemeTokens};

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
/// Radius inside the content panel's top-left corner (`RADIUS_LG` less its 1 px edge), for
/// what fills that corner.
pub const PANEL_INNER_RADIUS: Pixels = px(11.);

/// Width of the sidebar.
pub const SIDEBAR_WIDTH: Pixels = px(232.);
/// Width of the sidebar reduced to its icons: an 18 px icon in a 40 px entry, 8 px margins.
pub const SIDEBAR_COLLAPSED_WIDTH: Pixels = px(56.);
/// Width of the sidebar, open or reduced to its icons.
pub fn sidebar_width(collapsed: bool) -> Pixels {
  if collapsed {
    SIDEBAR_COLLAPSED_WIDTH
  } else {
    SIDEBAR_WIDTH
  }
}

/// Height of the title bar.
pub const TITLE_BAR_HEIGHT: Pixels = px(44.);

fn color(hex: u32) -> Hsla {
  rgb(hex).into()
}

fn color_alpha(hex: u32, alpha: f32) -> Hsla {
  let mut c: Rgba = rgb(hex);
  c.a = alpha;
  c.into()
}

/// Share of the space's accent mixed into the backgrounds, so each space has its own hue.
const BACKGROUND_TINT: f32 = 0.050;

/// `hex` with a touch of the accent of the open space.
fn accent_tinted(hex: u32, alpha: f32) -> Hsla {
  let mut c = rgb(hex).blend(Rgba {
    a: BACKGROUND_TINT,
    ..accent().into()
  });
  c.a = alpha;
  c.into()
}

/// Window background behind the sidebar and the title bar, when there is no Mica.
pub fn base() -> Hsla {
  accent_tinted(0x0b0b0c, 1.)
}
/// Content panel and page background.
pub fn background() -> Hsla {
  accent_tinted(0x121214, 1.)
}
/// [`background`] without the accent's tint: the opening animation, which belongs to no space.
pub fn neutral_background() -> Hsla {
  color(0x121214)
}
/// Cards, popovers, inputs.
pub fn surface() -> Hsla {
  accent_tinted(0x1c1c1f, 1.)
}
/// Hovered surfaces.
pub fn surface_hover() -> Hsla {
  accent_tinted(0x27272b, 1.)
}
/// Borders and separators.
pub fn border() -> Hsla {
  color(0x2f2f34)
}
/// Text.
pub fn foreground() -> Hsla {
  color(0xf2f2f3)
}
/// Secondary text.
pub fn muted() -> Hsla {
  color(0xa1a1aa)
}
/// Section labels, tertiary text.
pub fn subtle() -> Hsla {
  color(0x71717a)
}
/// Parts of a video whose audio was removed, on the seek bar: red whatever the space's accent.
pub fn muted_audio() -> Hsla {
  color(0xe5484d)
}
/// The accent color of a space and its hover and pressed shades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accent {
  /// Primary actions, progress, selection, focus.
  pub base: Hsla,
  /// Hovered accent elements and titles.
  pub hover: Hsla,
  /// Pressed accent elements.
  pub active: Hsla,
}

impl Accent {
  /// YouTube red.
  pub fn red() -> Self {
    Self {
      base: color(0xff4e45),
      hover: color(0xff6b63),
      active: color(0xe63e36),
    }
  }
}

thread_local! {
  // The accent of the open space. Kept per thread rather than as a GPUI global so the palette
  // functions below need no `cx`: they are called from every render, and GPUI renders on the
  // UI thread only.
  static ACCENT: Cell<Accent> = Cell::new(Accent::red());
}

/// Makes `accent` the app's accent and re-applies the theme so gpui-component's widgets follow.
/// Windows must be refreshed afterwards (`cx.refresh_windows()`).
pub fn set_accent(accent: Accent, cx: &mut App) {
  ACCENT.set(accent);
  apply(cx);
}

/// The accent: primary actions, progress, selection, focus.
pub fn accent() -> Hsla {
  ACCENT.get().base
}
/// Hovered accent elements and titles.
pub fn accent_hover() -> Hsla {
  ACCENT.get().hover
}
/// Pressed accent elements.
pub fn accent_active() -> Hsla {
  ACCENT.get().active
}
/// Text on the accent.
pub fn on_accent() -> Hsla {
  color(0xffffff)
}
/// Tinted accent for selected rows and soft highlights.
pub fn accent_soft() -> Hsla {
  accent().opacity(0.14)
}
/// Veil over Mica so it stays dark.
fn mica_tint() -> Hsla {
  accent_tinted(0x0b0b0c, 0.55)
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
    t.muted = surface();
    t.muted_foreground = muted();
    t.popover = surface();
    t.popover_foreground = foreground();
    t.overlay = color_alpha(0x000000, 0.6);
    t.skeleton = surface_hover();
    t.window_border = border();
    accent_colors(t);

    t.accent = surface_hover();
    t.accent_foreground = foreground();
    t.primary_foreground = on_accent();
    t.secondary = surface();
    t.secondary_hover = surface_hover();
    t.secondary_active = border();
    t.secondary_foreground = foreground();
    t.button_primary_foreground = on_accent();
    t.button = surface();
    t.button_hover = surface_hover();
    t.button_active = border();
    t.button_foreground = foreground();
    t.button_secondary = surface();
    t.button_secondary_hover = surface_hover();
    t.button_secondary_active = border();
    t.button_secondary_foreground = foreground();

    t.slider_thumb = foreground();
    t.switch = color(0x3a3a40);
    t.switch_thumb = foreground();

    t.colors.list = background();
    t.list_hover = surface_hover();
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

/// The parts of gpui-component's theme in the accent.
fn accent_colors(t: &mut Theme) {
  t.ring = accent();
  t.caret = accent();
  t.selection = accent().opacity(0.3);
  t.primary = accent();
  t.primary_hover = accent_hover();
  t.primary_active = accent_active();
  t.button_primary = accent();
  t.button_primary_hover = accent_hover();
  t.button_primary_active = accent_active();
  t.link = accent();
  t.link_hover = accent_hover();
  t.link_active = accent_active();
  t.progress_bar = accent();
  t.slider_bar = accent();
  t.list_active = accent_soft();
  t.list_active_border = accent();
  t.sidebar_primary = accent();
}

/// Runs `f` with `accent` as the accent, then puts the current one back: for elements built in
/// another space's accent (see [`tinted`]). The theme's colors and tokens (what gpui-component's
/// widgets read) are swapped in place and restored as they were; `Theme::update` would refresh
/// every window, and nothing observes the theme, so a swap in the middle of a frame redraws
/// nothing.
pub fn with_accent<R>(accent: Accent, cx: &mut App, f: impl FnOnce(&mut App) -> R) -> R {
  let previous = ACCENT.replace(accent);
  if previous == accent {
    return f(cx);
  }
  let theme = Theme::global_mut(cx);
  let saved = (theme.colors, theme.tokens);
  accent_colors(theme);
  // Tokens without gradients for this part only: Peeby's theme sets solid colors.
  theme.tokens = ThemeTokens::from(&theme.colors);
  let result = f(cx);
  ACCENT.set(previous);
  let theme = Theme::global_mut(cx);
  (theme.colors, theme.tokens) = saved;
  result
}

/// `child` drawn in `accent` rather than the open space's: gpui-component's widgets read the
/// theme while they lay out and paint, so the accent is swapped around each phase. Build `child`
/// inside [`with_accent`] for the colors read while building it.
pub fn tinted(accent: Accent, child: impl IntoElement) -> Tinted {
  Tinted {
    accent,
    child: child.into_any_element(),
  }
}

/// See [`tinted`].
pub struct Tinted {
  accent: Accent,
  child: AnyElement,
}

impl IntoElement for Tinted {
  type Element = Self;

  fn into_element(self) -> Self {
    self
  }
}

impl Element for Tinted {
  type RequestLayoutState = ();
  type PrepaintState = ();

  fn id(&self) -> Option<ElementId> {
    None
  }

  fn source_location(&self) -> Option<&'static Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, ()) {
    let child = &mut self.child;
    (
      with_accent(self.accent, cx, |cx| child.request_layout(window, cx)),
      (),
    )
  }

  fn prepaint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    _: Bounds<Pixels>,
    _: &mut (),
    window: &mut Window,
    cx: &mut App,
  ) {
    let child = &mut self.child;
    with_accent(self.accent, cx, |cx| child.prepaint(window, cx));
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    _: Bounds<Pixels>,
    _: &mut (),
    _: &mut (),
    window: &mut Window,
    cx: &mut App,
  ) {
    let child = &mut self.child;
    with_accent(self.accent, cx, |cx| child.paint(window, cx));
  }
}

/// Segoe UI Variable (Windows 11), else Segoe UI.
fn ui_font(cx: &App) -> &'static str {
  let names = cx.text_system().all_font_names();
  ["Segoe UI Variable Text", "Segoe UI"]
    .into_iter()
    .find(|name| names.iter().any(|n| n == name))
    .unwrap_or(".SystemUIFont")
}
