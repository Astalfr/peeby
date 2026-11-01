//! Root view: title bar (back, open a video by URL or id), sidebar and a stack of pages.

use gpui::prelude::*;
use gpui::{AnyView, Context, Entity, FontWeight, SharedString, Subscription, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Sizable, TitleBar, h_flex, v_flex};
use innertube::Client;

use crate::home::HomeView;
use crate::theme;
use crate::watch::WatchView;

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  /// Watch page of a video id.
  Watch(String),
}

struct Page {
  route: Route,
  view: AnyView,
}

pub struct AppView {
  client: Client,
  stack: Vec<Page>,
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
      open_input,
      _open_subscription: subscription,
    };
    app.open(Route::Home, window, cx);
    for id in initial_videos {
      app.open(Route::Watch(id), window, cx);
    }
    app
  }

  /// Shows `route`: the home page resets the stack, other pages are pushed.
  pub fn open(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self.stack.last().is_some_and(|page| page.route == route) {
      return;
    }
    // A covered watch page keeps its player (to resume after "back"), paused.
    if let Some(watch) = self
      .stack
      .last()
      .and_then(|page| page.view.clone().downcast::<WatchView>().ok())
    {
      watch.update(cx, |watch, _| watch.set_covered(true));
    }
    let nav = cx.weak_entity();
    let view: AnyView = match &route {
      Route::Home => {
        self.stack.clear();
        cx.new(|cx| HomeView::new(self.client.clone(), nav, cx))
          .into()
      }
      Route::Watch(id) => cx
        .new(|cx| WatchView::new(self.client.clone(), id.clone(), nav, window, cx))
        .into(),
    };
    self.stack.push(Page { route, view });
    cx.notify();
  }

  pub fn back(&mut self, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
      if let Some(watch) = self
        .stack
        .last()
        .and_then(|page| page.view.clone().downcast::<WatchView>().ok())
      {
        watch.update(cx, |watch, _| watch.set_covered(false));
      }
      cx.notify();
    }
  }

  fn title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
    let back = (self.stack.len() > 1).then(|| {
      Button::new("back")
        .ghost()
        .label("←")
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

  fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let on_home = matches!(
      self.stack.first(),
      Some(Page {
        route: Route::Home,
        ..
      })
    );
    let item = |id: &'static str, label: &'static str, active: bool| {
      div()
        .id(id)
        .px_3()
        .py_2()
        .rounded(theme::RADIUS)
        .cursor_pointer()
        .when(active, |el| el.bg(theme::surface_hover()))
        .hover(|el| el.bg(theme::surface()))
        .child(SharedString::from(label))
    };
    v_flex()
      .w(theme::SIDEBAR_WIDTH)
      .flex_none()
      .p_2()
      .gap_1()
      .child(
        item("nav-home", "Accueil", on_home && self.stack.len() == 1).on_click(cx.listener(
          |this, _, window, cx| {
            this.stack.clear();
            this.open(Route::Home, window, cx);
          },
        )),
      )
  }
}

impl Render for AppView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let page = self.stack.last().map(|page| page.view.clone());
    v_flex()
      .size_full()
      .bg(theme::chrome(cx))
      .text_color(theme::foreground())
      .child(self.title_bar(cx))
      .child(
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
