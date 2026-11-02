//! The window's root: a top bar shared by every space (menu button, back, logo, the spaces as
//! tabs, search, notifications, settings), the sidebar with the open space's entries, and the
//! space's current page in the content panel. Switching tabs while a video plays moves it to
//! the mini-player, a window of its own above the others (docs/design.md, ADR 0008).

mod mini_player;
mod settings_page;
pub mod space;

use gpui::prelude::*;
use gpui::{
  Anchor, AnyView, App, Context, Entity, FontWeight, ImageSource, ObjectFit, SharedString,
  Subscription, Window, WindowHandle, div, img, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, Selectable, TitleBar, h_flex, v_flex};
use ui::assets::{self, Icon};
use ui::images::Images;
use ui::player::{NowPlaying, PlayerView, now_playing};
use ui::theme;

use crate::settings::Settings;
use mini_player::MiniPlayer;
use settings_page::SettingsView;
use space::{Avatar, MenuEntry, SpaceEvent, SpaceHandle};

/// Width of the notifications panel.
const NOTIFICATIONS_WIDTH: f32 = 440.;

/// The mini-player window and the space whose video it shows.
struct Mini {
  window: WindowHandle<MiniPlayer>,
  space: usize,
}

/// The window's root view.
pub struct Shell {
  spaces: Vec<SpaceHandle>,
  active: usize,
  /// The settings page (gear), shown instead of the space's page while open.
  settings: Option<AnyView>,
  mini: Option<Mini>,
  _subscriptions: Vec<Subscription>,
}

impl Shell {
  /// The shell over `spaces`, the first one open.
  pub fn new(spaces: Vec<SpaceHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let mut subscriptions = Vec::new();
    for (index, space) in spaces.iter().enumerate() {
      let shell = cx.weak_entity();
      subscriptions.push(space.observe(
        cx,
        Box::new(move |cx| {
          shell
            .update(cx, |shell, cx| shell.space_changed(index, cx))
            .ok();
        }),
      ));
      let shell = cx.weak_entity();
      let handle = window.window_handle();
      subscriptions.push(space.on_event(
        cx,
        Box::new(move |event, cx| {
          let open_settings = matches!(event, SpaceEvent::OpenSettings);
          let shell = shell.clone();
          // Deferred: the event comes from inside the window (a click), which cannot be
          // updated again from there.
          cx.defer(move |cx| {
            handle
              .update(cx, |_, window, cx| {
                shell
                  .update(cx, |shell, cx| {
                    if open_settings {
                      shell.open_settings(window, cx)
                    } else {
                      shell.activate(index, window, cx)
                    }
                  })
                  .ok();
              })
              .ok();
          });
        }),
      ));
    }
    // Another player started: the mini-player shows only the one that plays.
    subscriptions.push(cx.observe_global::<NowPlaying>(|shell, cx| {
      let shown = shell.mini_player(cx);
      if shown.is_some() && shown != now_playing(cx) {
        shell.close_mini(cx);
      }
    }));
    if let Some(space) = spaces.first() {
      theme::set_accent(space.info(cx).accent, cx);
    }
    Self {
      spaces,
      active: 0,
      settings: None,
      mini: None,
      _subscriptions: subscriptions,
    }
  }

  fn space(&self) -> Option<&SpaceHandle> {
    self.spaces.get(self.active)
  }

