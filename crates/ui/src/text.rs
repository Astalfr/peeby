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

/// Text of an HTML snippet (synopses with `<br>`, `<i>`, entities): tags dropped, breaks and
/// paragraph ends turned into newlines, blank lines collapsed.
pub fn plain_text(html: &str) -> String {
  let mut out = String::with_capacity(html.len());
  let mut rest = html;
  while let Some(start) = rest.find('<') {
    out.push_str(&rest[..start]);
    let Some(end) = rest[start..].find('>') else {
      out.push_str(&rest[start..]);
      rest = "";
      break;
    };
    let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
    if tag.starts_with("br") || tag == "/p" {
      out.push('\n');
    }
    rest = &rest[start + end + 1..];
  }
  out.push_str(rest);
  let out = out
    .replace("&amp;", "&")
    .replace("&quot;", "\"")
    .replace("&#39;", "'")
    .replace("&apos;", "'")
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&nbsp;", " ");
  // Collapse the blank lines left by consecutive breaks.
  let mut lines: Vec<&str> = Vec::new();
  for line in out.lines().map(str::trim) {
    if !(line.is_empty() && lines.last().is_some_and(|l| l.is_empty())) {
      lines.push(line);
    }
  }
  lines.join("\n").trim().to_owned()
}

/// [`plain_text`] on a single line, for a clamped paragraph (`line_clamp` counts the lines of
/// each paragraph).
pub fn one_paragraph(html: &str) -> String {
  plain_text(html)
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plain_text_strips_html() {
    assert_eq!(
      plain_text("A<br>\n<br>\nB &amp; <i>C</i><br/>"),
      "A\n\nB & C"
    );
    assert_eq!(plain_text("no tags"), "no tags");
    assert_eq!(plain_text("broken <tag"), "broken <tag");
    assert_eq!(one_paragraph("A<br>B\n C"), "A B C");
  }

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
