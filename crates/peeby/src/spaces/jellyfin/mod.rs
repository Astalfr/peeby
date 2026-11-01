//! The Jellyfin space. For now a placeholder, so the tabs and the mini-player can be used; the
//! pages of the former peeby come with step 6 of docs/refactor-plan.md.

use gpui::prelude::*;
use gpui::{App, Context, EventEmitter, Window, div, rgb};
use ui::assets::Icon;
use ui::theme::Accent;
use ui::widgets::notice;

use crate::shell::space::{MenuEntry, Space, SpaceEvent, SpaceInfo};

/// The Jellyfin space.
pub struct JellyfinSpace;

impl JellyfinSpace {
  /// Peeby's amber.
  pub fn accent() -> Accent {
    Accent {
      base: rgb(0xff8a3d).into(),
      hover: rgb(0xff9f5e).into(),
      active: rgb(0xe8752a).into(),
    }
  }
}

impl EventEmitter<SpaceEvent> for JellyfinSpace {}

impl Space for JellyfinSpace {
  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "Jellyfin".into(),
      badge: "JF".into(),
      accent: Self::accent(),
    }
  }

  fn menu(&self, _cx: &App) -> Vec<MenuEntry> {
    vec![MenuEntry {
      id: "home".into(),
      label: "Accueil".into(),
      icon: Icon::Home,
      active: true,
      bottom: false,
    }]
  }

  fn open_entry(&mut self, _id: &str, _window: &mut Window, _cx: &mut Context<Self>) {}

  fn can_go_back(&self) -> bool {
    false
  }

  fn back(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for JellyfinSpace {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    div().size_full().p_8().child(notice(
      Icon::Home,
      "Jellyfin arrive bientôt",
      "Les bibliothèques, la reprise de lecture et le lecteur de peeby rejoindront cet onglet.",
    ))
  }
}
