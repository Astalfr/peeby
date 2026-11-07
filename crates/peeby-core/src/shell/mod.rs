//! The window's root: a top bar shared by every space (menu button, back and forward, logo, the
//! open tabs and "+" to open one of any space, search, audio output, notifications, settings),
//! the sidebar with the entries of the tab's space, and the tab's current page in the content
//! panel: the space's, or the settings page, which opens in a tab of its own (an entry of its
//! history, which pages opened from there follow). Switching tabs or opening the settings
//! while a video plays moves it to the mini-player, a window of its own above the others
//! (docs/design.md, ADR 0008, ADR 0015, ADR 0019). A tab held and moved is reordered along the
//! bar (`tab_drag`, ADR 0028).

pub mod lives;
mod mini_player;
mod output_volume;
mod settings_page;
pub mod space;
mod tab_drag;
pub mod windows;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Anchor, AnyView, App, Context, Entity, FontWeight, ImageSource, MouseButton, NavigationDirection,
  ObjectFit, SharedString, Subscription, Task, Window, WindowHandle, WindowId, anchored, canvas,
  deferred, div, img, px,
};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_component::popover::{Popover, PopoverState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, IconName, Selectable, Sizable, TitleBar, h_flex, v_flex};
use player::AudioDevice;
use ui::assets::{self, Icon};
use ui::i18n::t;
use ui::images::Images;
use ui::player::{AudioOutput, NowPlaying, PictureInPicture, PlayerView, now_playing};
use ui::splash::Splash;
use ui::{motion, tf, theme};

use crate::settings::{NewTabStyle, Settings};
use mini_player::MiniPlayer;
use settings_page::SettingsView;
pub use settings_page::delivery_row;
use space::{Avatar, HoverCard, MenuEntry, SpaceHandle, TabId};
use tab_drag::{Slide, TabDrag, TabLayout};

/// Width of the notifications panel.
const NOTIFICATIONS_WIDTH: f32 = 440.;
/// Width of the audio output menu.
const AUDIO_MENU_WIDTH: f32 = 320.;
/// A tab's widest; tabs narrow down to [`TAB_MIN_WIDTH`] when the bar is full.
const TAB_MAX_WIDTH: f32 = 220.;
const TAB_MIN_WIDTH: f32 = 140.;
/// A cell of the new tab frame: "+" and each space's logo.
const NEW_TAB_ITEM: f32 = 26.;
/// Width of the menu of new tabs (`NewTabStyle::Menu`).
const NEW_TAB_MENU_WIDTH: f32 = 220.;

/// A tab of the bar: its space (an index in `Shell::spaces`, which never changes) and its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TabRef {
  space: usize,
  id: TabId,
}

/// The mini-player window and the tab whose video it shows.
struct Mini {
  window: WindowHandle<MiniPlayer>,
  tab: TabRef,
  /// Opened by the player's picture-in-picture button: it stays while its page shows.
  pinned: bool,
  /// The player told it plays in the mini-player ([`PlayerView::set_picture_in_picture`]).
  player: Option<gpui::WeakEntity<PlayerView>>,
}

/// What a space's event asks of the shell, copied out of the event to be handled later.
enum Request {
  Activate(TabId),
  Add {
    tab: TabId,
    after: Option<TabId>,
    focus: bool,
  },
  Settings {
    reveal: bool,
  },
}

/// The window's root view.
pub struct Shell {
  /// Its window's (ADR 0028).
  window: WindowId,
  spaces: Vec<SpaceHandle>,
  /// The tabs, left to right.
  tabs: Vec<TabRef>,
  /// The tab shown; `None` only before the first one.
  active: Option<TabRef>,
  /// The last tab opened in the background from a tab: the next one from there goes after it,
  /// as in a browser. Reset when another tab shows.
  opened_from: Option<(TabId, TabId)>,
  /// The settings page (gear), made for the tab whose history shows it; made again when
  /// another tab shows its own.
  settings: Option<(TabId, Entity<SettingsView>)>,
  /// The space whose part the settings page shows next, unfolded and scrolled to (a sign-in
  /// request).
  reveal: Option<usize>,
  mini: Option<Mini>,
  /// The opening animation over the window, until it ends.
  splash: Option<Splash>,
  /// The sidebar entry whose [`HoverCard`] shows, or fades out when `false`.
  card: Option<(SharedString, bool)>,
  /// When the last card started fading out, for [`CARD_GRACE`].
  card_hidden: Option<Instant>,
  /// Shows the card of the entry under the pointer after [`CARD_DELAY`].
  card_delay: Task<()>,
  /// The audio outputs, as last listed (the menu opens on them while it lists them again);
  /// `None` before the first list.
  audio_devices: Option<Vec<AudioDevice>>,
  audio_listing: Task<()>,
  /// The menu's slider of the selected output's volume.
  volume_slider: output_volume::VolumeSlider,
  /// The tab held down with the mouse, moved along the bar once past a few pixels (ADR 0028).
  tab_drag: Option<TabDrag>,
  /// Where the tabs were drawn, shared with the canvases that measure them.
  tab_layout: Rc<TabLayout>,
  /// The tabs sliding to a new place (passed by the dragged one, or released).
  tab_slides: HashMap<TabId, Slide>,
  _subscriptions: Vec<Subscription>,
}

/// How long the pointer rests on a sidebar entry before its card shows, as GPUI's tooltips.
const CARD_DELAY: Duration = Duration::from_millis(500);
/// From one entry to the next, the pointer may leave the first (the gap between entries, or GPUI
/// telling the leave first) before it enters the next: the next card still shows at once.
const CARD_GRACE: Duration = Duration::from_millis(300);

impl Shell {
  /// The first window's shell, over `spaces` and the tabs they opened at startup, the first
  /// one shown, with the opening animation when `splash` (and GPUI's `reduce_motion` is off).
  pub fn new(
    spaces: Vec<SpaceHandle>,
    splash: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    Self::with_tabs(spaces, None, splash, window, cx)
  }

