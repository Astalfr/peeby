//! Creating a clip from the watch page: what to clip, and what the viewer is told (toasts and
//! error messages). The flow itself, which needs the page's state, is [`super::watch::WatchView`]'s.

use gpui::prelude::*;
use gpui::{ClipboardItem, SharedString};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::{Sizable, h_flex};
use ui::i18n::t;
use ui::tf;

use super::Nav;
use super::Route;
use super::watch::Kind;

/// Length of the clips Peeby asks for, seconds (Helix's default for a live; 5 to 60 for a VOD).
pub const CLIP_SECONDS: u32 = 30;

/// What a clip is made from.
#[derive(Debug, PartialEq, Eq)]
pub enum ClipTarget {
  /// The live's last seconds (`POST /clips`).
  Live,
  /// A VOD, the clip ending at `position` seconds (`POST /videos/clips`).
  Vod { id: String, position: u64 },
}

/// What the page clips now, or `None` for a page that cannot (a clip, or a live played back in
/// its recording when the recording is not known).
/// `rewound`: the live plays from its recording, `position` is then in the recording.
pub fn target(
  kind: &Kind,
  rewound: bool,
  position: f64,
  archive: Option<&str>,
) -> Option<ClipTarget> {
  // `as` saturates: a negative or NaN position is 0.
  let position = position as u64;
  match kind {
    Kind::Live(_) if !rewound => Some(ClipTarget::Live),
    Kind::Live(_) => archive.map(|id| ClipTarget::Vod {
      id: id.to_owned(),
      position,
    }),
    Kind::Vod { id, .. } => Some(ClipTarget::Vod {
      id: id.clone(),
      position,
    }),
    Kind::Clip(_) => None,
  }
}

/// The end of a VOD clip for a playhead at `position`: Helix wants it at least as late as the
/// clip is long.
pub fn vod_offset(position: u64) -> u64 {
  position.max(u64::from(CLIP_SECONDS))
}

/// Why a clip could not be made, for the viewer.
pub fn error_text(error: &twitch::Error) -> SharedString {
  use twitch::Error;
  match error {
    Error::Offline => t(
      "The live is over: there is nothing to clip.",
      "Le live est terminé : il n'y a rien à clipper.",
    )
    .into(),
    Error::Helix { status: 403, .. } => t(
      "Twitch refuses this clip: the channel turned clips off, or keeps them for its followers or subscribers.",
      "Twitch refuse ce clip : la chaîne a désactivé les clips, ou les réserve à ses followers ou abonnés.",
    )
    .into(),
    Error::Helix { message, .. } => {
      let message = message.clone();
      tf!("Twitch refused the clip: {message}", "Twitch a refusé le clip : {message}").into()
    }
    Error::RateLimited => t(
      "Too many requests to Twitch: try again in a moment.",
      "Trop de requêtes vers Twitch : réessayez dans un instant.",
    )
    .into(),
    Error::ClipNotReady => t(
      "Twitch is still making the clip: look for it in your clips in a moment.",
      "Twitch fabrique encore le clip : cherchez-le dans vos clips dans un instant.",
    )
    .into(),
    Error::Http(_) => t(
      "Twitch could not be reached: the clip was not made.",
      "Twitch est injoignable : le clip n'a pas été créé.",
    )
    .into(),
    _ => t(
      "The clip could not be created.",
      "Le clip n'a pas pu être créé.",
    )
    .into(),
  }
}

/// A plain error toast.
pub fn error_toast(text: impl Into<SharedString>) -> Notification {
  Notification::error(text)
}

