//! "Suivies": until the Helix catalog (step 7), a field to open a channel by its name or URL.

use gpui::prelude::*;
use gpui::{Context, Entity, Subscription, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{h_flex, v_flex};
use ui::theme;

use super::{Route, TwitchSpace};

pub struct FollowingView {
  nav: WeakEntity<TwitchSpace>,
  login: String,
  /// Made at the first render, which has the window.
  input: Option<Entity<InputState>>,
  _subscription: Option<Subscription>,
}

impl FollowingView {
  pub fn new(nav: WeakEntity<TwitchSpace>, login: String) -> Self {
    Self {
      nav,
      login,
      input: None,
      _subscription: None,
    }
  }

  fn open(&mut self, cx: &mut Context<Self>) {
    let Some(input) = &self.input else { return };
    let Some(login) = channel_login(&input.read(cx).value()) else {
      return;
    };
    self
      .nav
      .update(cx, |space, cx| space.open(Route::Watch(login), cx))
      .ok();
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

impl Render for FollowingView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.input.is_none() {
      let input = cx.new(|cx| InputState::new(window, cx).placeholder("Nom ou lien d'une chaîne"));
      self._subscription =
        Some(
          cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
              this.open(cx);
            }
          }),
        );
      self.input = Some(input);
    }
    let input = self.input.clone();
    div()
      .size_full()
      .flex()
      .items_center()
      .justify_center()
      .child(
        v_flex()
          .w(px(420.))
          .gap_3()
          .child(format!("Connecté à Twitch en tant que {}.", self.login))
          .child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .child("Vos chaînes suivies apparaîtront ici. En attendant, ouvrez une chaîne :"),
          )
          .child(
            h_flex()
              .gap_2()
              .children(input.map(|i| div().flex_1().child(Input::new(&i))))
              .child(
                Button::new("twitch-open-channel")
                  .primary()
                  .label("Regarder")
                  .on_click(cx.listener(|this, _, _, cx| this.open(cx))),
              ),
          ),
      )
  }
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
