//! Saving a video to a playlist: the "⋮" menu of the cards, the "Enregistrer" button of the
//! watch page, the "Watch later" shortcut over thumbnails. Every change ends with a toast,
//! success or failure.
//!
//! The list of playlists is loaded when it is opened (`playlist/get_add_to_playlist`), so it
//! shows where the video already is; a click adds the video, or removes it from a playlist
//! that holds it, as YouTube's "Save" dialog does.

use gpui::prelude::*;
use gpui::{
  Anchor, AnyWindowHandle, App, Context, ElementId, FontWeight, Global, MouseButton, SharedString,
  Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::popover::{Popover, PopoverState};
use gpui_component::skeleton::Skeleton;
use gpui_component::{Sizable, WindowExt, h_flex, v_flex};
use innertube::{Client, Error, PlaylistChoice};

use crate::assets::Icon;
use crate::ui::error_text;
use crate::{runtime, theme};

/// The app's current client, for actions started from any page. `AppView` sets it at startup
/// and on every session change.
pub struct Session(pub Client);

impl Global for Session {}

const WATCH_LATER: &str = "WL";
const WATCH_LATER_TITLE: &str = "À regarder plus tard";
const POPOVER_WIDTH: f32 = 280.;

/// Adds (`add`) or removes `video_id` in a playlist, then shows a toast in `window`. `done`
/// gets the outcome, e.g. to undo an optimistic change.
fn edit(
  playlist_id: String,
  playlist_title: String,
  video_id: String,
  add: bool,
  window: AnyWindowHandle,
  cx: &mut App,
  done: impl FnOnce(bool, &mut App) + 'static,
) {
  let Some(Session(client)) = cx.try_global::<Session>() else {
    return;
  };
  let client = client.clone();
  let task = runtime::spawn(cx, async move {
    if add {
      client.add_to_playlist(&playlist_id, &video_id).await
    } else {
      client.remove_from_playlist(&playlist_id, &video_id).await
    }
  });
  cx.spawn(async move |cx| {
    let Some(result) = task.await else { return };
    let note = match &result {
      Ok(()) if add => Notification::success(format!("Ajoutée à « {playlist_title} »")),
      Ok(()) => Notification::info(format!("Retirée de « {playlist_title} »")),
      Err(e) => {
        tracing::warn!("edit playlist: {e}");
        Notification::error(save_error(e))
      }
    };
    cx.update(|cx| {
      done(result.is_ok(), cx);
      window
        .update(cx, |_, window, cx| window.push_notification(note, cx))
        .ok();
    });
  })
  .detach();
}

fn save_error(error: &Error) -> String {
  match error {
    Error::SignInRequired => {
      "Connectez-vous pour enregistrer des vidéos : importez vos cookies depuis la page Compte."
        .into()
    }
    e => format!("Impossible d'enregistrer la vidéo. {}", error_text(e)),
  }
}

/// Adds `video_id` to "Watch later".
pub fn watch_later(video_id: &str, window: &mut Window, cx: &mut App) {
  edit(
    WATCH_LATER.into(),
    WATCH_LATER_TITLE.into(),
    video_id.into(),
    true,
    window.window_handle(),
    cx,
    |_, _| {},
  );
}

/// The round "Watch later" button over a thumbnail's top right corner, faded in by `t` (the
/// card's hover). Its clicks are its own, not the card's.
pub fn watch_later_button(id: &str, video_id: &str, t: f32) -> impl IntoElement {
  let video_id = video_id.to_owned();
  div()
    .id(ElementId::Name(format!("{id}-watch-later").into()))
    .absolute()
    .top(px(8.))
    .right(px(8.))
    .size(px(32.))
    .rounded_full()
    .flex()
    .items_center()
    .justify_center()
    .opacity(t)
    .bg(gpui::black().opacity(0.75))
    .text_color(gpui::white())
    .hover(|s| s.bg(theme::accent()).text_color(theme::on_accent()))
    .cursor_pointer()
    // The card opens the video on click: keep this click from reaching it.
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| {
      cx.stop_propagation();
      watch_later(&video_id, window, cx);
    })
    .tooltip(|window, cx| {
      gpui_component::tooltip::Tooltip::new(WATCH_LATER_TITLE).build(window, cx)
    })
    .child(gpui_component::Icon::new(Icon::WatchLater).size(px(16.)))
}

/// The "⋮" button of a card, faded in by `t`: a menu with "Watch later" and "Add to a
/// playlist", which turns into the list of playlists.
pub fn menu_button(id: &str, video_id: &str, t: f32) -> impl IntoElement {
  let key = format!("{id}-menu");
  let video_id = video_id.to_owned();
  div()
    .flex_none()
    .opacity(t)
    // The card opens the video on click: keep this click from reaching it.
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .child(
      Popover::new(ElementId::Name(key.clone().into()))
        .anchor(Anchor::TopRight)
        .trigger(
          Button::new(ElementId::Name(format!("{key}-button").into()))
            .ghost()
            .small()
            .icon(Icon::More)
            .tooltip("Plus d'actions"),
        )
        .content(move |_, window, cx| picker_content(&key, &video_id, false, window, cx)),
    )
}

/// The "Enregistrer" button of the watch page: the list of playlists right away.
pub fn save_button(video_id: &str) -> impl IntoElement {
  let key = format!("save-{video_id}");
  let video_id = video_id.to_owned();
  Popover::new(ElementId::Name(key.clone().into()))
    .anchor(Anchor::TopRight)
    .trigger(
      Button::new("save")
        .icon(Icon::Save)
        .label("Enregistrer")
        .tooltip("Ajouter à une playlist"),
    )
    .content(move |_, window, cx| picker_content(&key, &video_id, true, window, cx))
}

