//! Prints the first comments of a video, with the signed-in user's rating of each, and can save
//! the answer.
//!
//! `cargo run -p innertube --example comments -- <cookies.txt> <video-id> [--save <file>] [--rate <dir>]`
//! (an exported copy of the jar, as for `home`). `--save` writes the whole response, which holds
//! the account's session data (`responseContext`, visitorData) and, in its reply composer, the
//! account's name and avatar: trim it before it becomes a fixture.
//!
//! `--rate` WRITES FROM THE ACCOUNT (ask the user first): it likes the first comment, dislikes
//! it, then removes the dislike, saving into `<dir>` each `comment/perform_comment_action`
//! answer (`like.json`, `dislike.json`, `undislike.json`) and the comments page read after the
//! like and after the dislike (`page_liked.json`, `page_disliked.json`). The comment ends as it
//! was, unrated.

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::{Path, PathBuf};

use innertube::{Client, Comment, Cookies, LikeStatus};
use serde_json::Value;

const USAGE: &str = "usage: comments <cookies.txt> <video-id> [--save <file>] [--rate <dir>]";

#[tokio::main(flavor = "current_thread")]
async fn main() {
  let mut args = std::env::args().skip(1);
  let (Some(path), Some(video)) = (args.next(), args.next()) else {
    fail(USAGE);
  };
  let (mut save, mut rate) = (None, None);
  while let Some(flag) = args.next() {
    match (flag.as_str(), args.next()) {
      ("--save", Some(file)) => save = Some(file),
      ("--rate", Some(dir)) => rate = Some(PathBuf::from(dir)),
      _ => fail(USAGE),
    }
  }
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
  let watch = client
    .watch(&video)
    .await
    .unwrap_or_else(|e| fail(&e.to_string()));
  let Some(token) = watch.comments_continuation else {
    fail("no comments");
  };
  let response = page(&client, &token).await;
  if let Some(file) = save {
    write(Path::new(&file), &response);
  }
  let comments = innertube::parse_comments(&response).comments;
  for comment in &comments {
    print(comment);
  }
  let Some(dir) = rate else { return };
  let Some(first) = comments.first() else {
    fail("no comment to rate");
  };
  let Some(actions) = first.actions.clone() else {
    fail("no actions: signed out?");
  };
  if first.rating != LikeStatus::Indifferent {
    fail("the first comment is already rated: rate nothing");
  }
  std::fs::create_dir_all(&dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
  let steps = [
    ("like", &actions.like, Some("page_liked")),
    ("dislike", &actions.dislike, Some("page_disliked")),
    ("undislike", &actions.undislike, None),
  ];
  for (name, action, then_read) in steps {
    let answer = client
      .rate_comment_response(action)
      .await
      .unwrap_or_else(|e| fail(&format!("{name}: {e}")));
    write(&dir.join(format!("{name}.json")), &answer);
    println!("{name}: {:?}", innertube::parse_comment_action(&answer));
    if let Some(file) = then_read {
      let response = page(&client, &token).await;
      write(&dir.join(format!("{file}.json")), &response);
      match innertube::parse_comments(&response)
        .comments
        .iter()
        .find(|c| c.id == first.id)
      {
        Some(comment) => print(comment),
        None => println!("{file}: the comment is not on the page"),
      }
    }
  }
}

async fn page(client: &Client, token: &str) -> Value {
  client
    .comments_response(token)
    .await
    .unwrap_or_else(|e| fail(&e.to_string()))
}

fn print(comment: &Comment) {
  println!(
    "{} · {} likes ({} liked) · {:?} · actions: {}",
    comment.id,
    comment.likes,
    comment.likes_liked,
    comment.rating,
    comment.actions.is_some()
  );
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
