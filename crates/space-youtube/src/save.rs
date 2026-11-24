//! Saving a video to a playlist: the "⋮" menus of the cards and of the watch page, the "Watch
//! later" shortcut over thumbnails. Every change ends with a toast,
//! success or failure.
//!
//! The list of playlists is loaded when it is opened (`playlist/get_add_to_playlist`), so it
//! shows where the video already is; a click adds the video, or removes it from a playlist
//! that holds it, as YouTube's "Save" dialog does.
//!
//! The "x" of a playlist's videos removes one entry ([`removal`]); the pages showing that
//! playlist hear about it through [`Edits`] and drop the entry.
//!
//! The ids of the videos in "Watch later" are kept ([`load_watch_later`], then every change),
//! so the thumbnails' shortcut shows a check on those already in it.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Anchor, AnyWindowHandle, App, AppContext, ClipboardItem, Context, ElementId, Entity,
  EventEmitter, FontWeight, Global, MouseButton, SharedString, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::popover::{Popover, PopoverState};
use gpui_component::skeleton::Skeleton;
use gpui_component::{Sizable, WindowExt, h_flex, v_flex};
use innertube::{Client, Error, PlaylistChoice};

use crate::convert::error_text;
use ui::assets::Icon;
use ui::card::RowAction;
use ui::i18n::t;
use ui::widgets::loader;
use ui::{runtime, tf, theme};

/// The app's current client, for actions started from any page. `YouTubeSpace` sets it at startup
/// and on every session change.
pub struct Session(pub Client);

impl Global for Session {}

/// "Watch later": its playlist id, and [`watch_later_title`] its title in the app.
pub const WATCH_LATER: &str = "WL";

/// The title of "Watch later" in the app, in the interface's language.
pub fn watch_later_title() -> &'static str {
  t("Watch later", "À regarder plus tard")
}
const POPOVER_WIDTH: f32 = 280.;

/// A change to a playlist.
enum Change {
  /// Add this video.
  Add(String),
  /// Remove this video, wherever it is.
  Remove(String),
  /// Remove this entry ([`innertube::VideoItem::set_video_id`]) of this video.
  RemoveEntry { entry: String, video: String },
}

impl Change {
  fn video(&self) -> &str {
    match self {
      Self::Add(video) | Self::Remove(video) | Self::RemoveEntry { video, .. } => video,
    }
  }
}

/// The videos in "Watch later", once loaded. `generation` tells apart the loads of successive
/// sessions, so a late answer for an old session is dropped.
#[derive(Default)]
struct WatchLater {
  ids: HashSet<String>,
  generation: u64,
}

impl Global for WatchLater {}

/// Most pages of "Watch later" read at startup (100 videos each).
const WATCH_LATER_PAGES: usize = 50;

/// Reads the ids of the videos in "Watch later", in the background. Called at startup and on
/// every session change.
pub fn load_watch_later(cx: &mut App) {
  let state = cx.default_global::<WatchLater>();
  state.ids.clear();
  state.generation += 1;
  let generation = state.generation;
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return;
  }
  // Only the ids are kept: no original titles to look up.
  let client = client.with_original_titles(false);
  let task = runtime::spawn(cx, async move {
    let playlist = client.playlist(WATCH_LATER).await?;
    let mut ids: HashSet<String> = playlist.videos.into_iter().map(|v| v.id).collect();
    let mut token = playlist.continuation;
    for _ in 1..WATCH_LATER_PAGES {
      let Some(next) = token else { break };
      let page = client.playlist_more(&next).await?;
      ids.extend(page.items.into_iter().map(|v| v.id));
      token = page.continuation;
    }
    Ok::<_, Error>(ids)
  });
  cx.spawn(async move |cx| {
    let ids = match task.await {
      Some(Ok(ids)) => ids,
      Some(Err(e)) => return tracing::warn!("watch later: {e}"),
      None => return,
    };
    cx.update(|cx| {
      let state = cx.default_global::<WatchLater>();
      if state.generation == generation {
        tracing::info!("watch later: {} videos", ids.len());
        state.ids = ids;
        cx.refresh_windows();
      }
    });
  })
  .detach();
}

