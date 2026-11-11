//! What the shell asks of a space (YouTube, Jellyfin, Twitch), and a handle that hides the
//! space's type. A space is a GPUI entity: one per service, holding its session, and a history
//! of pages per tab ([`Tabs`]) of that service the user opened (ADR 0015, ADR 0019). It renders
//! the page shown in the tab shown; the shell draws the top bar with every tab, the sidebar from
//! the space's entries, the settings page when a tab's history is on it, and the mini-player.

use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, ClickEvent, Context, Entity, EntityId, EventEmitter, SharedString,
  Subscription, WeakEntity, Window, WindowId,
};
use gpui_component::popover::PopoverState;
use ui::assets::Icon;
use ui::i18n::t;
use ui::picture::Picture;
use ui::player::PlayerView;
use ui::theme::Accent;

/// How a space shows in the tabs.
#[derive(Clone)]
pub struct SpaceInfo {
  /// Stable id, e.g. `youtube`, as the settings name the space (`startup_tabs`).
  pub id: &'static str,
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

impl Avatar {
  /// `picture` over the initial of `name`.
  pub fn of(name: &str, picture: Option<Picture>) -> Self {
    let initial = name
      .chars()
      .next()
      .unwrap_or('?')
      .to_uppercase()
      .to_string();
    Self {
      picture,
      initial: initial.into(),
    }
  }
}

/// What an entry shows on hover, under a larger avatar and the entry's label: e.g. a live's title
/// and category.
#[derive(Clone)]
pub struct HoverCard {
  /// Wrapped on up to three lines.
  pub title: SharedString,
  /// A muted line under the title.
  pub detail: Option<SharedString>,
  /// The audience under it, after a red dot, e.g. `2.4K viewers`.
  pub viewers: Option<SharedString>,
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
  /// On air: a "LIVE" pill at the right of the entry (a red dot and [`Self::viewers`] when
  /// given), a dot on its icon when folded.
  pub live: bool,
  /// The audience of an entry on air, short (`2.4K`): shown with a red dot instead of "LIVE".
  pub viewers: Option<SharedString>,
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
      viewers: None,
      hover: None,
    }
  }
}

/// A followed channel on air, in the list of lives the notification area's icon opens.
#[derive(Clone)]
pub struct LiveEntry {
  /// Passed back to [`Space::open_live`].
  pub id: SharedString,
  /// The channel's name.
  pub channel: SharedString,
  /// The broadcast's title, on one line.
  pub title: SharedString,
  /// A muted line, e.g. the category.
  pub detail: Option<SharedString>,
  /// The audience as the row shows it, e.g. `1,2 k spectateurs`.
  pub viewers: Option<SharedString>,
  /// When it started, for its uptime.
  pub since: Option<SystemTime>,
  /// The live's preview (16:9).
  pub preview: Option<Picture>,
  /// The channel's avatar.
  pub avatar: Avatar,
}

/// A space's lives, for the notification area's list.
#[derive(Clone)]
pub enum Lives {
  /// Read for the first time.
  Loading,
  /// Unavailable until the user does something in Peeby (sign in): what to do.
  Unavailable(SharedString),
  /// On air now, in the order to show them.
  On(Vec<LiveEntry>),
}

/// A tab, unique for the app's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(u64);

impl TabId {
  /// A given id, for tests.
  #[cfg(test)]
  pub fn for_test(id: u64) -> Self {
    Self(id)
  }

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
  /// The navigation of a page of `space` in `tab`.
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

/// The most entries a tab's history keeps; the oldest go first.
const MAX_ENTRIES: usize = 50;
/// How many pages behind the one shown keep their view (their scroll, their search, a paused
/// player); older ones keep their route and title, and are built again when they show.
const KEPT_BEHIND: usize = 10;
const _: () = assert!(KEPT_BEHIND < MAX_ENTRIES);

/// A page of a tab's history: its route (the space's), its view while built, its title.
pub struct Page<R> {
  /// What the page shows, in the space's terms.
  pub route: R,
  /// `None` until the page shows (a tab opened in the background, a page left with "back" or
  /// too far behind): the space builds it from `route` at its next render.
  pub view: Option<AnyView>,
  /// The tab's label while this page shows (a video's title, once known).
  pub title: Option<SharedString>,
}

impl<R> Page<R> {
  /// A page of `route`, built (its view shows now).
  pub fn new(route: R, view: impl Into<AnyView>) -> Self {
    Self {
      route,
      view: Some(view.into()),
      title: None,
    }
  }

