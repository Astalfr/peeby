use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
  ($(#[$doc:meta])* $name:ident) => {
    $(#[$doc])*
    #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct $name(pub String);

    impl $name {
      /// The raw id as sent by the server (32 hex digits, sometimes dashed).
      pub fn as_str(&self) -> &str {
        &self.0
      }
    }

    impl fmt::Display for $name {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
      }
    }

    impl From<&str> for $name {
      fn from(value: &str) -> Self {
        Self(value.to_owned())
      }
    }
  };
}

id_type!(
  /// Id of a library item (movie, series, episode, folder, view…).
  ItemId
);
id_type!(
  /// Id of a Jellyfin user.
  UserId
);
