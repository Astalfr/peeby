//! A YouTube live through the local proxy (ADR 0025), anonymous: resolves it, prints its
//! qualities, how far back it goes and what its playlist lists, and fetches its first and last
//! segments through the proxy's redirects. Prints counts and status codes only (the signed URLs
//! stay in the proxy).
//!
//! `cargo run -p innertube --example live_probe -- [video-id | "search query"] [--serve <s>]`
//!
//! A search takes its first live younger than 12 h when there is one. `--serve` keeps the proxy
//! running that long and prints its playlist URLs, to play them with the spike:
//! `PEEBY_RESOLVER=raw PEEBY_AUDIO=<audio> PEEBY_START=<s> cargo run -p spike-yt-play --release -- <video>`.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

use innertube::{Client, SearchResult, Stream};

const DEFAULT_QUERY: &str = "live";
/// 12 h of 5 s segments: a live whose newest segment number is lower started less than 12 h ago.
const YOUNG: u64 = 12 * 720;

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let serve = match args.iter().position(|a| a == "--serve") {
    Some(i) => {
      let seconds = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(60);
      args.drain(i..(i + 2).min(args.len()));
      Some(Duration::from_secs(seconds))
    }
    None => None,
  };
  let arg = args
    .first()
    .cloned()
    .unwrap_or_else(|| DEFAULT_QUERY.into());
  let client = Client::new(None, None, "en", "US").unwrap_or_else(|e| fail(&e.to_string()));
  let ids: Vec<String> = if arg.len() == 11 && !arg.contains(' ') {
    vec![arg]
  } else {
    let page = client
      .search(&arg)
      .await
      .unwrap_or_else(|e| fail(&e.to_string()));
    page
      .results
      .into_iter()
      .filter_map(|r| match r {
        SearchResult::Video(v) if v.live => Some(v.id),
        _ => None,
      })
      .collect()
  };
  let http = reqwest::Client::new();
  let mut chosen: Option<(String, Stream)> = None;
  for id in ids {
    let started = Instant::now();
    let stream = match client.stream(&id, 1080).await {
      Ok(stream) => stream,
      Err(e) => {
        println!("{id}: {e}");
        continue;
      }
    };
    let Some(live) = &stream.live else {
      println!("{id}: not a live");
      continue;
    };
    let resolved = started.elapsed().as_secs_f64();
    let last = match &live.proxy {
      Some(_) => playlist(&http, &stream.video_url).await.map(|p| p.last),
      None => None,
    };
    println!(
      "{id}: resolved in {resolved:.2} s, {}, edge {:.0} s in, window {:?}, newest segment {last:?}",
      if live.proxy.is_some() {
        "proxied"
      } else {
        "HLS window"
      },
      live.length_now(),
      live.dvr_window.map(f64::round),
    );
    let young = last.is_some_and(|sq| sq < YOUNG);
    if chosen.is_none() || young {
      chosen = Some((id, stream));
    }
    if young {
      break;
    }
  }
  let Some((id, stream)) = chosen else {
    fail("no live found")
  };
  let Some(live) = &stream.live else {
    fail("not a live")
  };
  println!("\n{id}: {stream:?}");
  for quality in &stream.qualities {
    println!("  quality {quality:?}");
  }
  let Some(proxy) = &live.proxy else {
    println!("plays from the HLS window, no proxy");
    return;
  };
  // The proxy holds a shorter window until mpv would have read both playlists: read them first.
  let settled = Instant::now();
  for url in [Some(&stream.video_url), stream.audio_url.as_ref()]
    .into_iter()
    .flatten()
  {
    playlist(&http, url).await;
  }
  let window = proxy.settled().await;
  println!(
    "settled in {:.2} s: {window:.0} s back ({:.1} h), edge {:.0} s in",
    settled.elapsed().as_secs_f64(),
    window / 3600.0,
    proxy.length()
  );
  for (name, url) in [
    ("video", Some(&stream.video_url)),
    ("audio", stream.audio_url.as_ref()),
  ] {
    let Some(url) = url else { continue };
    let Some(list) = playlist(&http, url).await else {
      println!("{name}: no playlist");
      continue;
    };
    println!(
      "{name}: {} segments of {} s, {}..={}",
      list.uris.len(),
      list.extinf,
      list.first,
      list.last
    );
    let base = url.rsplit_once('/').map_or(url.as_str(), |(base, _)| base);
    // What the proxy's search asks: one byte of segment 0 (gone on a live older than 12 h) and
    // of the first one listed.
    if let Some(first) = list.uris.first() {
      let zero = first.rsplit_once('/').map(|(head, _)| format!("{head}/0"));
      for uri in zero.iter().chain(std::iter::once(first)) {
        let answer = http
          .get(format!("{base}/{uri}"))
          .header(reqwest::header::RANGE, "bytes=0-0")
          .send()
          .await;
        match answer {
          Ok(r) => {
            let status = r.status().as_u16();
            let length = r.bytes().await.map_or(0, |b| b.len());
            println!("  one byte of {uri}: {status}, {length} B");
          }
          Err(e) => println!("  one byte of {uri}: {}", e.without_url()),
        }
      }
    }
    for (label, uri) in [("first", list.uris.first()), ("last", list.uris.last())] {
      let Some(uri) = uri else { continue };
      let started = Instant::now();
      match http.get(format!("{base}/{uri}")).send().await {
        Ok(r) => {
          let status = r.status().as_u16();
          let length = r.bytes().await.map_or(0, |b| b.len());
          println!(
            "  {label} segment: {status}, {length} B in {} ms",
            started.elapsed().as_millis()
          );
        }
        Err(e) => println!("  {label} segment: {}", e.without_url()),
      }
    }
  }
  if let Some(duration) = serve {
    println!(
      "\nserving for {} s\nvideo {}\naudio {}\nedge {:.0}",
      duration.as_secs(),
      stream.video_url,
      stream.audio_url.as_deref().unwrap_or("-"),
      proxy.length()
    );
    tokio::time::sleep(duration).await;
  }
}

/// What a proxy playlist lists.
struct Listed {
  first: u64,
  last: u64,
  extinf: String,
  uris: Vec<String>,
}

async fn playlist(http: &reqwest::Client, url: &str) -> Option<Listed> {
  let text = http.get(url).send().await.ok()?.text().await.ok()?;
  let uris: Vec<String> = text
    .lines()
    .filter(|l| !l.starts_with('#') && !l.is_empty())
    .map(str::to_owned)
    .collect();
  let number = |uri: &String| uri.rsplit('/').next()?.parse::<u64>().ok();
  Some(Listed {
    first: uris.first().and_then(number)?,
    last: uris.last().and_then(number)?,
    extinf: text
      .lines()
      .find_map(|l| l.strip_prefix("#EXTINF:"))
      .unwrap_or("?")
      .trim_end_matches(',')
      .to_owned(),
    uris,
  })
}

fn fail(message: &str) -> ! {
  eprintln!("{message}");
  std::process::exit(1)
}
