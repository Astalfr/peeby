//! The multistream (ADR 0033, docs/twitch.md "Multistream"): up to [`MAX_LIVES`] lives at once,
//! from the sidebar's "Multistream" entry and nowhere else. The main live fills the page with its
//! whole player; the others play in a row under it, muted and in a small quality. A click on one
//! makes it the main live (the lives swap their places, no player restarts but for its quality);
//! the row's "+" opens [`AddLive`]: the followed channels on air, or a search of the whole of
//! Twitch.
//!
//! Each live is a [`WatchView`] tile: its own playback, chat, points and PubSub. Only the main one
//! has the sound, the mini-player, its recording's past, and earns points (minutes watched, the
//! bonus); the others' points can still be spent from their chat.
//!
//! The chat column shows a tab per channel (with its unread lines), or every chat merged into
//! one (`[twitch] multistream_merged`): each channel's panel mirrors its lines into the merged one
//! (`ui::chat::ChatPanel::set_mirror`), marked with the channel's picture and color; a channel's
//! eye leaves its lines out. The merged chat writes to the chosen channel: its field, emotes,
//! points and pinned items are that channel's, and what is sent leaves through its writer.
//!
//! The lineup is kept by the space while Peeby runs ([`super::TwitchSpace`]'s `multistream`), so
//! that coming back to the entry finds the same lives.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, ElementId, Entity, EntityId, EventEmitter, FontWeight, Hsla,
  KeyDownEvent, MouseButton, SharedString, Subscription, Task, Window, div, px, relative, rgb,
};
use gpui_component::IconName;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::tooltip::Tooltip;
use gpui_component::{h_flex, v_flex};
use peeby_core::settings::Settings;
use peeby_core::shell::space::{Lives, Space};
use std::rc::Rc;
use twitch::catalog::Catalog;
use ui::assets::Icon;
use ui::chat::{CHAT_WIDTH, ChatPanel, ChatPanelEvent, ChatSource, Mirror};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::{CornerAction, PlayerEvent};
use ui::{tf, theme};

use super::browse::load;
use super::watch::{WatchEvent, WatchView};
use super::{Nav, Route, cards, chat};

/// Lives played at once: the main one and three in the row.
pub const MAX_LIVES: usize = 4;
/// Height of the row of secondary lives.
const ROW_HEIGHT: f32 = 140.;
/// Each channel's color in the merged chat, by the first one free.
const COLORS: [u32; MAX_LIVES] = [0x9146ff, 0x1dd1a1, 0xff9f43, 0x48dbfb];
/// The search runs this long after the last key.
const SEARCH_DELAY: Duration = Duration::from_millis(350);
/// Size of a chat tab's picture.
const TAB_AVATAR: f32 = 18.;

/// One live of the multistream.
struct Tile {
  view: Entity<WatchView>,
  /// Its lines' color in the merged chat.
  color: Hsla,
  /// Its chat's counts (received, mentions) when its tab last showed: what is unread.
  seen: (u64, u64),
  /// The merged chat has its picture (given once its header is read).
  mirrored_avatar: bool,
  _subscriptions: Vec<Subscription>,
}

pub struct MultistreamView {
  nav: Nav,
  client: twitch::Client,
  catalog: Catalog,
  /// The main live first, then the row's in their order.
  tiles: Vec<Tile>,
  /// The merged chat, fed by the tiles' panels.
  merged: Entity<ChatPanel>,
  /// The chat tab shown, and the channel the merged chat writes to, by login.
  chat: Option<String>,
  /// Keeps the merged chat writing as the chosen channel's panel does.
  _writing: Option<Subscription>,
  /// The "Add a live" dialog, while open.
  adding: Option<Entity<AddLive>>,
  /// The main player last given the keyboard's focus.
  focused: Option<EntityId>,
}