  /// A page built when it shows.
  pub fn unbuilt(route: R) -> Self {
    Self {
      route,
      view: None,
      title: None,
    }
  }
}

/// An entry of a tab's history: one of the space's pages, or the settings page (the shell's,
/// shown over the page before it).
enum Entry<R> {
  Page(Page<R>),
  Settings,
}

impl<R> Entry<R> {
  fn page(&self) -> Option<&Page<R>> {
    match self {
      Self::Page(page) => Some(page),
      Self::Settings => None,
    }
  }

  fn page_mut(&mut self) -> Option<&mut Page<R>> {
    match self {
      Self::Page(page) => Some(page),
      Self::Settings => None,
    }
  }
}

/// A tab of a space: its history, as in a browser. Opening a page drops the entries after the
/// one shown and adds it; "back" and "forward" move along the entries.
pub struct Tab<R> {
  /// The tab, for the shell and the space's events.
  pub id: TabId,
  entries: Vec<Entry<R>>,
  /// The entry shown; 0 while `entries` is empty.
  current: usize,
}

impl<R> Tab<R> {
  fn current_entry(&self) -> Option<&Entry<R>> {
    self.entries.get(self.current)
  }

  /// The page shown; `None` while the settings show.
  pub fn current(&self) -> Option<&Page<R>> {
    self.current_entry()?.page()
  }

  fn current_mut(&mut self) -> Option<&mut Page<R>> {
    self.entries.get_mut(self.current)?.page_mut()
  }

  /// The page shown, or the one the settings show over: whose player the mini-player shows,
  /// and which a new page covers.
  pub fn page(&self) -> Option<&Page<R>> {
    self.behind().next()
  }

  /// The pages from the one shown (or the one the settings show over) back to the first.
  pub fn behind(&self) -> impl Iterator<Item = &Page<R>> {
    let end = (self.current + 1).min(self.entries.len());
    self.entries[..end].iter().rev().filter_map(Entry::page)
  }

  /// Every page of the history, e.g. to make some of them again.
  pub fn pages_mut(&mut self) -> impl Iterator<Item = &mut Page<R>> {
    self.entries.iter_mut().filter_map(Entry::page_mut)
  }

  /// Whether the entry shown is the settings page.
  pub fn settings_shown(&self) -> bool {
    matches!(self.current_entry(), Some(Entry::Settings))
  }

  /// Where the entry shown is in the history (a page's animation key).
  pub fn position(&self) -> usize {
    self.current
  }

  /// The route of the section the page shown belongs to (the sidebar entry lit): the last
  /// `top_level` route up to it, else the first page's. `None` while the settings show.
  pub fn section(&self, top_level: impl Fn(&R) -> bool) -> Option<&R> {
    self.current()?;
    let mut first = None;
    for page in self.behind() {
      if top_level(&page.route) {
        return Some(&page.route);
      }
      first = Some(&page.route);
    }
    first
  }

  /// The route shown while its page is not built: the space builds it now.
  pub fn unbuilt(&self) -> Option<&R> {
    self
      .current()
      .filter(|page| page.view.is_none())
      .map(|page| &page.route)
  }

  /// Gives the page shown (or under the settings) its view, once built.
  pub fn set_view(&mut self, view: AnyView) {
    let end = (self.current + 1).min(self.entries.len());
    if let Some(page) = self.entries[..end]
      .iter_mut()
      .rev()
      .find_map(Entry::page_mut)
    {
      page.view = Some(view);
    }
  }

  /// Opens `page` after the entry shown, dropping the entries that followed it.
  pub fn push(&mut self, page: Page<R>) {
    self.push_entry(Entry::Page(page));
  }

  fn push_entry(&mut self, entry: Entry<R>) {
    if !self.entries.is_empty() {
      self.entries.truncate(self.current + 1);
    }
    self.entries.push(entry);
    if self.entries.len() > MAX_ENTRIES {
      self.entries.remove(0);
    }
    self.current = self.entries.len() - 1;
    self.trim();
  }

  /// Whether the page shown (or under the settings) is built: the tab has shown, so a page
  /// opened in it now is built at once (the next episode, while the mini-player shows it).
  pub fn live(&self) -> bool {
    self.page().is_some_and(|page| page.view.is_some())
  }

