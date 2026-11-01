//! Root view: title bar (back, open a video by URL or id), sidebar (home, subscriptions, the
//! subscribed channels) and a stack of pages. In fullscreen only the page is shown.

use gpui::prelude::*;
use gpui::{
  AnyView, App, Context, Entity, FontWeight, SharedString, Subscription, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Sizable, TitleBar, h_flex, v_flex};
use innertube::{Channel, Client, Cookies};

use crate::account::AccountView;
use crate::assets::Icon;
use crate::feed::{FeedView, Source};
use crate::images::Images;
use crate::settings::Settings;
use crate::settings_page::SettingsView;
use crate::ui::image_fill;
use crate::watch::WatchView;
use crate::{prefetch, runtime, settings, theme};

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  Subscriptions,
  /// Session state and cookie import.
  Account,
  Settings,
  /// A channel's videos.
  Channel {
    id: String,
    name: String,
  },
  /// Watch page of a video id.
  Watch(String),
}

impl Route {
  /// Top-level pages reset the stack; the others are pushed on it.
  fn is_top_level(&self) -> bool {
    matches!(
      self,
      Self::Home | Self::Subscriptions | Self::Account | Self::Settings
    )
  }
}

struct Page {
  route: Route,
  view: AnyView,
}

pub struct AppView {
  client: Client,
  stack: Vec<Page>,
  /// Subscribed channels for the sidebar, once loaded (signed in only).
  channels: Vec<Channel>,
  /// Page to open at the next render (it needs the window), after a session change.
  reset_to: Option<Route>,
  open_input: Entity<InputState>,
  _open_subscription: Subscription,
}

