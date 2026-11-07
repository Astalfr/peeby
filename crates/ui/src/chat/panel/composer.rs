//! The panel's writing area, for a space that can send to its chat ([`ChatPanel::with_composer`]):
//! the message field with its emote and emoji picker, and under it the space's own element on the
//! left (a service's points), the chat's options and "Send" on the right. A message leaves as
//! [`ChatPanelEvent::Send`]: the space sends it, the panel knows no protocol.
//!
//! The options are the gear left of "Send": the space names them ([`ChatPanel::set_options`]) and
//! the one chosen comes back as [`ChatPanelEvent::Option`], so that the panel knows no service.
//! Without any option the gear is not drawn.
//!
//! In the field, a known emote's code followed by a space (picked or typed) shows as its image: an
//! atomic token whose text stays the code, so that what is sent is unchanged. Emoji are text.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  Anchor, AnyElement, AnyView, Context, ElementId, Entity, FontWeight, SharedString, Subscription,
  Window, div, px, uniform_list,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{
  InlineToken, InlineTokenContext, Input, InputEvent, InputState, Textarea, TextareaState,
};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::popover::Popover;
use gpui_component::{Disableable, Sizable, h_flex, v_flex};

use super::{ChatPanel, emote_stack};
use crate::assets::Icon;
use crate::chat::{EMOTE_HEIGHT, Emote, emoji};
use crate::i18n::t;
use crate::images::Images;
use crate::picture::Picture;
use crate::theme;

/// Picker cells per row.
const PER_ROW: usize = 8;
/// Side of a picker cell, logical pixels.
const CELL: f32 = 38.;
/// Height of the picker's grid.
const PICKER_HEIGHT: f32 = 300.;
/// Side of the picker's button.
const PICKER_BUTTON_SIZE: f32 = 31.;
/// Room kept at the field's right for the picker's button.
const PICKER_BUTTON: f32 = PICKER_BUTTON_SIZE + 8.;
/// The length counter shows from this share of the limit on.
const COUNTER_FROM: f32 = 0.8;
/// Side of the options' gear button.
const GEAR_SIZE: f32 = 26.;

/// What the panel tells its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatPanelEvent {
  /// The viewer sent this text (trimmed, not empty).
  Send(String),
  /// The viewer chose this option of the gear's menu, by its [`ChatOption::id`].
  Option(SharedString),
  /// The viewer asked for a user's channel in a new tab, from their card: its key
  /// ([`crate::chat::Author::channel`]) and the user's name, for the tab until it loads.
  OpenChannel {
    /// The channel's key.
    channel: SharedString,
    /// The user's name.
    name: SharedString,
  },
  /// The viewer closed the chat from its header (theater mode, [`ChatPanel::set_collapsible`]);
  /// the page reopens it with [`crate::chat::watch_layout`]'s button.
  Hide,
}

/// An entry of the composer's options menu (the gear left of "Send"), in the menu's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOption {
  /// Told back in [`ChatPanelEvent::Option`] when it is chosen; the space's own name for it.
  pub id: SharedString,
  /// What the menu shows, in the viewer's language.
  pub label: SharedString,
  /// Shown greyed and not choosable (the work it starts is already running).
  pub disabled: bool,
}

impl ChatOption {
  /// An option choosable now.
  pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
    Self {
      id: id.into(),
      label: label.into(),
      disabled: false,
    }
  }

  /// The same option, greyed and not choosable.
  pub fn disabled(mut self, disabled: bool) -> Self {
    self.disabled = disabled;
    self
  }
}

/// A titled set of emotes in the picker (a channel's, a service's global ones…).
#[derive(Clone)]
pub struct EmoteGroup {
  /// Shown above its emotes.
  pub title: SharedString,
  /// In the picker's order.
  pub emotes: Vec<Emote>,
}