  /// Puts `page` in place of the page shown, or of the one under the settings, which stay on
  /// top (the next video of a queue, the next episode, in the mini-player meanwhile). Opens it
  /// in an empty history.
  pub fn replace(&mut self, page: Page<R>) {
    let end = (self.current + 1).min(self.entries.len());
    match self.entries[..end]
      .iter_mut()
      .rev()
      .find(|entry| matches!(entry, Entry::Page(_)))
    {
      Some(entry) => *entry = Entry::Page(page),
      None => self.push(page),
    }
  }

  /// An entry before the one shown ("back" is enabled).
  pub fn can_go_back(&self) -> bool {
    self.current > 0
  }

  /// An entry after the one shown ("forward" is enabled).
  pub fn can_go_forward(&self) -> bool {
    self.current + 1 < self.entries.len()
  }

  /// Whether "forward" leads to the settings page.
  pub fn forward_to_settings(&self) -> bool {
    matches!(self.entries.get(self.current + 1), Some(Entry::Settings))
  }

  /// Goes back one entry: the page left goes (its player with it, as it did when pages were
  /// popped), its route stays for "forward". `false` at the first entry.
  pub fn back(&mut self) -> bool {
    if !self.can_go_back() {
      return false;
    }
    if let Some(page) = self.current_mut() {
      page.view = None;
    }
    self.current -= 1;
    true
  }

  /// Drops the view of the page shown, built again at the next render (the refresh button):
  /// its data is read again, a player on it starts over. `false` when nothing was built.
  pub fn reload(&mut self) -> bool {
    match self.current_mut() {
      Some(page) if page.view.is_some() => {
        page.view = None;
        true
      }
      _ => false,
    }
  }

  /// Goes forward one entry, built when it shows. `false` at the last entry.
  pub fn forward(&mut self) -> bool {
    if !self.can_go_forward() {
      return false;
    }
    self.current += 1;
    self.trim();
    true
  }

  /// Drops the views of the pages behind the one shown whose route `matches`: pages with a
  /// player, when a sidebar page opens, so that a long history keeps no paused players.
  pub fn unload_behind(&mut self, matches: impl Fn(&R) -> bool) {
    let end = self.current.min(self.entries.len());
    for page in self.entries[..end].iter_mut().filter_map(Entry::page_mut) {
      if matches(&page.route) {
        page.view = None;
      }
    }
  }

  /// The pages more than [`KEPT_BEHIND`] entries behind the one shown lose their view.
  fn trim(&mut self) {
    let end = self.current.saturating_sub(KEPT_BEHIND);
    for page in self.entries[..end].iter_mut().filter_map(Entry::page_mut) {
      page.view = None;
    }
  }

  /// Starts the history over on `routes`, the last one shown, each built when it shows. A tab
  /// on the settings stays there (a sign-in from the settings): `routes` go before them, except
  /// in the settings' own tab, which keeps them alone.
  pub fn reset(&mut self, routes: Vec<R>) {
    let settings = self.settings_shown();
    let settings_tab = settings && self.current == 0;
    self.entries = if settings_tab {
      Vec::new()
    } else {
      routes
        .into_iter()
        .map(|route| Entry::Page(Page::unbuilt(route)))
        .collect()
    };
    if settings {
      self.entries.push(Entry::Settings);
    }
    if self.entries.len() > MAX_ENTRIES {
      self.entries.drain(..self.entries.len() - MAX_ENTRIES);
    }
    self.current = self.entries.len().saturating_sub(1);
  }
}

/// The tabs of a space, the one it renders in each window (ADR 0028), and the last one shown
/// in any window (where a notification opens).
pub struct Tabs<R> {
  list: Vec<Tab<R>>,
  shown: Option<TabId>,
  windows: Vec<(WindowId, TabId)>,
}

impl<R> Default for Tabs<R> {
  fn default() -> Self {
    Self {
      list: Vec::new(),
      shown: None,
      windows: Vec::new(),
    }
  }
}

impl<R> Tabs<R> {
  /// A new tab whose history is `routes`, the last one shown. Nothing is built before the tab
  /// shows (a watch page would start playing, and report it to the account); `label` names it
  /// meanwhile (the link's text).
  pub fn add(&mut self, routes: Vec<R>, label: Option<SharedString>) -> TabId {
    let id = TabId::next();
    let mut tab = Tab {
      id,
      entries: Vec::new(),
      current: 0,
    };
    tab.reset(routes);
    if let Some(page) = tab.current_mut() {
      page.title = label;
    }
    self.list.push(tab);
    id
  }