impl MultistreamView {
  /// The multistream playing `lineup` (the main live first): the one the space kept, or none.
  pub fn new(
    nav: Nav,
    client: twitch::Client,
    catalog: Catalog,
    lineup: Vec<String>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let merged = cx.new(|cx| {
      ChatPanel::new(cx)
        .with_composer(twitch::chat::MAX_CHARS)
        .titled(t("Merged chats", "Chats fusionnés"))
    });
    let viewer_nav = nav.clone();
    let viewer: ui::chat::Viewer = Rc::new(move |cx: &App| {
      viewer_nav
        .upgrade()
        .and_then(|nav| nav.read(cx).account().map(|a| a.login.clone().into()))
    });
    let lookup = chat::user_lookup(catalog.clone());
    merged.update(cx, |panel, cx| {
      panel.set_viewer(viewer);
      panel.set_user_lookup(lookup);
      // Nothing to write to until a channel is chosen.
      panel.set_composing(
        ui::chat::Composing::Waiting(t("Connecting to chat…", "Connexion au chat…").into()),
        cx,
      );
    });
    cx.subscribe(&merged, |this, _, event: &ChatPanelEvent, cx| match event {
      // Sent through the chosen channel's panel, whose writer sends it.
      ChatPanelEvent::Send(text) => {
        if let Some(panel) = this.chosen_panel(cx) {
          let event = ChatPanelEvent::Send(text.clone());
          panel.update(cx, |_, cx| cx.emit(event));
        }
      }
      ChatPanelEvent::OpenChannel { channel, name } => {
        let route = Route::Channel(channel.to_string());
        this.nav.open_in_new_tab(route, Some(name.clone()), cx);
      }
      ChatPanelEvent::Option(_) | ChatPanelEvent::Hide => {}
    })
    .detach();
    let mut view = Self {
      nav,
      client,
      catalog,
      tiles: Vec::new(),
      merged,
      chat: None,
      _writing: None,
      adding: None,
      focused: None,
    };
    for login in lineup {
      view.add(login, cx);
    }
    view
  }

  /// The main live's player, for the shell's mini-player.
  pub fn player(&self, cx: &App) -> Option<Entity<ui::player::PlayerView>> {
    self.tiles.first().map(|tile| tile.view.read(cx).player())
  }

  /// Another page covers the multistream (every live pauses), or it shows again: the row's lives
  /// play on by themselves, the main one waits for the viewer as any player does.
  pub fn set_covered(&mut self, covered: bool, cx: &mut Context<Self>) {
    for (index, tile) in self.tiles.iter().enumerate() {
      let player = tile.view.read(cx).player();
      player.update(cx, |player, cx| {
        player.set_covered(covered);
        if !covered && index > 0 && player.is_paused() {
          player.toggle_pause(cx);
        }
      });
    }
  }

  fn logins(&self, cx: &App) -> Vec<String> {
    self
      .tiles
      .iter()
      .map(|tile| tile.view.read(cx).login())
      .collect()
  }

  fn position(&self, login: &str, cx: &App) -> Option<usize> {
    self
      .tiles
      .iter()
      .position(|tile| tile.view.read(cx).login() == login)
  }

  /// Adds `login`'s live: the main one when it is the first, else at the row's end.
  fn add(&mut self, login: String, cx: &mut Context<Self>) {
    let login = login.to_ascii_lowercase();
    if self.tiles.len() >= MAX_LIVES || self.position(&login, cx).is_some() {
      return;
    }
    let main = self.tiles.is_empty();
    let tile = self.make_tile(login.clone(), main, cx);
    self.tiles.push(tile);
    if self.chat.is_none() {
      self.choose_chat(login, cx);
    }
    self.lineup_changed(cx);
  }

  fn make_tile(&mut self, login: String, main: bool, cx: &mut Context<Self>) -> Tile {
    let (client, catalog, nav) = (self.client.clone(), self.catalog.clone(), self.nav.clone());
    let view = cx.new(|cx| WatchView::tile(login.clone(), client, catalog, nav, main, cx));
    let taken: Vec<Hsla> = self.tiles.iter().map(|tile| tile.color).collect();
    let color = COLORS
      .iter()
      .map(|&c| Hsla::from(rgb(c)))
      .find(|c| !taken.contains(c))
      .unwrap_or_else(|| rgb(COLORS[0]).into());
    let mut subscriptions = vec![
      cx.subscribe(&view, |this, view, event: &WatchEvent, cx| {
        let WatchEvent::Raid(to) = event;
        this.follow_raid(&view, to.clone(), cx);
      }),
      // Its header, read later, names its tile and its tab.
      cx.observe(&view, |_, _, cx| cx.notify()),
    ];
    let player = view.read(cx).player();
    // Its own page switches theater mode and its player; the others' corners follow.
    subscriptions.push(cx.subscribe(&player, |this, _, event: &PlayerEvent, cx| {
      if matches!(event, PlayerEvent::Theater) {
        let theater = Settings::get(cx).theater;
        for tile in &this.tiles {
          let player = tile.view.read(cx).player();
          player.update(cx, |player, cx| player.set_theater(theater, cx));
        }
        cx.notify();
      }
    }));
    // The main player's corner leaves the multistream (the row's have their own button).
    let (page, me) = (cx.weak_entity(), view.downgrade());
    let action = CornerAction {
      icon: Icon::Close,
      tooltip: t("Remove from the multistream", "Retirer du multistream").into(),
      on_click: Rc::new(move |_, cx| {
        let (page, me) = (page.clone(), me.clone());
        cx.defer(move |cx| {
          page
            .update(cx, |this, cx| {
              if let Some(index) = this.tiles.iter().position(|t| t.view.downgrade() == me) {
                this.remove(index, cx);
              }
            })
            .ok();
        });
      }),
    };
    player.update(cx, |player, cx| player.set_corner_action(Some(action), cx));
    if let Some(panel) = view.read(cx).chat() {
      let mirror = Mirror {
        panel: self.merged.downgrade(),
        source: Rc::new(ChatSource {
          key: login.clone().into(),
          name: login.into(),
          color,
          avatar: None,
        }),
      };
      panel.update(cx, |panel, _| panel.set_mirror(Some(mirror)));
      // Its unread lines, on its tab.
      subscriptions.push(cx.observe(&panel, |_, _, cx| cx.notify()));
    }
    Tile {
      view,
      color,
      seen: (0, 0),
      mirrored_avatar: false,
      _subscriptions: subscriptions,
    }
  }

