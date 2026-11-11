//! Marks the first video of the signed-in Home "Not interested", then undoes it, saving both
//! `feedback` answers.
//!
//! `cargo run -p innertube --example not_interested -- <cookies.txt> <dir>` (an exported copy of
//! the jar, as for `home`).
//!
//! WRITES FROM THE ACCOUNT (ask the user first): the video leaves the recommendations, then
//! comes back with the undo. Saves into `<dir>` `feedback.json` (the "Not interested") and
//! `undo_feedback.json` (its undo). The answers hold the account's session data
//! (`responseContext`): trim them before they become fixtures.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};

use innertube::{Client, Cookies};
use serde_json::Value;

const USAGE: &str = "usage: not_interested <cookies.txt> <dir>";

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let mut args = std::env::args().skip(1);
  let (Some(path), Some(dir)) = (args.next(), args.next()) else {
    fail(USAGE);
  };
  let dir = PathBuf::from(dir);
  let text = std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
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
  let home = client.home().await.unwrap_or_else(|e| fail(&e.to_string()));
  let Some((video, tokens)) = home
    .items
    .iter()
    .find_map(|v| v.not_interested.as_ref().map(|n| (v, n)))
  else {
    fail("no video with \"Not interested\" on Home: signed out?");
  };
  let Some(undo) = tokens.undo.as_deref() else {
    fail("no undo token: nothing sent");
  };
  println!("video {}", video.id);
  std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
  for (name, token) in [("feedback", tokens.token.as_str()), ("undo_feedback", undo)] {
    let answer = client
      .send_feedback_response(token)
      .await
      .unwrap_or_else(|e| fail(&format!("{name}: {e}")));
    let file = dir.join(format!("{name}.json"));
    write(&file, &answer);
    let keys: Vec<_> = answer
      .as_object()
      .map(|o| o.keys().cloned().collect())
      .unwrap_or_default();
    println!("{name}: keys {keys:?}");
  }
}

fn write(file: &Path, value: &Value) {
  let json = serde_json::to_string_pretty(value).unwrap_or_else(|e| fail(&e.to_string()));
  std::fs::write(file, json).unwrap_or_else(|e| fail(&format!("{}: {e}", file.display())));
  println!("saved to {}", file.display());
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