  /// A new tab on the settings page alone (the gear always opens them in a tab of their own).
  pub fn add_settings(&mut self) -> TabId {
    let id = TabId::next();
    self.list.push(Tab {
      id,
      entries: vec![Entry::Settings],
      current: 0,
    });
    id
  }

  /// Closes a tab: its pages (and a video playing there) go. When it was the last one shown,
  /// the last tab of the space left takes its place (where a notification opens); a window that
  /// rendered it renders nothing of the space until its shell shows another tab.
  pub fn remove(&mut self, id: TabId) {
    self.list.retain(|t| t.id != id);
    self.windows.retain(|&(_, tab)| tab != id);
    if self.shown == Some(id) {
      self.shown = self.list.last().map(|t| t.id);
    }
  }

  /// The tab `id`, if it is the space's.
  pub fn get(&self, id: TabId) -> Option<&Tab<R>> {
    self.list.iter().find(|t| t.id == id)
  }

  /// The tab `id`, if it is the space's, to change it.
  pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab<R>> {
    self.list.iter_mut().find(|t| t.id == id)
  }

  /// Every tab, in the bar's order.
  pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab<R>> {
    self.list.iter_mut()
  }

  /// Every tab, in the order they were opened.
  pub fn ids(&self) -> Vec<TabId> {
    self.list.iter().map(|t| t.id).collect()
  }

  /// Renders tab `id` in `window` from now on (a tab is in one window at a time: shown in
  /// another, it leaves the one it was in).
  pub fn show(&mut self, id: TabId, window: WindowId) {
    if self.get(id).is_none() {
      return;
    }
    self.shown = Some(id);
    self.windows.retain(|&(w, tab)| w != window && tab != id);
    self.windows.push((window, id));
  }

  /// Makes `id` the tab last shown before any window shows it (a space's first tab, at
  /// startup).
  pub fn remember(&mut self, id: TabId) {
    if self.get(id).is_some() {
      self.shown = Some(id);
    }
  }

  /// The tab of the space last shown, in any window: where a notification opens.
  pub fn shown(&self) -> Option<TabId> {
    self.shown
  }

  /// The tab the space renders in `window`, if that window shows one of the space's.
  pub fn shown_in(&self, window: WindowId) -> Option<TabId> {
    self
      .windows
      .iter()
      .find(|&&(w, _)| w == window)
      .map(|&(_, tab)| tab)
  }

  /// Names the page of `view` (a video's title, once loaded); `true` when a page has it.
  pub fn set_title(&mut self, view: EntityId, title: SharedString) -> bool {
    let page = self
      .list
      .iter_mut()
      .flat_map(|t| t.pages_mut())
      .find(|p| p.view.as_ref().is_some_and(|v| v.entity_id() == view));
    match page {
      Some(page) => {
        page.title = Some(title);
        true
      }
      None => false,
    }
  }

  /// Label of tab `id`: its page's title, else `name` of its route; "Settings" on the settings.
  pub fn title(&self, id: TabId, name: impl Fn(&R) -> SharedString) -> Option<SharedString> {
    match self.get(id)?.current_entry()? {
      Entry::Page(page) => Some(page.title.clone().unwrap_or_else(|| name(&page.route))),
      Entry::Settings => Some(t("Settings", "Réglages").into()),
    }
  }
}

/// What a space asks of the shell.
pub enum SpaceEvent {
  /// Show this tab (a notification of the space was clicked from another tab).
  Activate(TabId),
  /// The space opened a tab (a link's middle click, a notification with no tab of the space
  /// open): the shell puts it after `after` (after the tab shown when `None`), and shows it if
  /// `focus`.
  Opened {
    /// The new tab.
    tab: TabId,
    /// The tab it goes after.
    after: Option<TabId>,
    /// Shown once placed.
    focus: bool,
  },
  /// Open the settings in a tab of their own: the account entry, a page's "Sign in" button.
  /// With `reveal`, the space's part is unfolded and scrolled to (its account comes first),
  /// where the space may flash its sign-in ([`ui::widgets::flash`]).
  OpenSettings {
    /// Unfold the space's part and scroll to it.
    reveal: bool,
  },
}

/// A service's part of the app: its tabs, their pages, the sidebar entries and what they play.
pub trait Space: Render + EventEmitter<SpaceEvent> + Sized {
  /// A page of the space.
  type Route;

  /// Name, logo and accent.
  fn info(&self) -> SpaceInfo;