/// Whether `video_id` is in "Watch later", as far as the app knows.
fn in_watch_later(video_id: &str, cx: &App) -> bool {
  cx.try_global::<WatchLater>()
    .is_some_and(|w| w.ids.contains(video_id))
}

/// One entry left a playlist: pages showing it drop the entry.
pub struct EntryRemoved {
  pub playlist_id: String,
  pub set_video_id: String,
}

/// Where playlist edits are announced, for the pages that show playlists.
pub struct Edits;

impl EventEmitter<EntryRemoved> for Edits {}

#[derive(Clone)]
struct EditsHandle(Entity<Edits>);

impl Global for EditsHandle {}

/// The edit announcer, created the first time it is asked for.
pub fn edits(cx: &mut App) -> Entity<Edits> {
  if let Some(EditsHandle(edits)) = cx.try_global::<EditsHandle>() {
    return edits.clone();
  }
  let edits = cx.new(|_| Edits);
  cx.set_global(EditsHandle(edits.clone()));
  edits
}

/// Changes waiting for YouTube's answer, with when they were sent: they show at once (the
/// check, the row gone), a second click sends nothing, and their buttons show a spinner only
/// past [`SPINNER_AFTER`].
#[derive(Default)]
struct Pending(HashMap<String, Instant>);

/// How long a change waits for YouTube before its button shows a spinner: shorter answers
/// would only make it flicker.
pub(crate) const SPINNER_AFTER: Duration = Duration::from_millis(400);

impl Global for Pending {}

fn change_key(playlist_id: &str, change: &Change) -> String {
  match change {
    Change::Add(video) => format!("{playlist_id}:add:{video}"),
    Change::Remove(video) => format!("{playlist_id}:remove:{video}"),
    Change::RemoveEntry { entry, .. } => format!("{playlist_id}:entry:{entry}"),
  }
}

fn is_pending(key: &str, cx: &App) -> bool {
  cx.try_global::<Pending>()
    .is_some_and(|p| p.0.contains_key(key))
}

/// Whether the change `key` has waited long enough for a spinner.
fn is_slow(key: &str, cx: &App) -> bool {
  cx.try_global::<Pending>()
    .and_then(|p| p.0.get(key))
    .is_some_and(|sent| sent.elapsed() >= SPINNER_AFTER)
}

/// Whether the entry `set_video_id` of `playlist_id` is being removed: its row is hidden
/// meanwhile, and comes back if YouTube refuses.
pub fn removing(playlist_id: &str, set_video_id: &str, cx: &App) -> bool {
  let change = Change::RemoveEntry {
    entry: set_video_id.to_owned(),
    video: String::new(),
  };
  is_pending(&change_key(playlist_id, &change), cx)
}

/// Adds or removes `video_id` in the known "Watch later" ids.
fn set_in_watch_later(video_id: &str, saved: bool, cx: &mut App) {
  let ids = &mut cx.default_global::<WatchLater>().ids;
  if saved {
    ids.insert(video_id.to_owned());
  } else {
    ids.remove(video_id);
  }
}