impl AppView {
  /// The app on the home page, then on `initial_videos` in order (command-line URLs or ids).
  pub fn new(
    client: Client,
    initial_videos: Vec<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let open_input = cx.new(|cx| {
      InputState::new(window, cx).placeholder("Coller une URL ou un ID de vidéo YouTube…")
    });
    let subscription = cx.subscribe_in(
      &open_input,
      window,
      |this, input, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
          let text = input.read(cx).value().trim().to_string();
          match video_id(&text) {
            Some(id) => {
              input.update(cx, |state, cx| state.set_value("", window, cx));
              this.open(Route::Watch(id.to_owned()), window, cx);
            }
            None => tracing::info!("not a YouTube video URL or id"),
          }
        }
      },
    );
    let mut app = Self {
      client,
      stack: Vec::new(),
      channels: Vec::new(),
      reset_to: None,
      open_input,
      _open_subscription: subscription,
    };
    app.open(Route::Home, window, cx);
    for id in initial_videos {
      app.open(Route::Watch(id), window, cx);
    }
    app.load_channels(cx);
    app
  }

  /// Switches to a new session (imported cookies) or signs out (`None`), then shows Home.
  pub fn set_session(&mut self, cookies: Option<Cookies>, cx: &mut Context<Self>) {
    let Settings {
      language, region, ..
    } = Settings::get(cx).clone();
    let client = match Client::new(cookies, settings::cookies_path(), &language, &region) {
      Ok(client) => client,
      Err(e) => return tracing::error!("cannot create the HTTP client: {e}"),
    };
    tracing::info!(
      "session {}",
      if client.signed_in() {
        "imported"
      } else {
        "closed"
      }
    );
    let warmup = client.clone();
    drop(runtime::spawn(cx, async move {
      if let Err(e) = warmup.refresh_client_version().await {
        tracing::warn!("ytcfg: {e}");
      }
    }));
    self.client = client;
    self.channels.clear();
    self.load_channels(cx);
    // Pages hold the old client: they are rebuilt on the next navigation; Home now.
    self.reset_to = Some(Route::Home);
    cx.notify();
  }

  /// Rebuilds the clients after a language change: the session from the cookie file, and the
  /// anonymous one of the stream prefetch (caption names follow the language).
  pub fn reload_session(&mut self, cx: &mut Context<Self>) {
    let Settings {
      language, region, ..
    } = Settings::get(cx).clone();
    match Client::new(None, None, &language, &region) {
      Ok(anonymous) => prefetch::init(anonymous, cx),
      Err(e) => tracing::error!("cannot create the HTTP client: {e}"),
    }
    let load = cx
      .background_executor()
      .spawn(async { settings::load_cookies() });
    cx.spawn(async move |this, cx| {
      let cookies = load.await;
      this
        .update(cx, |this, cx| {
          this.set_session(cookies, cx);
          // Stay on the settings page; Home is rebuilt when opened.
          this.reset_to = None;
          this.stack.retain(|page| page.route.is_top_level());
        })
        .ok();
    })
    .detach();
  }

  fn load_channels(&self, cx: &mut Context<Self>) {
    if !self.client.signed_in() {
      return;
    }
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.subscribed_channels().await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok(channels)) => {
            tracing::info!("{} subscribed channels", channels.len());
            this.channels = channels;
            cx.notify();
          }
          Some(Err(e)) => tracing::warn!("subscribed channels: {e}"),
          None => {}
        })
        .ok();
    })
    .detach();
  }

  /// Shows `route`: top-level pages reset the stack, the others are pushed.
  pub fn open(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self.stack.last().is_some_and(|page| page.route == route) {
      return;
    }
    if route.is_top_level() {
      self.stack.clear();
    } else {
      // A covered watch page keeps its player (to resume after "back"), paused.
      self.set_top_covered(true, cx);
    }
    let nav = cx.weak_entity();
    let client = self.client.clone();
    let view: AnyView = match &route {
      Route::Home => cx
        .new(|cx| FeedView::new(client, Source::Home, nav, cx))
        .into(),
      Route::Subscriptions => cx
        .new(|cx| FeedView::new(client, Source::Subscriptions, nav, cx))
        .into(),
      Route::Settings => cx.new(|_| SettingsView::new(nav)).into(),
      Route::Account => {
        let signed_in = client.signed_in();
        cx.new(|_| AccountView::new(nav, signed_in)).into()
      }
      Route::Channel { id, name } => {
        let source = Source::Channel {
          id: id.clone(),
          name: name.clone(),
        };
        cx.new(|cx| FeedView::new(client, source, nav, cx)).into()
      }
      Route::Watch(id) => cx
        .new(|cx| WatchView::new(client, id.clone(), nav, window, cx))
        .into(),
    };
    self.stack.push(Page { route, view });
    cx.notify();
  }

  pub fn back(&mut self, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
      self.set_top_covered(false, cx);
      cx.notify();
    }
  }

  fn set_top_covered(&self, covered: bool, cx: &mut App) {
    if let Some(watch) = self
      .stack
      .last()
      .and_then(|page| page.view.clone().downcast::<WatchView>().ok())
    {
      watch.update(cx, |watch, cx| watch.set_covered(covered, cx));
    }
  }

  fn title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
    let back = (self.stack.len() > 1).then(|| {
      Button::new("back")
        .ghost()
        .icon(Icon::ArrowLeft)
        .tooltip("Retour")
        .on_click(cx.listener(|this, _, _, cx| this.back(cx)))
    });
    TitleBar::new().h(px(44.)).child(
      h_flex()
        .h_full()
        .w_full()
        .gap_3()
        .items_center()
        // The bar is a window drag area; occluding the controls keeps their clicks.
        .children(back.map(|b| div().occlude().child(b)))
        .child(
          div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child("PeebTube"),
        )
        .child(
          div()
            .occlude()
            .w(px(420.))
            .child(Input::new(&self.open_input).small()),
        ),
    )
  }

  fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let root = self.stack.first().map(|page| page.route.clone());
    let at_root = self.stack.len() == 1;
    let item = |id: SharedString, active: bool| {
      h_flex()
        .id(id)
        .gap_3()
        .items_center()
        .px_3()
        .py_2()
        .rounded(theme::RADIUS)
        .cursor_pointer()
        .when(active, |el| el.bg(theme::surface_hover()))
        .hover(|el| el.bg(theme::surface()))
    };
    let nav = |label: &'static str, icon: Icon, route: Route, cx: &mut Context<Self>| {
      let active = at_root && root.as_ref() == Some(&route);
      item(SharedString::from(label), active)
        .child(gpui_component::Icon::new(icon).size(px(18.)))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| this.open(route.clone(), window, cx)))
    };
    let current_channel = match self.stack.last().map(|page| &page.route) {
      Some(Route::Channel { id, .. }) => Some(id.clone()),
      _ => None,
    };
    let channels: Vec<_> = self
      .channels
      .iter()
      .map(|channel| {
        let route = Route::Channel {
          id: channel.id.clone(),
          name: channel.name.clone(),
        };
        let active = current_channel.as_deref() == Some(channel.id.as_str());
        item(
          SharedString::from(format!("channel-{}", channel.id)),
          active,
        )
        .child(div().size(px(24.)).flex_none().child(image_fill(
          &channel.avatar,
          24.,
          px(12.),
          window,
          cx,
        )))
        .child(
          div()
            .min_w_0()
            .text_sm()
            .line_clamp(1)
            .child(channel.name.clone()),
        )
        .on_click(cx.listener(move |this, _, window, cx| this.open(route.clone(), window, cx)))
      })
      .collect();
    v_flex()
      .id("sidebar")
      .w(theme::SIDEBAR_WIDTH)
      .h_full()
      .flex_none()
      .p_2()
      .gap_1()
      .overflow_y_scrollbar()
      .child(nav("Accueil", Icon::Home, Route::Home, cx))
      .child(nav(
        "Abonnements",
        Icon::Subscriptions,
        Route::Subscriptions,
        cx,
      ))
      .child(nav("Compte", Icon::Account, Route::Account, cx))
      .child(nav("Réglages", Icon::Settings, Route::Settings, cx))
      .when(!channels.is_empty(), |el| {
        el.child(
          div()
            .px_3()
            .pt_4()
            .pb_1()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::muted())
            .child("Chaînes"),
        )
        .children(channels)
      })
  }
}

