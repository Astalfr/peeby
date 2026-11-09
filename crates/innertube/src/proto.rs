//! The few protobuf messages Peeby builds itself, where YouTube's answers give no ready-made
//! command (the web client builds them in JavaScript).

/// `comment/perform_comment_action`'s action translating comment `comment_id` of `video_id`
/// into `target` (a language code such as `fr`). YouTube translates `text`, which it requires,
/// and keys the answer by the comment. Fields as the like action YouTube gives (type 1, comment
/// 3, video 5) and as YouTube.js sends this one (type 22, `translateCommentParams` 31: comment 2,
/// `{comment: {text}}` 3, target 4); checked live 2026-11-09, anonymous and with emoji and line
/// breaks in `text`.
pub(crate) fn translate_comment_action(
  video_id: &str,
  comment_id: &str,
  text: &str,
  target: &str,
) -> String {
  let mut comment = Vec::new();
  bytes(&mut comment, 1, text.as_bytes());
  let mut params = Vec::new();
  bytes(&mut params, 1, &comment);
  let mut translate = Vec::new();
  bytes(&mut translate, 2, comment_id.as_bytes());
  bytes(&mut translate, 3, &params);
  bytes(&mut translate, 4, target.as_bytes());
  let mut action = Vec::new();
  varint_field(&mut action, 1, 22);
  bytes(&mut action, 3, comment_id.as_bytes());
  bytes(&mut action, 5, video_id.as_bytes());
  bytes(&mut action, 31, &translate);
  base64_url(&action)
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
  loop {
    // The low 7 bits, the high one set while more follow.
    let byte = (value & 0x7f) as u8;
    value >>= 7;
    if value == 0 {
      return out.push(byte);
    }
    out.push(byte | 0x80);
  }
}

fn varint_field(out: &mut Vec<u8>, field: u64, value: u64) {
  varint(out, field << 3);
  varint(out, value);
}

/// A length-delimited field (wire type 2): bytes, a string or a message.
fn bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
  varint(out, (field << 3) | 2);
  varint(out, value.len() as u64);
  out.extend_from_slice(value);
}

/// URL-safe base64, padded, as YouTube's own actions are.
fn base64_url(bytes: &[u8]) -> String {
  const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
    for i in 0..=chunk.len() {
      out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
    }
  }
  while !out.len().is_multiple_of(4) {
    out.push('=');
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn base64_matches_the_standard() {
    assert_eq!(base64_url(b""), "");
    assert_eq!(base64_url(b"f"), "Zg==");
    assert_eq!(base64_url(b"fo"), "Zm8=");
    assert_eq!(base64_url(b"foo"), "Zm9v");
    assert_eq!(base64_url(&[0xfb, 0xff]), "-_8=");
  }

  #[test]
  fn varints() {
    let mut out = Vec::new();
    varint(&mut out, 300);
    assert_eq!(out, [0xac, 0x02]);
  }

  #[test]
  fn translate_action() {
    // Field 1 = 22, 3 = "c", 5 = "v", 31 = {2: "c", 3: {1: {1: "hi"}}, 4: "fr"}.
    let expected: &[u8] = &[
      0x08, 22, 0x1a, 1, b'c', 0x2a, 1, b'v', 0xfa, 0x01, 15, 0x12, 1, b'c', 0x1a, 6, 0x0a, 4,
      0x0a, 2, b'h', b'i', 0x22, 2, b'f', b'r',
    ];
    assert_eq!(
      translate_comment_action("v", "c", "hi", "fr"),
      base64_url(expected)
    );
  }
}
