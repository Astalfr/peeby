//! The notifications panel behind the bell: a tab per space that has notifications to show
//! (`Space::unseen` is `Some`), as the notification area's list of lives has them, each with its
//! unread count; the tab shown is the space's own section (`Space::notifications`), built only
//! while it shows, so that a space sees its notifications only once they are on screen.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, ElementId, FontWeight, Global, SharedString, Subscription, WeakEntity,
  Window, div, px,
};
use gpui_component::popover::PopoverState;
use gpui_component::v_flex;
use ui::i18n::t;
use ui::theme;

use super::space::SpaceHandle;
use super::space_tabs::{self, OnSelect, SpaceTab};

/// The tab shown last, by its space's name, for the next opening.
#[derive(Default)]
struct LastTab(Option<SharedString>);

impl Global for LastTab {}

/// The panel's content: made when the panel opens (the popover's element state), on `spaces`.
pub fn panel(
  spaces: Vec<SpaceHandle>,
  popover: WeakEntity<PopoverState>,
  width: f32,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let view = window.use_keyed_state(ElementId::Name("bell-panel".into()), cx, |_, cx| {
    BellPanel::new(spaces, popover, cx)
  });
  div().w(px(width)).child(view).into_any_element()
}

struct BellPanel {
  spaces: Vec<SpaceHandle>,
  popover: WeakEntity<PopoverState>,
  /// The space whose tab shows, by name.
  tab: SharedString,
  /// Where the tab shown came from: -1 from a tab on its left, 1 on its right, 0 at the start.
  came_from: f32,
  _subscriptions: Vec<Subscription>,
}

impl BellPanel {
  fn new(
    spaces: Vec<SpaceHandle>,
    popover: WeakEntity<PopoverState>,
    cx: &mut Context<Self>,
  ) -> Self {
    let tabs = tabs(&spaces, cx);
    let remembered = cx.try_global::<LastTab>().and_then(|l| l.0.clone());
    let tab = first_tab(&tabs, remembered.as_ref()).unwrap_or_default();
    // The counts follow the spaces while the panel is open.
    let subscriptions = spaces
      .iter()
      .map(|space| {
        let view = cx.weak_entity();
        space.observe(
          cx,
          Box::new(move |cx| {
            view.update(cx, |_, cx| cx.notify()).ok();
          }),
        )
      })
      .collect();
    Self {
      spaces,
      popover,
      tab,
      came_from: 0.,
      _subscriptions: subscriptions,
    }
  }

  fn select(&mut self, tab: SharedString, cx: &mut Context<Self>) {
    if tab == self.tab {
      return;
    }
    self.came_from = space_tabs::direction(&tabs(&self.spaces, cx), &self.tab, &tab);
    cx.default_global::<LastTab>().0 = Some(tab.clone());
    self.tab = tab;
    cx.notify();
  }
}

impl Render for BellPanel {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let tabs = tabs(&self.spaces, cx);
    // A space may have signed out while the panel was open.
    if !tabs.iter().any(|t| t.info.name == self.tab) {
      self.tab = first_tab(&tabs, None).unwrap_or_default();
      self.came_from = 0.;
    }
    let header = div()
      .px_2()
      .pb_1()
      .font_weight(FontWeight::SEMIBOLD)
      .child(t("Notifications", "Notifications"));
    if tabs.is_empty() {
      return v_flex()
        .gap_2()
        .child(header)
        .child(
          div()
            .px_2()
            .pb_2()
            .text_sm()
            .text_color(theme::muted())
            .child(t(
              "No notifications. YouTube's and Twitch's appear once signed in.",
              "Aucune notification. Celles de YouTube et de Twitch s'affichent une fois connecté.",
            )),
        )
        .into_any_element();
    }
    let view = cx.weak_entity();
    let on_select: OnSelect = Rc::new(move |tab, _, cx| {
      view.update(cx, |this, cx| this.select(tab, cx)).ok();
    });
    let bar = space_tabs::bar("bell", &tabs, &self.tab, on_select, window, cx);
    let content = self
      .spaces
      .iter()
      .find(|s| s.info(cx).name == self.tab)
      .and_then(|space| space.notifications(self.popover.clone(), window, cx))
      .unwrap_or_else(|| div().into_any_element());
    v_flex()
      .gap_2()
      .child(header)
      .child(div().border_b_1().border_color(theme::border()).child(bar))
      .child(space_tabs::sliding(
        "bell",
        &self.tab,
        self.came_from,
        content,
        cx,
      ))
      .into_any_element()
  }
}

/// A tab per space that has notifications, with its unread count when it has some.
fn tabs(spaces: &[SpaceHandle], cx: &App) -> Vec<SpaceTab> {
  spaces
    .iter()
    .filter_map(|space| {
      let unseen = space.unseen(cx)?;
      Some(SpaceTab {
        info: space.info(cx),
        count: (unseen > 0).then_some(unseen),
      })
    })
    .collect()
}

/// The tab the panel opens on: the one shown last while it has unread notifications or no tab
/// has any, else the first with some, else the first.
fn first_tab(tabs: &[SpaceTab], remembered: Option<&SharedString>) -> Option<SharedString> {
  let remembered = remembered.and_then(|name| tabs.iter().find(|t| t.info.name == *name));
  let unread = tabs.iter().find(|t| t.count.is_some());
  match (remembered, unread) {
    (Some(last), _) if last.count.is_some() => Some(last.info.name.clone()),
    (_, Some(unread)) => Some(unread.info.name.clone()),
    (Some(last), None) => Some(last.info.name.clone()),
    (None, None) => tabs.first().map(|t| t.info.name.clone()),
  }
}