impl Render for AppView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if let Some(route) = self.reset_to.take() {
      // The account page stays on screen with its message; Home is rebuilt underneath.
      let on_account = matches!(self.stack.last().map(|p| &p.route), Some(Route::Account));
      if !on_account {
        self.open(route, window, cx);
      } else {
        self.stack.retain(|page| page.route == Route::Account);
      }
    }
    let page = self.stack.last().map(|page| page.view.clone());
    let root = v_flex()
      .size_full()
      .bg(theme::chrome(cx))
      .text_color(theme::foreground());
    // Fullscreen belongs to the player: no title bar, no sidebar.
    if window.is_fullscreen() {
      return root.children(page);
    }
    root.child(self.title_bar(cx)).child(
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
            .bg(theme::background())
            .overflow_hidden()
            .children(page),
        ),
    )
  }
}

/// Video id of a watch URL (`watch?v=`, `youtu.be/`, `shorts/`) or of a bare 11-character id.
pub fn video_id(text: &str) -> Option<&str> {
  let is_id = |s: &str| {
    s.len() == 11
      && s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
  };
  if is_id(text) {
    return Some(text);
  }
  let rest = text
    .split_once("v=")
    .map(|(_, rest)| rest)
    .or_else(|| text.split_once("youtu.be/").map(|(_, rest)| rest))
    .or_else(|| text.split_once("/shorts/").map(|(_, rest)| rest))?;
  let id = rest.split(['&', '?', '#', '/']).next()?;
  is_id(id).then_some(id)
}

#[cfg(test)]
mod tests {
  use super::video_id;

  #[test]
  fn video_ids() {
    for url in [
      "dQw4w9WgXcQ",
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "https://www.youtube.com/watch?list=x&v=dQw4w9WgXcQ&t=10",
      "https://youtu.be/dQw4w9WgXcQ?si=abc",
      "https://www.youtube.com/shorts/dQw4w9WgXcQ",
    ] {
      assert_eq!(video_id(url), Some("dQw4w9WgXcQ"), "{url}");
    }
    assert_eq!(video_id("hello"), None);
    assert_eq!(video_id("https://www.youtube.com/"), None);
  }
}
