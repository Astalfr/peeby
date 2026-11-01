//! Root view: title bar (back, notifications), sidebar (home, subscriptions,
//! playlists, account, settings) and a stack of pages. In fullscreen only the page is shown.

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyView, App, Context, Entity, FontWeight, SharedString, Window, div,
  ease_out_quint, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{TitleBar, h_flex, v_flex};
use innertube::{Client, Cookies};

use crate::account::AccountView;
use crate::channels::ChannelsView;
use crate::feed::{FeedView, Source};
use crate::playlists::{PlaylistView, PlaylistsView};
use crate::queue::Queue;
use crate::settings::Settings;
use crate::settings_page::SettingsView;
use crate::watch::WatchView;
use crate::{bell, inbox, prefetch, save, settings};
use ui::assets::Icon;
use ui::images::Images;
use ui::{assets, runtime, theme};

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  Subscriptions,
  /// Session state and cookie import.
  Account,
  Settings,
  /// The subscribed channels.
  Channels,
  /// A channel's videos.
  Channel {
    id: String,
    name: String,
  },
  /// The user's playlists.
  Playlists,
  /// A playlist's videos.
  Playlist {
    id: String,
    title: String,
  },
  /// Watch page of a video id.
  Watch(String),
}

impl Route {
  /// "Watch later", a sidebar page.
  fn watch_later() -> Self {
    Self::Playlist {
      id: save::WATCH_LATER.into(),
      title: save::WATCH_LATER_TITLE.into(),
    }
  }

  /// Top-level pages (the sidebar's) reset the stack; the others are pushed on it.
  fn is_top_level(&self) -> bool {
    match self {
      Self::Home | Self::Subscriptions | Self::Playlists | Self::Account | Self::Settings => true,
      Self::Playlist { id, .. } => id == save::WATCH_LATER,
      _ => false,
    }
  }
}

struct Page {
  route: Route,
  view: AnyView,
}