  /// Makes the live at `index` of the row the main one: it and the main one swap their places.
  fn promote(&mut self, index: usize, cx: &mut Context<Self>) {
    if index == 0 || index >= self.tiles.len() {
      return;
    }
    self.tiles.swap(0, index);
    // The old main one first: the new one must not find it playing in the window.
    let (old, new) = (self.tiles[index].view.clone(), self.tiles[0].view.clone());
    old.update(cx, |view, cx| view.set_main(false, cx));
    new.update(cx, |view, cx| view.set_main(true, cx));
    if Settings::get(cx).twitch.multistream_chat_follows != Some(false) {
      let login = new.read(cx).login();
      self.choose_chat(login, cx);
    }
    self.lineup_changed(cx);
  }

  /// Removes the live at `index`; the main one's place goes to the first of the row.
  fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
    if index >= self.tiles.len() {
      return;
    }
    let gone = self.tiles.remove(index).view.read(cx).login();
    if index == 0
      && let Some(first) = self.tiles.first()
    {
      first.view.update(cx, |view, cx| view.set_main(true, cx));
    }
    if self.chat.as_deref() == Some(gone.as_str()) {
      self.chat = None;
      self._writing = None;
      if let Some(login) = self.tiles.first().map(|t| t.view.read(cx).login()) {
        self.choose_chat(login, cx);
      }
    }
    self.lineup_changed(cx);
  }

  /// The raid a tile's viewer follows has gone to `to`: the tile plays it, in the same place.
  fn follow_raid(&mut self, view: &Entity<WatchView>, to: String, cx: &mut Context<Self>) {
    let Some(index) = self.tiles.iter().position(|t| &t.view == view) else {
      return;
    };
    let to = to.to_ascii_lowercase();
    if self.position(&to, cx).is_some() {
      return self.remove(index, cx);
    }
    let from = view.read(cx).login();
    let tile = self.make_tile(to.clone(), index == 0, cx);
    self.tiles[index] = tile;
    if self.chat.as_deref() == Some(from.as_str()) {
      self.choose_chat(to, cx);
    }
    self.lineup_changed(cx);
  }

  /// Shows `login`'s chat tab, and makes the merged chat write to it.
  fn choose_chat(&mut self, login: String, cx: &mut Context<Self>) {
    let Some(index) = self.position(&login, cx) else {
      return;
    };
    self.chat = Some(login);
    let Some(panel) = self.tiles[index].view.read(cx).chat() else {
      return;
    };
    let writing = panel.read(cx).writing();
    self
      .merged
      .update(cx, |merged, cx| merged.write_like(writing, cx));
    self._writing = Some(cx.observe(&panel, |this, panel, cx| {
      let writing = panel.read(cx).writing();
      this
        .merged
        .update(cx, |merged, cx| merged.write_like(writing, cx));
    }));
    cx.notify();
  }

  /// The chosen channel's chat panel.
  fn chosen_panel(&self, cx: &App) -> Option<Entity<ChatPanel>> {
    let index = self.position(self.chat.as_deref()?, cx)?;
    self.tiles[index].view.read(cx).chat()
  }

  /// Keeps the lineup in the space, for the next time the entry opens.
  fn lineup_changed(&mut self, cx: &mut Context<Self>) {
    let logins = self.logins(cx);
    self
      .nav
      .update(cx, |space, _| space.multistream = logins)
      .ok();
    cx.notify();
  }

  fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.tiles.len() >= MAX_LIVES {
      return;
    }
    let followed = self
      .nav
      .upgrade()
      .and_then(|space| space.read(cx).lives(cx));
    let taken = self.logins(cx);
    let (nav, catalog) = (self.nav.clone(), self.catalog.clone());
    let dialog = cx.new(|cx| AddLive::new(nav, catalog, followed, taken, window, cx));
    cx.subscribe(&dialog, |this, _, event: &AddLiveEvent, cx| {
      this.adding = None;
      // Back to the main player's keys.
      this.focused = None;
      if let AddLiveEvent::Pick(login) = event {
        this.add(login.clone(), cx);
      }
      cx.notify();
    })
    .detach();
    self.adding = Some(dialog);
    cx.notify();
  }

  /// A live's tile in the row, at `index` of the lineup.
  fn render_tile(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
    let tile = &self.tiles[index];
    let summary = tile.view.read(cx).summary();
    let player = tile.view.read(cx).player();
    let group: SharedString = format!("multistream-tile-{index}").into();
    let close = div()
      .id(ElementId::NamedInteger(
        "multistream-tile-close".into(),
        index as u64,
      ))
      .size(px(24.))
      .flex_none()
      .rounded_full()
      .flex()
      .items_center()
      .justify_center()
      .bg(gpui::black().opacity(0.6))
      .hover(|el| el.bg(gpui::black().opacity(0.85)))
      .text_color(gpui::white())
      .child(gpui_component::Icon::new(Icon::Close).size(px(14.)))
      .tooltip(|window, cx| {
        Tooltip::new(t("Remove from the multistream", "Retirer du multistream")).build(window, cx)
      })
      // Not a click on the tile, which would make it the main live.
      .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
      .on_click(cx.listener(move |this, _, _, cx| this.remove(index, cx)));
    let overlay = v_flex()
      .id(ElementId::NamedInteger(
        "multistream-tile".into(),
        index as u64,
      ))
      .absolute()
      .inset_0()
      .p_2()
      .justify_between()
      .cursor_pointer()
      .opacity(0.)
      .bg(gpui::black().opacity(0.35))
      .group_hover(group.clone(), |el| el.opacity(1.))
      .on_click(cx.listener(move |this, _, _, cx| this.promote(index, cx)))
      .child(
        h_flex()
          .gap_2()
          .items_start()
          .justify_between()
          .child(
            div()
              .min_w_0()
              .text_sm()
              .font_weight(FontWeight::SEMIBOLD)
              .text_color(gpui::white())
              .truncate()
              .child(summary.name.clone()),
          )
          .child(close),
      )
      .child(
        h_flex()
          .gap_1()
          .items_center()
          .text_xs()
          .text_color(gpui::white().opacity(0.85))
          .child(gpui_component::Icon::new(Icon::Maximize).size(px(12.)))
          .child(
            div()
              .min_w_0()
              .truncate()
              .child(summary.line.unwrap_or_else(|| {
                t(
                  "Click to watch it here",
                  "Cliquer pour le regarder en grand",
                )
                .into()
              })),
          ),
      );
    div()
      .group(group)
      .relative()
      .h_full()
      .flex_1()
      .min_w_0()
      .max_w(px(ROW_HEIGHT * 16. / 9.))
      .rounded(theme::RADIUS)
      .overflow_hidden()
      .bg(gpui::black())
      .border_b_2()
      .border_color(tile.color)
      .child(div().absolute().inset_0().child(player))
      .child(overlay)
      .into_any_element()
  }

  /// The row's "+": opens the "Add a live" dialog.
  fn render_add_tile(&self, cx: &mut Context<Self>) -> AnyElement {
    v_flex()
      .id("multistream-add")
      .h_full()
      .flex_1()
      .min_w_0()
      .max_w(px(ROW_HEIGHT * 16. / 9.))
      .rounded(theme::RADIUS)
      .border_1()
      .border_color(theme::border())
      .items_center()
      .justify_center()
      .gap_1()
      .cursor_pointer()
      .text_color(theme::muted())
      .hover(|el| {
        el.bg(theme::surface_hover())
          .text_color(theme::foreground())
      })
      .on_click(cx.listener(|this, _, window, cx| this.open_add(window, cx)))
      .child(gpui_component::Icon::new(Icon::Plus).size(px(28.)))
      .child(div().text_sm().child(t("Add a live", "Ajouter un live")))
      .into_any_element()
  }

  /// The page before any live: one big "+".
  fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
    v_flex()
      .size_full()
      .items_center()
      .justify_center()
      .gap_4()
      .child(
        div()
          .id("multistream-first")
          .size(px(96.))
          .rounded_full()
          .flex()
          .items_center()
          .justify_center()
          .cursor_pointer()
          .bg(theme::accent_soft())
          .text_color(theme::accent())
          .hover(|el| el.bg(theme::accent()).text_color(theme::on_accent()))
          .on_click(cx.listener(|this, _, window, cx| this.open_add(window, cx)))
          .child(gpui_component::Icon::new(Icon::Plus).size(px(44.))),
      )
      .child(
        div()
          .text_lg()
          .font_weight(FontWeight::SEMIBOLD)
          .child(t("Multistream", "Multistream")),
      )
      .child(div().text_sm().text_color(theme::muted()).child(tf!(
        "Watch up to {MAX_LIVES} lives at once: add the first one.",
        "Regardez jusqu'à {MAX_LIVES} lives en même temps : ajoutez le premier."
      )))
      .into_any_element()
  }

  /// The chat column: the channels' tabs (or the merged chat's channels), then the chat.
  fn render_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let merged_mode = Settings::get(cx).twitch.multistream_merged == Some(true);
    let mut tabs = h_flex().flex_1().min_w_0().gap_1().flex_wrap();
    for index in 0..self.tiles.len() {
      tabs = tabs.child(self.render_chat_tab(index, merged_mode, window, cx));
    }
    let toggle = div()
      .id("multistream-chat-mode")
      .size(px(28.))
      .flex_none()
      .rounded(theme::RADIUS)
      .flex()
      .items_center()
      .justify_center()
      .cursor_pointer()
      .text_color(theme::muted())
      .hover(|el| {
        el.bg(theme::surface_hover())
          .text_color(theme::foreground())
      })
      .child(
        gpui_component::Icon::new(if merged_mode {
          Icon::List
        } else {
          Icon::Comments
        })
        .size(px(16.)),
      )
      .tooltip(move |window, cx| {
        Tooltip::new(if merged_mode {
          t("One tab per chat", "Un onglet par chat")
        } else {
          t("Merge the chats", "Fusionner les chats")
        })
        .build(window, cx)
      })
      .on_click(cx.listener(move |_, _, _, cx| {
        Settings::update(cx, |s| s.twitch.multistream_merged = Some(!merged_mode));
        cx.notify();
      }));
    let header = h_flex()
      .flex_none()
      .gap_2()
      .px_2()
      .py_2()
      .items_start()
      .border_b_1()
      .border_color(theme::border())
      .child(tabs)
      .child(toggle);
    let body = if merged_mode {
      Some(self.merged.clone().into_any_element())
    } else {
      self.chosen_panel(cx).map(|panel| panel.into_any_element())
    };
    v_flex()
      .size_full()
      .child(header)
      .child(div().flex_1().min_h_0().children(body))
      .into_any_element()
  }

  /// A channel's tab over the chat: its picture and name; in tabs, its unread lines (red when
  /// they mention the viewer); merged, whether it is the one written to, and its eye.
  fn render_chat_tab(
    &mut self,
    index: usize,
    merged_mode: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let tile = &self.tiles[index];
    let view = tile.view.read(cx);
    let (summary, login, panel) = (view.summary(), view.login(), view.chat());
    let chosen = self.chat.as_deref() == Some(login.as_str());
    let counts = panel
      .as_ref()
      .map(|p| p.read(cx).received())
      .unwrap_or_default();
    // The tab shown is read as it comes.
    if chosen && !merged_mode {
      self.tiles[index].seen = counts;
    }
    let seen = self.tiles[index].seen;
    let unread = !chosen && !merged_mode && counts.0 > seen.0;
    let mentioned = !chosen && !merged_mode && counts.1 > seen.1;
    let hidden = merged_mode && self.merged.read(cx).is_hidden(&login);
    let color = self.tiles[index].color;
    let initial: SharedString = summary
      .name
      .chars()
      .next()
      .map(|c| c.to_uppercase().to_string())
      .unwrap_or_default()
      .into();
    let face = match &summary.avatar {
      Some(url) => picture::avatar(&cards::template(url), TAB_AVATAR, window, cx),
      None => div()
        .size(px(TAB_AVATAR))
        .rounded_full()
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(gpui::white())
        .child(initial)
        .into_any_element(),
    };
    let eye = merged_mode.then(|| {
      let key: SharedString = login.clone().into();
      let merged = self.merged.clone();
      div()
        .id(ElementId::NamedInteger(
          "multistream-eye".into(),
          index as u64,
        ))
        .flex_none()
        .text_color(theme::muted())
        .hover(|el| el.text_color(theme::foreground()))
        .child(
          gpui_component::Icon::new(if hidden {
            IconName::EyeOff
          } else {
            IconName::Eye
          })
          .size(px(14.)),
        )
        .tooltip(move |window, cx| {
          Tooltip::new(if hidden {
            t("Show its messages", "Afficher ses messages")
          } else {
            t("Hide its messages", "Masquer ses messages")
          })
          .build(window, cx)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| {
          merged.update(cx, |merged, cx| merged.set_hidden(key.clone(), !hidden, cx));
        })
    });
    let tip = if merged_mode {
      tf!("Write to {}", "Écrire à {}", summary.name)
    } else {
      summary.name.clone()
    };
    h_flex()
      .id(ElementId::NamedInteger(
        "multistream-chat-tab".into(),
        index as u64,
      ))
      .gap_1p5()
      .pl_1()
      .pr_2()
      .py_1()
      .rounded_full()
      .cursor_pointer()
      .border_1()
      .border_color(if chosen { color } else { theme::border() })
      .when(chosen, |el| el.bg(theme::surface_hover()))
      .when(hidden, |el| el.opacity(0.5))
      .hover(|el| el.bg(theme::surface_hover()))
      .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
      .on_click(cx.listener(move |this, _, _, cx| this.choose_chat(login.clone(), cx)))
      .child(face)
      .child(
        div()
          .max_w(px(72.))
          .truncate()
          .text_xs()
          .font_weight(FontWeight::MEDIUM)
          .child(summary.name),
      )
      .when(unread, |el| {
        el.child(
          div()
            .size(px(7.))
            .flex_none()
            .rounded_full()
            .bg(if mentioned {
              theme::mention()
            } else {
              theme::muted()
            }),
        )
      })
      .children(eye)
      .into_any_element()
  }

  /// Gives the merged chat each channel's picture once its header is read.
  fn mirror_avatars(&mut self, cx: &mut Context<Self>) {
    let merged = self.merged.downgrade();
    for tile in &mut self.tiles {
      if tile.mirrored_avatar {
        continue;
      }
      let view = tile.view.read(cx);
      let (summary, login) = (view.summary(), view.login());
      let (Some(url), Some(panel)) = (summary.avatar.clone(), view.chat()) else {
        continue;
      };
      tile.mirrored_avatar = true;
      let mirror = Mirror {
        panel: merged.clone(),
        source: Rc::new(ChatSource {
          key: login.into(),
          name: summary.name.into(),
          color: tile.color,
          avatar: Some(Picture::sized(move |_, _| url.clone())),
        }),
      };
      panel.update(cx, |panel, _| panel.set_mirror(Some(mirror)));
    }
  }
}

