//! How long a Twitch live's segments stay served after they leave its media playlist (a local
//! window of the live's last minutes, docs/decisions/0026), anonymous: reads one variant's
//! playlist every 2 s and asks for one byte of each segment that left it, 30 s, 1, 2, 3 and 5 min
//! after it left. Prints status codes and counts only (no token, no URL).
//!
//! `cargo run -p twitch --example segment_ttl -- <login> [minutes]` (default 7 minutes).

// An example reports on the console.
#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use twitch::usher::Codec;
use twitch::{Client, Target, gql};

/// When, after a segment left the playlist, one byte of it is asked for.
const AGES: [u64; 5] = [30, 60, 120, 180, 300];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut args = std::env::args().skip(1);
  let login = args.next().ok_or("segment_ttl <login> [minutes]")?;
  let minutes: u64 = args.next().and_then(|m| m.parse().ok()).unwrap_or(7);
  let client = Client::new(None, None)?;
  client.refresh_client_id().await.ok();
  let target = Target::Live(login.clone());
  let token = client.playback_token(&target, gql::PLAYER_TYPE).await?;
  let master = client.master(&target, &token, &[Codec::H264]).await?;
  let variant = master
    .variants
    .iter()
    .filter(|v| !v.portrait && !v.audio_only())
    .max_by_key(|v| (v.height, v.bandwidth))
    .ok_or("no variant")?;
  println!(
    "{login}: variant {:?}p, watching {minutes} min",
    variant.height
  );
  let http = reqwest::Client::new();
  // Segment URL → when it left the playlist; the ages already asked.
  let mut listed: HashSet<String> = HashSet::new();
  let mut left: HashMap<String, (Instant, usize)> = HashMap::new();
  // Age → status → count.
  let mut results: BTreeMap<u64, BTreeMap<String, u32>> = BTreeMap::new();
  let mut window = Vec::new();
  let (mut ads, mut discontinuities) = (0u32, 0u32);
  let until = Instant::now() + Duration::from_secs(minutes * 60);
  while Instant::now() < until {
    match http.get(&variant.url).send().await {
      Ok(response) => {
        let text = response.text().await.unwrap_or_default();
        let now: HashSet<String> = text
          .lines()
          .filter(|l| l.starts_with("http"))
          .map(str::to_owned)
          .collect();
        window.push(now.len());
        ads += u32::from(text.contains("stitched-ad") || text.contains("Amazon"));
        discontinuities += text.matches("#EXT-X-DISCONTINUITY\n").count() as u32;
        for gone in listed.difference(&now) {
          left.entry(gone.clone()).or_insert((Instant::now(), 0));
        }
        listed = now;
      }
      Err(e) => println!("playlist: {}", e.without_url()),
    }
    for (url, (at, asked)) in left.iter_mut() {
      let Some(&age) = AGES.get(*asked) else {
        continue;
      };
      if at.elapsed() < Duration::from_secs(age) {
        continue;
      }
      *asked += 1;
      let status = match http.get(url).header("Range", "bytes=0-0").send().await {
        Ok(r) => {
          let code = r.status().as_u16();
          let length = r.bytes().await.map_or(0, |b| b.len());
          format!("{code} ({length} B)")
        }
        Err(e) => e.without_url().to_string(),
      };
      *results.entry(age).or_default().entry(status).or_default() += 1;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
  }
  window.sort_unstable();
  println!(
    "playlist: {} to {} segments (median {}), {} reloads with ad markers, {} discontinuities seen",
    window.first().unwrap_or(&0),
    window.last().unwrap_or(&0),
    window.get(window.len() / 2).unwrap_or(&0),
    ads,
    discontinuities
  );
  for (age, statuses) in &results {
    println!("  {age:>3} s after leaving: {statuses:?}");
  }
  Ok(())
}
