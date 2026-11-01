//! Text helpers shared by the spaces.

/// Undoes UTF-8 text that was decoded as Windows-1252 somewhere upstream (`Ã©` for `é`), as in
/// the chapter titles of some releases. Text that does not round-trip is returned unchanged.
pub fn repair_mojibake(text: &str) -> std::borrow::Cow<'_, str> {
  const CP1252: [(char, u8); 27] = [
    ('€', 0x80),
    ('‚', 0x82),
    ('ƒ', 0x83),
    ('„', 0x84),
    ('…', 0x85),
    ('†', 0x86),
    ('‡', 0x87),
    ('ˆ', 0x88),
    ('‰', 0x89),
    ('Š', 0x8a),
    ('‹', 0x8b),
    ('Œ', 0x8c),
    ('Ž', 0x8e),
    ('‘', 0x91),
    ('’', 0x92),
    ('“', 0x93),
    ('”', 0x94),
    ('•', 0x95),
    ('–', 0x96),
    ('—', 0x97),
    ('˜', 0x98),
    ('™', 0x99),
    ('š', 0x9a),
    ('›', 0x9b),
    ('œ', 0x9c),
    ('ž', 0x9e),
    ('Ÿ', 0x9f),
  ];
  if text.is_ascii() {
    return text.into();
  }
  let bytes: Option<Vec<u8>> = text
    .chars()
    .map(|c| match u8::try_from(u32::from(c)) {
      Ok(byte) => Some(byte),
      Err(_) => CP1252.iter().find(|(ch, _)| *ch == c).map(|(_, b)| *b),
    })
    .collect();
  match bytes.map(String::from_utf8) {
    Some(Ok(repaired)) => repaired.into(),
    _ => text.into(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mojibake_is_repaired_and_good_text_kept() {
    let broken: String = "Witch Hunt 「魔女探し」"
      .bytes()
      .map(|b| {
        // Encode as Windows-1252 would have decoded each byte.
        match b {
          0x80 => '€',
          0x8c => 'Œ',
          0x9c => 'œ',
          0x94 => '”',
          b => char::from(b),
        }
      })
      .collect();
    assert_ne!(broken, "Witch Hunt 「魔女探し」");
    assert_eq!(repair_mojibake(&broken), "Witch Hunt 「魔女探し」");
    for good in ["Opening", "Café", "魔女探し", "Ending – Part 2"] {
      assert_eq!(repair_mojibake(good), good);
    }
  }
}