impl Render for MultistreamView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.mirror_avatars(cx);
    let main = self.tiles.first().map(|tile| tile.view.read(cx).player());
    // The keyboard drives the main live's player.
    if let Some(player) = &main
      && self.focused != Some(player.entity_id())
      && self.adding.is_none()
    {
      self.focused = Some(player.entity_id());
      let focus = player.read(cx).focus_handle().clone();
      window.focus(&focus, cx);
    }
    if window.is_fullscreen()
      && let Some(player) = main.clone()
    {
      return div().size_full().child(player).into_any_element();
    }
    let theater = Settings::get(cx).theater;
    let content = match main {
      None => self.render_empty(cx),
      Some(player) => {
        let mut row = h_flex().w_full().flex_none().h(px(ROW_HEIGHT)).gap_3();
        for index in 1..self.tiles.len() {
          row = row.child(self.render_tile(index, cx));
        }
        if self.tiles.len() < MAX_LIVES {
          row = row.child(self.render_add_tile(cx));
        }
        v_flex()
          .size_full()
          .gap_3()
          .when(!theater, |el| el.p_6())
          .when(theater, |el| el.pb_3())
          .child(
            div()
              .flex_1()
              .min_h_0()
              .w_full()
              .relative()
              .when(!theater, |el| el.rounded(theme::RADIUS_LG))
              .when(theater, |el| el.rounded_tl(theme::PANEL_INNER_RADIUS))
              .overflow_hidden()
              .bg(gpui::black())
              .child(div().absolute().inset_0().child(player)),
          )
          .child(div().w_full().when(theater, |el| el.px_3()).child(row))
          .into_any_element()
      }
    };
    let chat = (!self.tiles.is_empty()).then(|| self.render_chat(window, cx));
    let chat = chat.map(|chat| {
      let column = div().w(px(CHAT_WIDTH)).flex_none().h_full();
      if theater {
        column
          .bg(theme::surface())
          .border_l_1()
          .border_color(theme::border())
          .child(chat)
      } else {
        column.py_6().pr_6().child(
          div()
            .size_full()
            .rounded(theme::RADIUS_LG)
            .bg(theme::surface())
            .border_1()
            .border_color(theme::border())
            .overflow_hidden()
            .child(chat),
        )
      }
    });
    h_flex()
      .size_full()
      .relative()
      .child(div().flex_1().min_w_0().h_full().child(content))
      .children(chat)
      .children(self.adding.clone())
      .into_any_element()
  }
}

