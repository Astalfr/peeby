//! What the shell asks of a space (YouTube, Jellyfin, Twitch), and a handle that hides the
//! space's type. A space is a GPUI entity: one per service, holding its session, and a stack of
//! pages per tab ([`Tabs`]) of that service the user opened (ADR 0015). It renders the page on
//! top of the tab shown; the shell draws the top bar with every tab, the sidebar from the space's
//! entries, and the mini-player.

use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, ClickEvent, Context, Entity, EntityId, EventEmitter, SharedString,
  Subscription, WeakEntity, Window,
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

/// What an entry shows on hover, under a larger avatar and the entry's label: e.g. a live's title
/// and category.
#[derive(Clone)]
pub struct HoverCard {
  /// Wrapped on up to three lines.
  pub title: SharedString,
  /// A muted line under the title.
  pub detail: Option<SharedString>,
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
  /// A card on hover, in place of the folded sidebar's tooltip.
  pub hover: Option<HoverCard>,
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
      hover: None,
    }
  }
}

/// A tab, unique for the app's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
  fn next() -> Self {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Self(NEXT.fetch_add(1, Ordering::Relaxed))
  }
}

impl std::fmt::Display for TabId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

/// What a page holds to navigate: its space (session, errors) and the tab it is in, so that what
/// it opens (a click, the next video of a queue) lands in that tab even when another one shows.
pub struct Nav<S> {
  space: WeakEntity<S>,
  /// The page's tab.
  pub tab: TabId,
}

impl<S> Nav<S> {
  pub fn new(space: WeakEntity<S>, tab: TabId) -> Self {
    Self { space, tab }
  }
}

impl<S> Clone for Nav<S> {
  fn clone(&self) -> Self {
    Self {
      space: self.space.clone(),
      tab: self.tab,
    }
  }
}

/// The space itself, for what is not the tab's (the session, its errors, the settings).
impl<S> Deref for Nav<S> {
  type Target = WeakEntity<S>;

  fn deref(&self) -> &WeakEntity<S> {
    &self.space
  }
}

/// An `on_aux_click` listener that runs `open` on a middle click: a link opened in a new tab.
pub fn middle_click(
  open: impl Fn(&mut App) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
  move |event, _, cx| {
    if event.is_middle_click() {
      cx.stop_propagation();
      open(cx);
    }
  }
}

/// A page of a tab: its route (the space's) and its view.
pub struct Page<R> {
  pub route: R,
  pub view: AnyView,
  /// The tab's label while this page is on top (a video's title, once known).
  pub title: Option<SharedString>,
}

impl<R> Page<R> {
  pub fn new(route: R, view: impl Into<AnyView>) -> Self {
    Self {
      route,
      view: view.into(),
      title: None,
    }
  }
}

/// A tab of a space: its stack of pages, bottom first.
pub struct Tab<R> {
  pub id: TabId,
  pub stack: Vec<Page<R>>,
  /// Pages to open, in order, the next time the tab shows: a tab opened in the background builds
  /// nothing (a watch page would start playing, and report it to the account), and a session
  /// change rebuilds every tab this way. Not empty only while `stack` is.
  pub pending: Vec<R>,
  /// Label of the last pending page until it has a title of its own (the link's text).
  label: Option<SharedString>,
}

impl<R> Tab<R> {
  /// The route at the bottom of the stack (the sidebar entry lit), built or pending.
  pub fn root(&self) -> Option<&R> {
    self
      .stack
      .first()
      .map(|p| &p.route)
      .or(self.pending.first())
  }

  /// The route on top, built or pending.
  pub fn top(&self) -> Option<&R> {
    self.stack.last().map(|p| &p.route).or(self.pending.last())
  }

  /// Drops the pages and opens `pending` the next time the tab shows.
  pub fn reset(&mut self, pending: Vec<R>) {
    self.stack.clear();
    self.pending = pending;
    self.label = None;
  }
}

/// The tabs of a space, and the one it shows (the shell's, or the last one of this space that
/// showed).
pub struct Tabs<R> {
  list: Vec<Tab<R>>,
  shown: Option<TabId>,
}

impl<R> Default for Tabs<R> {
  fn default() -> Self {
    Self {
      list: Vec::new(),
      shown: None,
    }
  }
}

impl<R> Tabs<R> {
  /// A new tab, opening `pending` when it shows; `label` names it meanwhile.
  pub fn add(&mut self, pending: Vec<R>, label: Option<SharedString>) -> TabId {
    let id = TabId::next();
    self.list.push(Tab {
      id,
      stack: Vec::new(),
      pending,
      label,
    });
    id
  }

  /// Closes a tab: its pages (and a video playing there) go. When it was the one shown, the
  /// last tab of the space left takes its place (where a notification opens).
  pub fn remove(&mut self, id: TabId) {
    self.list.retain(|t| t.id != id);
    if self.shown == Some(id) {
      self.shown = self.list.last().map(|t| t.id);
    }
  }

  pub fn get(&self, id: TabId) -> Option<&Tab<R>> {
    self.list.iter().find(|t| t.id == id)
  }

  pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab<R>> {
    self.list.iter_mut().find(|t| t.id == id)
  }

  pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab<R>> {
    self.list.iter_mut()
  }

  /// Every tab, in the order they were opened.
  pub fn ids(&self) -> Vec<TabId> {
    self.list.iter().map(|t| t.id).collect()
  }

  pub fn show(&mut self, id: TabId) {
    if self.get(id).is_some() {
      self.shown = Some(id);
    }
  }

  /// The tab rendered: the shell's, or the last of this space it showed.
  pub fn shown(&self) -> Option<TabId> {
    self.shown
  }

  /// Takes the pages waiting to be opened in tab `id`.
  pub fn take_pending(&mut self, id: TabId) -> Vec<R> {
    self
      .get_mut(id)
      .map(|t| std::mem::take(&mut t.pending))
      .unwrap_or_default()
  }

  /// While tab `id` has not built its pages yet, `route` joins them (opened last when the tab
  /// shows) and `None` comes back; else `route` comes back, to be opened now. Opening it now
  /// would build a page that the pending ones clear (a top-level page first) or bury.
  pub fn queue_if_pending(&mut self, id: TabId, route: R) -> Option<R> {
    match self.get_mut(id) {
      Some(tab) if !tab.pending.is_empty() => {
        tab.label = None;
        tab.pending.push(route);
        None
      }
      _ => Some(route),
    }
  }

  /// After the pending pages are built: the tab's label goes to the one on top.
  pub fn label_top(&mut self, id: TabId) {
    if let Some(tab) = self.get_mut(id)
      && let Some(label) = tab.label.take()
      && let Some(page) = tab.stack.last_mut()
    {
      page.title.get_or_insert(label);
    }
  }

  /// Names the page of `view` (a video's title, once loaded); `true` when a page has it.
  pub fn set_title(&mut self, view: EntityId, title: SharedString) -> bool {
    let page = self
      .list
      .iter_mut()
      .flat_map(|t| t.stack.iter_mut())
      .find(|p| p.view.entity_id() == view);
    match page {
      Some(page) => {
        page.title = Some(title);
        true
      }
      None => false,
    }
  }

  /// Label of tab `id`: its top page's title, else `name` of its top route.
  pub fn title(&self, id: TabId, name: impl Fn(&R) -> SharedString) -> Option<SharedString> {
    let tab = self.get(id)?;
    match tab.stack.last() {
      Some(page) => Some(page.title.clone().unwrap_or_else(|| name(&page.route))),
      None => tab.label.clone().or_else(|| tab.pending.last().map(name)),
    }
  }
}

/// What a space asks of the shell.
pub enum SpaceEvent {
  /// Show this tab (a notification of the space was clicked from another tab).
  Activate(TabId),
  /// The space opened a tab (a link's middle click, a notification with no tab of the space
  /// open): the shell puts it after `after` (at the end when `None`), and shows it if `focus`.
  Opened {
    tab: TabId,
    after: Option<TabId>,
    focus: bool,
  },
  /// Open the settings (a page's "import cookies" button, the account entry).
  OpenSettings,
}

/// A service's part of the app: its tabs, their pages, the sidebar entries and what they play.
pub trait Space: Render + EventEmitter<SpaceEvent> {
  /// Name, logo and accent.
  fn info(&self) -> SpaceInfo;

  /// The tabs the space opened by itself at startup.
  fn tabs(&self) -> Vec<TabId>;

  /// Opens a tab on the space's first page (built when it shows).
  fn new_tab(&mut self, cx: &mut Context<Self>) -> TabId;

