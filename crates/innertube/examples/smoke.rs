//! Calls every endpoint once against YouTube and prints counts only (no titles, no cookies):
//! a quick check that the parsers still match YouTube's responses.
//!
//! `cargo run -p innertube --example smoke -- [cookies.txt]`

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use innertube::{Client, Cookies};

const VIDEO: &str = "dQw4w9WgXcQ";
const CHANNEL: &str = "UCuAXFkgsw1L7xaCfnd5JJOw";

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let path = std::env::args().nth(1);
  let cookies = path.as_ref().map(|path| {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    Cookies::from_netscape(&text).unwrap_or_else(|e| fail(&e.to_string()))
  });
  let signed_in = cookies.is_some();
  // Rotated cookies are written back, as the app does.
  let client =
    Client::new(cookies, path.map(Into::into), "fr", "FR").unwrap_or_else(|e| fail(&e.to_string()));
  let mut failures = 0;
  let mut check = |name: &str, result: Result<String, innertube::Error>| match result {
    Ok(summary) => println!("ok    {name:<20} {summary}"),
    Err(e) => {
      failures += 1;
      println!("FAIL  {name:<20} {e}");
    }
  };

  check(
    "home",
    client
      .home()
      .await
      .map(|f| format!("logged_in={} · {} videos", f.logged_in, f.items.len())),
  );
  if signed_in {
    check(
      "subscriptions",
      client.subscriptions().await.map(|f| {
        format!(
          "{} videos · more: {}",
          f.items.len(),
          f.continuation.is_some()
        )
      }),
    );
    check(
      "subscribed_channels",
      client
        .subscribed_channels()
        .await
        .map(|c| format!("{} channels", c.len())),
    );
  }
  let channel = client.channel_videos(CHANNEL).await;
  let next = channel.as_ref().ok().and_then(|f| f.continuation.clone());
  check(
    "channel_videos",
    channel.map(|f| format!("{} videos", f.items.len())),
  );
  if let Some(token) = next {
    check(
      "continuation",
      client
        .continuation(&token)
        .await
        .map(|f| format!("{} videos", f.items.len())),
    );
  }
  let watch = client.watch(VIDEO).await;
  let comments_token = watch
    .as_ref()
    .ok()
    .and_then(|w| w.comments_continuation.clone());
  check(
    "watch",
    watch.map(|w| format!("{} related", w.related.len())),
  );
  if let Some(token) = comments_token {
    check(
      "comments",
      client
        .comments(&token)
        .await
        .map(|c| format!("{} comments · total {:?}", c.comments.len(), c.count)),
    );
  }
  check(
    "stream",
    client
      .stream(VIDEO, 1080)
      .await
      .map(|s| format!("{} {}", s.resolution, s.codecs)),
  );
  check(
    "votes (RYD)",
    client
      .votes(VIDEO)
      .await
      .map(|v| format!("{} likes · {} dislikes", v.likes, v.dislikes)),
  );
  if failures > 0 {
    std::process::exit(1);
  }
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
