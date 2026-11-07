//! Who a user card is about ([`UserQuery`]) and what the space finds about them
//! ([`UserProfile`]): the panel knows a chat's authors from their messages, the space knows how
//! to find the rest (a picture, a channel) from its service.

use std::rc::Rc;

use gpui::{App, SharedString, Task};

use crate::picture::Picture;

/// Who a card is about: an author of the chat (a click on their name), or a name mentioned
/// (`@name`), filled with what the chat's messages tell about them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserQuery {
  /// Their user id ([`super::Author::id`]), once they wrote in the chat.
  pub id: Option<SharedString>,
  /// Their name as shown or as mentioned, `@` included when it was written.
  pub name: SharedString,
  /// Their channel's key ([`super::Author::channel`]), when known.
  pub channel: Option<SharedString>,
}

/// What a space found about a user.
#[derive(Clone)]
pub struct UserProfile {
  /// The name their service shows.
  pub name: SharedString,
  /// Their picture.
  pub avatar: Option<Picture>,
  /// What opens their channel ([`super::ChatPanelEvent::OpenChannel`]).
  pub channel: Option<SharedString>,
}

/// How a space finds what the chat does not tell about a user (their picture, their channel):
/// called when a card opens without them; `None` when nobody is found.
pub type UserLookup = Rc<dyn Fn(UserQuery, &mut App) -> Task<Option<UserProfile>>>;

/// Whether `a` and `b` name the same user: case aside, with or without their `@`.
pub fn same_user(a: &str, b: &str) -> bool {
  let bare = |s: &str| s.trim_start_matches('@').to_lowercase();
  !a.trim_start_matches('@').is_empty() && bare(a) == bare(b)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_match_whatever_their_case_and_at() {
    assert!(same_user("@Viewer1", "viewer1"));
    assert!(same_user("viewer1", "VIEWER1"));
    assert!(!same_user("viewer1", "viewer2"));
    assert!(!same_user("@", "@"));
  }
}
