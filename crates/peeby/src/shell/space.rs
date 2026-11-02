//! What the shell asks of a space (YouTube, Jellyfin, later Twitch), and a handle that hides
//! the space's type. A space is a GPUI entity that renders its current page; the shell draws
//! the top bar, the sidebar from the space's entries, and the mini-player.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, Context, Entity, EventEmitter, SharedString, Subscription, WeakEntity,
  Window,
};
use gpui_component::popover::PopoverState;
use ui::assets::Icon;
use ui::picture::Picture;
use ui::player::PlayerView;
use ui::theme::Accent;

/// How a space shows in the tabs.
#[derive(Clone)]
pub struct SpaceInfo {
  /// Tab label, e.g. `YouTube`.
  pub name: SharedString,
  /// The service's logo on the tab: a colored SVG with a square view box.
  pub icon: &'static [u8],
  /// The app's accent while the space is open.
  pub accent: Accent,
}

/// A user's picture in the sidebar: their avatar over a disc in the accent with their initial,
/// which shows while it loads or when there is none.
#[derive(Clone)]
pub struct Avatar {
  /// The avatar, if the user has one.
  pub picture: Option<Picture>,
  /// The first letter of their name.
  pub initial: SharedString,
}

/// An entry of the sidebar.
#[derive(Clone)]
pub struct MenuEntry {
  /// Passed back to [`Space::open_entry`].
  pub id: SharedString,
  /// Label, and tooltip of the folded sidebar.
  pub label: SharedString,
  /// Icon.
  pub icon: Icon,
  /// The page shown belongs to this entry.
  pub active: bool,
  /// Pinned at the bottom of the sidebar (account).
  pub bottom: bool,
  /// Shown instead of the icon (the signed-in user).
  pub avatar: Option<Avatar>,
  /// A muted second line under the label, e.g. the server's address.
  pub detail: Option<SharedString>,
  /// A heading above the entry, e.g. `Bibliothèques` above the first library.
  pub heading: Option<SharedString>,
  /// On air: a "LIVE" pill at the right of the entry, a dot on its icon when folded.
  pub live: bool,
}

impl MenuEntry {
  /// An entry with its id, label and icon.
  pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: Icon) -> Self {
    Self {
      id: id.into(),
      label: label.into(),
      icon,
      active: false,
      bottom: false,
      avatar: None,
      detail: None,
      heading: None,
      live: false,
    }
  }
}

/// What a space asks of the shell.
pub enum SpaceEvent {
  /// Show this space (a notification of it was clicked from another tab).
  Activate,
  /// Open the settings (a page's "import cookies" button, the account entry).
  OpenSettings,
}

/// A service's part of the app: its pages, its sidebar entries and what it plays.
pub trait Space: Render + EventEmitter<SpaceEvent> {
  /// Name, logo and accent.
  fn info(&self) -> SpaceInfo;

  /// Sidebar entries, top to bottom.
  fn menu(&self, cx: &App) -> Vec<MenuEntry>;

