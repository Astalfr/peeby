//! Choosing a variant in YouTube's HLS master playlists. Variants are video only; each names
//! its audio rendition (`EXT-X-MEDIA` of `TYPE=AUDIO`) through its `AUDIO` group. A live's media
//! playlist is a window of its last segments ([`playlist_length`]).

/// A video variant of the master playlist.
#[derive(Clone, PartialEq, Eq)]
pub struct Variant {
  /// Height in pixels; a vertical video's width (its shorter side), so that a 1080×1920 Short
  /// is "1080p" and fits a 1080 cap as YouTube labels it.
  pub height: u32,
  /// `FRAME-RATE` rounded, e.g. 60.
  pub frame_rate: Option<u32>,
  /// Video media playlist URL. Contains the user's IP: never log it.
  pub url: String,
  /// Audio media playlist URL.
  pub audio_url: Option<String>,
  /// `RESOLUTION`, e.g. `1920x1080`.
  pub resolution: String,
  /// `CODECS`.
  pub codecs: String,
}

impl std::fmt::Debug for Variant {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.label())?;
    write!(f, " {}", self.codecs)
  }
}

impl Variant {
  /// `1080p`, `720p60`.
  pub fn label(&self) -> String {
    match self.frame_rate {
      Some(fps) if fps > 30 => format!("{}p{fps}", self.height),
      _ => format!("{}p", self.height),
    }
  }
}

/// Highest variant up to `max_height`; at equal height VP9 (better quality per bit) over AVC,
/// then the highest bandwidth.
pub fn best_variant(master: &str, max_height: u32) -> Option<Variant> {
  ladder(master)
    .into_iter()
    .find(|variant| variant.height <= max_height)
}

/// One variant per height, highest first; at equal height VP9 over AVC, then the highest
/// bandwidth.
pub fn ladder(master: &str) -> Vec<Variant> {
  let mut lines = master.lines();
  let mut best: Vec<((u32, bool, u64), Variant)> = Vec::new();
  while let Some(line) = lines.next() {
    let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
      continue;
    };
    let Some(url) = lines.next() else { break };
    let resolution = attribute(attrs, "RESOLUTION").unwrap_or_default();
    let height = resolution
      .split_once('x')
      .and_then(|(w, h)| Some(w.parse::<u32>().ok()?.min(h.parse().ok()?)))
      .unwrap_or(0);
    let bandwidth = attribute(attrs, "BANDWIDTH")
      .and_then(|b| b.parse().ok())
      .unwrap_or(0);
    let codecs = attribute(attrs, "CODECS").unwrap_or_default();
    let rank = (height, codecs.starts_with("vp09"), bandwidth);
    let slot = best.iter_mut().find(|((h, _, _), _)| *h == height);
    if slot
      .as_ref()
      .is_some_and(|(best_rank, _)| rank <= *best_rank)
    {
      continue;
    }
    let audio_url = attribute(attrs, "AUDIO").and_then(|group| audio_rendition(master, &group));
    let variant = Variant {
      height,
      frame_rate: attribute(attrs, "FRAME-RATE")
        .and_then(|r| r.parse::<f32>().ok())
        .map(|r| r.round() as u32),
      url: url.to_owned(),
      audio_url,
      resolution,
      codecs,
    };
    match slot {
      Some(slot) => *slot = (rank, variant),
      None => best.push((rank, variant)),
    }
  }
  best.sort_by(|(a, _), (b, _)| b.cmp(a));
  best.into_iter().map(|(_, variant)| variant).collect()
}

/// Seconds a media playlist covers: the sum of its segments' `EXTINF` durations. For a live, the
/// end of the window is its live edge.
pub fn playlist_length(media: &str) -> f64 {
  media
    .lines()
    .filter_map(|line| line.strip_prefix("#EXTINF:"))
    .filter_map(|rest| rest.split(',').next()?.trim().parse::<f64>().ok())
    .sum()
}

/// URI of the audio rendition of `group`. A dubbed video lists one rendition per language, dubs
/// first and none marked `DEFAULT=YES`: take the original track (`acont=original` in the URI's
/// `xtags`), then the default one, then the first.
fn audio_rendition(master: &str, group: &str) -> Option<String> {
  master
    .lines()
    .filter_map(|line| line.strip_prefix("#EXT-X-MEDIA:"))
    .filter(|attrs| attribute(attrs, "TYPE").as_deref() == Some("AUDIO"))
    .filter(|attrs| attribute(attrs, "GROUP-ID").as_deref() == Some(group))
    .filter_map(|attrs| Some((attribute(attrs, "URI")?, attrs)))
    // `min_by_key` keeps the first of equal ranks.
    .min_by_key(|(uri, attrs)| {
      std::cmp::Reverse((
        audio_content(uri) == Some("original"),
        attribute(attrs, "DEFAULT").as_deref() == Some("YES"),
      ))
    })
    .map(|(uri, _)| uri)
}

/// `acont` of the `xtags` in a rendition's URI (`xtags%3Dacont%3Ddubbed:lang%3Dfr`): `original`,
/// `dubbed`, `dubbed-auto`, `descriptive`…
fn audio_content(uri: &str) -> Option<&str> {
  let start = uri
    .find("acont%3D")
    .map(|i| i + "acont%3D".len())
    .or_else(|| uri.find("acont=").map(|i| i + "acont=".len()))?;
  let rest = &uri[start..];
  let end = rest
    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .unwrap_or(rest.len());
  Some(&rest[..end])
}

/// Value of `name` in an HLS attribute list (`A=1,B="x,y"`). The name must start the list or
/// follow a comma, so `ID` does not match inside `GROUP-ID`.
fn attribute(attrs: &str, name: &str) -> Option<String> {
  let key = format!("{name}=");
  let start = attrs
    .match_indices(&key)
    .find(|(i, _)| *i == 0 || attrs[..*i].ends_with(','))?
    .0
    + key.len();
  let rest = &attrs[start..];
  if let Some(quoted) = rest.strip_prefix('"') {
    Some(quoted[..quoted.find('"')?].to_owned())
  } else {
    Some(rest.split(',').next()?.to_owned())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn quoted_attributes_with_commas() {
    let attrs = r#"BANDWIDTH=1,CODECS="avc1.4D401F,mp4a.40.2",RESOLUTION=854x480,AUDIO="234""#;
    assert_eq!(attribute(attrs, "CODECS").unwrap(), "avc1.4D401F,mp4a.40.2");
    assert_eq!(attribute(attrs, "AUDIO").unwrap(), "234");
    assert_eq!(attribute(r#"GROUP-ID="1",ID=2"#, "ID").unwrap(), "2");
  }

  #[test]
  fn vertical_variants_count_their_width() {
    let master = "#EXT-X-STREAM-INF:BANDWIDTH=2,CODECS=\"avc1\",RESOLUTION=1080x1920\nhigh.m3u8\n\
      #EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"avc1\",RESOLUTION=720x1280\nlow.m3u8\n";
    let best = best_variant(master, 1080).unwrap();
    assert_eq!((best.height, best.url.as_str()), (1080, "high.m3u8"));
    assert_eq!(best.label(), "1080p");
  }
}