/// Whether the viewer can write now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Composing {
  /// Messages can be sent.
  Ready,
  /// Not yet (connecting): typing is allowed, sending waits; the text says why.
  Waiting(SharedString),
  /// Not possible (signed out, refused): the field is disabled; the text says why.
  Off(SharedString),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
  #[default]
  Emotes,
  Emoji,
}

/// A row of the picker's grid.
enum Row {
  Title(SharedString),
  Emotes(Vec<Emote>),
  Emoji(&'static [&'static str]),
}

pub(super) struct Composer {
  /// The longest message, in `char`s.
  max_chars: usize,
  /// Made at the first render (it needs the window), as the search field.
  input: Option<Entity<TextareaState>>,
  search: Option<Entity<InputState>>,
  groups: Vec<EmoteGroup>,
  /// The groups' emotes by code, the first group's winning: what the field draws as images.
  known: Rc<HashMap<SharedString, Emote>>,
  footer: Option<AnyView>,
  /// The gear's menu, in its order; empty: no gear.
  options: Vec<ChatOption>,
  state: Composing,
  tab: Tab,
  /// The picker's rows for the tab and the search, made again when either changes.
  rows: Vec<Row>,
  /// The picked item under the pointer, named at the picker's bottom.
  hovered: Option<SharedString>,
  /// The picker is open (the popover follows it: a pick closes it).
  picker_open: bool,
  _subscriptions: Vec<Subscription>,
}

impl Composer {
  pub(super) fn new(max_chars: usize) -> Self {
    Self {
      max_chars,
      input: None,
      search: None,
      groups: Vec::new(),
      known: Rc::default(),
      footer: None,
      options: Vec::new(),
      state: Composing::Waiting(t("Connecting to chat…", "Connexion au chat…").into()),
      tab: Tab::default(),
      rows: Vec::new(),
      hovered: None,
      picker_open: false,
      _subscriptions: Vec::new(),
    }
  }

  fn query(&self, cx: &gpui::App) -> String {
    self
      .search
      .as_ref()
      .map(|s| s.read(cx).value().trim().to_lowercase())
      .unwrap_or_default()
  }

  fn make_rows(&mut self, cx: &gpui::App) {
    let query = self.query(cx);
    self.rows = match self.tab {
      Tab::Emotes => emote_rows(&self.groups, &query),
      Tab::Emoji => emoji_rows(),
    };
  }
}

/// The emote rows: each group with an emote matching `query` (lowercase; empty: all), its title
/// then its emotes by [`PER_ROW`].
fn emote_rows(groups: &[EmoteGroup], query: &str) -> Vec<Row> {
  let mut rows = Vec::new();
  for group in groups {
    let emotes: Vec<Emote> = group
      .emotes
      .iter()
      .filter(|e| query.is_empty() || e.name.to_lowercase().contains(query))
      .cloned()
      .collect();
    if emotes.is_empty() {
      continue;
    }
    rows.push(Row::Title(group.title.clone()));
    rows.extend(
      emotes
        .chunks(PER_ROW)
        .map(|chunk| Row::Emotes(chunk.to_vec())),
    );
  }
  rows
}

fn emoji_rows() -> Vec<Row> {
  let mut rows = Vec::new();
  for group in emoji::GROUPS {
    rows.push(Row::Title(t(group.title.0, group.title.1).into()));
    rows.extend(group.emoji.chunks(PER_ROW).map(Row::Emoji));
  }
  rows
}

/// The text to insert for a picked `item` after `before` (the field's text): spaced from what
/// precedes it, a space after.
/// The words of `text` to show as emotes: those `known` and followed by a space (a word still
/// being typed may grow), outside the `taken` ranges (tokens already). Byte ranges, in order.
fn emote_words(
  text: &str,
  taken: &[Range<usize>],
  known: impl Fn(&str) -> bool,
) -> Vec<Range<usize>> {
  let mut words = Vec::new();
  let mut start = 0;
  for (at, c) in text.char_indices() {
    if c == ' ' {
      let range = start..at;
      if !range.is_empty()
        && known(&text[range.clone()])
        && !taken
          .iter()
          .any(|t| t.start < range.end && range.start < t.end)
      {
        words.push(range);
      }
      start = at + 1;
    }
  }
  words
}