  /// Shows space `index` (its tab, a notification of it, the mini-player's button).
  fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    if index >= self.spaces.len() || (index == self.active && self.settings.is_none()) {
      return;
    }
    self.leave_page(window, cx);
    self.settings = None;
    self.active = index;
    self.enter_page(cx);
    theme::set_accent(self.spaces[index].info(cx).accent, cx);
    cx.refresh_windows();
  }

  /// The page on screen goes away (another tab, the settings): a video playing on it moves to
  /// the mini-player.
  fn leave_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.settings.is_some() || self.mini.is_some() {
      return;
    }
    let Some(player) = self.space().and_then(|s| s.player(cx)) else {
      return;
    };
    if player.read(cx).is_paused() {
      return;
    }
    self.open_mini(self.active, window, cx);
  }

  /// The space's page is on screen again: its video leaves the mini-player and plays on.
  fn enter_page(&mut self, cx: &mut Context<Self>) {
    if self.settings.is_none() && self.mini.as_ref().is_some_and(|m| m.space == self.active) {
      self.close_mini(cx);
    }
  }

  fn open_mini(&mut self, space: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(handle) = self.spaces.get(space).cloned() else {
      return;
    };
    let shell = cx.weak_entity();
    let main = window.window_handle();
    match mini_player::open(handle, shell, main, window, cx) {
      Ok(window) => self.mini = Some(Mini { window, space }),
      Err(e) => tracing::warn!("cannot open the mini-player: {e:#}"),
    }
  }

  /// Closes the mini-player window; its video stays where it is (paused or not). Deferred: the
  /// click that closes it may come from that window, which cannot be updated from inside itself.
  fn close_mini(&mut self, cx: &mut App) {
    if let Some(mini) = self.mini.take() {
      cx.defer(move |cx| {
        if let Err(e) = mini
          .window
          .update(cx, |_, window, _| window.remove_window())
        {
          tracing::warn!("cannot close the mini-player: {e:#}");
        }
      });
    }
  }

  /// The mini-player's "×": the video pauses and the window closes.
  pub(crate) fn dismiss_mini(&mut self, cx: &mut Context<Self>) {
    if let Some(player) = self.mini_player(cx) {
      player.read(cx).pause();
    }
    self.close_mini(cx);
    cx.notify();
  }

  /// The mini-player's "back to the page": its space opens in the main window.
  pub(crate) fn return_from_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(space) = self.mini.as_ref().map(|m| m.space) {
      window.activate_window();
      self.activate(space, window, cx);
      // The space was the active one behind the settings page.
      self.settings = None;
      self.enter_page(cx);
      cx.notify();
    }
  }

  /// The player the mini-player shows: the current one of its space.
  fn mini_player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    let mini = self.mini.as_ref()?;
    self.spaces.get(mini.space)?.player(cx)
  }

  /// A space changed (a page opened, a queue moved to its next video): the mini-player follows
  /// its current player, and closes when there is none.
  fn space_changed(&mut self, index: usize, cx: &mut Context<Self>) {
    if let Some(mini) = &self.mini
      && mini.space == index
    {
      if self.mini_player(cx).is_none() {
        self.close_mini(cx);
      } else {
        mini.window.update(cx, |_, _, cx| cx.notify()).ok();
      }
    }
    cx.notify();
  }

  fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.settings.is_some() {
      return;
    }
    self.leave_page(window, cx);
    let spaces = self.spaces.clone();
    self.settings = Some(
      cx.new(|cx| SettingsView::new(spaces, self.active, cx))
        .into(),
    );
    cx.notify();
  }

  fn back(&mut self, cx: &mut Context<Self>) {
    if self.settings.take().is_some() {
      self.enter_page(cx);
    } else if let Some(space) = self.space() {
      space.back(cx);
    }
    cx.notify();
  }

  /// The top bar's search: the open space's search page.
  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.settings.take().is_some() {
      self.enter_page(cx);
    }
    if let Some(space) = self.space().cloned() {
      space.open_search(window, cx);
    }
    cx.notify();
  }

  fn open_entry(&mut self, id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
    if self.settings.take().is_some() {
      self.enter_page(cx);
    }
    if let Some(space) = self.space().cloned() {
      space.open_entry(&id, window, cx);
    }
    cx.notify();
  }

  // --- Top bar ------------------------------------------------------------------------------

  fn top_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> TitleBar {
    let collapsed = Settings::get(cx).sidebar_collapsed;
    let menu = Button::new("menu")
      .ghost()
      .icon(Icon::Menu)
      .tooltip(if collapsed {
        "Développer le menu"
      } else {
        "Réduire le menu"
      })
      .on_click(cx.listener(|_, _, _, cx| {
        Settings::update(cx, |s| s.sidebar_collapsed = !s.sidebar_collapsed);
        cx.notify();
      }));
    let can_go_back = self.settings.is_some() || self.space().is_some_and(|s| s.can_go_back(cx));
    let search = self
      .space()
      .filter(|s| s.searchable(cx))
      .map(|s| s.info(cx).name);
    // Always present (disabled when there is nowhere to go) so the bar does not shift.
    let back = Button::new("back")
      .ghost()
      .icon(Icon::ArrowLeft)
      .disabled(!can_go_back)
      .tooltip("Retour")
      .on_click(cx.listener(|this, _, _, cx| this.back(cx)));
    TitleBar::new().h(theme::TITLE_BAR_HEIGHT).child(
      h_flex()
        .h_full()
        .w_full()
        .gap_3()
        .items_center()
        // The bar is a window drag area; occluding the controls keeps their clicks.
        .child(div().occlude().child(menu))
        .child(div().occlude().child(back))
        .child(mark(window, cx))
        .child(div().occlude().ml_4().child(self.tabs(window, cx)))
        .child(div().flex_1())
        .child(
          h_flex()
            .occlude()
            .mr_2()
            .gap_1()
            .child(
              // Each space searches its own catalog.
              Button::new("search")
                .ghost()
                .icon(Icon::Search)
                .disabled(search.is_none())
                .tooltip(match &search {
                  Some(name) => format!("Rechercher dans {name}"),
                  None => "Recherche (bientôt)".to_owned(),
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .child(self.bell(cx))
            .child(
              Button::new("settings")
                .ghost()
                .icon(Icon::Settings)
                .tooltip("Réglages")
                .when(self.settings.is_some(), |b| b.selected(true))
                .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            ),
        ),
    )
  }

  /// The spaces as tabs: the service's logo and the name; the open one has a light background
  /// and a line in its accent underneath.
  fn tabs(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    h_flex()
      .h(px(34.))
      .gap_1()
      .children(self.spaces.iter().enumerate().map(|(index, space)| {
        let info = space.info(cx);
        let active = index == self.active && self.settings.is_none();
        h_flex()
          .id(SharedString::from(format!("tab-{index}")))
          .relative()
          .h_full()
          .px_3()
          .gap_2()
          .items_center()
          .rounded(theme::RADIUS)
          .cursor_pointer()
          .text_sm()
          .text_color(if active {
            theme::foreground()
          } else {
            theme::muted()
          })
          .when(active, |el| {
            el.bg(gpui::white().opacity(0.06))
              .font_weight(FontWeight::SEMIBOLD)
              .child(
                div()
                  .absolute()
                  .bottom_0()
                  .left(px(12.))
                  .right(px(12.))
                  .h(px(2.))
                  .rounded_full()
                  .bg(info.accent.base),
              )
          })
          .hover(|s| {
            s.bg(gpui::white().opacity(0.04))
              .text_color(theme::foreground())
          })
          .child(assets::color_svg(info.icon, 18., window, cx))
          .child(info.name.clone())
          .on_click(cx.listener(move |this, _, window, cx| this.activate(index, window, cx)))
      }))
  }

  /// The notifications of every space in one panel, a section per space that has some; the
  /// badge adds up their unread counts.
  fn bell(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let unseen: usize = self.spaces.iter().filter_map(|s| s.unseen(cx)).sum();
    let badge = (unseen > 0).then(|| {
      div()
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .min_w(px(16.))
        .h(px(16.))
        .px(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(if unseen > 9 {
          "9+".to_owned()
        } else {
          unseen.to_string()
        })
    });
    let spaces = self.spaces.clone();
    let trigger = Button::new("notifications-button")
      .ghost()
      .icon(Icon::Bell)
      .tooltip("Notifications");
    div()
      .relative()
      .child(
        Popover::new("notifications")
          .anchor(Anchor::TopRight)
          .trigger(trigger)
          .content(move |_, window, cx| {
            let popover = cx.entity().downgrade();
            let sections: Vec<_> = spaces
              .iter()
              .filter_map(|space| {
                let panel = space.notifications(popover.clone(), window, cx)?;
                Some((space.info(cx), panel))
              })
              .collect();
            let empty = sections.is_empty();
            v_flex()
              .w(px(NOTIFICATIONS_WIDTH))
              .gap_2()
              .child(
                div()
                  .px_2()
                  .pb_1()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child("Notifications"),
              )
              .when(empty, |el| {
                el.child(
                  div()
                    .px_2()
                    .pb_2()
                    .text_sm()
                    .text_color(theme::muted())
                    .child("Aucune notification. Celles de YouTube s'affichent une fois connecté."),
                )
              })
              .children(sections.into_iter().map(|(info, panel)| {
                v_flex()
                  .gap_1()
                  .child(
                    div()
                      .px_2()
                      .text_xs()
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(info.accent.base)
                      .child(info.name),
                  )
                  .child(panel)
              }))
          }),
      )
      .children(badge)
  }

  // --- Sidebar ------------------------------------------------------------------------------

  /// The open space's entries; folded, each shows its icon alone with its label as a tooltip.
  fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let collapsed = Settings::get(cx).sidebar_collapsed;
    let entries = self.space().map(|s| s.menu(cx)).unwrap_or_default();
    let in_settings = self.settings.is_some();
    let (bottom, top): (Vec<MenuEntry>, Vec<MenuEntry>) =
      entries.into_iter().partition(|e| e.bottom);
    let item = |entry: MenuEntry, window: &mut Window, cx: &mut Context<Self>| {
      let label = entry.label.clone();
      let id = entry.id.clone();
      let icon = match &entry.avatar {
        Some(user) => user_avatar(user, window, cx),
        None => gpui_component::Icon::new(entry.icon)
          .size(px(18.))
          .flex_none()
          .into_any_element(),
      };
      let heading = entry.heading.clone().map(|heading| {
        if collapsed {
          div()
            .mx_3()
            .my_2()
            .h(px(1.))
            .bg(theme::border())
            .into_any_element()
        } else {
          div()
            .px_3()
            .pt_4()
            .pb_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::subtle())
            .child(heading.to_uppercase())
            .into_any_element()
        }
      });
      let row = sidebar_entry(entry.id.clone(), entry.active && !in_settings, collapsed)
        .when(entry.detail.is_some() && !collapsed, |el| el.h(px(48.)))
        .child(icon)
        .map(|el| {
          if collapsed {
            el.tooltip(move |window, cx| {
              gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
            })
          } else {
            el.child(
              v_flex()
                .min_w_0()
                .child(div().truncate().child(entry.label))
                .children(entry.detail.map(|detail| {
                  div()
                    .truncate()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child(detail)
                })),
            )
          }
        })
        .on_click(cx.listener(move |this, _, window, cx| this.open_entry(id.clone(), window, cx)));
      v_flex().children(heading).child(row)
    };
    let top: Vec<_> = top.into_iter().map(|e| item(e, window, cx)).collect();
    let bottom: Vec<_> = bottom.into_iter().map(|e| item(e, window, cx)).collect();
    v_flex()
      .w(theme::sidebar_width(collapsed))
      .h_full()
      .flex_none()
      .child(
        v_flex()
          .id("sidebar")
          .flex_1()
          .min_h_0()
          .px_2()
          .pt_2()
          .gap_1()
          .overflow_y_scrollbar()
          .children(top),
      )
      .when(!bottom.is_empty(), |el| {
        el.child(
          v_flex()
            .flex_none()
            .px_2()
            .py_2()
            .gap_1()
            .border_t_1()
            .border_color(theme::border())
            .children(bottom),
        )
      })
  }
}

/// A sidebar entry: icon and label, muted until hovered or active; the active one has a light
/// background and a pill in the accent on its left edge. `collapsed`: the icon alone, centered.
fn sidebar_entry(id: SharedString, active: bool, collapsed: bool) -> gpui::Stateful<gpui::Div> {
  h_flex()
    .id(id)
    .relative()
    .flex_none()
    .h(px(40.))
    .map(|el| {
      if collapsed {
        el.justify_center()
      } else {
        el.px_3().gap_3()
      }
    })
    .items_center()
    .rounded(px(6.))
    .cursor_pointer()
    .text_sm()
    .text_color(if active {
      theme::foreground()
    } else {
      theme::muted()
    })
    .when(active, |el| {
      el.bg(gpui::white().opacity(0.06))
        .font_weight(FontWeight::MEDIUM)
        .child(
          div()
            .absolute()
            .left_0()
            .top(px(12.))
            .w(px(3.))
            .h(px(16.))
            .rounded_full()
            .bg(theme::accent()),
        )
    })
    .hover(|s| {
      s.bg(gpui::white().opacity(0.04))
        .text_color(theme::foreground())
    })
}

/// A user's avatar in the sidebar, the size of an entry's icon plus a little: their picture once
/// loaded, a disc in the accent with their initial meanwhile or without one. Scaled on the CPU
/// to its exact size: servers do not resize user pictures, and GPUI samples without mipmaps.
fn user_avatar(user: &Avatar, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
  const SIZE: f32 = 24.;
  let side = (SIZE * window.scale_factor()).round() as u32;
  let ready = user
    .picture
    .as_ref()
    .and_then(|picture| picture.url(side, side))
    .and_then(|url| {
      let images = cx.try_global::<Images>()?.0.clone();
      images.update(cx, |store, cx| store.get_sized(&url, side, side, cx))
    });
  let disc = div()
    .size(px(SIZE))
    .flex_none()
    .rounded_full()
    .flex()
    .items_center()
    .justify_center();
  match ready {
    Some(image) => disc
      .child(
        img(ImageSource::Render(image))
          .size_full()
          .rounded_full()
          .object_fit(ObjectFit::Cover),
      )
      .into_any_element(),
    None => disc
      .bg(theme::accent())
      .text_color(theme::on_accent())
      .text_xs()
      .font_weight(FontWeight::BOLD)
      .child(user.initial.clone())
      .into_any_element(),
  }
}

/// Peeby's mark: the round logo and the name.
fn mark(window: &Window, cx: &mut App) -> impl IntoElement {
  h_flex()
    .gap_2()
    .items_center()
    .child(assets::logo(22., window, cx))
    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Peeby"))
}

impl Render for Shell {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let root = v_flex()
      .size_full()
      .bg(theme::chrome(cx))
      .text_color(theme::foreground());
    let page = self
      .settings
      .clone()
      .or_else(|| self.space().map(|s| s.view()));
    // Fullscreen belongs to the player: no top bar, no sidebar.
    if window.is_fullscreen() {
      return root.children(page);
    }
    root.child(self.top_bar(window, cx)).child(
      h_flex()
        .flex_1()
        .min_h_0()
        .items_start()
        .child(self.sidebar(window, cx))
        .child(
          div()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded_tl(theme::RADIUS_LG)
            .border_t_1()
            .border_l_1()
            .border_color(theme::border())
            .bg(theme::background())
            .overflow_hidden()
            .children(page),
        ),
    )
  }
}