/// Applies `change` to a playlist, then shows a toast in `window`. `done` gets the outcome,
/// e.g. to undo an optimistic change. Nothing is sent while the same change is pending.
/// "Watch later" changes at once (its button shows the outcome), and goes back if YouTube
/// refuses.
fn edit(
  playlist_id: String,
  playlist_title: String,
  change: Change,
  window: AnyWindowHandle,
  cx: &mut App,
  done: impl FnOnce(bool, &mut App) + 'static,
) {
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  let key = change_key(&playlist_id, &change);
  let pending = &mut cx.default_global::<Pending>().0;
  if pending.contains_key(&key) {
    return;
  }
  pending.insert(key.clone(), Instant::now());
  let adding = matches!(change, Change::Add(_));
  if playlist_id == WATCH_LATER {
    set_in_watch_later(change.video(), adding, cx);
  }
  cx.refresh_windows();
  // A slow answer: the button shows its spinner.
  cx.spawn({
    let key = key.clone();
    async move |cx| {
      cx.background_executor().timer(SPINNER_AFTER).await;
      cx.update(|cx| {
        if is_pending(&key, cx) {
          cx.refresh_windows();
        }
      });
    }
  })
  .detach();
  let removed_entry = match &change {
    Change::RemoveEntry { entry, .. } => Some(entry.clone()),
    _ => None,
  };
  let video_id = change.video().to_owned();
  let playlist = playlist_id.clone();
  let task = runtime::spawn(cx, async move {
    match change {
      Change::Add(video) => client.add_to_playlist(&playlist, &video).await,
      Change::Remove(video) => client.remove_from_playlist(&playlist, &video).await,
      Change::RemoveEntry { entry, .. } => client.remove_playlist_entry(&playlist, &entry).await,
    }
  });
  cx.spawn(async move |cx| {
    let Some(result) = task.await else { return };
    let note = match &result {
      Ok(()) if adding => Notification::success(tf!(
        "Added to \"{playlist_title}\"",
        "Ajoutée à « {playlist_title} »"
      )),
      Ok(()) => Notification::info(tf!(
        "Removed from \"{playlist_title}\"",
        "Retirée de « {playlist_title} »"
      )),
      Err(e) => {
        tracing::warn!("edit playlist: {e}");
        Notification::error(save_error(e, adding))
      }
    };
    cx.update(|cx| {
      cx.default_global::<Pending>().0.remove(&key);
      if playlist_id == WATCH_LATER {
        // Refused: back to what it was.
        set_in_watch_later(&video_id, adding == result.is_ok(), cx);
      }
      cx.refresh_windows();
      if result.is_ok()
        && let Some(set_video_id) = removed_entry
      {
        edits(cx).update(cx, |_, cx| {
          cx.emit(EntryRemoved {
            playlist_id,
            set_video_id,
          })
        });
      }
      done(result.is_ok(), cx);
      window
        .update(cx, |_, window, cx| window.push_notification(note, cx))
        .ok();
    });
  })
  .detach();
}

fn save_error(error: &Error, adding: bool) -> String {
  match error {
    Error::SignInRequired => t(
      "To edit your playlists, sign in to YouTube in the settings.",
      "Pour modifier vos playlists, connectez-vous à YouTube dans les réglages.",
    )
    .into(),
    e if adding => tf!(
      "Cannot save the video. {}",
      "Impossible d'enregistrer la vidéo. {}",
      error_text(e)
    ),
    e => tf!(
      "Cannot remove the video. {}",
      "Impossible de retirer la vidéo. {}",
      error_text(e)
    ),
  }
}

/// The "x" of a playlist's entry: removes it from the playlist; on success the pages showing
/// the playlist drop it ([`EntryRemoved`]); meanwhile the pages hide it ([`removing`]).
pub fn removal(
  playlist_id: &str,
  playlist_title: &str,
  set_video_id: &str,
  video_id: &str,
  cx: &App,
) -> RowAction {
  let change = || Change::RemoveEntry {
    entry: set_video_id.to_owned(),
    video: video_id.to_owned(),
  };
  let busy = is_slow(&change_key(playlist_id, &change()), cx);
  let (playlist_id, playlist_title, entry, video) = (
    playlist_id.to_owned(),
    playlist_title.to_owned(),
    set_video_id.to_owned(),
    video_id.to_owned(),
  );
  RowAction {
    label: t("Remove from the playlist", "Retirer de la playlist").into(),
    busy,
    run: Box::new(move |window, cx| {
      edit(
        playlist_id.clone(),
        playlist_title.clone(),
        Change::RemoveEntry {
          entry: entry.clone(),
          video: video.clone(),
        },
        window.window_handle(),
        cx,
        |_, _| {},
      )
    }),
  }
}