/// The clip exists: its link is on the clipboard (done by the caller), and the viewer can open it
/// in a Peeby tab or edit it on twitch.tv.
pub fn created_toast(nav: Nav, slug: String, title: String, edit_url: String) -> Notification {
  Notification::success(t(
    "Clip created, its link is copied.",
    "Clip créé, son lien est copié.",
  ))
  .content(move |_, _, cx| {
    let note = cx.entity();
    let (open_nav, slug, title) = (nav.clone(), slug.clone(), title.clone());
    let open_note = note.clone();
    let edit_url = edit_url.clone();
    h_flex()
      .gap_2()
      .mt_2()
      .child(
        Button::new("clip-open")
          .small()
          .primary()
          .label(t("Open", "Ouvrir"))
          .on_click(move |_, window, cx| {
            open_nav.open_in_new_tab(Route::Clip(slug.clone()), Some(title.clone().into()), cx);
            open_note.update(cx, |note, cx| note.dismiss(window, cx));
          }),
      )
      .child(
        Button::new("clip-edit")
          .small()
          .label(t("Edit", "Éditer"))
          .on_click(move |_, window, cx| {
            cx.open_url(&edit_url);
            note.update(cx, |note, cx| note.dismiss(window, cx));
          }),
      )
      .into_any_element()
  })
}

/// The Helix session predates clips: offers the code approval again.
pub fn reconnect_toast(nav: Nav) -> Notification {
  Notification::warning(t(
    "Creating clips needs a new approval from Twitch.",
    "Créer des clips demande une nouvelle approbation de Twitch.",
  ))
  .action(move |_, _, _| {
    let nav = nav.clone();
    Button::new("clip-reconnect")
      .label(t("Reconnect", "Reconnecter"))
      .on_click(move |_, _, cx| {
        let tab = nav.tab;
        nav
          .update(cx, |space, cx| space.reconnect_helix(tab, cx))
          .ok();
      })
  })
}

/// Twitch reserves clips of a video to the channel and its editors: the page opened it on
/// twitch.tv instead.
pub fn editor_only_toast() -> Notification {
  Notification::info(t(
    "Only the channel and its editors can clip a video: opened on twitch.tv.",
    "Seuls la chaîne et ses éditeurs peuvent clipper une vidéo : ouverte sur twitch.tv.",
  ))
}

/// Puts a clip's public link on the clipboard.
pub fn copy_link(slug: &str, cx: &mut gpui::App) {
  cx.write_to_clipboard(ClipboardItem::new_string(twitch::helix::clip_url(slug)));
}

#[cfg(test)]
mod tests {
  use super::*;

  fn vod() -> Kind {
    Kind::Vod {
      id: "42".into(),
      start: None,
    }
  }

  #[test]
  fn a_live_at_its_edge_is_clipped_through_helix() {
    let kind = Kind::Live("chan".into());
    assert_eq!(
      target(&kind, false, 3600.0, Some("7")),
      Some(ClipTarget::Live)
    );
  }

  #[test]
  fn a_live_in_its_recording_is_clipped_as_that_vod() {
    let kind = Kind::Live("chan".into());
    assert_eq!(
      target(&kind, true, 125.9, Some("7")),
      Some(ClipTarget::Vod {
        id: "7".into(),
        position: 125
      })
    );
    assert_eq!(target(&kind, true, 125.9, None), None);
  }

  #[test]
  fn a_vod_is_clipped_at_the_playhead() {
    assert_eq!(
      target(&vod(), false, 10.0, None),
      Some(ClipTarget::Vod {
        id: "42".into(),
        position: 10
      })
    );
    assert_eq!(target(&Kind::Clip("slug".into()), false, 1.0, None), None);
  }

  #[test]
  fn errors_tell_the_viewer_what_happened() {
    use twitch::Error;
    let text = |e: Error| error_text(&e).to_string();
    assert!(text(Error::Offline).contains("live is over"));
    assert!(text(Error::RateLimited).contains("Too many requests"));
    assert!(text(Error::ClipNotReady).contains("still making"));
    assert!(
      text(Error::Helix {
        status: 403,
        message: "Clips are disabled".into()
      })
      .contains("turned clips off")
    );
    // Any other Helix refusal says what Twitch said.
    assert!(
      text(Error::Helix {
        status: 400,
        message: "title too long".into()
      })
      .contains("title too long")
    );
    assert!(text(Error::SessionExpired).contains("could not be created"));
  }

  #[test]
  fn a_vod_clip_ends_at_least_its_length_in() {
    assert_eq!(vod_offset(4), 30);
    assert_eq!(vod_offset(500), 500);
  }
}
