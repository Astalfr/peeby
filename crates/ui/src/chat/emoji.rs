//! The emoji the chat's picker offers: a short list of common ones by theme, drawn as text (the
//! system's color emoji font). No data crate: a full Unicode list is not worth a dependency here.

/// A theme of emoji: its title (English, French) and the emoji.
pub struct EmojiGroup {
  pub title: (&'static str, &'static str),
  pub emoji: &'static [&'static str],
}

/// Every group, in the picker's order.
pub const GROUPS: &[EmojiGroup] = &[
  EmojiGroup {
    title: ("Smileys", "Smileys"),
    emoji: &[
      "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😂", "🙂", "🙃", "😉", "😊", "😇", "🥰", "😍",
      "🤩", "😘", "😋", "😛", "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫", "🤔", "🤐", "🤨", "😐",
      "😑", "😶", "😏", "😒", "🙄", "😬", "😌", "😔", "😪", "😴", "😷", "🤒", "🤕", "🤢", "🤮",
      "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "😎", "🤓", "🧐", "😕", "😟", "🙁", "😮", "😯",
      "😲", "😳", "🥺", "😦", "😧", "😨", "😰", "😥", "😢", "😭", "😱", "😖", "😣", "😞", "😓",
      "😩", "😫", "🥱", "😤", "😡", "😠", "🤬", "😈", "👿", "💀", "💩", "🤡", "👻", "👽", "🤖",
    ],
  },
  EmojiGroup {
    title: ("Gestures", "Gestes"),
    emoji: &[
      "👋", "🤚", "✋", "🖖", "👌", "🤌", "🤏", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆",
      "👇", "☝️", "👍", "👎", "✊", "👊", "🤛", "🤜", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "💪",
      "👀", "🧠", "🫡", "🫠", "🫣", "🫶",
    ],
  },
  EmojiGroup {
    title: ("Hearts and symbols", "Cœurs et symboles"),
    emoji: &[
      "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "💯", "💢", "💥", "💫", "💦",
      "💨", "💬", "💤", "🔥", "✨", "⭐", "🌟", "⚡", "✅", "❌", "❓", "❗", "⚠️", "🚫", "🆗",
    ],
  },
  EmojiGroup {
    title: ("Things", "Objets"),
    emoji: &[
      "🎉", "🎊", "🎁", "🏆", "🥇", "🎮", "🕹️", "🎲", "🎯", "🎵", "🎶", "🎤", "🎧", "📺", "📷",
      "💻", "⌨️", "🖱️", "📱", "⏰", "💰", "💎", "🔔", "📌", "🚀", "🍕", "🍔", "🍟", "🍿", "☕",
      "🍺", "🍷", "🐶", "🐱", "🐸", "🐍", "🦀", "🐟", "🌈", "🌙",
    ],
  },
];