/// What the "Add a live" dialog tells the multistream.
pub enum AddLiveEvent {
  /// Add this channel's live (a login).
  Pick(String),
  /// Closed without a choice.
  Close,
}

/// A live the dialog offers.
#[derive(Clone)]
struct Candidate {
  login: String,
  name: SharedString,
  avatar: Option<Picture>,
  /// The category, the viewers.
  detail: Option<SharedString>,
}

/// The "Add a live" dialog, over the multistream: the followed channels on air (the sidebar's,
/// with Helix), or, once something is typed, the lives of the whole of Twitch whose channel
/// matches (GQL `searchUsers`, on air only). The lives already in the multistream are greyed.
pub struct AddLive {
  nav: Nav,
  catalog: Catalog,
  input: Entity<InputState>,
  /// The followed channels on air, or why they are not there.
  followed: Result<Vec<Candidate>, SharedString>,
  /// The logins already playing.
  taken: Vec<String>,
  /// The search's answer for `query`.
  results: Option<Result<Vec<Candidate>, String>>,
  query: String,
  searching: bool,
  _search: Option<Task<()>>,
  _input: Subscription,
}

impl EventEmitter<AddLiveEvent> for AddLive {}

impl AddLive {
  fn new(
    nav: Nav,
    catalog: Catalog,
    followed: Option<Lives>,
    taken: Vec<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let input = cx.new(|cx| {
      InputState::new(window, cx).placeholder(t(
        "Search a live on Twitch",
        "Rechercher un live sur Twitch",
      ))
    });
    input.update(cx, |state, cx| state.focus(window, cx));
    let subscription = cx.subscribe_in(
      &input,
      window,
      |this: &mut Self, input, event: &InputEvent, _, cx| match event {
        InputEvent::Change => {
          let query = input.read(cx).value().trim().to_owned();
          this.schedule(query, cx);
        }
        InputEvent::PressEnter { .. } => {
          let query = input.read(cx).value().trim().to_owned();
          this.search(query, cx);
        }
        _ => {}
      },
    );
    let followed = match followed {
      Some(Lives::On(lives)) => Ok(
        lives
          .into_iter()
          .map(|live| Candidate {
            login: live.id.to_string(),
            name: live.channel,
            avatar: live.avatar.picture,
            detail: match (live.detail, live.viewers) {
              (Some(detail), Some(viewers)) => Some(format!("{detail} · {viewers}").into()),
              (detail, viewers) => detail.or(viewers),
            },
          })
          .collect(),
      ),
      Some(Lives::Loading) => Err(t("Loading…", "Chargement…").into()),
      Some(Lives::Unavailable(why)) => Err(why),
      None => Err(SharedString::default()),
    };
    Self {
      nav,
      catalog,
      input,
      followed,
      taken,
      results: None,
      query: String::new(),
      searching: false,
      _search: None,
      _input: subscription,
    }
  }