  /// The tabs and their histories.
  fn history(&self) -> &Tabs<Self::Route>;

  /// The tabs and their histories, to change them.
  fn history_mut(&mut self) -> &mut Tabs<Self::Route>;

  /// Pauses the player of the page `tab` shows (or shows the settings over) while another page
  /// covers it (`covered`), or lets it go on when it shows again.
  fn cover(&self, _tab: TabId, _covered: bool, _cx: &mut Context<Self>) {}

  /// The tabs the space opened by itself at startup.
  fn tabs(&self) -> Vec<TabId>;

  /// `route` as a key kept until the next start (ADR 0035, [`route_key`]); `None` for a page
  /// not kept (a debug page).
  fn route_key(&self, _route: &Self::Route) -> Option<String> {
    None
  }

  /// The route a key of [`Space::route_key`] stands for, if it still does.
  fn route_from_key(&self, _key: &str) -> Option<Self::Route> {
    None
  }

  /// Whether `route`'s page plays (a watch page): a tab restored on it is not the one shown at
  /// startup, so that nothing plays before a click.
  fn route_plays(&self, _route: &Self::Route) -> bool {
    false
  }

  /// Opens a tab on the space's first page (built when it shows).
  fn new_tab(&mut self, cx: &mut Context<Self>) -> TabId;

  /// Closes a tab: its pages go.
  fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>);

  /// Renders `tab` in `window` from now on.
  fn show_tab(&mut self, tab: TabId, window: WindowId, cx: &mut Context<Self>) {
    self.history_mut().show(tab, window);
    cx.notify();
  }

  /// The tab's label: the page on top.
  fn title(&self, tab: TabId, cx: &App) -> SharedString;

  /// Sidebar entries, top to bottom, those of `tab` lit.
  fn menu(&self, tab: TabId, cx: &App) -> Vec<MenuEntry>;

  /// Opens the page of a sidebar entry in `tab`.
  fn open_entry(&mut self, tab: TabId, id: &str, window: &mut Window, cx: &mut Context<Self>);

  /// Opens the page of a sidebar entry in a new tab, after `tab` (a middle click).
  fn open_entry_in_new_tab(&mut self, tab: TabId, id: &str, cx: &mut Context<Self>);

  /// Whether "back" leads somewhere within the tab.
  fn can_go_back(&self, tab: TabId) -> bool {
    self.history().get(tab).is_some_and(Tab::can_go_back)
  }

  /// Whether "forward" leads somewhere within the tab.
  fn can_go_forward(&self, tab: TabId) -> bool {
    self.history().get(tab).is_some_and(Tab::can_go_forward)
  }

  /// Goes back one entry in `tab`: the page left goes, the one before plays on.
  fn back(&mut self, tab: TabId, cx: &mut Context<Self>) {
    if self.history_mut().get_mut(tab).is_some_and(Tab::back) {
      self.cover(tab, false, cx);
      cx.notify();
    }
  }

  /// Goes forward one entry in `tab`. The page left is covered, unless the settings come next
  /// (the shell moves a playing video to the mini-player instead).
  fn forward(&mut self, tab: TabId, cx: &mut Context<Self>) {
    let Some(t) = self.history().get(tab) else {
      return;
    };
    if !t.can_go_forward() {
      return;
    }
    if !t.forward_to_settings() {
      self.cover(tab, true, cx);
    }
    if let Some(t) = self.history_mut().get_mut(tab) {
      t.forward();
    }
    cx.notify();
  }

  /// Builds the page `tab` shows again (the refresh button).
  fn reload(&mut self, tab: TabId, cx: &mut Context<Self>) {
    if self.history_mut().get_mut(tab).is_some_and(Tab::reload) {
      cx.notify();
    }
  }

  /// Opens a tab of the space on the settings page alone (the gear, a page's request).
  fn settings_tab(&mut self, cx: &mut Context<Self>) -> TabId {
    let id = self.history_mut().add_settings();
    cx.notify();
    id
  }

  /// Whether `tab` shows the settings page.
  fn settings_shown(&self, tab: TabId) -> bool {
    self.history().get(tab).is_some_and(Tab::settings_shown)
  }

  /// The space's search page, when the top bar's search button searches this space (signed
  /// in, with a search page).
  fn search_route(&self) -> Option<Self::Route> {
    None
  }

  /// Opens a tab of the space on its search page (built when it shows); `None` when the space
  /// has none.
  fn search_tab(&mut self, cx: &mut Context<Self>) -> Option<TabId> {
    let route = self.search_route()?;
    let id = self.history_mut().add(vec![route], None);
    cx.notify();
    Some(id)
  }

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