/// Adds `video_id` to "Watch later", or removes it when it is in already.
pub fn toggle_watch_later(video_id: &str, window: &mut Window, cx: &mut App) {
  // The button already shows the pending change: a second click would undo it unseen.
  if watch_later_busy(video_id, false, cx) {
    return;
  }
  let change = if in_watch_later(video_id, cx) {
    Change::Remove(video_id.into())
  } else {
    Change::Add(video_id.into())
  };
  edit(
    WATCH_LATER.into(),
    watch_later_title().into(),
    change,
    window.window_handle(),
    cx,
    |_, _| {},
  );
}

/// Whether a "Watch later" change of `video_id` is waiting for YouTube (`slow`: for longer
/// than [`SPINNER_AFTER`]).
fn watch_later_busy(video_id: &str, slow: bool, cx: &App) -> bool {
  let video = video_id.to_owned();
  [Change::Add(video.clone()), Change::Remove(video)]
    .iter()
    .map(|change| change_key(WATCH_LATER, change))
    .any(|key| {
      if slow {
        is_slow(&key, cx)
      } else {
        is_pending(&key, cx)
      }
    })
}

/// The round "Watch later" button over a thumbnail's top right corner, faded in by `t` (the
/// card's hover): a clock, or a check when the video is in "Watch later" already (a click
/// then removes it), shown at the click, and a spinner while a slow answer is awaited. Its
/// clicks are its own, not the card's.
pub fn watch_later_button(id: &str, video_id: &str, t: f32, cx: &App) -> impl IntoElement {
  let pending = watch_later_busy(video_id, false, cx);
  let busy = watch_later_busy(video_id, true, cx);
  let saved = in_watch_later(video_id, cx);
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
    // Kept in view while YouTube answers, even once the mouse has left the card.
    .opacity(if pending { 1. } else { t })
    .bg(gpui::black().opacity(0.75))
    .text_color(gpui::white())
    .when(!pending, |el| {
      el.hover(|s| s.bg(theme::accent()).text_color(theme::on_accent()))
        .active(|s| s.bg(theme::accent_active()))
        .cursor_pointer()
    })
    // The card opens the video on click: keep this click from reaching it.
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| {
      cx.stop_propagation();
      if !watch_later_busy(&video_id, false, cx) {
        toggle_watch_later(&video_id, window, cx);
      }
    })
    .tooltip(move |window, cx| {
      let label = if saved {
        ui::i18n::t(
          "Remove from \"Watch later\"",
          "Retirer de « À regarder plus tard »",
        )
      } else {
        watch_later_title()
      };
      gpui_component::tooltip::Tooltip::new(label).build(window, cx)
    })
    .child(if busy {
      loader(18., gpui::white()).into_any_element()
    } else {
      gpui_component::Icon::new(if saved { Icon::Check } else { Icon::WatchLater })
        .size(px(16.))
        .into_any_element()
    })
}

/// What a card's menu item does, given by the page showing the card.
pub type CardAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// The "⋮" button of a card, faded in by `t`: a menu with "Watch later" and "Add to a
/// playlist", which turns into the list of playlists, then "Not interested" when the page
/// gives `not_interested` (a recommendation, signed in).
pub fn menu_button(
  id: &str,
  video_id: &str,
  t: f32,
  not_interested: Option<CardAction>,
) -> impl IntoElement {
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
            // 1.5× the small button's 14 px icon: a `Size::Size` button draws its icon at 75 %.
            .with_size(px(28.))
            .icon(Icon::More)
            .tooltip(ui::i18n::t("More actions", "Plus d'actions")),
        )
        .content(move |_, window, cx| {
          let menu = Menu::Card(not_interested.clone());
          picker_content(&key, &video_id, menu, window, cx)
        }),
    )
}

/// The "⋮" button of the watch page: a menu with "Save to a playlist", which turns into the
/// list of playlists, and "Copy video link".
pub fn watch_menu_button(video_id: &str) -> impl IntoElement {
  let key = format!("watch-menu-{video_id}");
  let video_id = video_id.to_owned();
  Popover::new(ElementId::Name(key.clone().into()))
    .anchor(Anchor::TopRight)
    .trigger(
      Button::new("watch-menu")
        .ghost()
        .icon(Icon::More)
        .tooltip(t("More actions", "Plus d'actions")),
    )
    .content(move |_, window, cx| picker_content(&key, &video_id, Menu::Watch, window, cx))
}

