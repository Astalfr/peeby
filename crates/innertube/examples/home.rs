//! Prints the Home feed, or the subscriptions one. Replaces the poc-home prototype.
//!
//! `cargo run -p innertube --example home -- [cookies.txt] [subscriptions]`
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
  let client =
    Client::new(cookies, path.map(Into::into), "fr", "FR").unwrap_or_else(|e| fail(&e.to_string()));
  if let Err(e) = client.refresh_client_version().await {
    eprintln!("warning: {e}");
  }
  let feed = if std::env::args().nth(2).as_deref() == Some("subscriptions") {
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