  /// The followed channels on air, for the notification area's list; `None` when the service
  /// has no lives.
  fn lives(&self, _cx: &App) -> Option<Lives> {
    None
  }

  /// The list of lives opens: the space may read them again (it notifies when they change).
  fn refresh_lives(&mut self, _cx: &mut Context<Self>) {}

  /// Opens the live `id` (a [`LiveEntry::id`]) in the tab of the space last shown, in the main
  /// window.
  fn open_live(&mut self, _id: &str, _window: &mut Window, _cx: &mut Context<Self>) {}
}

/// What the shell does with a space's [`SpaceEvent`]s.
pub type EventHandler = dyn Fn(&SpaceEvent, &mut App);

/// The separator of a route key's parts: a control character no id or title holds.
const KEY_SEPARATOR: char = '\u{1f}';

/// A route's key ([`Space::route_key`]) from its parts: its kind, then its fields.
pub fn route_key(parts: &[&str]) -> String {
  parts.join(&KEY_SEPARATOR.to_string())
}

/// The parts of a route key ([`route_key`]).
pub fn route_parts(key: &str) -> Vec<&str> {
  key.split(KEY_SEPARATOR).collect()
}

/// A tab kept for the next start (ADR 0035): its space, its pages up to the one shown (their
/// keys, oldest first), the title shown, and whether that page plays.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedTab {
  /// [`SpaceInfo::id`].
  pub space: String,
  /// [`Space::route_key`]s, the last one shown.
  pub routes: Vec<String>,
  /// The tab's label (a video's title).
  #[serde(default)]
  pub title: Option<String>,
  /// The page shown plays ([`Space::route_plays`]).
  #[serde(default)]
  pub plays: bool,
}

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
  fn show_tab(&self, tab: TabId, window: WindowId, cx: &mut App);
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
  /// See [`Space::can_go_forward`].
  fn can_go_forward(&self, tab: TabId, cx: &App) -> bool;
  /// See [`Space::forward`].
  fn forward(&self, tab: TabId, cx: &mut App);
  /// See [`Space::settings_tab`].
  fn settings_tab(&self, cx: &mut App) -> TabId;
  /// See [`Space::reload`].
  fn reload(&self, tab: TabId, cx: &mut App);
  /// See [`Space::settings_shown`].
  fn settings_shown(&self, tab: TabId, cx: &App) -> bool;
  /// Whether the space has a [`Space::search_route`].
  fn searchable(&self, cx: &App) -> bool;
  /// See [`Space::search_tab`].
  fn search_tab(&self, cx: &mut App) -> Option<TabId>;
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
  /// See [`Space::lives`].
  fn lives(&self, cx: &App) -> Option<Lives>;
  /// See [`Space::refresh_lives`].
  fn refresh_lives(&self, cx: &mut App);
  /// See [`Space::open_live`].
  fn open_live(&self, id: &str, window: &mut Window, cx: &mut App);
  /// Calls `on_change` whenever the space changes (a page opened or replaced).
  fn observe(&self, cx: &mut App, on_change: Box<dyn Fn(&mut App)>) -> Subscription;
  /// Calls `on_event` with what the space asks of the shell.
  fn on_event(&self, cx: &mut App, on_event: Box<EventHandler>) -> Subscription;
  /// Tab `tab` as kept for the next start; `None` for one with nothing to keep (the settings'
  /// own tab, pages without keys).
  fn save_tab(&self, tab: TabId, cx: &App) -> Option<SavedTab>;
  /// A tab of the space made again from `saved` (built when it shows); `None` when none of its
  /// pages is known any more.
  fn restore_tab(&self, saved: &SavedTab, cx: &mut App) -> Option<TabId>;
}

impl<T: Space> AnySpace for Entity<T> {
  fn save_tab(&self, tab: TabId, cx: &App) -> Option<SavedTab> {
    let space = self.read(cx);
    let history = space.history().get(tab)?;
    let pages: Vec<&Page<T::Route>> = history.behind().collect();
    let routes: Vec<String> = pages
      .iter()
      .rev()
      .filter_map(|page| space.route_key(&page.route))
      .collect();
    if routes.is_empty() {
      return None;
    }
    let shown = pages.first()?;
    Some(SavedTab {
      space: space.info().id.to_owned(),
      routes,
      title: shown.title.as_ref().map(ToString::to_string),
      plays: space.route_plays(&shown.route),
    })
  }