  /// A shell over `spaces` holding `tabs` (a tab moved to a new window), or the tabs the
  /// spaces opened at startup when `None`. The spaces' events reach it through
  /// `windows::route_events`.
  fn with_tabs(
    spaces: Vec<SpaceHandle>,
    tabs: Option<Vec<TabRef>>,
    splash: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut subscriptions = Vec::new();
    for (index, space) in spaces.iter().enumerate() {
      let shell = cx.weak_entity();
      subscriptions.push(space.observe(
        cx,
        Box::new(move |cx| {
          shell
            .update(cx, |shell, cx| shell.space_changed(index, cx))
            .ok();
        }),
      ));
    }
    // The window used last takes what has no window of its own; the accent is the whole app's,
    // so it follows the window in front.
    subscriptions.push(cx.observe_window_activation(window, |shell, window, cx| {
      if window.is_window_active() {
        windows::used(shell.window, cx);
        if let Some(tab) = shell.active {
          theme::transition_accent(shell.spaces[tab.space].info(cx).accent, cx);
        }
      }
    }));
    // Another player started in this window: the mini-player shows only the one that plays.
    // Another window's player is its own.
    subscriptions.push(cx.observe_global::<NowPlaying>(|shell, cx| {
      let shown = shell.mini_player(cx);
      if shown.is_some() && shown != now_playing(shell.window, cx) {
        shell.close_mini(cx);
      }
    }));
    // A player's picture-in-picture button. Deferred: the click comes from this window, which
    // cannot be updated from inside its own event, and the mini-player needs it (its screen).
    let handle = window.window_handle();
    subscriptions.push(cx.observe_global::<PictureInPicture>(move |_, cx| {
      let Some((player, open)) = cx.global::<PictureInPicture>().request() else {
        return;
      };
      let shell = cx.weak_entity();
      cx.defer(move |cx| {
        handle
          .update(cx, |_, window, cx| {
            shell
              .update(cx, |shell, cx| {
                shell.picture_in_picture(player, open, window, cx)
              })
              .ok();
          })
          .ok();
      });
    }));
    // An update's installer is ready: the title bar shows its button (ADR 0027).
    subscriptions.push(cx.observe_global::<crate::update::Updater>(|_, cx| cx.notify()));
    // F5 and Ctrl+R refresh the page, as in a browser. Observed after dispatch, so they work
    // whatever has the focus (or nothing); only in this window, and not in fullscreen, which
    // belongs to the player.
    let main = window.window_handle().window_id();
    subscriptions.push(cx.observe_keystrokes(move |shell, event, window, cx| {
      let keystroke = &event.keystroke;
      let m = &keystroke.modifiers;
      let refresh = match keystroke.key.as_str() {
        "f5" => !m.control && !m.alt && !m.shift && !m.platform,
        "r" => m.control && !m.alt && !m.shift && !m.platform,
        _ => false,
      };
      if refresh && window.window_handle().window_id() == main && !window.is_fullscreen() {
        shell.reload(cx);
      }
    }));
    let startup = tabs.is_none();
    let mut tabs: Vec<TabRef> = tabs.unwrap_or_else(|| {
      spaces
        .iter()
        .enumerate()
        .flat_map(|(space, handle)| {
          handle
            .tabs(cx)
            .into_iter()
            .map(move |id| TabRef { space, id })
        })
        .collect()
    });
    // No space opened one (none chosen to start with, or only a service that is unavailable):
    // the first space's.
    if startup
      && tabs.is_empty()
      && let Some(first) = spaces.first()
    {
      tabs.push(TabRef {
        space: 0,
        id: first.new_tab(cx),
      });
    }
    let mut shell = Self {
      window: window.window_handle().window_id(),
      spaces,
      tabs,
      active: None,
      opened_from: None,
      settings: None,
      reveal: None,
      mini: None,
      splash: (splash && !cx.reduce_motion()).then(Splash::new),
      card: None,
      card_hidden: None,
      card_delay: Task::ready(()),
      audio_devices: None,
      audio_listing: Task::ready(()),
      volume_slider: Default::default(),
      tab_drag: None,
      tab_layout: Rc::default(),
      tab_slides: HashMap::new(),
      _subscriptions: subscriptions,
    };
    if let Some(&first) = shell.tabs.first() {
      shell.active = Some(first);
      let space = &shell.spaces[first.space];
      space.show_tab(first.id, shell.window, cx);
      theme::set_accent(space.info(cx).accent, cx);
    }
    shell
  }

  /// The pointer entered (`hovered`) or left the sidebar entry `id`, which has a card. From one
  /// card to the next, the next shows at once.
  fn hover_entry(&mut self, id: SharedString, hovered: bool, cx: &mut Context<Self>) {
    if !hovered {
      self.card_delay = Task::ready(());
      if let Some((shown, visible)) = &mut self.card
        && *shown == id
      {
        *visible = false;
        self.card_hidden = Some(Instant::now());
        cx.notify();
      }
      return;
    }
    let recent = self
      .card_hidden
      .is_some_and(|hidden| hidden.elapsed() < CARD_GRACE);
    if matches!(&self.card, Some((_, true))) || recent {
      self.card = Some((id, true));
      cx.notify();
      return;
    }
    self.card_delay = cx.spawn(async move |this, cx| {
      cx.background_executor().timer(CARD_DELAY).await;
      this
        .update(cx, |this, cx| {
          this.card = Some((id, true));
          cx.notify();
        })
        .ok();
    });
  }

  /// The tab shown and its space.
  fn current(&self) -> Option<(TabId, &SpaceHandle)> {
    let tab = self.active?;
    Some((tab.id, self.spaces.get(tab.space)?))
  }

  fn find(&self, id: TabId) -> Option<TabRef> {
    self.tabs.iter().find(|t| t.id == id).copied()
  }

  /// Whether tab `id` is in this window.
  pub fn holds(&self, id: TabId) -> bool {
    self.find(id).is_some()
  }

  /// What a space asked, routed to this window (`windows::route_events`).
  fn handle(
    &mut self,
    space: usize,
    request: Request,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match request {
      Request::Activate(tab) => self.activate(tab, window, cx),
      Request::Add { tab, after, focus } => self.add_tab(space, tab, after, focus, window, cx),
      Request::Settings { reveal } => self.open_settings(Some(space), reveal, window, cx),
    }
  }

  /// A tab's menu: it leaves this window for a new one (or a spare), with its pages and its
  /// video. Not the window's only tab.
  fn move_to_new_window(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
    let Some(tab) = self.detach_tab(id, true, cx) else {
      return;
    };
    let (bounds, display) = windows::cascade(window, cx);
    // Deferred: a spare window is updated, which cannot happen from inside this one's event.
    cx.defer(move |cx| {
      windows::open_with(tab, bounds, display, true, cx);
    });
  }

  /// Takes tab `id` out of this window without closing it (it moves to another window): the
  /// next tab shows if it was the one shown, without moving its video to the mini-player. With
  /// `keep_one`, the last tab stays (a new window for it would leave this one empty); without,
  /// it goes too (it joins another window, this one is put aside).
  fn detach_tab(&mut self, id: TabId, keep_one: bool, cx: &mut Context<Self>) -> Option<TabRef> {
    if keep_one && self.tabs.len() <= 1 {
      return None;
    }
    let index = self.tabs.iter().position(|t| t.id == id)?;
    let tab = self.tabs.remove(index);
    if self.mini.as_ref().is_some_and(|m| m.tab == tab) {
      self.close_mini(cx);
    }
    if self.settings.as_ref().is_some_and(|(t, _)| *t == id) {
      self.settings = None;
    }
    self.tab_drag = None;
    if self.active == Some(tab) {
      match self
        .tabs
        .get(index.min(self.tabs.len().saturating_sub(1)))
        .copied()
      {
        Some(next) => {
          self.show(next, cx);
          cx.refresh_windows();
        }
        None => self.active = None,
      }
    }
    cx.notify();
    Some(tab)
  }

  /// A tab moved in from another window, at `index` (at the end when `None`), shown.
  fn adopt_tab(&mut self, tab: TabRef, index: Option<usize>, cx: &mut Context<Self>) {
    if self.holds(tab.id) {
      return;
    }
    let at = index.unwrap_or(self.tabs.len()).min(self.tabs.len());
    self.tabs.insert(at, tab);
    self.show(tab, cx);
  }

  /// The window closes while others stay open: its tabs close (their pages and videos with
  /// them), as a browser's do, and it waits hidden for the next new window.
  pub(crate) fn close_all(&mut self, cx: &mut Context<Self>) {
    self.close_mini(cx);
    for tab in std::mem::take(&mut self.tabs) {
      self.spaces[tab.space].close_tab(tab.id, cx);
    }
    self.active = None;
    self.settings = None;
    self.opened_from = None;
    self.tab_drag = None;
    cx.notify();
  }

  /// Whether the tab shown is on the settings page.
  fn in_settings(&self, cx: &App) -> bool {
    self
      .current()
      .is_some_and(|(tab, space)| space.settings_shown(tab, cx))
  }

  /// Shows tab `id` (its tab, a notification of its space, the mini-player's button).
  fn activate(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
    let Some(tab) = self.find(id) else {
      return;
    };
    if self.active == Some(tab) {
      return;
    }
    self.leave_page(window, cx);
    self.show(tab, cx);
    cx.refresh_windows();
  }