/// Which "⋮" menu a popover shows.
#[derive(Clone)]
enum Menu {
  /// A card's: "Watch later", "Add to a playlist" and, given, "Not interested".
  Card(Option<CardAction>),
  /// The watch page's: "Save to a playlist" and "Copy video link".
  Watch,
}

/// What a popover shows: the menu, or the playlists once asked for. The picker lives in the
/// popover's element state, so it starts over (and reloads) each time it opens.
fn picker_content(
  key: &str,
  video_id: &str,
  menu: Menu,
  window: &mut Window,
  cx: &mut Context<PopoverState>,
) -> gpui::AnyElement {
  let video = video_id.to_owned();
  let picker = window.use_keyed_state(
    ElementId::Name(format!("{key}-picker").into()),
    cx,
    move |_, _| Picker {
      video_id: video,
      choices: Choices::Idle,
    },
  );
  if !matches!(picker.read(cx).choices, Choices::Idle) {
    return div().w(px(POPOVER_WIDTH)).child(picker).into_any_element();
  }
  let video_id = video_id.to_owned();
  let link = format!("https://www.youtube.com/watch?v={video_id}");
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
      .active(|s| s.bg(theme::surface_pressed()))
      .child(gpui_component::Icon::new(icon).size(px(16.)))
      .child(label)
  };
  let save_label = match menu {
    Menu::Card(_) => t("Add to a playlist", "Ajouter à une playlist"),
    Menu::Watch => t("Save to a playlist", "Enregistrer dans une playlist"),
  };
  let (card, watch) = (matches!(menu, Menu::Card(_)), matches!(menu, Menu::Watch));
  let not_interested = match menu {
    Menu::Card(action) => action,
    Menu::Watch => None,
  };
  v_flex()
    .w(px(220.))
    .gap_0p5()
    .when(card, |el| {
      el.child(
        if in_watch_later(&video_id, cx) {
          item(
            "menu-watch-later",
            Icon::Check,
            t(
              "Remove from \"Watch later\"",
              "Retirer de « À regarder plus tard »",
            ),
          )
        } else {
          item("menu-watch-later", Icon::WatchLater, watch_later_title())
        }
        .on_click(cx.listener(move |state, _, window, cx| {
          toggle_watch_later(&video_id, window, cx);
          state.dismiss(window, cx);
        })),
      )
    })
    .child(
      item("menu-save", Icon::Save, save_label).on_click(move |_, _, cx| {
        picker.update(cx, |picker, cx| picker.load(cx));
      }),
    )
    .when_some(not_interested, |el, action| {
      el.child(
        item(
          "menu-not-interested",
          Icon::Hide,
          t("Not interested", "Pas intéressé"),
        )
        .on_click(cx.listener(move |state, _, window, cx| {
          state.dismiss(window, cx);
          action(window, cx);
        })),
      )
    })
    .when(watch, |el| {
      el.child(
        item(
          "menu-copy-link",
          Icon::Link,
          t("Copy video link", "Copier le lien de la vidéo"),
        )
        .on_click(cx.listener(move |state, _, window, cx| {
          cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
          window.push_notification(Notification::success(t("Link copied", "Lien copié")), cx);
          state.dismiss(window, cx);
        })),
      )
    })
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
      self.choices = Choices::Failed(save_error(&Error::SignInRequired, true).into());
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
              Choices::Failed(save_error(&e, true).into())
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
    let video = self.video_id.clone();
    edit(
      choice.id.clone(),
      choice.title.clone(),
      if add {
        Change::Add(video)
      } else {
        Change::Remove(video)
      },
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
            .active(|s| s.bg(theme::surface_pressed()))
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
          .child(t("Save to…", "Enregistrer dans…")),
      )
      .child(body)
  }
}