  /// Closes a tab: its pages go.
  fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>);

  /// Renders `tab` from now on.
  fn show_tab(&mut self, tab: TabId, cx: &mut Context<Self>);

  /// The tab's label: the page on top.
  fn title(&self, tab: TabId, cx: &App) -> SharedString;

  /// Sidebar entries, top to bottom, those of `tab` lit.
  fn menu(&self, tab: TabId, cx: &App) -> Vec<MenuEntry>;

  /// Opens the page of a sidebar entry in `tab`.
  fn open_entry(&mut self, tab: TabId, id: &str, window: &mut Window, cx: &mut Context<Self>);

  /// Opens the page of a sidebar entry in a new tab, after `tab` (a middle click).
  fn open_entry_in_new_tab(&mut self, tab: TabId, id: &str, cx: &mut Context<Self>);

  /// Whether "back" leads somewhere within the tab.
  fn can_go_back(&self, tab: TabId) -> bool;

  /// Goes back one page in `tab`.
  fn back(&mut self, tab: TabId, cx: &mut Context<Self>);

  /// Whether the top bar's search button searches this space (signed in, with a search page).
  fn searchable(&self) -> bool {
    false
  }

  /// Opens the space's search page in `tab`.
  fn open_search(&mut self, _tab: TabId, _window: &mut Window, _cx: &mut Context<Self>) {}

  /// The player of the page on top of `tab`, if it has one. The shell moves it to the
  /// mini-player when another tab opens while it plays.
  fn player(&self, _tab: TabId, _cx: &App) -> Option<Entity<PlayerView>> {
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
  /// The space's entity as a view (the page on top of the tab it shows).
  fn view(&self) -> AnyView;
  /// See [`Space::info`].
  fn info(&self, cx: &App) -> SpaceInfo;
  /// See [`Space::tabs`].
  fn tabs(&self, cx: &App) -> Vec<TabId>;
  /// See [`Space::new_tab`].
  fn new_tab(&self, cx: &mut App) -> TabId;
  /// See [`Space::close_tab`].
  fn close_tab(&self, tab: TabId, cx: &mut App);
  /// See [`Space::show_tab`].
  fn show_tab(&self, tab: TabId, cx: &mut App);
  /// See [`Space::title`].
  fn title(&self, tab: TabId, cx: &App) -> SharedString;
  /// See [`Space::menu`].
  fn menu(&self, tab: TabId, cx: &App) -> Vec<MenuEntry>;
  /// See [`Space::open_entry`].
  fn open_entry(&self, tab: TabId, id: &str, window: &mut Window, cx: &mut App);
  /// See [`Space::open_entry_in_new_tab`].
  fn open_entry_in_new_tab(&self, tab: TabId, id: &str, cx: &mut App);
  /// See [`Space::can_go_back`].
  fn can_go_back(&self, tab: TabId, cx: &App) -> bool;
  /// See [`Space::back`].
  fn back(&self, tab: TabId, cx: &mut App);
  /// See [`Space::searchable`].
  fn searchable(&self, cx: &App) -> bool;
  /// See [`Space::open_search`].
  fn open_search(&self, tab: TabId, window: &mut Window, cx: &mut App);
  /// See [`Space::player`].
  fn player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>>;
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

  fn tabs(&self, cx: &App) -> Vec<TabId> {
    self.read(cx).tabs()
  }

  fn new_tab(&self, cx: &mut App) -> TabId {
    self.update(cx, |space, cx| space.new_tab(cx))
  }

  fn close_tab(&self, tab: TabId, cx: &mut App) {
    self.update(cx, |space, cx| space.close_tab(tab, cx));
  }

  fn show_tab(&self, tab: TabId, cx: &mut App) {
    self.update(cx, |space, cx| space.show_tab(tab, cx));
  }

  fn title(&self, tab: TabId, cx: &App) -> SharedString {
    self.read(cx).title(tab, cx)
  }

  fn menu(&self, tab: TabId, cx: &App) -> Vec<MenuEntry> {
    self.read(cx).menu(tab, cx)
  }

  fn open_entry(&self, tab: TabId, id: &str, window: &mut Window, cx: &mut App) {
    self.update(cx, |space, cx| space.open_entry(tab, id, window, cx));
  }

  fn open_entry_in_new_tab(&self, tab: TabId, id: &str, cx: &mut App) {
    self.update(cx, |space, cx| space.open_entry_in_new_tab(tab, id, cx));
  }

  fn can_go_back(&self, tab: TabId, cx: &App) -> bool {
    self.read(cx).can_go_back(tab)
  }

  fn back(&self, tab: TabId, cx: &mut App) {
    self.update(cx, |space, cx| space.back(tab, cx));
  }

  fn searchable(&self, cx: &App) -> bool {
    self.read(cx).searchable()
  }

  fn open_search(&self, tab: TabId, window: &mut Window, cx: &mut App) {
    self.update(cx, |space, cx| space.open_search(tab, window, cx));
  }

  fn player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>> {
    self.read(cx).player(tab, cx)
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

#[cfg(test)]
mod tests {
  use super::Tabs;

  #[test]
  fn background_tabs_wait_and_keep_their_label() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let first = tabs.add(vec!["home"], None);
    let second = tabs.add(vec!["watch"], Some("A video".into()));
    let name = |route: &&str| route.to_string().into();
    assert_eq!(tabs.title(first, name).as_deref(), Some("home"));
    assert_eq!(tabs.title(second, name).as_deref(), Some("A video"));
    assert_eq!(tabs.get(second).and_then(|t| t.root()), Some(&"watch"));
    assert_eq!(tabs.take_pending(second), vec!["watch"]);
    assert!(tabs.take_pending(second).is_empty());
  }

  #[test]
  fn a_route_for_a_tab_not_built_yet_waits_after_its_pages() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let waiting = tabs.add(vec!["home"], None);
    let built = tabs.add(Vec::new(), None);
    assert_eq!(tabs.queue_if_pending(waiting, "watch"), None);
    assert_eq!(tabs.queue_if_pending(built, "watch"), Some("watch"));
    assert_eq!(tabs.take_pending(waiting), vec!["home", "watch"]);
    assert_eq!(tabs.queue_if_pending(waiting, "channel"), Some("channel"));
  }

  #[test]
  fn closing_the_shown_tab_falls_back_to_another() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let first = tabs.add(Vec::new(), None);
    let second = tabs.add(Vec::new(), None);
    tabs.show(second);
    tabs.remove(second);
    assert_eq!(tabs.shown(), Some(first));
    assert_eq!(tabs.ids(), vec![first]);
    tabs.remove(first);
    assert_eq!(tabs.shown(), None);
  }
}