  /// Makes `tab` the one shown, its video back from the mini-player.
  fn show(&mut self, tab: TabRef, cx: &mut Context<Self>) {
    self.active = Some(tab);
    self.opened_from = None;
    let space = self.spaces[tab.space].clone();
    space.show_tab(tab.id, self.window, cx);
    self.enter_page(cx);
    theme::transition_accent(space.info(cx).accent, cx);
    cx.notify();
  }

  /// Adds a tab a space opened, after `after` (and after the tabs opened from there before it),
  /// the tab shown when `None`, else at the end; shows it when `focus`.
  fn add_tab(
    &mut self,
    space: usize,
    id: TabId,
    after: Option<TabId>,
    focus: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.find(id).is_some() {
      return;
    }
    let after = after.or(self.active.map(|t| t.id));
    let position = |shell: &Self, id: TabId| shell.tabs.iter().position(|t| t.id == id);
    let at = match after.and_then(|from| position(self, from).map(|i| (from, i))) {
      Some((from, i)) => match self.opened_from {
        Some((opener, last)) if opener == from => position(self, last).map_or(i + 1, |j| j + 1),
        _ => i + 1,
      },
      None => self.tabs.len(),
    };
    self.tabs.insert(at, TabRef { space, id });
    self.opened_from = after.map(|from| (from, id));
    if focus {
      self.activate(id, window, cx);
    }
    cx.notify();
  }

  /// The "+" menu: a tab of `space` on its first page, shown.
  fn new_tab(&mut self, space: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(handle) = self.spaces.get(space).cloned() else {
      return;
    };
    let id = handle.new_tab(cx);
    self.tabs.push(TabRef { space, id });
    self.activate(id, window, cx);
  }

