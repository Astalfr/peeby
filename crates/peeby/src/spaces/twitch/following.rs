//! "Suivies": the followed channels on air as stream cards (most viewers first), then the others
//! with their avatars, by name, under a field that filters them. Helix `streams/followed`,
//! `channels/followed` and `users`.

use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{Context, Entity, Subscription, Task, WeakEntity, Window, div, px};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::v_flex;
use twitch::helix::{Followed, Helix, Stream};
use ui::i18n::t;
use ui::images::Images;
use ui::runtime;

use super::TwitchSpace;
use super::cards::{self, Avatars};

/// Pages read at most: 300 live channels, 500 followed ones.
const LIVE_PAGES: usize = 3;
const FOLLOWED_PAGES: usize = 5;

struct Loaded {
  live: Vec<Stream>,
  offline: Vec<Followed>,
  avatars: Avatars,
}

pub struct FollowingView {
  nav: WeakEntity<TwitchSpace>,
  loaded: Option<Loaded>,
  error: Option<String>,
  /// Filters the offline channels; made on the first render, which has the window.
  filter: Option<(Entity<InputState>, Subscription)>,
  _load: Task<()>,
}

impl FollowingView {
  pub fn new(
    nav: WeakEntity<TwitchSpace>,
    helix: Helix,
    user_id: String,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let fetch = runtime::spawn(cx, load(helix, user_id));
    let load = cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(loaded)) => this.loaded = Some(loaded),
            Some(Err(e)) => {
              this.error = Some(super::error_text(&e));
              super::report(&this.nav, &e, cx);
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    });
    Self {
      nav,
      loaded: None,
      error: None,
      filter: None,
      _load: load,
    }
  }
}

async fn load(helix: Helix, user_id: String) -> twitch::Result<Loaded> {
  let mut live = Vec::new();
  let mut after = None;
  for _ in 0..LIVE_PAGES {
    let page = helix.followed_streams(&user_id, after.as_deref()).await?;
    live.extend(page.items);
    after = page.cursor;
    if after.is_none() {
      break;
    }
  }
  let mut followed = Vec::new();
  let mut after = None;
  for _ in 0..FOLLOWED_PAGES {
    let page = helix.followed_channels(&user_id, after.as_deref()).await?;
    followed.extend(page.items);
    after = page.cursor;
    if after.is_none() {
      break;
    }
  }
  let on_air: HashSet<&str> = live.iter().map(|s| s.user_id.as_str()).collect();
  let mut offline: Vec<Followed> = followed
    .into_iter()
    .filter(|f| !on_air.contains(f.broadcaster_id.as_str()))
    .collect();
  offline.sort_by_cached_key(|f| f.broadcaster_name.to_lowercase());
  let mut avatars = Avatars::new();
  let ids: Vec<String> = live.iter().map(|s| s.user_id.clone()).collect();
  for chunk in ids.chunks(100) {
    avatars.extend(cards::avatars(&helix, chunk.to_vec()).await);
  }
  // All of them (5 requests at most): the filter can show any.
  let ids: Vec<String> = offline.iter().map(|f| f.broadcaster_id.clone()).collect();
  for chunk in ids.chunks(100) {
    avatars.extend(cards::avatars(&helix, chunk.to_vec()).await);
  }
  Ok(Loaded {
    live,
    offline,
    avatars,
  })
}

impl FollowingView {
  fn filter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
    let (input, _) = self.filter.get_or_insert_with(|| {
      let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(t("Filter channels", "Filtrer les chaînes"))
      });
      let subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::Change) {
          cx.notify();
        }
      });
      (input, subscription)
    });
    input.clone()
  }
}

/// Whether `f`'s name or login contains `query` (lowercase), ignoring case.
fn shown(f: &Followed, query: &str) -> bool {
  query.is_empty()
    || f.broadcaster_name.to_lowercase().contains(query)
    || f.broadcaster_login.contains(query)
}

impl Render for FollowingView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let width = cards::stream_width(window, cx);
    let filter = self.filter(window, cx);
    let query = filter.read(cx).value().trim().to_lowercase();
    let body = match (&self.loaded, &self.error) {
      (Some(loaded), _) => {
        let live: Vec<_> = loaded
          .live
          .iter()
          .map(|s| {
            cards::stream_card(
              s,
              width,
              loaded.avatars.get(&s.user_id),
              &self.nav,
              window,
              cx,
            )
          })
          .collect();
        let offline: Vec<_> = loaded
          .offline
          .iter()
          .filter(|f| shown(f, &query))
          .map(|f| {
            cards::channel_chip(
              cards::Chip {
                login: &f.broadcaster_login,
                name: &f.broadcaster_name,
                detail: None,
                avatar: loaded.avatars.get(&f.broadcaster_id),
                live: false,
              },
              &self.nav,
              window,
              cx,
            )
          })
          .collect();
        let live = if live.is_empty() {
          cards::note(t(
            "None of your channels is live.",
            "Aucune de vos chaînes n'est en direct.",
          ))
          .into_any_element()
        } else {
          cards::wrap(live).into_any_element()
        };
        v_flex()
          .gap_10()
          .child(cards::section(t("Live", "En direct"), live))
          .when(!loaded.offline.is_empty(), |el| {
            let list = if offline.is_empty() {
              cards::note(t("No channel matches.", "Aucune chaîne ne correspond."))
                .into_any_element()
            } else {
              cards::wrap(offline).into_any_element()
            };
            el.child(cards::section(
              t("Offline", "Hors ligne"),
              v_flex()
                .gap_4()
                .child(div().w(px(320.)).child(Input::new(&filter)))
                .child(list),
            ))
          })
          .into_any_element()
      }
      (None, Some(error)) => cards::note(error.clone()).into_any_element(),
      (None, None) => cards::note(t("Loading…", "Chargement…")).into_any_element(),
    };
    v_flex()
      .id("twitch-following")
      .size_full()
      .overflow_y_scroll()
      .p_8()
      .gap_6()
      .child(
        div()
          .text_2xl()
          .font_weight(gpui::FontWeight::BOLD)
          .child(t("Following", "Suivies")),
      )
      .child(body)
  }
}

/// The login in `text`: a name, or a `twitch.tv/<login>` URL.
pub fn channel_login(text: &str) -> Option<String> {
  let text = text.trim().trim_end_matches('/');
  let login = match text.split_once("twitch.tv/") {
    Some((_, rest)) => rest.split(['/', '?', '#']).next()?,
    None => text,
  };
  let valid = !login.is_empty()
    && login.len() <= 25
    && login
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'_');
  valid.then(|| login.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
  use super::channel_login;

  #[test]
  fn logins() {
    assert_eq!(channel_login("Squeezie").as_deref(), Some("squeezie"));
    assert_eq!(
      channel_login("https://www.twitch.tv/zerator/").as_deref(),
      Some("zerator")
    );
    assert_eq!(
      channel_login("twitch.tv/some_one?x=1").as_deref(),
      Some("some_one")
    );
    assert_eq!(channel_login("not a login"), None);
    assert_eq!(channel_login(""), None);
  }
}