  /// Searches `query` once the keys stop for [`SEARCH_DELAY`]; an empty one shows the followed
  /// channels again.
  fn schedule(&mut self, query: String, cx: &mut Context<Self>) {
    if query == self.query {
      return;
    }
    if query.is_empty() {
      self.query = query;
      self.results = None;
      self.searching = false;
      self._search = None;
      return cx.notify();
    }
    self._search = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SEARCH_DELAY).await;
      this.update(cx, |this, cx| this.search(query, cx)).ok();
    }));
  }

  fn search(&mut self, query: String, cx: &mut Context<Self>) {
    if query.is_empty() || (query == self.query && self.results.is_some()) {
      return;
    }
    self.query = query.clone();
    self.searching = true;
    cx.notify();
    let catalog = self.catalog.clone();
    self._search = Some(load(
      cx,
      async move { catalog.search_channels(&query).await },
      |this: &mut Self, result, cx| {
        this.searching = false;
        this.results = Some(result.map(|channels| {
          channels
            .into_iter()
            .filter(|c| c.is_live)
            .map(|c| Candidate {
              avatar: (!c.thumbnail_url.is_empty()).then(|| cards::template(&c.thumbnail_url)),
              detail: (!c.game_name.is_empty()).then(|| c.game_name.into()),
              login: c.login,
              name: c.display_name.into(),
            })
            .collect()
        }));
        cx.notify();
      },
      self.nav.clone(),
    ));
  }

  fn render_row(
    &self,
    at: usize,
    candidate: &Candidate,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let taken = self
      .taken
      .iter()
      .any(|l| l.eq_ignore_ascii_case(&candidate.login));
    let initial: SharedString = candidate
      .name
      .chars()
      .next()
      .map(|c| c.to_uppercase().to_string())
      .unwrap_or_default()
      .into();
    let face = match &candidate.avatar {
      Some(avatar) => picture::avatar(avatar, 36., window, cx),
      None => div()
        .size(px(36.))
        .flex_none()
        .rounded_full()
        .bg(theme::accent_soft())
        .flex()
        .items_center()
        .justify_center()
        .child(initial)
        .into_any_element(),
    };
    let login = candidate.login.clone();
    h_flex()
      .id(ElementId::NamedInteger(
        "multistream-candidate".into(),
        at as u64,
      ))
      .gap_3()
      .p_2()
      .rounded(theme::RADIUS)
      .when(!taken, |el| {
        el.cursor_pointer()
          .hover(|el| el.bg(theme::surface_hover()))
          .on_click(cx.listener(move |_, _, _, cx| cx.emit(AddLiveEvent::Pick(login.clone()))))
      })
      .when(taken, |el| el.opacity(0.5))
      .child(face)
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .child(
            div()
              .truncate()
              .font_weight(FontWeight::SEMIBOLD)
              .child(candidate.name.clone()),
          )
          .children(candidate.detail.clone().map(|detail| {
            div()
              .truncate()
              .text_xs()
              .text_color(theme::muted())
              .child(detail)
          })),
      )
      .when(taken, |el| {
        el.child(
          div()
            .flex_none()
            .text_xs()
            .text_color(theme::muted())
            .child(t("Already added", "Déjà ajouté")),
        )
      })
      .into_any_element()
  }
}

