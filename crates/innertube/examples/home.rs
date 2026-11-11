//! Prints the Home feed, or the subscriptions one. Replaces the poc-home prototype.
//!
//! `cargo run -p innertube --example home -- [cookies.txt] [subscriptions] [--save <file>]`
//! (`--save` writes the whole response, account data included: trim it before a fixture)
//! Without cookies the feed is empty (YouTube shows no anonymous recommendations). With cookies
//! it fails loudly if YouTube answers as signed out.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use innertube::{Client, Cookies};

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let path = std::env::args().nth(1);
  let cookies = path.as_ref().map(|path| {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    Cookies::from_netscape(&text).unwrap_or_else(|e| fail(&e.to_string()))
  });
  // Rotated cookies are written back, as the app does.
  let client = Client::new(
    cookies,
    path.map(|p| innertube::save_to_file(p.into())),
    "fr",
    "FR",
  )
  .unwrap_or_else(|e| fail(&e.to_string()));
  if let Err(e) = client.refresh_client_version().await {
    eprintln!("warning: {e}");
  }
  let args: Vec<String> = std::env::args().collect();
  let subscriptions = args.get(2).map(String::as_str) == Some("subscriptions");
  if let Some(file) = args
    .iter()
    .position(|a| a == "--save")
    .and_then(|i| args.get(i + 1))
  {
    let id = if subscriptions {
      "FEsubscriptions"
    } else {
      "FEwhat_to_watch"
    };
    let response = client
      .browse_response(id, None)
      .await
      .unwrap_or_else(|e| fail(&e.to_string()));
    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|e| fail(&e.to_string()));
    std::fs::write(file, json).unwrap_or_else(|e| fail(&format!("{file}: {e}")));
    println!("saved to {file}");
  }
  let feed = if subscriptions {
    client.subscriptions().await
  } else {
    client.home().await
  };
  let feed = feed.unwrap_or_else(|e| fail(&e.to_string()));
  for v in &feed.items {
    println!(
      "{:<12} {:>8} {:>4}  {}  —  {} · {}",
      v.id,
      v.duration.as_deref().unwrap_or(""),
      v.progress
        .map(|p| format!("{}%", p.percent))
        .unwrap_or_default(),
      v.title,
      v.channel.as_deref().unwrap_or("?"),
      v.metadata.join(" · ")
    );
  }
  println!(
    "\nlogged_in={} · {} videos · continuation: {}",
    feed.logged_in,
    feed.items.len(),
    feed.continuation.is_some()
  );
  if let Some(message) = feed.message {
    println!("YouTube: {message}");
  }
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