pub struct AppView {
  client: Client,
  stack: Vec<Page>,
  /// Page to open at the next render (it needs the window), after a session change.
  reset_to: Option<Route>,
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
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    let unseen = inbox::inbox(cx);
    cx.observe(&unseen, |_, _, cx| cx.notify()).detach();
    inbox::reset(cx);
    let mut app = Self {
      client,
      stack: Vec::new(),
      reset_to: None,
    };
    app.open(Route::Home, window, cx);
    for id in initial_videos {
      app.open(Route::Watch(id), window, cx);
    }
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
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    inbox::reset(cx);
    self.client = client;
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
      Route::Channels => cx
        .new(|cx| ChannelsView::new(client, nav, window, cx))
        .into(),
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
      Route::Playlists => cx.new(|cx| PlaylistsView::new(client, nav, cx)).into(),
      Route::Playlist { id, title } => cx
        .new(|cx| PlaylistView::new(client, id.clone(), title.clone(), nav, cx))
        .into(),
      Route::Watch(id) => cx
        .new(|cx| WatchView::new(client, id.clone(), None, nav, window, cx))
        .into(),
    };
    self.stack.push(Page { route, view });
    cx.notify();
  }

  /// Plays the current video of `queue` on a watch page: pushed on the stack, or replacing the
  /// watch page on top (`replace`, from one video of the queue to the next). Replacing drops
  /// the previous page and its player, so a long queue keeps a single player, and "back"
  /// still leads to the page the queue was started from.
  pub fn play_queue(
    &mut self,
    queue: Entity<Queue>,
    replace: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(id) = queue.read(cx).current_video().map(|v| v.id.clone()) else {
      return;
    };
    if replace
      && self
        .stack
        .last()
        .is_some_and(|page| matches!(page.route, Route::Watch(_)))
    {
      self.stack.pop();
    } else {
      self.set_top_covered(true, cx);
    }
    let nav = cx.weak_entity();
    let client = self.client.clone();
    let route = Route::Watch(id.clone());
    let view = cx
      .new(|cx| WatchView::new(client, id, Some(queue), nav, window, cx))
      .into();
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

  fn title_bar(&self, window: &Window, cx: &mut Context<Self>) -> TitleBar {
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
    let back = (self.stack.len() > 1).then(|| {
      Button::new("back")
        .ghost()
        .icon(Icon::ArrowLeft)
        .tooltip("Retour")
        .on_click(cx.listener(|this, _, _, cx| this.back(cx)))
    });
    TitleBar::new().h(theme::TITLE_BAR_HEIGHT).child(
      h_flex()
        .h_full()
        .w_full()
        .gap_3()
        .items_center()
        // The bar is a window drag area; occluding the controls keeps their clicks.
        .child(div().occlude().child(menu))
        .children(back.map(|b| div().occlude().child(b)))
        .child(mark(window, cx))
        .child(div().flex_1())
        .children(
          inbox::bell_button(cx.weak_entity(), cx).map(|bell| div().occlude().mr_2().child(bell)),
        ),
    )
  }

  /// A sidebar entry: icon and label, muted until hovered or active; the active
  /// one has a light background and a red pill on its left edge. `collapsed`: the icon alone,
  /// centered.
  fn entry(id: SharedString, active: bool, collapsed: bool) -> gpui::Stateful<gpui::Div> {
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

  /// The entry of the page at the bottom of the stack is active, so Subscriptions stays lit
  /// on the channels and channel pages opened from it. Collapsed, entries show their icon
  /// alone and their label as a tooltip.
  fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let root = self.stack.first().map(|page| page.route.clone());
    let collapsed = Settings::get(cx).sidebar_collapsed;
    let nav = |label: &'static str, icon: Icon, route: Route, cx: &mut Context<Self>| {
      let active = root.as_ref() == Some(&route);
      Self::entry(SharedString::from(label), active, collapsed)
        .child(gpui_component::Icon::new(icon).size(px(18.)).flex_none())
        .map(|el| {
          if collapsed {
            el.tooltip(move |window, cx| {
              gpui_component::tooltip::Tooltip::new(label).build(window, cx)
            })
          } else {
            el.child(div().truncate().child(label))
          }
        })
        .on_click(cx.listener(move |this, _, window, cx| this.open(route.clone(), window, cx)))
    };
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
          .child(nav("Accueil", Icon::Home, Route::Home, cx))
          .child(nav(
            "Abonnements",
            Icon::Subscriptions,
            Route::Subscriptions,
            cx,
          ))
          .child(nav(
            save::WATCH_LATER_TITLE,
            Icon::WatchLater,
            Route::watch_later(),
            cx,
          ))
          .child(nav("Playlists", Icon::Playlists, Route::Playlists, cx)),
      )
      .child(
        v_flex()
          .flex_none()
          .px_2()
          .py_2()
          .gap_1()
          .border_t_1()
          .border_color(theme::border())
          .child(nav("Compte", Icon::Account, Route::Account, cx))
          .child(nav("Réglages", Icon::Settings, Route::Settings, cx)),
      )
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
    let root = v_flex()
      .size_full()
      .bg(theme::chrome(cx))
      .text_color(theme::foreground());
    // Fullscreen belongs to the player: no title bar, no sidebar.
    if window.is_fullscreen() {
      return root.children(self.stack.last().map(|page| page.view.clone()));
    }
    let page = self.stack.last().map(|page| {
      let view = div().size_full().relative().child(page.view.clone());
      // A short fade and rise each time a page appears. Not on the watch page: its video
      // surface is an external texture, and leaving fullscreen would replay the fade.
      if matches!(page.route, Route::Watch(_)) {
        return view.into_any_element();
      }
      view
        .with_animation(
          SharedString::from(format!("page-{}-{:?}", self.stack.len(), page.route)),
          Animation::new(theme::NORMAL).with_easing(ease_out_quint()),
          |el, t| el.opacity(t).top(px(8. * (1. - t))),
        )
        .into_any_element()
    });
    root.child(self.title_bar(window, cx)).child(
      h_flex()
        .flex_1()
        .min_h_0()
        .items_start()
        .child(self.sidebar(cx))
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