  fn restore_tab(&self, saved: &SavedTab, cx: &mut App) -> Option<TabId> {
    self.update(cx, |space, cx| {
      let routes: Vec<T::Route> = saved
        .routes
        .iter()
        .filter_map(|key| space.route_from_key(key))
        .collect();
      if routes.is_empty() {
        return None;
      }
      let id = space
        .history_mut()
        .add(routes, saved.title.clone().map(Into::into));
      cx.notify();
      Some(id)
    })
  }

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

  fn show_tab(&self, tab: TabId, window: WindowId, cx: &mut App) {
    self.update(cx, |space, cx| space.show_tab(tab, window, cx));
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

  fn can_go_forward(&self, tab: TabId, cx: &App) -> bool {
    self.read(cx).can_go_forward(tab)
  }

  fn forward(&self, tab: TabId, cx: &mut App) {
    self.update(cx, |space, cx| space.forward(tab, cx));
  }

  fn settings_tab(&self, cx: &mut App) -> TabId {
    self.update(cx, |space, cx| space.settings_tab(cx))
  }

  fn reload(&self, tab: TabId, cx: &mut App) {
    self.update(cx, |space, cx| space.reload(tab, cx));
  }

  fn settings_shown(&self, tab: TabId, cx: &App) -> bool {
    self.read(cx).settings_shown(tab)
  }

  fn searchable(&self, cx: &App) -> bool {
    self.read(cx).search_route().is_some()
  }

  fn search_tab(&self, cx: &mut App) -> Option<TabId> {
    self.update(cx, |space, cx| space.search_tab(cx))
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

  fn lives(&self, cx: &App) -> Option<Lives> {
    self.read(cx).lives(cx)
  }

  fn refresh_lives(&self, cx: &mut App) {
    self.update(cx, |space, cx| space.refresh_lives(cx));
  }

  fn open_live(&self, id: &str, window: &mut Window, cx: &mut App) {
    self.update(cx, |space, cx| space.open_live(id, window, cx));
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
  use gpui::WindowId;

  use super::{MAX_ENTRIES, Page, Tab, TabId, Tabs};

  fn routes(tabs: &Tabs<&'static str>, id: TabId) -> Vec<&'static str> {
    let tab = tabs.get(id).unwrap();
    tab.behind().map(|p| p.route).collect()
  }

  fn tab<'a>(tabs: &'a mut Tabs<&'static str>, id: TabId) -> &'a mut Tab<&'static str> {
    tabs.get_mut(id).unwrap()
  }

  #[test]
  fn background_tabs_wait_and_keep_their_label() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let first = tabs.add(vec!["home"], None);
    let second = tabs.add(vec!["home", "watch"], Some("A video".into()));
    let name = |route: &&str| route.to_string().into();
    assert_eq!(tabs.title(first, name).as_deref(), Some("home"));
    assert_eq!(tabs.title(second, name).as_deref(), Some("A video"));
    // Nothing is built: the space builds the page shown when the tab shows.
    assert_eq!(tabs.get(second).and_then(|t| t.unbuilt()), Some(&"watch"));
    assert!(tabs.get(second).is_some_and(|t| t.can_go_back()));
  }

  #[test]
  fn opening_a_page_drops_the_ones_ahead() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["home"], None);
    tab(&mut tabs, id).push(Page::unbuilt("channel"));
    tab(&mut tabs, id).push(Page::unbuilt("watch"));
    assert!(tab(&mut tabs, id).back());
    assert!(tab(&mut tabs, id).back());
    assert!(!tab(&mut tabs, id).back());
    assert!(tab(&mut tabs, id).forward());
    assert_eq!(routes(&tabs, id), vec!["channel", "home"]);
    tab(&mut tabs, id).push(Page::unbuilt("search"));
    assert!(!tab(&mut tabs, id).can_go_forward());
    assert_eq!(routes(&tabs, id), vec!["search", "channel", "home"]);
  }