  /// Closes tab `id` (its "×", a middle click on it): its pages and its video go, and the next
  /// tab shows if it was the one shown. The last tab stays.
  fn close_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
    if self.tabs.len() <= 1 {
      return;
    }
    let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
      return;
    };
    let tab = self.tabs.remove(index);
    if self.mini.as_ref().is_some_and(|m| m.tab == tab) {
      self.close_mini(cx);
    }
    self.spaces[tab.space].close_tab(tab.id, cx);
    if self.active == Some(tab) {
      // Nothing to move to the mini-player: the page that showed is gone.
      let next = self.tabs[index.min(self.tabs.len() - 1)];
      self.show(next, cx);
      cx.refresh_windows();
    }
    cx.notify();
  }

  /// The page on screen goes away (another tab, the settings): a video playing on it moves to
  /// the mini-player. With the settings shown, it is the page under them.
  fn leave_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.mini.is_some() {
      return;
    }
    let Some(tab) = self.active else {
      return;
    };
    let Some(player) = self.spaces[tab.space].player(tab.id, cx) else {
      return;
    };
    if player.read(cx).is_paused() {
      return;
    }
    self.open_mini(tab, false, window, cx);
  }

  /// The tab's page is on screen again: its video leaves the mini-player and plays on.
  /// Unless the user put it there with the picture-in-picture button.
  fn enter_page(&mut self, cx: &mut Context<Self>) {
    let unpinned = self.mini.as_ref().filter(|m| !m.pinned).map(|m| m.tab);
    if !self.in_settings(cx) && unpinned.is_some() && unpinned == self.active {
      self.close_mini(cx);
    }
  }

  fn open_mini(&mut self, tab: TabRef, pinned: bool, window: &mut Window, cx: &mut Context<Self>) {
    let Some(handle) = self.spaces.get(tab.space).cloned() else {
      return;
    };
    let shell = cx.weak_entity();
    let main = window.window_handle();
    match mini_player::open(handle, tab.id, shell, main, window, cx) {
      Ok(window) => {
        self.mini = Some(Mini {
          window,
          tab,
          pinned,
          player: None,
        });
        self.sync_pip(cx);
      }
      Err(e) => tracing::warn!("cannot open the mini-player: {e:#}"),
    }
  }

  /// The picture-in-picture button of `player` (or I): the mini-player opens on it, its page
  /// staying, or closes. Only for the player of the tab shown here: another window's is its own.
  fn picture_in_picture(
    &mut self,
    player: Entity<PlayerView>,
    open: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(tab) = self.active else {
      return;
    };
    if self.in_settings(cx) || self.spaces[tab.space].player(tab.id, cx) != Some(player) {
      return;
    }
    let shown = self.mini.as_ref().is_some_and(|m| m.tab == tab);
    match (open, shown) {
      (true, true) => {
        if let Some(mini) = &mut self.mini {
          mini.pinned = true;
        }
      }
      (true, false) => {
        // Another tab's video: the one asked for takes its place.
        self.close_mini(cx);
        self.open_mini(tab, true, window, cx);
      }
      (false, true) => self.close_mini(cx),
      (false, false) => {}
    }
    cx.notify();
  }

  /// Tells the players whether the mini-player shows them: the one it shows now, and the one
  /// it showed before (a queue moved on, the mini-player closed). Their pages show a
  /// placeholder meanwhile.
  fn sync_pip(&mut self, cx: &mut App) {
    let current = self.mini_player(cx);
    let Some(mini) = &mut self.mini else {
      return;
    };
    let previous = mini.player.as_ref().and_then(|p| p.upgrade());
    if previous == current {
      return;
    }
    mini.player = current.as_ref().map(|p| p.downgrade());
    if let Some(previous) = previous {
      previous.update(cx, |p, cx| p.set_picture_in_picture(false, cx));
    }
    if let Some(current) = current {
      current.update(cx, |p, cx| p.set_picture_in_picture(true, cx));
    }
  }

  /// Closes the mini-player window; its video stays where it is (paused or not). Deferred: the
  /// click that closes it may come from that window, which cannot be updated from inside itself.
  fn close_mini(&mut self, cx: &mut App) {
    if let Some(mini) = self.mini.take() {
      if let Some(player) = mini.player.as_ref().and_then(|p| p.upgrade()) {
        player.update(cx, |p, cx| p.set_picture_in_picture(false, cx));
      }
      cx.defer(move |cx| {
        if let Err(e) = mini
          .window
          .update(cx, |_, window, _| window.remove_window())
        {
          tracing::warn!("cannot close the mini-player: {e:#}");
        }
      });
    }
  }

  /// The window goes to the notification area (ADR 0020): the video of the page shown pauses;
  /// the mini-player's, still on screen, goes on.
  pub(crate) fn go_background(&mut self, cx: &mut Context<Self>) {
    let Some(tab) = self.active else {
      return;
    };
    if self.mini.as_ref().is_some_and(|m| m.tab == tab) {
      return;
    }
    if let Some(player) = self.spaces[tab.space].player(tab.id, cx) {
      player.read(cx).pause();
    }
  }

  /// The mini-player's "×": the video pauses and the window closes.
  pub(crate) fn dismiss_mini(&mut self, cx: &mut Context<Self>) {
    if let Some(player) = self.mini_player(cx) {
      player.read(cx).pause();
    }
    self.close_mini(cx);
    cx.notify();
  }

  /// The mini-player's "back to the page": its tab shows in the main window.
  pub(crate) fn return_from_mini(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(tab) = self.mini.as_ref().map(|m| m.tab) {
      // Back on its page, it closes even if the button opened it.
      if let Some(mini) = &mut self.mini {
        mini.pinned = false;
      }
      // The window may be hidden in the notification area.
      crate::desktop::reveal(self.window, cx);
      window.activate_window();
      self.activate(tab.id, window, cx);
      // The tab is on the settings page: back to the page under it.
      if self.in_settings(cx)
        && let Some((id, space)) = self.current()
      {
        space.clone().back(id, cx);
      }
      self.enter_page(cx);
      cx.notify();
    }
  }

  /// The player the mini-player shows: the current one of its tab.
  fn mini_player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    let mini = self.mini.as_ref()?;
    self.spaces.get(mini.tab.space)?.player(mini.tab.id, cx)
  }

  /// A space changed (a page opened, a queue moved to its next video, a title arrived): the
  /// mini-player follows its tab's current player, and closes when there is none, or when its
  /// tab shows its page again (a page opened from the settings).
  fn space_changed(&mut self, index: usize, cx: &mut Context<Self>) {
    if let Some(mini) = &self.mini
      && mini.tab.space == index
    {
      let on_its_page = !mini.pinned && Some(mini.tab) == self.active && !self.in_settings(cx);
      if on_its_page || self.mini_player(cx).is_none() {
        self.close_mini(cx);
      } else {
        mini.window.update(cx, |_, _, cx| cx.notify()).ok();
        self.sync_pip(cx);
      }
    }
    cx.notify();
  }

  /// The gear, or a request of space `from`: the settings page in a tab of its own, after the
  /// tab shown, or the tab that already shows it. A video playing goes to the mini-player. With
  /// `reveal`, the page shows `from`'s part, unfolded and scrolled to.
  fn open_settings(
    &mut self,
    from: Option<usize>,
    reveal: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if reveal {
      self.reveal = from;
    }
    if self.in_settings(cx) {
      // Already shown: revealed now; not built yet: when it is.
      if let Some((_, view)) = &self.settings
        && let Some(space) = self.reveal.take()
      {
        view.update(cx, |view, cx| view.reveal(space, cx));
      }
      return;
    }
    let open = self
      .tabs
      .iter()
      .find(|t| self.spaces[t.space].settings_shown(t.id, cx))
      .copied();
    if let Some(tab) = open {
      self.activate(tab.id, window, cx);
      return;
    }
    let Some(current) = self.active else {
      self.reveal = None;
      return;
    };
    let space = from
      .filter(|&s| s < self.spaces.len())
      .unwrap_or(current.space);
    let id = self.spaces[space].settings_tab(cx);
    self.add_tab(space, id, Some(current.id), true, window, cx);
  }

  /// The settings page of the tab shown, made for it if needed.
  fn settings_view(&mut self, cx: &mut Context<Self>) -> Option<AnyView> {
    let tab = self.active?;
    match &self.settings {
      Some((id, view)) if *id == tab.id => Some(view.clone().into()),
      _ => {
        let spaces = self.spaces.clone();
        let reveal = self.reveal.take();
        let view = cx.new(|cx| {
          let mut view = SettingsView::new(spaces, tab.space, cx);
          if let Some(space) = reveal {
            view.reveal(space, cx);
          }
          view
        });
        self.settings = Some((tab.id, view.clone()));
        Some(view.into())
      }
    }
  }

  /// The refresh button: the page shown is built again (its data read again), or the settings
  /// page made again.
  fn reload(&mut self, cx: &mut Context<Self>) {
    if self.in_settings(cx) {
      self.settings = None;
    } else if let Some((tab, space)) = self.current() {
      space.clone().reload(tab, cx);
    }
    cx.notify();
  }

  /// Back one entry in the tab's history.
  fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some((tab, space)) = self.current() {
      space.clone().back(tab, cx);
    }
    self.navigated(window, cx);
  }

  /// Forward one entry in the tab's history.
  fn forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some((tab, space)) = self.current() {
      space.clone().forward(tab, cx);
    }
    self.navigated(window, cx);
  }

  /// After "back" or "forward": on the settings, a playing video goes to the mini-player; on its
  /// page, it comes back from there.
  fn navigated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.in_settings(cx) {
      self.leave_page(window, cx);
    } else {
      self.enter_page(cx);
    }
    cx.notify();
  }

  /// The top bar's search: the search page of the shown tab's space, in a new tab after it,
  /// shown (a video playing goes to the mini-player).
  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(current) = self.active else {
      return;
    };
    if let Some(id) = self.spaces[current.space].search_tab(cx) {
      self.add_tab(current.space, id, Some(current.id), true, window, cx);
    }
  }

  fn open_entry(&mut self, id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
    if let Some((tab, space)) = self.current() {
      space.clone().open_entry(tab, &id, window, cx);
    }
    self.enter_page(cx);
    cx.notify();
  }

  /// A middle click on a sidebar entry: its page in a new tab, in the background.
  fn open_entry_in_new_tab(&mut self, id: SharedString, cx: &mut Context<Self>) {
    if let Some((tab, space)) = self.current() {
      space.clone().open_entry_in_new_tab(tab, &id, cx);
    }
  }

  /// Lists the audio outputs again (the menu opened: a headset may have been plugged in).
  fn list_audio_devices(&mut self, cx: &mut Context<Self>) {
    let listing = cx
      .background_executor()
      .spawn(async { player::audio_devices(None) });
    self.audio_listing = cx.spawn(async move |this, cx| {
      let devices = match listing.await {
        Ok(devices) => devices,
        Err(e) => {
          tracing::warn!("cannot list the audio outputs: {e}");
          Vec::new()
        }
      };
      this
        .update(cx, |this, cx| {
          this.audio_devices = Some(devices);
          cx.notify();
        })
        .ok();
    });
  }

  // --- Top bar ------------------------------------------------------------------------------

  fn top_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> TitleBar {
    let collapsed = Settings::get(cx).sidebar_collapsed;
    let menu = Button::new("menu")
      .ghost()
      .icon(Icon::Menu)
      .tooltip(if collapsed {
        t("Expand the menu", "Développer le menu")
      } else {
        t("Collapse the menu", "Réduire le menu")
      })
      .on_click(cx.listener(|_, _, _, cx| {
        Settings::update(cx, |s| s.sidebar_collapsed = !s.sidebar_collapsed);
        cx.notify();
      }));
    let can_go_back = self
      .current()
      .is_some_and(|(tab, s)| s.can_go_back(tab, cx));
    let can_go_forward = self
      .current()
      .is_some_and(|(tab, s)| s.can_go_forward(tab, cx));
    let in_settings = self.in_settings(cx);
    let search = self
      .current()
      .filter(|(_, s)| s.searchable(cx))
      .map(|(_, s)| s.info(cx).name);
    // Always present (disabled when there is nowhere to go) so the bar does not shift.
    let back = Button::new("back")
      .ghost()
      .icon(Icon::ArrowLeft)
      .disabled(!can_go_back)
      .tooltip(t("Back", "Retour"))
      .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)));
    let forward = Button::new("forward")
      .ghost()
      .icon(Icon::ArrowRight)
      .disabled(!can_go_forward)
      .tooltip(t("Forward", "Suivant"))
      .on_click(cx.listener(|this, _, window, cx| this.forward(window, cx)));
    let reload = Button::new("reload")
      .ghost()
      .icon(Icon::Refresh)
      .tooltip(t("Refresh (F5)", "Actualiser (F5)"))
      .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));
    TitleBar::new().h(theme::TITLE_BAR_HEIGHT).child(
      h_flex()
        .h_full()
        .w_full()
        .gap_3()
        .items_center()
        // The bar is a window drag area; occluding the controls keeps their clicks.
        .child(div().occlude().child(menu))
        .child(h_flex().occlude().child(back).child(forward).child(reload))
        .child(mark(window, cx))
        // The bar's empty part, right of "+", still drags the window: a flex row, so the occluding
        // wrapper is as wide as the tabs (a block child would stretch over the whole part).
        .child(
          h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .ml_4()
            .child(div().occlude().min_w_0().child(self.tabs(window, cx))),
        )
        .child(
          h_flex()
            .occlude()
            .mr_2()
            .gap_1()
            .children(update_button(cx))
            .child(
              // Each space searches its own catalog.
              Button::new("search")
                .ghost()
                .icon(Icon::Search)
                .disabled(search.is_none())
                .tooltip(match &search {
                  Some(name) => tf!("Search {name}", "Rechercher dans {name}"),
                  None => t("Search (soon)", "Recherche (bientôt)").to_owned(),
                })
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .child(self.audio_output(cx))
            .child(self.bell(cx))
            .child(
              Button::new("settings")
                .ghost()
                .icon(Icon::Settings)
                .tooltip(t("Settings", "Réglages"))
                .when(in_settings, |b| b.selected(true))
                .on_click(
                  cx.listener(|this, _, window, cx| this.open_settings(None, false, window, cx)),
                ),
            ),
        ),
    )
  }

  /// The open tabs: the service's logo and the page's title; the shown one has a light
  /// background and a line in its accent underneath. Hovered or shown, a tab has its "×" (not
  /// the last one); a middle click closes it. A tab held and moved follows the pointer along the
  /// bar, its slot left empty, and takes its neighbors' places (ADR 0028). Then "+", a menu of the
  /// spaces to open a tab of.
  fn tabs(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let dragged = self.tab_drag.filter(|d| d.moving);
    // Placed from the last frame's tabs, before they are measured again.
    let dragged_left = dragged.map(|drag| self.dragged_left(&drag));
    // Refilled by each tab's canvas as it is laid out.
    self.tab_layout.tabs.borrow_mut().clear();
    // A window keeps a tab.
    let movable = self.tabs.len() > 1;
    let tabs: Vec<_> = self
      .tabs
      .iter()
      .map(|&tab| {
        let id = tab.id;
        let group = SharedString::from(format!("tab-{id}"));
        let tooltip = self.spaces[tab.space].title(id, cx);
        let layout = self.tab_layout.clone();
        let slide = self.tab_slide(id);
        if slide.is_some() {
          window.request_animation_frame();
        }
        let held = dragged.filter(|d| d.tab == id);
        self
          .tab_face(tab, window, cx)
          .id(group.clone())
          .group(group)
          .when_some(slide, |el, offset| el.left(offset))
          .when(held.is_some(), |el| el.invisible())
          // Pulled off the bar: its slot closes.
          .when(held.is_some_and(|d| d.detached), |el| el.hidden())
          .child(
            canvas(
              move |bounds, _, _| layout.tabs.borrow_mut().push((id, bounds)),
              |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
          )
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
          })
          .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
              this.press_tab(id, event.position, cx)
            }),
          )
          .on_click(cx.listener(move |this, _, window, cx| this.activate(id, window, cx)))
          .on_aux_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
            if event.is_middle_click() {
              this.close_tab(id, cx);
            }
          }))
          .context_menu({
            let shell = cx.weak_entity();
            move |menu, _, _| {
              let shell = shell.clone();
              menu.item(
                PopupMenuItem::new(t(
                  "Move to a new window",
                  "Déplacer dans une nouvelle fenêtre",
                ))
                .disabled(!movable)
                .on_click(move |_, window, cx| {
                  shell
                    .update(cx, |shell, cx| shell.move_to_new_window(id, window, cx))
                    .ok();
                }),
              )
            }
          })
      })
      .collect();
    // The dragged tab over the others, opaque, occluding the slots under it (drawn over the
    // whole window instead once detached).
    let copy = dragged
      .zip(dragged_left)
      .filter(|(drag, _)| !drag.detached)
      .and_then(|(drag, left)| {
        let tab = self.find(drag.tab)?;
        Some(
          div()
            .id("tab-dragged")
            .occlude()
            .absolute()
            .top_0()
            .h_full()
            .left(left)
            .w(drag.width)
            .rounded(theme::RADIUS)
            .bg(theme::chrome(cx))
            .shadow_md()
            .child(
              self
                .tab_face(tab, window, cx)
                .w_full()
                .max_w_full()
                .min_w_0(),
            ),
        )
      });
    let layout = self.tab_layout.clone();
    let (held, shell) = (self.tab_drag.is_some(), cx.weak_entity());
    h_flex()
      .relative()
      .h(px(34.))
      .min_w_0()
      .gap_1()
      .items_center()
      .child(
        canvas(
          move |bounds, window, _| {
            layout.row.set(bounds);
            layout.origin.set(window.bounds().origin);
          },
          move |_, _, window, _| {
            if held {
              tab_drag::listen(shell, window);
            }
          },
        )
        .absolute()
        .inset_0(),
      )
      .children(tabs)
      .child(self.new_tab_menu(window, cx))
      .children(copy)
  }

  /// What a tab shows (its logo or the gear, its title, its "×"), without the slot's listeners:
  /// drawn in its slot and, while it is dragged, as the copy that follows the pointer.
  fn tab_face(&self, tab: TabRef, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
    let closable = self.tabs.len() > 1;
    let space = &self.spaces[tab.space];
    let info = space.info(cx);
    let title = space.title(tab.id, cx);
    let active = Some(tab) == self.active;
    let group = SharedString::from(format!("tab-{}", tab.id));
    let id = tab.id;
    let close = closable.then(|| {
      div()
        .id(SharedString::from(format!("tab-close-{id}")))
        .flex_none()
        .size(px(18.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::muted())
        .when(!active, |el| {
          el.invisible().group_hover(group.clone(), |s| s.visible())
        })
        .hover(|s| {
          s.bg(gpui::white().opacity(0.08))
            .text_color(theme::foreground())
        })
        .child(gpui_component::Icon::new(IconName::Close).size(px(12.)))
        .on_click(cx.listener(move |this, _, _, cx| {
          cx.stop_propagation();
          this.close_tab(id, cx);
        }))
    });
    // The settings' tab has the gear rather than the service's logo.
    let icon = if space.settings_shown(tab.id, cx) {
      gpui_component::Icon::new(Icon::Settings)
        .size(px(16.))
        .into_any_element()
    } else {
      assets::color_svg(info.icon, 18., window, cx).into_any_element()
    };
    h_flex()
      .relative()
      .h_full()
      .min_w(px(TAB_MIN_WIDTH))
      .max_w(px(TAB_MAX_WIDTH))
      .flex_shrink_1()
      .pl_3()
      .pr_1p5()
      .gap_2()
      .items_center()
      .rounded(theme::RADIUS)
      .cursor_pointer()
      .text_sm()
      .text_color(if active {
        theme::foreground()
      } else {
        theme::muted()
      })
      .when(active, |el| {
        el.bg(gpui::white().opacity(0.06))
          .font_weight(FontWeight::SEMIBOLD)
          .child(
            div()
              .absolute()
              .bottom_0()
              .left(px(12.))
              .right(px(12.))
              .h(px(2.))
              .rounded_full()
              .bg(info.accent.base),
          )
      })
      .hover(|s| {
        s.bg(gpui::white().opacity(0.04))
          .text_color(theme::foreground())
      })
      .child(
        div()
          .flex_none()
          .size(px(18.))
          .flex()
          .items_center()
          .justify_center()
          .child(icon),
      )
      .child(div().flex_1().min_w_0().truncate().child(title))
      .children(close)
  }

  /// The new tab control of the setting `new_tab_style`.
  fn new_tab_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
    match Settings::get(cx).new_tab_style {
      NewTabStyle::Menu => self.new_tab_popover(cx).into_any_element(),
      NewTabStyle::Expanded => self.new_tab_frame(false, window, cx),
      NewTabStyle::OnHover => self.new_tab_frame(true, window, cx),
    }
  }

  /// A frame with "+" (not clickable) and the spaces' logos, unfolded while hovered when
  /// `folded`, else always: a click on one opens a tab of that space on its first page.
  fn new_tab_frame(
    &self,
    folded: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let hovered = motion::hover_state("new-tab-hover", window, cx);
    let open = !folded || *hovered.read(cx);
    let duration = if !folded || cx.reduce_motion() {
      Duration::ZERO
    } else {
      theme::FAST
    };
    let t = motion::tween(
      "new-tab-unfold",
      0.,
      if open { 1. } else { 0. },
      duration,
      window,
      cx,
    );
    let logos: Vec<_> = self
      .spaces
      .iter()
      .enumerate()
      .map(|(index, space)| {
        let info = space.info(cx);
        let name = info.name.clone();
        let tooltip = tf!("New {name} tab", "Nouvel onglet {name}");
        div()
          .id(SharedString::from(format!("new-tab-{index}")))
          .flex_none()
          .size(px(NEW_TAB_ITEM))
          .rounded(px(5.))
          .flex()
          .items_center()
          .justify_center()
          .cursor_pointer()
          .hover(|s| s.bg(gpui::white().opacity(0.08)))
          .child(assets::color_svg(info.icon, 18., window, cx))
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
          })
          .on_click(cx.listener(move |this, _, window, cx| this.new_tab(index, window, cx)))
      })
      .collect();
    let unfolded = self.spaces.len() as f32 * NEW_TAB_ITEM;
    h_flex()
      .id("new-tab")
      .flex_none()
      .ml_1()
      .p(px(1.))
      .items_center()
      .rounded(px(7.))
      .border_1()
      .border_color(theme::border())
      .when(folded && open, |el| el.bg(gpui::white().opacity(0.03)))
      .when(folded, |el| {
        el.on_hover(move |hover, window, cx| {
          hovered.update(cx, |h, _| *h = *hover);
          window.refresh();
        })
      })
      .child(
        div()
          .flex_none()
          .size(px(NEW_TAB_ITEM))
          .flex()
          .items_center()
          .justify_center()
          .text_color(theme::muted())
          .child(gpui_component::Icon::new(Icon::Plus).size(px(14.))),
      )
      // Clipped to a width growing with the unfolding, the logos fading in as they show.
      .child(
        h_flex()
          .flex_none()
          .w(px(unfolded * t))
          .overflow_hidden()
          .opacity(t)
          .children(logos),
      )
      .into_any_element()
  }

  /// "+" opening a menu of the spaces (`NewTabStyle::Menu`): a click on one opens a tab of it
  /// on its first page.
  fn new_tab_popover(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let shell = cx.weak_entity();
    let spaces: Vec<_> = self.spaces.iter().map(|space| space.info(cx)).collect();
    let trigger = Button::new("new-tab-button")
      .ghost()
      .icon(Icon::Plus)
      .tooltip(t("New tab", "Nouvel onglet"));
    div().flex_none().child(
      Popover::new("new-tab")
        .anchor(Anchor::TopLeft)
        .trigger(trigger)
        .content(move |_, window, cx| {
          let rows: Vec<_> = spaces
            .iter()
            .enumerate()
            .map(|(index, info)| {
              let shell = shell.clone();
              h_flex()
                .id(SharedString::from(format!("new-tab-{index}")))
                .w_full()
                .px_2()
                .py_1p5()
                .gap_3()
                .items_center()
                .rounded(theme::RADIUS)
                .cursor_pointer()
                .text_sm()
                .hover(|s| s.bg(theme::surface_hover()))
                .child(assets::color_svg(info.icon, 18., window, cx))
                .child(info.name.clone())
                .on_click(cx.listener(move |state, _, window, cx| {
                  state.dismiss(window, cx);
                  let shell = shell.clone();
                  let handle = window.window_handle();
                  // The popover is drawn in the window: open the tab once it is done.
                  cx.defer(move |cx| {
                    handle
                      .update(cx, |_, window, cx| {
                        shell
                          .update(cx, |shell, cx| shell.new_tab(index, window, cx))
                          .ok();
                      })
                      .ok();
                  });
                }))
            })
            .collect();
          v_flex()
            .w(px(NEW_TAB_MENU_WIDTH))
            .gap_1()
            .child(
              div()
                .px_2()
                .pb_1()
                .font_weight(FontWeight::SEMIBOLD)
                .child(t("New tab", "Nouvel onglet")),
            )
            .children(rows)
        }),
    )
  }

  /// The notifications of every space in one panel, a section per space that has some; the
  /// badge adds up their unread counts.
  fn bell(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let unseen: usize = self.spaces.iter().filter_map(|s| s.unseen(cx)).sum();
    let badge = (unseen > 0).then(|| {
      div()
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .min_w(px(16.))
        .h(px(16.))
        .px(px(4.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .child(if unseen > 9 {
          "9+".to_owned()
        } else {
          unseen.to_string()
        })
    });
    let spaces = self.spaces.clone();
    let trigger = Button::new("notifications-button")
      .ghost()
      .icon(Icon::Bell)
      .tooltip(t("Notifications", "Notifications"));
    div()
      .relative()
      .child(
        Popover::new("notifications")
          .anchor(Anchor::TopRight)
          .trigger(trigger)
          .content(move |_, window, cx| {
            let popover = cx.entity().downgrade();
            let sections: Vec<_> = spaces
              .iter()
              .filter_map(|space| {
                let panel = space.notifications(popover.clone(), window, cx)?;
                Some((space.info(cx), panel))
              })
              .collect();
            let empty = sections.is_empty();
            v_flex()
              .w(px(NOTIFICATIONS_WIDTH))
              .gap_2()
              .child(
                div()
                  .px_2()
                  .pb_1()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child(t("Notifications", "Notifications")),
              )
              .when(empty, |el| {
                el.child(
                  div()
                    .px_2()
                    .pb_2()
                    .text_sm()
                    .text_color(theme::muted())
                    .child(t(
                      "No notifications. YouTube's appear once signed in.",
                      "Aucune notification. Celles de YouTube s'affichent une fois connecté.",
                    )),
                )
              })
              .children(sections.into_iter().map(|(info, panel)| {
                v_flex()
                  .gap_1()
                  .child(
                    div()
                      .px_2()
                      .text_xs()
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(info.accent.base)
                      .child(info.name),
                  )
                  .child(panel)
              }))
          }),
      )
      .children(badge)
  }

  /// The audio output menu: the selected output's volume (Windows' own), then Windows'
  /// default and each output; the pick moves every player and is kept for the next runs.
  fn audio_output(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let shell = cx.weak_entity();
    let trigger = Button::new("audio-output-button")
      .ghost()
      .icon(Icon::Speaker)
      .tooltip(t("Audio output", "Sortie audio"));
    Popover::new("audio-output")
      .anchor(Anchor::TopRight)
      .trigger(trigger)
      .on_open_change({
        let shell = shell.clone();
        move |open, _, cx| {
          if *open {
            shell
              .update(cx, |shell, cx| {
                shell.list_audio_devices(cx);
                shell.read_output_volume(cx);
              })
              .ok();
          }
        }
      })
      .content(move |_, _, cx| {
        let devices = shell
          .upgrade()
          .and_then(|shell| shell.read(cx).audio_devices.clone());
        let current = Settings::get(cx).player.audio_device.clone();
        let volume = output_volume::slider_row(&shell, cx);
        let row = |id: SharedString,
                   device: Option<String>,
                   label: SharedString,
                   cx: &mut Context<PopoverState>| {
          let selected = device == current;
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
            .when(selected, |el| el.text_color(theme::accent()))
            .on_click(cx.listener(move |state, _, window, cx| {
              state.dismiss(window, cx);
              if !selected {
                choose_audio_device(device.clone(), cx);
              }
            }))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when(selected, |el| {
              el.child(
                gpui_component::Icon::new(Icon::Check)
                  .size(px(14.))
                  .flex_none(),
              )
            })
        };
        let default = row(
          "audio-device-default".into(),
          None,
          t("Windows default", "Par défaut de Windows").into(),
          cx,
        );
        let rows: Vec<_> = devices
          .iter()
          .flatten()
          .enumerate()
          .map(|(index, device)| {
            row(
              SharedString::from(format!("audio-device-{index}")),
              Some(device.name.clone()),
              device.description.clone().into(),
              cx,
            )
          })
          .collect();
        v_flex()
          .w(px(AUDIO_MENU_WIDTH))
          .gap_1()
          .child(
            div()
              .px_2()
              .pb_1()
              .font_weight(FontWeight::SEMIBOLD)
              .child(t("Audio output", "Sortie audio")),
          )
          .when_some(volume, |el, volume| {
            el.child(volume)
              .child(div().my_1().mx_2().h(px(1.)).bg(theme::border()))
          })
          .child(default)
          .children(rows)
          .when(devices.is_none(), |el| {
            el.child(
              div()
                .px_2()
                .py_1p5()
                .text_sm()
                .text_color(theme::muted())
                .child(t("Looking for outputs…", "Recherche des sorties…")),
            )
          })
      })
  }

  // --- Sidebar ------------------------------------------------------------------------------

  /// The open space's entries; folded, each shows its icon alone with its label as a tooltip.
  fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let collapsed = Settings::get(cx).sidebar_collapsed;
    let entries = self
      .current()
      .map(|(tab, s)| s.menu(tab, cx))
      .unwrap_or_default();
    let in_settings = self.in_settings(cx);
    let (bottom, top): (Vec<MenuEntry>, Vec<MenuEntry>) =
      entries.into_iter().partition(|e| e.bottom);
    let item = |entry: MenuEntry, window: &mut Window, cx: &mut Context<Self>| {
      let label = entry.label.clone();
      let id = entry.id.clone();
      let icon = match &entry.avatar {
        Some(user) => user_avatar(user, 24., window, cx),
        None => gpui_component::Icon::new(entry.icon)
          .size(px(18.))
          .flex_none()
          .into_any_element(),
      };
      let icon = if entry.live && collapsed {
        div()
          .relative()
          .flex_none()
          .child(icon)
          .child(
            div()
              .absolute()
              .bottom(px(-1.))
              .right(px(-1.))
              .size(px(9.))
              .rounded_full()
              .border_2()
              .border_color(theme::background())
              .bg(live_red()),
          )
          .into_any_element()
      } else {
        icon
      };
      let heading = entry.heading.clone().map(|heading| {
        if collapsed {
          div()
            .mx_3()
            .my_2()
            .h(px(1.))
            .bg(theme::border())
            .into_any_element()
        } else {
          div()
            .px_3()
            .pt_4()
            .pb_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::subtle())
            .child(heading.to_uppercase())
            .into_any_element()
        }
      });
      let row = sidebar_entry(entry.id.clone(), entry.active && !in_settings, collapsed)
        .when(entry.detail.is_some() && !collapsed, |el| el.h(px(48.)))
        .child(icon)
        .map(|el| match entry.hover.clone() {
          Some(card) => {
            let visible = match &self.card {
              Some((shown, visible)) if *shown == entry.id => Some(*visible),
              _ => None,
            };
            let hovered = entry.id.clone();
            el.on_hover(cx.listener(move |this, hovered_now: &bool, _, cx| {
              this.hover_entry(hovered.clone(), *hovered_now, cx)
            }))
            .children(visible.and_then(|visible| {
              hover_card(
                &entry.id,
                label.clone(),
                entry.avatar.as_ref(),
                card,
                visible,
                window,
                cx,
              )
            }))
          }
          None if collapsed => el.tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
          }),
          None => el,
        })
        .map(|el| {
          if collapsed {
            el
          } else {
            el.child(
              v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(entry.label))
                .children(entry.detail.map(|detail| {
                  div()
                    .truncate()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child(detail)
                })),
            )
            .when(entry.live, |el| el.child(live_pill()))
          }
        })
        .on_aux_click(cx.listener({
          let id = id.clone();
          move |this, event: &gpui::ClickEvent, _, cx| {
            if event.is_middle_click() {
              this.open_entry_in_new_tab(id.clone(), cx)
            }
          }
        }))
        .on_click(cx.listener(move |this, _, window, cx| {
          // A click puts the card away, as GPUI does a tooltip.
          this.card_delay = Task::ready(());
          this.card = None;
          this.open_entry(id.clone(), window, cx)
        }));
      v_flex().children(heading).child(row)
    };
    let top: Vec<_> = top.into_iter().map(|e| item(e, window, cx)).collect();
    let bottom: Vec<_> = bottom.into_iter().map(|e| item(e, window, cx)).collect();
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
          .children(top),
      )
      .when(!bottom.is_empty(), |el| {
        el.child(
          v_flex()
            .flex_none()
            .px_2()
            .py_2()
            .gap_1()
            .border_t_1()
            .border_color(theme::border())
            .children(bottom),
        )
      })
  }
}