/// What a popover shows: the menu, or the playlists once asked for (`list`). The picker lives
/// in the popover's element state, so it starts over (and reloads) each time it opens.
fn picker_content(
  key: &str,
  video_id: &str,
  list: bool,
  window: &mut Window,
  cx: &mut Context<PopoverState>,
) -> gpui::AnyElement {
  let video = video_id.to_owned();
  let picker = window.use_keyed_state(
    ElementId::Name(format!("{key}-picker").into()),
    cx,
    move |_, cx| {
      let mut picker = Picker {
        video_id: video,
        choices: Choices::Idle,
      };
      if list {
        picker.load(cx);
      }
      picker
    },
  );
  if !matches!(picker.read(cx).choices, Choices::Idle) {
    return div().w(px(POPOVER_WIDTH)).child(picker).into_any_element();
  }
  let video_id = video_id.to_owned();
  let item = |id: &'static str, icon: Icon, label: &'static str| {
    h_flex()
      .id(id)
      .w_full()
      .px_2()
      .py_1p5()
      .gap_3()
      .items_center()
      .rounded(theme::RADIUS)
      .cursor_pointer()
      .text_sm()
      .hover(|s| s.bg(theme::surface_hover()))
      .child(gpui_component::Icon::new(icon).size(px(16.)))
      .child(label)
  };
  v_flex()
    .w(px(220.))
    .gap_0p5()
    .child(
      item("menu-watch-later", Icon::WatchLater, WATCH_LATER_TITLE).on_click(cx.listener(
        move |state, _, window, cx| {
          watch_later(&video_id, window, cx);
          state.dismiss(window, cx);
        },
      )),
    )
    .child(
      item("menu-save", Icon::Save, "Ajouter à une playlist").on_click(move |_, _, cx| {
        picker.update(cx, |picker, cx| picker.load(cx));
      }),
    )
    .into_any_element()
}

enum Choices {
  /// Not asked for yet (the menu shows).
  Idle,
  Loading,
  Loaded(Vec<PlaylistChoice>),
  Failed(SharedString),
}

/// The playlists of the user, a check on those holding the video.
struct Picker {
  video_id: String,
  choices: Choices,
}

impl Picker {
  fn load(&mut self, cx: &mut Context<Self>) {
    let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
      return;
    };
    if !client.signed_in() {
      self.choices = Choices::Failed(save_error(&Error::SignInRequired).into());
      return cx.notify();
    }
    self.choices = Choices::Loading;
    cx.notify();
    let video_id = self.video_id.clone();
    let fetch = runtime::spawn(cx, async move { client.playlist_choices(&video_id).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.choices = match result {
            Some(Ok(choices)) => Choices::Loaded(choices),
            Some(Err(e)) => {
              tracing::warn!("playlist choices: {e}");
              Choices::Failed(save_error(&e).into())
            }
            None => return,
          };
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Adds the video to playlist `index`, or removes it. The check flips at once and flips
  /// back if YouTube refuses.
  fn toggle(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Choices::Loaded(choices) = &mut self.choices else {
      return;
    };
    let Some(choice) = choices.get_mut(index) else {
      return;
    };
    choice.contains = !choice.contains;
    let add = choice.contains;
    let this = cx.weak_entity();
    edit(
      choice.id.clone(),
      choice.title.clone(),
      self.video_id.clone(),
      add,
      window.window_handle(),
      cx,
      move |ok, cx| {
        if ok {
          return;
        }
        this
          .update(cx, |this, cx| {
            if let Choices::Loaded(choices) = &mut this.choices
              && let Some(choice) = choices.get_mut(index)
            {
              choice.contains = !add;
              cx.notify();
            }
          })
          .ok();
      },
    );
    cx.notify();
  }
}

impl Render for Picker {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let body = match &self.choices {
      Choices::Idle | Choices::Loading => v_flex()
        .gap_2()
        .children((0..4).map(|_| {
          h_flex()
            .px_2()
            .py_1p5()
            .gap_3()
            .child(Skeleton::new().size(px(16.)).rounded(px(4.)))
            .child(Skeleton::new().w(px(160.)).h(px(14.)))
        }))
        .into_any_element(),
      Choices::Failed(message) => div()
        .px_2()
        .text_sm()
        .text_color(theme::muted())
        .child(message.clone())
        .into_any_element(),
      Choices::Loaded(choices) => v_flex()
        .id("playlist-choices")
        .max_h(px(320.))
        .overflow_y_scroll()
        .gap_0p5()
        .children(choices.iter().enumerate().map(|(i, choice)| {
          h_flex()
            .id(i)
            .px_2()
            .py_1p5()
            .gap_3()
            .items_center()
            .rounded(theme::RADIUS)
            .cursor_pointer()
            .text_sm()
            .hover(|s| s.bg(theme::surface_hover()))
            .on_click(cx.listener(move |this, _, window, cx| this.toggle(i, window, cx)))
            .child(
              div()
                .size(px(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .map(|el| {
                  if choice.contains {
                    el.bg(theme::accent())
                      .border_color(theme::accent())
                      .text_color(theme::on_accent())
                      .child(gpui_component::Icon::new(Icon::Check).size(px(12.)))
                  } else {
                    el.border_color(theme::muted())
                  }
                }),
            )
            .child(
              div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(choice.title.clone()),
            )
            .when(choice.private, |el| {
              el.child(
                gpui_component::Icon::new(Icon::Lock)
                  .size(px(14.))
                  .text_color(theme::muted()),
              )
            })
        }))
        .into_any_element(),
    };
    v_flex()
      .gap_2()
      .child(
        div()
          .px_2()
          .font_weight(FontWeight::SEMIBOLD)
          .child("Enregistrer dans…"),
      )
      .child(body)
  }
}
