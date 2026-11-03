//! The follow button of a channel (watch page, channel page): "Follow", or "Following" when the
//! auth-token's account follows it (`self.follower`, read with the channel). Twitch's
//! `followUser` and `unfollowUser` fail its integrity check, even with the auth-token (checked
//! 2026-11-03), and Helix has no follow endpoint: the button opens the channel on twitch.tv,
//! where the follow happens, as the points' bonus does (ADR 0012).

use gpui::prelude::*;
use gpui::{ElementId, div};
use gpui_component::button::{Button, ButtonVariants};
use ui::i18n::t;

/// The follow button of `login`'s channel; `followed` as read with the channel (`None` without
/// an auth-token).
pub fn follow_button(id: &str, login: &str, followed: Option<bool>) -> impl IntoElement + use<> {
  let url = format!("https://www.twitch.tv/{login}");
  let button = Button::new(ElementId::Name(format!("twitch-follow-{id}").into()));
  let button = if followed == Some(true) {
    button
      .ghost()
      .label(t("Following", "Suivi"))
      .tooltip(t("Unfollow on twitch.tv", "Ne plus suivre sur twitch.tv"))
  } else {
    button
      .primary()
      .label(t("Follow", "Suivre"))
      .tooltip(t("Follow on twitch.tv", "Suivre sur twitch.tv"))
  };
  div()
    .flex_none()
    .child(button.on_click(move |_, _, cx| cx.open_url(&url)))
}
