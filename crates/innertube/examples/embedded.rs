//! A video's stream through the signed-in web_embedded client, its `n` challenge solved in the
//! bundled Deno: timings, qualities, captions, storyboard and whether the first segment is
//! served. Prints no URL.
//!
//! `cargo run -p innertube --example embedded -- <cookies.txt> <video-id>… [--save <file>] [--no-pyv] [--tv]`
//! (an exported copy of the jar, as for `home`; needs `scripts/fetch-tools.ps1`). `--save` writes
//! the first video's whole `player` answer (`--tv`: tv_samsung's): stream URLs with the IP and
//! signatures, the account's visitorData: redact and trim it before it becomes a fixture. `--tv`
//! also prints each video's direct formats (`Client::tv_offer`) and reads their first and last
//! megabyte once served.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use innertube::jsc::Solver;
use innertube::{Client, Cookies};

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let pyv = !take_flag(&mut args, "--no-pyv");
  let tv = take_flag(&mut args, "--tv");
  let save = args
    .iter()
    .position(|a| a == "--save")
    .map(|i| {
      args.remove(i);
      (i < args.len()).then(|| args.remove(i))
    })
    .map(|file| file.unwrap_or_else(|| fail("--save needs a file")));
  let Some((path, ids)) = args.split_first() else {
    fail("usage: embedded <cookies.txt> <video-id>… [--save <file>] [--no-pyv] [--tv]")
  };
  let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
  let cookies = Cookies::from_netscape(&text).unwrap_or_else(|e| fail(&e.to_string()));
  // Rotated cookies are written back, as the app does.
  let client = Client::new(
    Some(cookies),
    Some(innertube::save_to_file(path.into())),
    "fr",
    "FR",
  )
  .unwrap_or_else(|e| fail(&e.to_string()));
  if let Err(e) = client.refresh_client_version().await {
    eprintln!("warning: {e}");
  }
  let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/yt-dlp");
  let work = std::env::temp_dir();
  let solver = Arc::new(
    Solver::new(&tools.join("deno.exe"), &tools, &work).unwrap_or_else(|e| fail(&e.to_string())),
  );
  let started = Instant::now();
  solver.warm_up();
  println!("Deno started in {} ms", started.elapsed().as_millis());

  if let Some(file) = &save {
    let id = &ids[0];
    let answer = if tv {
      client.tv_player(id).await
    } else {
      client.embedded_player(id, pyv).await
    };
    let (player, _, _) = answer.unwrap_or_else(|e| fail(&e.to_string()));
    let json = serde_json::to_string_pretty(&player).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::write(file, json).unwrap_or_else(|e| fail(&format!("{file}: {e}")));
    println!(
      "{id}: saved to {file} (ads to wait: {} s)",
      innertube::ad_wait(&player)
    );
  }

  let http = reqwest::Client::new();
  if tv {
    for id in ids {
      direct(&client, &http, &solver, id).await;
    }
    return;
  }
  for id in ids {
    let t = Instant::now();
    let stream = match client.embedded_stream(id, 1080, &solver).await {
      Ok(stream) => stream,
      Err(e) => {
        println!("{id}: {e} ({} ms)", t.elapsed().as_millis());
        continue;
      }
    };
    let resolved = t.elapsed().as_millis();
    let heights: Vec<u32> = stream.qualities.iter().map(|q| q.height).collect();
    // The first segment of the chosen variant.
    let status = async {
      let media = http
        .get(&stream.video_url)
        .header("User-Agent", stream.user_agent)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
      let segment = media.lines().find(|l| l.starts_with("http"))?.to_owned();
      let response = http
        .get(segment)
        .header("User-Agent", stream.user_agent)
        .send()
        .await
        .ok()?;
      Some(response.status().as_u16())
    }
    .await;
    println!(
      "{id}: stream in {resolved} ms · {} {} · qualities {heights:?} · {} captions · storyboard {} · first segment HTTP {} at {} ms",
      stream.resolution,
      stream.codecs,
      stream.captions.len(),
      stream.storyboard.is_some(),
      status.map_or("?".into(), |s| s.to_string()),
      t.elapsed().as_millis()
    );
  }
}

/// `id`'s direct formats through tv_samsung, then the first and last megabyte of the best video
/// and of the audio once served.
async fn direct(client: &Client, http: &reqwest::Client, solver: &Arc<Solver>, id: &str) {
  let t = Instant::now();
  let offer = match client.tv_offer(id, solver).await {
    Ok(Some(offer)) => offer,
    Ok(None) => return println!("{id}: no pre-roll ad, nothing offered"),
    Err(e) => return println!("{id}: {e}"),
  };
  let labels: Vec<String> = offer.videos.iter().map(|v| v.label()).collect();
  let wait = offer.served_at.saturating_duration_since(Instant::now());
  println!(
    "{id}: offer in {} ms · {labels:?} + {} · served in {} ms",
    t.elapsed().as_millis(),
    offer.audio.codecs,
    wait.as_millis()
  );
  tokio::time::sleep(wait).await;
  let Some(best) = offer.videos.first() else {
    return;
  };
  let formats = [
    ("video", &best.url, best.length),
    ("audio", &offer.audio.url, offer.audio.length),
  ];
  for (name, url, length) in formats {
    let mut out = Vec::new();
    let last = length.map_or("0-1".into(), |l| {
      format!("{}-{}", l.saturating_sub(1_048_576), l - 1)
    });
    for range in ["0-1048575".to_owned(), last] {
      let status = http
        .get(url)
        .header("User-Agent", offer.user_agent)
        .header("Range", format!("bytes={range}"))
        .send()
        .await
        .map_or(0, |r| r.status().as_u16());
      out.push(format!("{range}: {status}"));
    }
    println!("  {name}: {}", out.join(", "));
  }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
  let found = args.iter().position(|a| a == flag);
  if let Some(i) = found {
    args.remove(i);
  }
  found.is_some()
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