  #[test]
  fn a_sidebar_page_keeps_the_pages_before_it() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["home"], None);
    tab(&mut tabs, id).push(Page::unbuilt("watch"));
    tab(&mut tabs, id).push(Page::unbuilt("subscriptions"));
    tab(&mut tabs, id).push(Page::unbuilt("channel"));
    let top_level = |r: &&str| matches!(*r, "home" | "subscriptions");
    assert_eq!(
      tabs.get(id).and_then(|t| t.section(top_level)),
      Some(&"subscriptions")
    );
    tab(&mut tabs, id).back();
    tab(&mut tabs, id).back();
    assert_eq!(
      tabs.get(id).and_then(|t| t.current()).map(|p| p.route),
      Some("watch")
    );
    assert_eq!(
      tabs.get(id).and_then(|t| t.section(top_level)),
      Some(&"home")
    );
  }

  #[test]
  fn replace_swaps_the_page_shown() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["playlist", "watch"], None);
    tab(&mut tabs, id).replace(Page::unbuilt("next"));
    assert_eq!(routes(&tabs, id), vec!["next", "playlist"]);
  }

  #[test]
  fn the_settings_open_in_a_tab_of_their_own() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add_settings();
    let t = tabs.get(id).unwrap();
    assert!(t.settings_shown());
    assert!(!t.can_go_back());
    assert!(t.current().is_none());
    assert!(t.page().is_none());
    assert!(t.section(|_| true).is_none());
    let name = |route: &&str| route.to_string().into();
    assert_eq!(tabs.title(id, name).as_deref(), Some("Settings"));
    // A page opened from the settings comes after them.
    tab(&mut tabs, id).push(Page::unbuilt("sign-in"));
    tab(&mut tabs, id).back();
    assert!(tabs.get(id).unwrap().settings_shown());
    tab(&mut tabs, id).forward();
    tab(&mut tabs, id).back();
    // A session change keeps them alone in their tab.
    tab(&mut tabs, id).reset(vec!["home"]);
    let t = tabs.get(id).unwrap();
    assert!(t.settings_shown() && !t.can_go_back() && !t.can_go_forward());
  }

  #[test]
  fn a_reset_keeps_the_settings_shown() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["home", "watch"], None);
    tab(&mut tabs, id).reset(vec!["home"]);
    assert!(!tabs.get(id).unwrap().settings_shown());
    let settings = tabs.add_settings();
    tab(&mut tabs, settings).push(Page::unbuilt("watch"));
    tab(&mut tabs, settings).reset(vec!["home"]);
    // Not on the settings: the history starts over.
    assert_eq!(routes(&tabs, settings), vec!["home"]);
    assert!(!tab(&mut tabs, settings).can_go_back());
  }

  #[test]
  fn reload_needs_a_built_page() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["home"], None);
    // Not built yet (or on the settings): nothing to reload.
    assert!(!tab(&mut tabs, id).reload());
    let settings = tabs.add_settings();
    assert!(!tab(&mut tabs, settings).reload());
  }

  #[test]
  fn the_history_is_bounded() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let id = tabs.add(vec!["home"], None);
    for _ in 0..MAX_ENTRIES + 5 {
      tab(&mut tabs, id).push(Page::unbuilt("page"));
    }
    assert_eq!(routes(&tabs, id).len(), MAX_ENTRIES);
    assert_eq!(tabs.get(id).unwrap().position(), MAX_ENTRIES - 1);
  }

  #[test]
  fn closing_the_shown_tab_falls_back_to_another() {
    let mut tabs: Tabs<&str> = Tabs::default();
    let first = tabs.add(Vec::new(), None);
    let second = tabs.add(Vec::new(), None);
    tabs.show(second, WindowId::from(1));
    tabs.remove(second);
    assert_eq!(tabs.shown(), Some(first));
    assert_eq!(tabs.shown_in(WindowId::from(1)), None);
    assert_eq!(tabs.ids(), vec![first]);
    tabs.remove(first);
    assert_eq!(tabs.shown(), None);
  }

  #[test]
  fn each_window_renders_its_own_tab() {
    let (one, two) = (WindowId::from(1), WindowId::from(2));
    let mut tabs: Tabs<&str> = Tabs::default();
    let (a, b) = (tabs.add(Vec::new(), None), tabs.add(Vec::new(), None));
    tabs.show(a, one);
    tabs.show(b, two);
    assert_eq!(tabs.shown_in(one), Some(a));
    assert_eq!(tabs.shown_in(two), Some(b));
    // A notification opens in the tab shown last.
    assert_eq!(tabs.shown(), Some(b));
    // Tab `a` moved to window two: window one renders nothing of the space until it shows
    // another of its tabs.
    tabs.show(a, two);
    assert_eq!(tabs.shown_in(one), None);
    assert_eq!(tabs.shown_in(two), Some(a));
  }
}
