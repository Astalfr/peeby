//! The Twitch search, from the top bar's button: channels (on air first) and categories matching
//! the text, run on Enter. Helix `search/channels` and `search/categories`.

use gpui::prelude::*;
use gpui::{Context, Entity, Subscription, Task, Window};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::v_flex;
use twitch::helix::{Category, ChannelResult, Helix};
use ui::i18n::t;
use ui::images::Images;

use super::Nav;
use super::browse::{load, page};
use super::cards;

type Results = (Vec<ChannelResult>, Vec<Category>);

pub struct SearchView {
  nav: Nav,
  helix: Helix,
  input: Entity<InputState>,
  results: Option<Result<Results, String>>,
  searching: bool,
  _task: Option<Task<()>>,
  _subscription: Subscription,
}

impl SearchView {
  pub fn new(nav: Nav, helix: Helix, window: &mut Window, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let input = cx.new(|cx| {
      InputState::new(window, cx).placeholder(t("Search Twitch", "Rechercher sur Twitch"))
    });
    input.update(cx, |state, cx| state.focus(window, cx));
    let subscription = cx.subscribe_in(
      &input,
      window,
      |this: &mut Self, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
          this.search(cx);
        }
      },
    );
    Self {
      nav,
      helix,
      input,
      results: None,
      searching: false,
      _task: None,
      _subscription: subscription,
    }
  }

  fn search(&mut self, cx: &mut Context<Self>) {
    let text = self.input.read(cx).value().trim().to_owned();
    if text.is_empty() {
      return;
    }
    self.searching = true;
    cx.notify();
    let helix = self.helix.clone();
    self._task = Some(load(
      cx,
      async move {
        let mut channels = helix.search_channels(&text).await?.items;
        // On air first, keeping Twitch's order otherwise.
        channels.sort_by_key(|c| !c.is_live);
        let categories = helix.search_categories(&text).await?.items;
        Ok((channels, categories))
      },
      |this: &mut Self, result, cx| {
        this.searching = false;
        this.results = Some(result);
        cx.notify();
      },
      self.nav.clone(),
    ));
  }
}

impl Render for SearchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let results = match &self.results {
      _ if self.searching => cards::note(t("Searching…", "Recherche…")).into_any_element(),
      Some(Ok((channels, categories))) if channels.is_empty() && categories.is_empty() => {
        cards::note(t("No results.", "Aucun résultat.")).into_any_element()
      }
      Some(Ok((channels, categories))) => {
        let channels: Vec<_> = channels
          .iter()
          .map(|c| {
            let detail = (!c.game_name.is_empty()).then(|| c.game_name.clone());
            cards::channel_chip(
              cards::Chip {
                login: &c.login,
                name: &c.display_name,
                detail,
                avatar: Some(&c.thumbnail_url),
                live: c.is_live,
              },
              &self.nav,
              window,
              cx,
            )
          })
          .collect();
        let categories: Vec<_> = categories
          .iter()
          .map(|c| cards::category_card(c, &self.nav, window, cx))
          .collect();
        v_flex()
          .gap_10()
          .when(!channels.is_empty(), |el| {
            el.child(cards::section(
              t("Channels", "Chaînes"),
              cards::wrap(channels),
            ))
          })
          .when(!categories.is_empty(), |el| {
            el.child(cards::section(
              t("Categories", "Catégories"),
              cards::wrap(categories),
            ))
          })
          .into_any_element()
      }
      Some(Err(error)) => cards::note(error.clone()).into_any_element(),
      None => cards::note(t(
        "A channel or a category, then Enter.",
        "Une chaîne ou une catégorie, puis Entrée.",
      ))
      .into_any_element(),
    };
    let body = v_flex()
      .gap_6()
      .child(Input::new(&self.input))
      .child(results)
      .into_any_element();
    page("twitch-search", t("Search", "Rechercher"), None, body)
  }
}
