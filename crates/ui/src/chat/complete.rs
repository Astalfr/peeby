//! Completion in the chat's field: the word before the cursor and what it may become. `@` and a
//! letter offer the chatters seen (most recent first), `:` and a letter the emotes whose code
//! holds them (those starting so first); Tab on any other word offers both, by their start, the
//! emotes first. No service here: the panel gives the emotes it knows and the authors it showed.

use std::ops::Range;

use gpui::SharedString;

/// Suggestions shown at most.
pub(crate) const MAX: usize = 8;

/// What a word may become.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Candidate {
  /// An emote, by its code.
  Emote(SharedString),
  /// A chatter, by the name shown.
  User(SharedString),
}

impl Candidate {
  /// What replaces the word: the emote's code, or `@name`.
  pub(crate) fn text(&self) -> String {
    match self {
      Self::Emote(code) => code.to_string(),
      Self::User(name) => format!("@{name}"),
    }
  }
}

/// The word that ends at `cursor` (a byte offset in `text`): from the last whitespace before it.
/// `None` when the cursor follows a space or starts the text.
pub(crate) fn word_before(text: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
  let before = text.get(..cursor)?;
  let start = before
    .char_indices()
    .rev()
    .find(|(_, c)| c.is_whitespace())
    .map_or(0, |(i, c)| i + c.len_utf8());
  (start < cursor).then(|| (start..cursor, &text[start..cursor]))
}

/// Whether typing `word` opens the suggestions by itself: `@` or `:` and a letter.
pub(crate) fn opens_by_itself(word: &str) -> bool {
  match word.chars().next() {
    Some('@') => word.chars().count() >= 2,
    Some(':') => word.chars().count() >= 2,
    _ => false,
  }
}

/// What `word` may become, [`MAX`] at most: for `@x` the `users` (in their order, the most
/// recent first) whose name starts with `x`; for `:x` the `emotes` whose code holds `x`, those
/// starting with it first; for a plain word the emotes then the users starting with it. Case is
/// ignored; a candidate equal to the word is left out.
pub(crate) fn candidates<'a>(
  word: &str,
  emotes: impl IntoIterator<Item = &'a SharedString>,
  users: &[SharedString],
) -> Vec<Candidate> {
  let lower = |s: &str| s.to_lowercase();
  let users_starting = |prefix: &str| -> Vec<Candidate> {
    let prefix = lower(prefix);
    users
      .iter()
      .filter(|name| lower(name).starts_with(&prefix))
      .map(|name| Candidate::User(name.clone()))
      .collect()
  };
  let mut emotes: Vec<&SharedString> = emotes.into_iter().collect();
  emotes.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
  let mut out: Vec<Candidate> = if let Some(name) = word.strip_prefix('@') {
    users_starting(name)
  } else if let Some(part) = word.strip_prefix(':') {
    let part = lower(part);
    let (mut starting, mut holding) = (Vec::new(), Vec::new());
    for code in emotes {
      let code_lower = lower(code);
      if code_lower.starts_with(&part) {
        starting.push(Candidate::Emote(code.clone()));
      } else if code_lower.contains(&part) {
        holding.push(Candidate::Emote(code.clone()));
      }
    }
    starting.append(&mut holding);
    starting
  } else {
    let prefix = lower(word);
    let mut both: Vec<Candidate> = emotes
      .into_iter()
      .filter(|code| lower(code).starts_with(&prefix))
      .map(|code| Candidate::Emote(code.clone()))
      .collect();
    both.extend(users_starting(word));
    both
  };
  out.retain(|c| c.text() != word);
  out.truncate(MAX);
  out
}

#[cfg(test)]
mod tests {
  use gpui::SharedString;

  use super::{Candidate, candidates, opens_by_itself, word_before};

  fn s(text: &str) -> SharedString {
    text.to_owned().into()
  }

  #[test]
  fn the_word_before_the_cursor() {
    assert_eq!(word_before("hello @vie", 10), Some((6..10, "@vie")));
    assert_eq!(word_before("hello", 3), Some((0..3, "hel")));
    assert_eq!(word_before("hello ", 6), None);
    assert_eq!(word_before("", 0), None);
    // Multi-byte letters count as bytes.
    assert_eq!(word_before("é :KEK", 7), Some((3..7, ":KEK")));
  }

  #[test]
  fn at_and_colon_open_the_list() {
    assert!(opens_by_itself("@v"));
    assert!(!opens_by_itself("@"));
    assert!(opens_by_itself(":k"));
    assert!(!opens_by_itself(":"));
    assert!(!opens_by_itself("kek"));
  }

  #[test]
  fn users_by_their_start_emotes_by_what_they_hold() {
    let emotes = [s("KEKW"), s("OMEGALUL"), s("LUL"), s("monkaS")];
    let users = [s("viewer2"), s("Viewer1"), s("other")];
    assert_eq!(
      candidates("@vi", &emotes, &users),
      [Candidate::User(s("viewer2")), Candidate::User(s("Viewer1"))]
    );
    // Starting with it first (shortest first), then holding it.
    assert_eq!(
      candidates(":lu", &emotes, &users),
      [Candidate::Emote(s("LUL")), Candidate::Emote(s("OMEGALUL"))]
    );
    // A plain word: emotes then users, by their start.
    assert_eq!(
      candidates("o", &emotes, &users),
      [Candidate::Emote(s("OMEGALUL")), Candidate::User(s("other"))]
    );
    // What is typed already is not offered.
    assert_eq!(candidates("LUL", &emotes, &users), []);
  }
}