/// A sidebar entry: icon and label, muted until hovered or active; the active one has a light
/// background and a pill in the accent on its left edge. `collapsed`: the icon alone, centered.
fn sidebar_entry(id: SharedString, active: bool, collapsed: bool) -> gpui::Stateful<gpui::Div> {
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

/// The red of a live, whatever the space's accent.
fn live_red() -> gpui::Hsla {
  theme::Accent::red().base
}

/// "LIVE" with a white dot in red, at the right of a sidebar entry on air.
fn live_pill() -> impl IntoElement {
  h_flex()
    .flex_none()
    .gap_1()
    .items_center()
    .px(px(5.))
    .py(px(1.))
    .rounded(px(4.))
    .bg(live_red())
    .text_size(px(10.))
    .font_weight(FontWeight::BOLD)
    .text_color(gpui::white())
    .child(div().size(px(5.)).rounded_full().bg(gpui::white()))
    .child("LIVE")
}

/// A sidebar entry's [`HoverCard`] at its right, over the page: the avatar, larger, beside the
/// entry's label, the card's title and its detail. It fades in sliding toward the entry while
/// `visible`, and out sliding away; `None` once it has faded out.
fn hover_card(
  id: &SharedString,
  label: SharedString,
  avatar: Option<&Avatar>,
  card: HoverCard,
  visible: bool,
  window: &mut Window,
  cx: &mut App,
) -> Option<gpui::AnyElement> {
  let duration = if cx.reduce_motion() {
    Duration::ZERO
  } else {
    theme::FAST
  };
  let target = if visible { 1. } else { 0. };
  let t = motion::tween(
    SharedString::from(format!("hover-card-{id}")),
    0.,
    target,
    duration,
    window,
    cx,
  );
  if !visible && t <= 0. {
    return None;
  }
  let content = h_flex()
    .ml(px(10. + 6. * (1. - t)))
    .w(px(280.))
    .p_3()
    .gap_3()
    .items_start()
    .rounded(px(8.))
    .border_1()
    .border_color(theme::border())
    .bg(theme::surface())
    .shadow_md()
    .text_sm()
    .text_color(theme::foreground())
    .opacity(t)
    .children(avatar.map(|user| user_avatar(user, 48., window, cx)))
    .child(
      v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(
          div()
            .truncate()
            .font_weight(FontWeight::SEMIBOLD)
            .child(label),
        )
        .child(
          div()
            .line_clamp(3)
            .text_color(theme::muted())
            .child(card.title),
        )
        .children(card.detail.map(|detail| {
          div()
            .truncate()
            .text_xs()
            .text_color(theme::accent())
            .child(detail)
        })),
    );
  // A point at the entry's top right corner; deferred, the card escapes the sidebar's clip.
  Some(
    div()
      .absolute()
      .top_0()
      .right_0()
      .child(
        deferred(anchored().snap_to_window_with_margin(px(8.)).child(content)).with_priority(1),
      )
      .into_any_element(),
  )
}

/// A user's avatar in the sidebar, the size of an entry's icon plus a little (24 px): their picture once
/// loaded, a disc in the accent with their initial meanwhile or without one. Scaled on the CPU
/// to its exact size: servers do not resize user pictures, and GPUI samples without mipmaps.
fn user_avatar(user: &Avatar, size: f32, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
  let side = (size * window.scale_factor()).round() as u32;
  let ready = user
    .picture
    .as_ref()
    .and_then(|picture| picture.url(side, side))
    .and_then(|url| {
      let images = cx.try_global::<Images>()?.0.clone();
      images.update(cx, |store, cx| store.get_sized(&url, side, side, cx))
    });
  let disc = div()
    .size(px(size))
    .flex_none()
    .rounded_full()
    .flex()
    .items_center()
    .justify_center();
  match ready {
    Some(image) => disc
      .child(
        img(ImageSource::Render(image))
          .size_full()
          .rounded_full()
          .object_fit(ObjectFit::Cover),
      )
      .into_any_element(),
    None => disc
      .bg(theme::accent())
      .text_color(theme::on_accent())
      .text_xs()
      .font_weight(FontWeight::BOLD)
      .child(user.initial.clone())
      .into_any_element(),
  }
}

/// Peeby's mark: the logo and the name.
fn mark(window: &Window, cx: &mut App) -> impl IntoElement {
  h_flex()
    .gap_2()
    .items_center()
    .child(assets::logo(22., window, cx))
    .child(div().text_sm().font_weight(FontWeight::BOLD).child("Peeby"))
}

impl Render for Shell {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Over everything while it plays; a click skips it.
    let splash = self.splash.as_mut().and_then(|s| s.render(window));
    if splash.is_none() {
      self.splash = None;
    }
    let splash = splash.map(|frame| {
      div()
        .id("splash")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
          MouseButton::Left,
          cx.listener(|this, _, _, cx| {
            if let Some(splash) = &mut this.splash {
              splash.skip();
            }
            cx.notify();
          }),
        )
        .child(frame)
    });
    // The mouse's side buttons go back and forward, as in a browser; not in fullscreen, which
    // belongs to the player (there would be no bar to come back from another page).
    let root = v_flex()
      .id("shell")
      .relative()
      .size_full()
      .on_mouse_down(
        MouseButton::Navigate(NavigationDirection::Back),
        cx.listener(|this, _, window, cx| {
          if !window.is_fullscreen() {
            this.back(window, cx);
          }
        }),
      )
      .on_mouse_down(
        MouseButton::Navigate(NavigationDirection::Forward),
        cx.listener(|this, _, window, cx| {
          if !window.is_fullscreen() {
            this.forward(window, cx);
          }
        }),
      )
      .bg(theme::chrome(cx))
      .text_color(theme::foreground());
    // The settings page is made each time it shows (its cache size measured again).
    let page = if self.in_settings(cx) {
      self.settings_view(cx)
    } else {
      self.settings = None;
      self.current().map(|(_, s)| s.view())
    };
    // Fullscreen belongs to the player: no top bar, no sidebar.
    if window.is_fullscreen() {
      return root.children(page).children(splash);
    }
    root
      .child(self.top_bar(window, cx))
      .child(
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
              .border_t_1()
              .border_l_1()
              .border_color(theme::border())
              .bg(theme::background())
              .overflow_hidden()
              .children(page),
          ),
      )
      .children(self.detached_copy(window, cx))
      .children(splash)
  }
}