  /// Opens the page of a sidebar entry.
  fn open_entry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>);

  /// Whether "back" leads somewhere within the space.
  fn can_go_back(&self) -> bool;

  /// Goes back one page.
  fn back(&mut self, cx: &mut Context<Self>);

  /// Whether the top bar's search button searches this space (signed in, with a search page).
  fn searchable(&self) -> bool {
    false
  }

  /// Opens the space's search page.
  fn open_search(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

  /// The player of the page on top, if it has one. The shell moves it to the mini-player
  /// when another tab opens while it plays.
  fn player(&self, _cx: &App) -> Option<Entity<PlayerView>> {
    None
  }

  /// Unread notifications; `None` when the space has none to show (no inbox, signed out).
  fn unseen(&self, _cx: &App) -> Option<usize> {
    None
  }

  /// The space's part of the settings page (behind the gear), under its name.
  fn settings(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<AnyElement> {
    None
  }

  /// The space's section of the notifications panel; `popover` closes the panel.
  fn notifications(
    &self,
    _popover: WeakEntity<PopoverState>,
    _window: &mut Window,
    _cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    None
  }
}

/// What the shell does with a space's [`SpaceEvent`]s.
pub type EventHandler = dyn Fn(&SpaceEvent, &mut App);

/// A space whatever its type, as the shell holds it.
pub trait AnySpace {
  /// The space's entity as a view (its current page).
  fn view(&self) -> AnyView;
  /// See [`Space::info`].
  fn info(&self, cx: &App) -> SpaceInfo;
  /// See [`Space::menu`].
  fn menu(&self, cx: &App) -> Vec<MenuEntry>;
  /// See [`Space::open_entry`].
  fn open_entry(&self, id: &str, window: &mut Window, cx: &mut App);
  /// See [`Space::can_go_back`].
  fn can_go_back(&self, cx: &App) -> bool;
  /// See [`Space::back`].
  fn back(&self, cx: &mut App);
  /// See [`Space::searchable`].
  fn searchable(&self, cx: &App) -> bool;
  /// See [`Space::open_search`].
  fn open_search(&self, window: &mut Window, cx: &mut App);
  /// See [`Space::player`].
  fn player(&self, cx: &App) -> Option<Entity<PlayerView>>;
  /// See [`Space::unseen`].
  fn unseen(&self, cx: &App) -> Option<usize>;
  /// See [`Space::notifications`].
  fn notifications(
    &self,
    popover: WeakEntity<PopoverState>,
    window: &mut Window,
    cx: &mut App,
  ) -> Option<AnyElement>;
  /// See [`Space::settings`].
  fn settings(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement>;
  /// Calls `on_change` whenever the space changes (a page opened or replaced).
  fn observe(&self, cx: &mut App, on_change: Box<dyn Fn(&mut App)>) -> Subscription;
  /// Calls `on_event` with what the space asks of the shell.
  fn on_event(&self, cx: &mut App, on_event: Box<EventHandler>) -> Subscription;
}

impl<T: Space> AnySpace for Entity<T> {
  fn view(&self) -> AnyView {
    self.clone().into()
  }

  fn info(&self, cx: &App) -> SpaceInfo {
    self.read(cx).info()
  }

  fn menu(&self, cx: &App) -> Vec<MenuEntry> {
    self.read(cx).menu(cx)
  }

  fn open_entry(&self, id: &str, window: &mut Window, cx: &mut App) {
    self.update(cx, |space, cx| space.open_entry(id, window, cx));
  }

  fn can_go_back(&self, cx: &App) -> bool {
    self.read(cx).can_go_back()
  }

  fn back(&self, cx: &mut App) {
    self.update(cx, |space, cx| space.back(cx));
  }

  fn searchable(&self, cx: &App) -> bool {
    self.read(cx).searchable()
  }

  fn open_search(&self, window: &mut Window, cx: &mut App) {
    self.update(cx, |space, cx| space.open_search(window, cx));
  }

  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    self.read(cx).player(cx)
  }

  fn unseen(&self, cx: &App) -> Option<usize> {
    self.read(cx).unseen(cx)
  }

  fn notifications(
    &self,
    popover: WeakEntity<PopoverState>,
    window: &mut Window,
    cx: &mut App,
  ) -> Option<AnyElement> {
    self.update(cx, |space, cx| space.notifications(popover, window, cx))
  }

  fn settings(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    self.update(cx, |space, cx| space.settings(window, cx))
  }

  fn observe(&self, cx: &mut App, on_change: Box<dyn Fn(&mut App)>) -> Subscription {
    cx.observe(self, move |_, cx| on_change(cx))
  }

  fn on_event(&self, cx: &mut App, on_event: Box<EventHandler>) -> Subscription {
    cx.subscribe(self, move |_, event, cx| on_event(event, cx))
  }
}

/// A space as the shell holds it.
pub type SpaceHandle = Rc<dyn AnySpace>;
