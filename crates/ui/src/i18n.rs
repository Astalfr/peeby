//! The interface's language (ADR 0014). Every UI string is written at its call site in each
//! language, English first: `t("Settings", "Réglages")`, or `tf!("{n} videos", "{n} vidéos")`
//! for a formatted one. The language is a process-wide value, readable without a GPUI context
//! (builders such as `format_size` have none); changing it re-renders the windows, and texts
//! kept in a page's state follow on its next load.
//!
//! Only Peeby's own texts go through here: what a service sends (titles, YouTube's "il y a
//! 3 jours") keeps its own language, set by the space's settings. Logs stay in English.

use std::sync::atomic::{AtomicU8, Ordering};

/// A language of the interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
  /// English, the default.
  #[default]
  En,
  /// French.
  Fr,
}

impl Lang {
  /// Every language, in the order the settings offer them.
  pub const ALL: [Lang; 2] = [Lang::En, Lang::Fr];

  /// Its code in `settings.toml` (`en`, `fr`).
  pub fn code(self) -> &'static str {
    match self {
      Lang::En => "en",
      Lang::Fr => "fr",
    }
  }

  /// The language of `code`; English for an unknown one.
  pub fn from_code(code: &str) -> Self {
    Self::ALL
      .into_iter()
      .find(|lang| lang.code().eq_ignore_ascii_case(code))
      .unwrap_or_default()
  }

  /// Its name in itself, as the language picker shows it.
  pub fn name(self) -> &'static str {
    match self {
      Lang::En => "English",
      Lang::Fr => "Français",
    }
  }

  /// `en` or `fr`, whichever is this language's.
  pub fn pick<T>(self, en: T, fr: T) -> T {
    match self {
      Lang::En => en,
      Lang::Fr => fr,
    }
  }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The interface's language.
pub fn lang() -> Lang {
  match CURRENT.load(Ordering::Relaxed) {
    1 => Lang::Fr,
    _ => Lang::En,
  }
}

/// Changes the interface's language. The caller re-renders the windows
/// (`App::refresh_windows`). Tests never call it: they run in parallel and read the default.
pub fn set(lang: Lang) {
  let value = match lang {
    Lang::En => 0,
    Lang::Fr => 1,
  };
  CURRENT.store(value, Ordering::Relaxed);
}

/// `en` or `fr`, in the interface's language.
pub fn t(en: &'static str, fr: &'static str) -> &'static str {
  lang().pick(en, fr)
}

/// `format!` in the interface's language: `tf!("{n} videos", "{n} vidéos")`. Both format
/// strings take the same arguments (captured names or trailing expressions).
#[macro_export]
macro_rules! tf {
  ($en:literal, $fr:literal $(, $arg:expr)* $(,)?) => {
    match $crate::i18n::lang() {
      $crate::i18n::Lang::En => ::std::format!($en $(, $arg)*),
      $crate::i18n::Lang::Fr => ::std::format!($fr $(, $arg)*),
    }
  };
}

#[cfg(test)]
mod tests {
  use super::Lang;

  #[test]
  fn codes_round_trip() {
    for lang in Lang::ALL {
      assert_eq!(Lang::from_code(lang.code()), lang);
    }
    assert_eq!(Lang::from_code("FR"), Lang::Fr);
    assert_eq!(Lang::from_code("xx"), Lang::En);
  }

  #[test]
  fn defaults_to_english() {
    assert_eq!(super::lang(), Lang::En);
    assert_eq!(super::t("Settings", "Réglages"), "Settings");
    let n = 3;
    assert_eq!(tf!("{n} videos", "{n} vidéos"), "3 videos");
  }
}