impl Shell {
  /// A tab pulled off the bar: its copy under the pointer, over the whole window, a little
  /// transparent.
  fn detached_copy(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
    let drag = self.tab_drag.filter(|d| d.moving && d.detached)?;
    let tab = self.find(drag.tab)?;
    let corner = Self::detached_corner(&drag);
    Some(
      div()
        .absolute()
        .left(corner.x)
        .top(corner.y)
        .w(drag.width)
        .h(px(34.))
        .opacity(0.9)
        .rounded(theme::RADIUS)
        .bg(theme::chrome(cx))
        .shadow_lg()
        .child(
          self
            .tab_face(tab, window, cx)
            .w_full()
            .max_w_full()
            .min_w_0(),
        ),
    )
  }
}

/// Plays to `device` (`None`: Windows' default) from now on, in every player.
fn choose_audio_device(device: Option<String>, cx: &mut App) {
  Settings::update(cx, |s| s.player.audio_device = device.clone());
  cx.set_global(AudioOutput(device));
}

/// The green button of the title bar while an update's installer is ready (ADR 0027): a click
/// quits Peeby, installs the update and starts it again. While the installer downloads, and
/// while it starts, the button shows greyed out with the app's loader and takes no click.
fn update_button(cx: &App) -> Option<Button> {
  use crate::update::State;
  let (version, busy) = match crate::update::state(cx) {
    State::Ready(version) => (version, None),
    State::Downloading(version) => (
      version,
      Some(tf!(
        "Downloading Peeby {version}…",
        "Téléchargement de Peeby {version}…"
      )),
    ),
    State::Installing(version) => (
      version,
      Some(tf!(
        "Starting the installer of Peeby {version}…",
        "Lancement de l'installation de Peeby {version}…"
      )),
    ),
    _ => return None,
  };
  if let Some(tooltip) = busy {
    // Desaturated rather than the disabled style, which would grey it out entirely; `loading`
    // makes it inert and dims it a little more.
    let mut color = theme::update();
    color.s *= 0.35;
    return Some(
      Button::new("update")
        .custom(
          ButtonCustomVariant::new(cx)
            .color(color)
            .foreground(theme::on_accent())
            .hover(color)
            .active(color),
        )
        .small()
        .loading(true)
        .child(ui::widgets::loader(14., theme::on_accent()))
        .child(t("Update", "Mettre à jour"))
        .tooltip(tooltip),
    );
  }
  Some(
    Button::new("update")
      .custom(
        ButtonCustomVariant::new(cx)
          .color(theme::update())
          .foreground(theme::on_accent())
          .hover(theme::update_hover())
          .active(theme::update_active()),
      )
      .small()
      .icon(Icon::Download)
      .label(t("Update", "Mettre à jour"))
      .tooltip(tf!(
        "Restart to install Peeby {version}",
        "Redémarrer pour installer Peeby {version}"
      ))
      .on_click(|_, window, cx| {
        // Out of the click's event: the update quits the app (CLAUDE.md, never from inside
        // the window's own event).
        let handle = window.window_handle();
        cx.defer(move |cx| {
          handle
            .update(cx, |_, window, cx| crate::update::install(window, cx))
            .ok();
        });
      }),
  )
}
