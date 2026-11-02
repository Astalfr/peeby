//! Subscribed channels: a filterable grid of avatars, each opening the channel's videos, with
//! the channel's notification bell in a corner.
//! Opened from the subscriptions page.

use gpui::prelude::*;
use gpui::{Context, Entity, FontWeight, SharedString, Subscription, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::{IconName, h_flex, v_flex};
use innertube::{Channel, Client, Error};

use crate::spaces::youtube::bell;
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::assets::Icon;
use ui::i18n::{Lang, lang, t};
use ui::images::Images;
use ui::picture::image_fill;
use ui::widgets::{notice, page_title};
use ui::{motion, runtime, tf, theme};

/// Width of a channel tile, and side of its avatar.
const TILE_WIDTH: f32 = 168.;
const AVATAR: f32 = 96.;

enum State {
  Loading,
  Loaded(Vec<Channel>),
  /// Signed out, or YouTube no longer accepts the session.
  SignedOut,
  Failed(SharedString),
}

pub struct ChannelsView {
  client: Client,
  nav: WeakEntity<YouTubeSpace>,
  state: State,
  filter: Entity<InputState>,
  _filter_subscription: Subscription,
}

impl ChannelsView {
  pub fn new(
    client: Client,
    nav: WeakEntity<YouTubeSpace>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let filter = cx.new(|cx| {
      InputState::new(window, cx).placeholder(t("Filter channels…", "Filtrer les chaînes…"))
    });
    let subscription = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
      if matches!(event, InputEvent::Change) {
        cx.notify();
      }
    });
    let mut view = Self {
      client,
      nav,
      state: State::Loading,
      filter,
      _filter_subscription: subscription,
    };
    view.load(cx);
    view
  }

  fn load(&mut self, cx: &mut Context<Self>) {
    if !self.client.signed_in() {
      self.state = State::SignedOut;
      return;
    }
    self.state = State::Loading;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.subscribed_channels().await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(channels)) => {
              tracing::info!("{} subscribed channels", channels.len());
              this.state = State::Loaded(channels);
            }
            Some(Err(Error::SignInRequired | Error::NotLoggedIn)) => {
              this.state = State::SignedOut;
            }
            Some(Err(e)) => {
              tracing::warn!("subscribed channels: {e}");
              this.state = State::Failed(error_text(&e).into());
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// A channel: round avatar, name and subscriber count, on a soft surface while hovered, and
  /// its notification bell at the top right.
  fn tile(
    &self,
    channel: &Channel,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let route = Route::Channel {
      id: channel.id.clone(),
      name: channel.name.clone(),
    };
    let nav = self.nav.clone();
    let group = SharedString::from(format!("channel-{}", channel.id));
    let detail = channel
      .subscribers
      .clone()
      .or_else(|| channel.handle.clone())
      .unwrap_or_default();
    v_flex()
      .id(group.clone())
      .group(group.clone())
      .w(px(TILE_WIDTH))
      .flex_none()
      .px_2()
      .py_4()
      .gap_3()
      .items_center()
      .rounded(theme::RADIUS_LG)
      .cursor_pointer()
      .hover(|s| s.bg(theme::surface()))
      .on_click(move |_, window, cx| {
        nav
          .update(cx, |app, cx| app.open(route.clone(), window, cx))
          .ok();
      })
      .relative()
      .children(channel.notifications.as_ref().map(|notifications| {
        div()
          .absolute()
          .top(px(4.))
          .right(px(4.))
          .text_color(theme::muted())
          .child(bell::bell_button(
            &channel.id,
            &channel.id,
            notifications,
            true,
            cx,
          ))
      }))
      .child(
        div()
          .size(px(AVATAR))
          .flex_none()
          .rounded_full()
          .border_2()
          .border_color(gpui::transparent_black())
          .group_hover(group.clone(), |s| s.border_color(theme::accent()))
          .child(image_fill(
            &convert::picture(&channel.avatar),
            AVATAR,
            1.,
            px(AVATAR / 2.),
            window,
            cx,
          )),
      )
      .child(
        v_flex()
          .w_full()
          .items_center()
          .gap_0p5()
          .child(
            div()
              .max_w_full()
              .truncate()
              .text_sm()
              .font_weight(FontWeight::SEMIBOLD)
              .group_hover(group, |s| s.text_color(theme::accent_hover()))
              .child(channel.name.clone()),
          )
          .child(
            div()
              .max_w_full()
              .truncate()
              .text_xs()
              .text_color(theme::muted())
              .child(detail),
          ),
      )
      .into_any_element()
  }
}

