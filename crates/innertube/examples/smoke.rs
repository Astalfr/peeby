//! Calls every endpoint once against YouTube and prints counts only (no titles, no cookies):
//! a quick check that the parsers still match YouTube's responses.
//!
//! `cargo run -p innertube --example smoke -- [cookies.txt]`

// An example prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use innertube::{Client, Cookies};

const VIDEO: &str = "dQw4w9WgXcQ";
const CHANNEL: &str = "UCuAXFkgsw1L7xaCfnd5JJOw";
const LIVE_QUERY: &str = "lofi hip hop radio";

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
      "history",
      client.history().await.map(|f| {
        format!(
          "{} videos · more: {}",
          f.items.len(),
          f.continuation.is_some()
        )
      }),
    );
    check(
      "subscribed_channels",
      client.subscribed_channels().await.map(|c| {
        let bells = c.iter().filter(|c| c.notifications.is_some()).count();
        format!("{} channels · {bells} bells", c.len())
      }),
    );
    let inbox = client.notifications().await;
    let inbox_next = inbox.as_ref().ok().and_then(|p| p.continuation.clone());
    check(
      "notifications",
      inbox.map(|p| {
        let count = p.sections.iter().map(|s| s.items.len()).sum::<usize>();
        format!("{} sections · {count} notifications", p.sections.len())
      }),
    );
    if let Some(token) = inbox_next {
      check(
        "notifications_more",
        client.notifications_more(&token).await.map(|p| {
          let count = p.sections.iter().map(|s| s.items.len()).sum::<usize>();
          format!("{count} notifications · more: {}", p.continuation.is_some())
        }),
      );
    }
    check(
      "unseen_notifications",
      client
        .unseen_notifications()
        .await
        .map(|n| format!("{n} unseen")),
    );
    let playlists = client.playlists().await;
    let first = playlists
      .as_ref()
      .ok()
      // "Liked videos" is usually long enough to have a next page.
      .and_then(|p| p.iter().find(|p| p.id == "LL").or(p.first()))
      .map(|p| p.id.clone());
    check(
      "playlists",
      playlists.map(|p| format!("{} playlists", p.len())),
    );
    if let Some(id) = first {
      let playlist = client.playlist(&id).await;
      let more = playlist.as_ref().ok().and_then(|p| p.continuation.clone());
      check(
        "playlist",
        playlist.map(|p| {
          format!(
            "{} videos · more: {}",
            p.videos.len(),
            p.continuation.is_some()
          )
        }),
      );
      if let Some(token) = more {
        check(
          "playlist_more",
          client.playlist_more(&token).await.map(|f| {
            format!(
              "{} videos · more: {}",
              f.items.len(),
              f.continuation.is_some()
            )
          }),
        );
      }
    }
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
  let search = client.search("rust programming").await;
  let search_next = search.as_ref().ok().and_then(|p| p.continuation.clone());
  let count = |results: &[innertube::SearchResult]| {
    let channels = results
      .iter()
      .filter(|r| matches!(r, innertube::SearchResult::Channel(_)))
      .count();
    format!("{} videos · {channels} channels", results.len() - channels)
  };
  check("search", search.map(|p| count(&p.results)));
  if let Some(token) = search_next {
    check(
      "search_more",
      client.search_more(&token).await.map(|p| count(&p.results)),
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
  // A live found by search (24/7 radios are always on): its window and its chat.
  let live = client.search(LIVE_QUERY).await.ok().and_then(|page| {
    page.results.into_iter().find_map(|r| match r {
      innertube::SearchResult::Video(v) if v.live => Some(v.id),
      _ => None,
    })
  });
  match live {
    None => check(
      "live",
      Err(innertube::Error::Parse {
        endpoint: "search",
        detail: format!("no live found for {LIVE_QUERY:?}"),
      }),
    ),
    Some(id) => {
      check(
        "live stream",
        client.stream(&id, 1080).await.map(|s| match s.live {
          Some(live) => format!(
            "window {:.0} s · DVR {:?}",
            live.length,
            live.dvr_window.map(|w| w.round())
          ),
          None => "not a live".into(),
        }),
      );
      let chat_token = client.watch(&id).await.map(|w| w.live_chat);
      match chat_token {
        Ok(Some(token)) => check(
          "live chat",
          client.live_chat(&token).await.map(|p| {
            format!(
              "{} actions · next in {} ms",
              p.actions.len(),
              p.poll_after.as_millis()
            )
          }),
        ),
        Ok(None) => check(
          "live chat",
          Err(innertube::Error::Parse {
            endpoint: "next",
            detail: "no live chat".into(),
          }),
        ),
        Err(e) => check("live chat", Err(e)),
      }
    }
  }
  if failures > 0 {
    std::process::exit(1);
  }
}

fn fail(message: &str) -> ! {
  eprintln!("error: {message}");
  std::process::exit(1)
}
