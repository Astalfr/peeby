//! 7TV's cosmetics (docs/twitch.md, "7TV cosmetics", ADR 0031): the paints that color a user's
//! name and the badges before it, as the EventAPI pushes them to a channel's subscribers
//! (`cosmetic.*`, `entitlement.*` with the `channel` context, [`super::events::Topic::Channel`]).
//! A cosmetic's definition comes first (`cosmetic.create`), then who wears it
//! (`entitlement.create`, for each 7TV user who announces their presence in that chat).
//! Tested on a saved stream (`tests/fixtures/seventv_cosmetics.jsonl`).

use serde_json::Value;

/// A color as 7TV packs it: `0xRRGGBBAA` (sent as a signed 32-bit integer).
pub type Rgba = u32;

/// What colors a user's name.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
  /// 7TV's paint id.
  pub id: String,
  /// Its name ("Ghoulish"…).
  pub name: String,
  /// How its stops are laid over the name.
  pub fill: Fill,
  /// Its gradient's stops, in order: where (0 to 1 along the gradient) and which color.
  pub stops: Vec<(f32, Rgba)>,
  /// The gradient repeats past its last stop (CSS `repeating-*-gradient`).
  pub repeat: bool,
  /// A plain color the paint gives besides (mostly `None`).
  pub color: Option<Rgba>,
}

/// How a [`Paint`] is laid over the name.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
  /// A linear gradient, at this CSS angle (degrees, 0 towards the top, 90 towards the right).
  Linear(f32),
  /// A radial gradient from the middle: a circle, else an ellipse.
  Radial {
    /// `shape: "circle"`.
    circle: bool,
  },
  /// An image (`image_url`) drawn through the letters.
  Image(String),
}

/// A 7TV badge, before the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
  /// 7TV's badge id.
  pub id: String,
  /// What it says ("7TV Translator"…).
  pub tooltip: String,
  /// `https://cdn.7tv.app/badge/<id>`.
  base: String,
  /// WebP files, smallest first: width, name.
  files: Vec<(u32, String)>,
}

impl Badge {
  /// The URL of the smallest WebP at least `width` pixels wide (else the largest).
  pub fn url(&self, width: u32) -> Option<String> {
    let (_, name) = self
      .files
      .iter()
      .find(|(w, _)| *w >= width)
      .or_else(|| self.files.last())?;
    Some(format!("{}/{name}", self.base))
  }
}

/// A cosmetic's definition.
#[derive(Clone, Debug, PartialEq)]
pub enum Cosmetic {
  /// A paint.
  Paint(Paint),
  /// A badge.
  Badge(Badge),
}

/// Which cosmetic an [`Entitlement`] gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
  /// A [`Paint`].
  Paint,
  /// A [`Badge`].
  Badge,
}

/// A Twitch user wearing a cosmetic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entitlement {
  /// A paint or a badge.
  pub kind: Kind,
  /// The cosmetic's 7TV id.
  pub id: String,
  /// The user's Twitch id (their 7TV account's Twitch connection), as the chat's lines give it.
  pub user: String,
}

/// A cosmetic's definition from a `cosmetic.create` (or `cosmetic.update`) body; `None` for a kind
/// Peeby does not draw or a malformed one.
pub fn cosmetic(body: &Value) -> Option<Cosmetic> {
  let object = body.get("object")?;
  let data = object.get("data")?;
  let id = text(data, "id").or_else(|| text(object, "id"))?;
  match object.get("kind")?.as_str()? {
    "PAINT" => paint(id, data).map(Cosmetic::Paint),
    "BADGE" => badge(id, data).map(Cosmetic::Badge),
    _ => None,
  }
}