/// "12 subscriptions", singular for one (and for zero in French).
fn subscription_count(n: usize) -> String {
  match (lang(), n) {
    (Lang::En, 1) => "1 subscription".to_owned(),
    (Lang::Fr, 0 | 1) => format!("{n} abonnement"),
    _ => tf!("{n} subscriptions", "{n} abonnements"),
  }
}

fn skeleton_tile() -> impl IntoElement {
  v_flex()
    .w(px(TILE_WIDTH))
    .flex_none()
    .px_2()
    .py_4()
    .gap_3()
    .items_center()
    .child(Skeleton::new().size(px(AVATAR)).rounded_full())
    .child(Skeleton::new().w(px(110.)).h(px(14.)))
    .child(Skeleton::new().w(px(70.)).h(px(12.)))
}

impl Render for ChannelsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let query = self.filter.read(cx).value().trim().to_lowercase();
    let mut count = None;
    let content = match &self.state {
      // Replaced by the skeleton below.
      State::Loading => div().into_any_element(),
      State::SignedOut => {
        let nav = self.nav.clone();
        notice(
          Icon::Account,
          t(
            "Sign in to see your subscriptions",
            "Connectez-vous pour voir vos abonnements",
          ),
          t(
            "Import the cookies of a YouTube session from the Account page.",
            "Importez les cookies d'une session YouTube depuis la page Compte.",
          ),
        )
        .child(
          h_flex().pt_2().child(
            Button::new("to-account")
              .primary()
              .label(t("Import cookies", "Importer des cookies"))
              .on_click(move |_, _, cx| {
                nav.update(cx, |app, cx| app.open_settings(cx)).ok();
              }),
          ),
        )
        .into_any_element()
      }
      State::Failed(e) => notice(
        IconName::TriangleAlert,
        t(
          "Cannot load the channels",
          "Impossible de charger les chaînes",
        ),
        e.clone(),
      )
      .child(
        h_flex().pt_2().child(
          Button::new("retry")
            .primary()
            .label(t("Retry", "Réessayer"))
            .on_click(cx.listener(|this, _, _, cx| {
              this.load(cx);
              cx.notify();
            })),
        ),
      )
      .into_any_element(),
      State::Loaded(channels) => {
        let shown: Vec<&Channel> = channels
          .iter()
          .filter(|c| query.is_empty() || c.name.to_lowercase().contains(&query))
          .collect();
        count = Some(channels.len());
        if shown.is_empty() {
          div()
            .text_color(theme::muted())
            .child(if channels.is_empty() {
              t("No subscriptions.", "Aucun abonnement.")
            } else {
              t("No channel matches.", "Aucune chaîne ne correspond.")
            })
            .into_any_element()
        } else {
          let tiles: Vec<_> = shown
            .into_iter()
            .map(|channel| self.tile(channel, window, cx))
            .collect();
          h_flex()
            .flex_wrap()
            .items_start()
            .gap_2()
            .children(tiles)
            .into_any_element()
        }
      }
    };
    let skeleton = h_flex()
      .flex_wrap()
      .gap_2()
      .children((0..18).map(|_| skeleton_tile()))
      .into_any_element();
    let body = motion::crossfade(
      SharedString::from(format!("channels-load-{}", cx.entity_id())),
      matches!(self.state, State::Loading),
      skeleton,
      content,
      window,
      cx,
    );
    let show_filter = matches!(self.state, State::Loaded(_) | State::Loading);
    v_flex()
      .id("channels")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .px_8()
          .py_6()
          .gap_6()
          .child(
            h_flex()
              .gap_4()
              .items_center()
              .child(page_title(t("Channels", "Chaînes")))
              .children(count.map(|n| {
                div()
                  .text_color(theme::muted())
                  .child(subscription_count(n))
              }))
              .child(div().flex_1())
              .when(show_filter, |el| {
                el.child(
                  div().w(px(280.)).child(
                    Input::new(&self.filter).prefix(
                      gpui_component::Icon::new(Icon::Search)
                        .size(px(16.))
                        .text_color(theme::muted()),
                    ),
                  ),
                )
              }),
          )
          .child(body),
      )
  }
}