fn insertion(before: &str, item: &str) -> String {
  let lead = if before.is_empty() || before.ends_with(' ') {
    ""
  } else {
    " "
  };
  format!("{lead}{item} ")
}

impl ChatPanel {
  /// The panel with a writing area: messages longer than `max_chars` cannot be sent. It waits
  /// ("Connecting…") until [`ChatPanel::set_composing`] says otherwise.
  pub fn with_composer(mut self, max_chars: usize) -> Self {
    self.composer = Some(Composer::new(max_chars));
    self
  }

  /// Whether the viewer can write now.
  pub fn set_composing(&mut self, state: Composing, cx: &mut Context<Self>) {
    if let Some(composer) = &mut self.composer
      && composer.state != state
    {
      composer.state = state;
      cx.notify();
    }
  }

  /// The emotes the picker offers, by group.
  pub fn set_emotes(&mut self, groups: Vec<EmoteGroup>, cx: &mut Context<Self>) {
    if let Some(composer) = &mut self.composer {
      let mut known = HashMap::new();
      for emote in groups.iter().flat_map(|g| &g.emotes) {
        known
          .entry(emote.name.clone())
          .or_insert_with(|| emote.clone());
      }
      composer.known = Rc::new(known);
      composer.groups = groups;
      composer.make_rows(cx);
      cx.notify();
    }
  }

  /// The element on the left under the field (a service's points).
  pub fn set_footer(&mut self, footer: AnyView, cx: &mut Context<Self>) {
    if let Some(composer) = &mut self.composer {
      composer.footer = Some(footer);
      cx.notify();
    }
  }

  /// What the gear's menu offers, left of "Send"; empty (the default) draws no gear. Choosing one
  /// emits [`ChatPanelEvent::Option`] with its id. Call it again to change a label or grey an
  /// option out while its work runs.
  pub fn set_options(&mut self, options: Vec<ChatOption>, cx: &mut Context<Self>) {
    if let Some(composer) = &mut self.composer
      && composer.options != options
    {
      composer.options = options;
      cx.notify();
    }
  }