fn paint(id: String, data: &Value) -> Option<Paint> {
  let fill = match data.get("function")?.as_str()? {
    "LINEAR_GRADIENT" => {
      Fill::Linear(data.get("angle").and_then(Value::as_f64).unwrap_or(0.) as f32)
    }
    "RADIAL_GRADIENT" => Fill::Radial {
      circle: data.get("shape").and_then(Value::as_str) != Some("ellipse"),
    },
    "URL" => Fill::Image(text(data, "image_url").filter(|u| !u.is_empty())?),
    _ => return None,
  };
  let mut stops: Vec<(f32, Rgba)> = data
    .get("stops")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|stop| {
      let at = stop.get("at")?.as_f64()? as f32;
      Some((at.clamp(0., 1.), rgba(stop.get("color")?)?))
    })
    .collect();
  stops.sort_by(|a, b| a.0.total_cmp(&b.0));
  if stops.is_empty() && !matches!(fill, Fill::Image(_)) {
    return None;
  }
  Some(Paint {
    id,
    name: text(data, "name").unwrap_or_default(),
    fill,
    stops,
    repeat: data.get("repeat").and_then(Value::as_bool).unwrap_or(false),
    color: data.get("color").and_then(rgba),
  })
}

fn badge(id: String, data: &Value) -> Option<Badge> {
  let host = data.get("host")?;
  let base = host.get("url")?.as_str()?;
  let base = if base.starts_with("//") {
    format!("https:{base}")
  } else {
    base.to_owned()
  };
  let mut files: Vec<(u32, String)> = host
    .get("files")?
    .as_array()?
    .iter()
    .filter(|f| f.get("format").and_then(Value::as_str) == Some("WEBP"))
    .filter_map(|f| {
      let width = u32::try_from(f.get("width")?.as_u64()?).ok()?;
      Some((width, f.get("name")?.as_str()?.to_owned()))
    })
    .collect();
  if files.is_empty() {
    return None;
  }
  files.sort_by_key(|(w, _)| *w);
  Some(Badge {
    id,
    tooltip: text(data, "tooltip")
      .or_else(|| text(data, "name"))
      .unwrap_or_default(),
    base,
    files,
  })
}

/// Who wears what, from an `entitlement.create` or `entitlement.delete` body; `None` for an
/// emote set (7TV's personal emotes, not drawn yet), a user without a Twitch connection, or a
/// paint or badge other than the one the user has on (their `style` says which).
pub fn entitlement(body: &Value) -> Option<Entitlement> {
  let object = body.get("object")?;
  let (kind, worn) = match object.get("kind")?.as_str()? {
    "PAINT" => (Kind::Paint, "paint_id"),
    "BADGE" => (Kind::Badge, "badge_id"),
    _ => return None,
  };
  let id = text(object, "ref_id")?;
  let user = object.get("user")?;
  if let Some(on) = user
    .pointer(&format!("/style/{worn}"))
    .and_then(Value::as_str)
    && on != id
  {
    return None;
  }
  let twitch = user
    .get("connections")?
    .as_array()?
    .iter()
    .find(|c| c.get("platform").and_then(Value::as_str) == Some("TWITCH"))?;
  Some(Entitlement {
    kind,
    id,
    user: text(twitch, "id")?,
  })
}

/// A 7TV color: a signed or unsigned 32-bit `0xRRGGBBAA`.
fn rgba(value: &Value) -> Option<Rgba> {
  let n = value.as_i64()?;
  // The signed form wraps: -1 is 0xFFFFFFFF, opaque white.
  i32::try_from(n)
    .map(|signed| signed as u32)
    .or_else(|_| u32::try_from(n))
    .ok()
}

fn text(value: &Value, key: &str) -> Option<String> {
  value.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn colors_unpack_from_signed_integers() {
    assert_eq!(rgba(&Value::from(-1)), Some(0xFFFF_FFFF));
    // -1140915969 is 0xBBFF_00FF: an opaque lime green.
    assert_eq!(rgba(&Value::from(-1_140_915_969_i64)), Some(0xBBFF_00FF));
    assert_eq!(rgba(&Value::from(16_712_959)), Some(0x00FF_04FF));
    assert_eq!(rgba(&Value::from(0xFFFF_FFFF_u32)), Some(0xFFFF_FFFF));
    assert_eq!(rgba(&Value::Null), None);
  }
}
