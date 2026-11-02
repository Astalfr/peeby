//! Choosing a variant in YouTube's HLS master playlists. Variants are video only; each names
//! its audio rendition (`EXT-X-MEDIA` of `TYPE=AUDIO`) through its `AUDIO` group. A live's media
//! playlist is a window of its last segments ([`playlist_length`]).

/// A video variant of the master playlist.
#[derive(Clone, PartialEq, Eq)]
pub struct Variant {
  /// Height in pixels.
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
      .and_then(|(_, h)| h.parse().ok())
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

/// URI of the audio rendition of `group`.
fn audio_rendition(master: &str, group: &str) -> Option<String> {
  master
    .lines()
    .filter_map(|line| line.strip_prefix("#EXT-X-MEDIA:"))
    .filter(|attrs| attribute(attrs, "TYPE").as_deref() == Some("AUDIO"))
    .find(|attrs| attribute(attrs, "GROUP-ID").as_deref() == Some(group))
    .and_then(|attrs| attribute(attrs, "URI"))
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
}