  /// Sends the field's text, if it can be, and empties the field.
  fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(composer) = &self.composer else {
      return;
    };
    let Some(input) = composer.input.clone() else {
      return;
    };
    let text = input.read(cx).value().trim().to_owned();
    if composer.state != Composing::Ready
      || text.is_empty()
      || text.chars().count() > composer.max_chars
    {
      return;
    }
    cx.emit(ChatPanelEvent::Send(text));
    input.update(cx, |state, cx| state.set_value("", window, cx));
    // What was sent shows at the end, as on the services' sites.
    if !self.following() {
      self.resume(cx);
    }
    // `set_value` emits no change: "Send" is disabled again here.
    cx.notify();
  }

  /// Inserts a picked emote's code or an emoji in the field, at the cursor.
  fn pick(&mut self, item: SharedString, window: &mut Window, cx: &mut Context<Self>) {
    let Some(input) = self.composer.as_ref().and_then(|c| c.input.clone()) else {
      return;
    };
    input.update(cx, |state, cx| {
      let text = insertion(&state.value(), &item);
      state.insert(text, window, cx);
    });
    // `insert` emits no change: the emote becomes a token, and "Send" is enabled, here.
    self.show_emotes(window, cx);
    // A pick closes the picker and goes back to the field, after the popover gives the focus
    // back to its button.
    if let Some(composer) = &mut self.composer {
      composer.picker_open = false;
      composer.hovered = None;
    }
    window.defer(cx, move |window, cx| {
      input.update(cx, |input, cx| input.focus(window, cx));
    });
    cx.notify();
  }

  /// Turns the field's complete emote codes into tokens drawn as their image.
  fn show_emotes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(composer) = &self.composer else {
      return;
    };
    let Some(input) = composer.input.clone() else {
      return;
    };
    let known = composer.known.clone();
    if known.is_empty() {
      return;
    }
    input.update(cx, |state, cx| {
      let text = state.value();
      let taken: Vec<Range<usize>> = state.tokens().iter().map(|t| t.range()).collect();
      let words = emote_words(&text, &taken, |w| known.contains_key(w));
      if words.is_empty() {
        return;
      }
      // A replacement leaves the cursor at the code's end, before the space that follows it:
      // the selection goes back where it was (the text keeps its length).
      let selection = state.selected_range();
      // From the end: a replacement keeps the ranges before it (the same text in either case).
      for range in words.into_iter().rev() {
        let code = text[range.clone()].to_owned();
        let token = InlineToken::new(code.clone(), code);
        // A range the field refuses stays plain text.
        state
          .replace_range_with_token(range, token, window, cx)
          .ok();
      }
      state.set_selected_range(selection, cx);
    });
  }

  fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(composer) = &mut self.composer {
      composer.tab = tab;
      composer.hovered = None;
      composer.make_rows(cx);
      cx.notify();
    }
    self.focus_search(window, cx);
  }

  /// Puts the cursor in the picker's search, on the emotes' tab (the only one with a search).
  /// Deferred: the popover takes the focus for its content as it opens.
  fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(composer) = &self.composer else {
      return;
    };
    let Some(search) = composer
      .search
      .clone()
      .filter(|_| composer.tab == Tab::Emotes)
    else {
      return;
    };
    window.defer(cx, move |window, cx| {
      search.update(cx, |search, cx| search.focus(window, cx));
    });
  }

  /// The writing area, under the messages; `None` without a composer.
  pub(super) fn render_composer(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let composer = self.composer.as_mut()?;
    if composer.input.is_none() {
      let input = cx.new(|cx| {
        // Two lines high; Enter sends (Shift+Enter's line break leaves as a space).
        TextareaState::new(window, cx)
          .rows(2)
          .submit_on_enter(true)
          .placeholder(t("Send a message", "Envoyer un message"))
      });
      composer._subscriptions.push(cx.subscribe_in(
        &input,
        window,
        |this, _, event: &InputEvent, window, cx| match event {
          InputEvent::PressEnter { .. } => this.submit(window, cx),
          InputEvent::Change => {
            this.show_emotes(window, cx);
            cx.notify();
          }
          _ => {}
        },
      ));
      let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("Search", "Rechercher")));
      composer._subscriptions.push(cx.subscribe_in(
        &search,
        window,
        |this, _, event: &InputEvent, _, cx| {
          if matches!(event, InputEvent::Change)
            && let Some(composer) = &mut this.composer
          {
            composer.make_rows(cx);
            cx.notify();
          }
        },
      ));
      composer.input = Some(input);
      composer.search = Some(search);
      composer.make_rows(cx);
    }
    let input = composer.input.clone()?;
    let length = input.read(cx).value().chars().count();
    let max = composer.max_chars;
    let ready = composer.state == Composing::Ready;
    let off = matches!(composer.state, Composing::Off(_));
    let hint = match &composer.state {
      Composing::Ready => None,
      Composing::Waiting(text) | Composing::Off(text) => Some(text.clone()),
    };
    let footer = composer.footer.clone();
    let known = composer.known.clone();
    let this = cx.entity().downgrade();
    let opened = this.clone();
    let picker = Popover::new("chat-picker")
      .anchor(Anchor::BottomRight)
      .open(composer.picker_open)
      .on_open_change(move |open, window, cx| {
        let open = *open;
        opened
          .update(cx, |this, cx| {
            if let Some(composer) = &mut this.composer {
              composer.picker_open = open;
            }
            if open {
              this.focus_search(window, cx);
            }
            cx.notify();
          })
          .ok();
      })
      .trigger(
        // A small button (24 px, its icon 14) made 30 % larger: the button sizes its own icon,
        // so the icon is a child.
        Button::new("chat-picker-open")
          .ghost()
          .with_size(px(PICKER_BUTTON_SIZE))
          .size(px(PICKER_BUTTON_SIZE))
          .child(gpui_component::Icon::new(Icon::Smile).size(px(18.)))
          .disabled(off)
          .tooltip(t("Emotes and emoji", "Emotes et emoji")),
      )
      .content(move |_, _, cx| {
        this
          .update(cx, |this, cx| this.render_picker(cx))
          .unwrap_or_else(|_| div().into_any_element())
      });
    // The options' gear. Its menu is built again at every opening, so a label or a greyed entry
    // set meanwhile shows. The button is not disabled with the field: its options are about the
    // chat (reloading emotes), not about writing, and the popover opens its trigger whatever the
    // button's own state.
    let options = composer.options.clone();
    let chosen = cx.entity().downgrade();
    let gear = (!options.is_empty()).then(|| {
      Button::new("chat-options")
        .ghost()
        .with_size(px(GEAR_SIZE))
        .size(px(GEAR_SIZE))
        .child(gpui_component::Icon::new(Icon::Settings).size(px(16.)))
        .tooltip(t("Chat options", "Options du chat"))
        .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, _| {
          for option in &options {
            let (chosen, id) = (chosen.clone(), option.id.clone());
            menu = menu.item(
              PopupMenuItem::new(option.label.clone())
                .disabled(option.disabled)
                .on_click(move |_, _, cx| {
                  let id = id.clone();
                  chosen
                    .update(cx, |_, cx| cx.emit(ChatPanelEvent::Option(id)))
                    .ok();
                }),
            );
          }
          menu
        })
    });
    let counter = (length as f32 >= max as f32 * COUNTER_FROM).then(|| {
      div()
        .text_xs()
        .text_color(if length > max {
          gpui::rgb(0xff6b63).into()
        } else {
          theme::muted()
        })
        .child(format!("{length}/{max}"))
    });
    Some(
      v_flex()
        .flex_none()
        .gap_2()
        .px_3()
        .pt_2()
        .pb_3()
        .border_t_1()
        .border_color(theme::border())
        .children(hint.map(|hint| div().text_xs().text_color(theme::muted()).child(hint)))
        .child(
          // The picker's button inside the field, at its top right; the text keeps clear of it.
          div()
            .relative()
            .child(
              Textarea::new(&input)
                .token(move |token, window, cx| emote_token(&known, token, window, cx))
                .small()
                .pr(px(PICKER_BUTTON))
                .disabled(off),
            )
            .child(div().absolute().top_1().right_1().child(picker)),
        )
        .child(
          h_flex()
            .gap_2()
            .items_center()
            .child(div().flex_1().min_w_0().children(footer))
            .children(counter)
            .children(gear)
            .child(
              Button::new("chat-send")
                .primary()
                .small()
                .px_4()
                .label(t("Send", "Envoyer"))
                .disabled(!ready || length == 0 || length > max)
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            ),
        )
        .into_any_element(),
    )
  }

  fn render_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let Some(composer) = &self.composer else {
      return div().into_any_element();
    };
    let tab = composer.tab;
    let empty = composer.rows.is_empty();
    let hovered = composer.hovered.clone();
    let search = composer.search.clone();
    let tab_button = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Self>| {
      Button::new(id)
        .xsmall()
        .when(tab == which, |b| b.primary())
        .when(tab != which, |b| b.ghost())
        .label(label)
        .on_click(cx.listener(move |this, _, window, cx| this.set_tab(which, window, cx)))
    };
    let grid = if empty {
      div()
        .h(px(PICKER_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(theme::subtle())
        .child(match tab {
          Tab::Emotes if !composer.groups.is_empty() => t("No match", "Aucun résultat"),
          _ => t("No emotes yet", "Pas encore d'emotes"),
        })
        .into_any_element()
    } else {
      uniform_list(
        "chat-picker-grid",
        composer.rows.len(),
        cx.processor(|this, range: Range<usize>, window, cx| this.picker_rows(range, window, cx)),
      )
      .h(px(PICKER_HEIGHT))
      .into_any_element()
    };
    v_flex()
      .w(px(CELL * PER_ROW as f32 + 8.))
      .gap_2()
      .child(
        h_flex()
          .gap_1()
          .child(tab_button("chat-picker-emotes", "Emotes", Tab::Emotes, cx))
          .child(tab_button("chat-picker-emoji", "Emoji", Tab::Emoji, cx)),
      )
      .when(tab == Tab::Emotes, |el| {
        el.children(search.map(|search| Input::new(&search).small().cleanable(true)))
      })
      .child(grid)
      .child(
        div()
          .h(px(18.))
          .text_xs()
          .truncate()
          .text_color(theme::muted())
          .child(hovered.unwrap_or_default()),
      )
      .into_any_element()
  }

  fn picker_rows(
    &mut self,
    range: Range<usize>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Vec<AnyElement> {
    let Some(composer) = &self.composer else {
      return Vec::new();
    };
    let scale = window.scale_factor();
    let images = cx.try_global::<Images>().cloned();
    let mut elements = Vec::with_capacity(range.len());
    for ix in range {
      let row = h_flex().h(px(CELL)).w_full().items_center();
      let element = match composer.rows.get(ix) {
        Some(Row::Title(title)) => row
          .px_1()
          .text_xs()
          .font_weight(FontWeight::SEMIBOLD)
          .text_color(theme::muted())
          .child(title.clone())
          .into_any_element(),
        Some(Row::Emotes(emotes)) => {
          let mut row = row;
          for (column, emote) in emotes.iter().enumerate() {
            // A wide emote is drawn lower, so that it fits in its cell.
            let height = EMOTE_HEIGHT.min((CELL - 6.) / emote.aspect.max(0.1));
            let drawn = emote_stack(
              std::iter::once(emote),
              height,
              |picture: &Picture, effects, w, h| {
                super::store_image(images.as_ref(), picture, effects, (w, h), scale, cx)
              },
            );
            row = row.child(cell(
              ElementId::NamedInteger("chat-pick".into(), (ix * PER_ROW + column) as u64),
              emote.name.clone(),
              drawn.into_any_element(),
              cx,
            ));
          }
          row.into_any_element()
        }
        Some(Row::Emoji(emoji)) => {
          let mut row = row;
          for (column, emoji) in emoji.iter().enumerate() {
            let glyph = div()
              .text_xl()
              .font_family("Segoe UI Emoji")
              .child(*emoji)
              .into_any_element();
            row = row.child(cell(
              ElementId::NamedInteger("chat-pick-emoji".into(), (ix * PER_ROW + column) as u64),
              SharedString::new_static(emoji),
              glyph,
              cx,
            ));
          }
          row.into_any_element()
        }
        None => row.into_any_element(),
      };
      elements.push(element);
    }
    elements
  }
}

/// A token of the field: its emote's image, a line high (its code if it is not known any more).
fn emote_token(
  known: &HashMap<SharedString, Emote>,
  token: &InlineTokenContext,
  window: &mut Window,
  cx: &mut gpui::App,
) -> AnyElement {
  let code = token.token().text().clone();
  let Some(emote) = known.get(&code) else {
    return div().child(code).into_any_element();
  };
  let height = f32::from(token.line_height()).max(1.);
  let scale = window.scale_factor();
  let images = cx.try_global::<Images>().cloned();
  emote_stack(
    std::iter::once(emote),
    height,
    |picture: &Picture, effects, w, h| {
      super::store_image(images.as_ref(), picture, effects, (w, h), scale, cx)
    },
  )
  .rounded(px(2.))
  .when(token.is_selected(), |el| el.bg(theme::accent_soft()))
  .into_any_element()
}

/// A picker cell showing `content`: a click inserts `item`, the pointer names it.
fn cell(
  id: ElementId,
  item: SharedString,
  content: AnyElement,
  cx: &mut Context<ChatPanel>,
) -> impl IntoElement {
  let named = item.clone();
  div()
    .id(id)
    .size(px(CELL))
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(theme::RADIUS)
    .cursor_pointer()
    .hover(|s| s.bg(theme::surface_hover()))
    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
      if let Some(composer) = &mut this.composer {
        if *hovered {
          composer.hovered = Some(named.clone());
        } else if composer.hovered.as_ref() == Some(&named) {
          composer.hovered = None;
        }
        cx.notify();
      }
    }))
    .on_click(cx.listener(move |this, _, window, cx| this.pick(item.clone(), window, cx)))
    .child(content)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn emote(name: &str) -> Emote {
    Emote {
      name: name.to_owned().into(),
      picture: Picture::sized(|_, _| String::new()),
      aspect: 1.,
      overlay: false,
      modifiers: Default::default(),
    }
  }

  #[test]
  fn complete_emote_words_are_found() {
    let known = |w: &str| matches!(w, "Pog" | "KEKW");
    let text = "so Pog KEKW Pogs Pog";
    // The last `Pog` is still being typed; `KEKW` is a token already.
    let kekw = 7..11;
    let found = emote_words(text, std::slice::from_ref(&kekw), known);
    assert_eq!((found.len(), found.first()), (1, Some(&(3..6))));
    assert_eq!(emote_words("Pog  Pog ", &[], known), [0..3, 5..8]);
    assert!(emote_words("", &[], known).is_empty());
  }

  #[test]
  fn an_option_greyed_out_differs_from_the_same_one_ready() {
    // `set_options` only notifies when the list changed: an option greyed out must not compare
    // equal to the one it replaces, or the gear's menu would keep offering it.
    let ready = ChatOption::new("reload", "Reload");
    assert!(!ready.disabled);
    assert_ne!(vec![ready.clone().disabled(true)], vec![ready]);
  }

  #[test]
  fn picks_are_spaced() {
    assert_eq!(insertion("", "Kappa"), "Kappa ");
    assert_eq!(insertion("salut", "Kappa"), " Kappa ");
    assert_eq!(insertion("salut ", "😀"), "😀 ");
  }

  #[test]
  fn emote_rows_follow_the_search() {
    let groups = vec![
      EmoteGroup {
        title: "Channel".into(),
        emotes: (0..10).map(|i| emote(&format!("Pog{i}"))).collect(),
      },
      EmoteGroup {
        title: "Global".into(),
        emotes: vec![emote("KEKW"), emote("pogU")],
      },
    ];
    let shape = |rows: &[Row]| -> Vec<String> {
      rows
        .iter()
        .map(|r| match r {
          Row::Title(t) => format!("t:{t}"),
          Row::Emotes(e) => format!("e:{}", e.len()),
          Row::Emoji(e) => format!("j:{}", e.len()),
        })
        .collect()
    };
    assert_eq!(
      shape(&emote_rows(&groups, "")),
      ["t:Channel", "e:8", "e:2", "t:Global", "e:2"]
    );
    assert_eq!(shape(&emote_rows(&groups, "pogu")), ["t:Global", "e:1"]);
    assert!(emote_rows(&groups, "nothing").is_empty());
  }

  #[test]
  fn emoji_rows_have_titles() {
    let rows = emoji_rows();
    assert!(matches!(rows.first(), Some(Row::Title(_))));
    assert!(rows.iter().any(|r| matches!(r, Row::Emoji(_))));
  }
}