impl Render for AddLive {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let (title, list): (SharedString, Result<Vec<Candidate>, SharedString>) =
      match (&self.results, self.query.is_empty()) {
        (_, true) => (
          t("Followed channels on air", "Chaînes suivies en direct").into(),
          self.followed.clone(),
        ),
        _ if self.searching => (
          t("Lives on Twitch", "Lives sur Twitch").into(),
          Err(t("Searching…", "Recherche…").into()),
        ),
        (Some(Ok(found)), false) => (
          t("Lives on Twitch", "Lives sur Twitch").into(),
          Ok(found.clone()),
        ),
        (Some(Err(error)), false) => (
          t("Lives on Twitch", "Lives sur Twitch").into(),
          Err(error.clone().into()),
        ),
        (None, false) => (
          t("Lives on Twitch", "Lives sur Twitch").into(),
          Err(SharedString::default()),
        ),
      };
    let body = match list {
      Ok(candidates) if candidates.is_empty() => cards::note(if self.query.is_empty() {
        t(
          "None of the followed channels is on air.",
          "Aucune chaîne suivie n'est en direct.",
        )
      } else {
        t("No live matches.", "Aucun live ne correspond.")
      })
      .into_any_element(),
      Ok(candidates) => {
        let rows: Vec<AnyElement> = candidates
          .iter()
          .enumerate()
          .map(|(at, c)| self.render_row(at, c, window, cx))
          .collect();
        v_flex().children(rows).into_any_element()
      }
      Err(note) => cards::note(note).into_any_element(),
    };
    let close = div()
      .id("multistream-add-close")
      .size(px(28.))
      .flex_none()
      .rounded(theme::RADIUS)
      .flex()
      .items_center()
      .justify_center()
      .cursor_pointer()
      .text_color(theme::muted())
      .hover(|el| {
        el.bg(theme::surface_hover())
          .text_color(theme::foreground())
      })
      .child(gpui_component::Icon::new(Icon::Close).size(px(16.)))
      .on_click(cx.listener(|_, _, _, cx| cx.emit(AddLiveEvent::Close)));
    div()
      .id("multistream-add-backdrop")
      .absolute()
      .inset_0()
      .flex()
      .items_center()
      .justify_center()
      .bg(gpui::black().opacity(0.5))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|_, _, _, cx| cx.emit(AddLiveEvent::Close)),
      )
      .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
        if event.keystroke.key == "escape" {
          cx.emit(AddLiveEvent::Close);
        }
      }))
      .child(
        v_flex()
          .id("multistream-add-dialog")
          .w(px(460.))
          .max_w(relative(0.9))
          .h(relative(0.7))
          .p_4()
          .gap_3()
          .rounded(theme::RADIUS_LG)
          .bg(theme::surface())
          .border_1()
          .border_color(theme::border())
          .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
          .child(
            h_flex()
              .justify_between()
              .child(
                div()
                  .text_lg()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child(t("Add a live", "Ajouter un live")),
              )
              .child(close),
          )
          .child(Input::new(&self.input))
          .child(
            div()
              .text_xs()
              .font_weight(FontWeight::SEMIBOLD)
              .text_color(theme::muted())
              .child(title),
          )
          .child(
            div()
              .id("multistream-add-list")
              .flex_1()
              .min_h_0()
              .overflow_y_scroll()
              .child(body),
          ),
      )
  }
}
