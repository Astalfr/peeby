//! Tests against a real Jellyfin server. Ignored by default.
//!
//! ```powershell
//! $env:PEEBY_TEST_SERVER = "https://jellyfin.example.org"
//! $env:PEEBY_TEST_USER = "…"; $env:PEEBY_TEST_PASSWORD = "…"
//! cargo test -p jellyfin-api --test live -- --ignored --nocapture
//! ```
//! Set `PEEBY_CAPTURE_FIXTURES=1` to (re)write the anonymized fixtures in `tests/fixtures/`.
//! Nothing secret is printed: only versions, counts and item kinds.

// Test helpers panic on purpose, and the live test reports what it saw.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

use std::path::PathBuf;

use jellyfin_api::models::{BaseItem, CollectionType, ItemKind};
use jellyfin_api::{Client, ClientInfo, ItemsQuery};
use serde_json::Value;

fn client() -> Client {
  let server = std::env::var("PEEBY_TEST_SERVER").expect("PEEBY_TEST_SERVER not set");
  let info = ClientInfo {
    client: "Peeby".into(),
    device: "peeby-tests".into(),
    device_id: "peeby-live-tests".into(),
    version: env!("CARGO_PKG_VERSION").into(),
  };
  Client::new(&server, info).expect("server URL")
}

fn credentials() -> (String, String) {
  let user = std::env::var("PEEBY_TEST_USER").expect("PEEBY_TEST_USER not set");
  let password = std::env::var("PEEBY_TEST_PASSWORD").expect("PEEBY_TEST_PASSWORD not set");
  (user, password)
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see the module docs"]
async fn login_browse_logout() {
  let mut client = client();
  let info = client.check_server().await.expect("server check");
  println!(
    "server version {:?}, supported {}",
    info.version,
    info.is_supported()
  );

  let (user, password) = credentials();
  client
    .authenticate_by_name(&user, &password)
    .await
    .expect("login");
  println!("logged in");

  let views = client.user_views().await.expect("views");
  println!("{} views", views.items.len());
  for v in &views.items {
    println!(
      "  view kind {:?}, collection {:?}",
      v.kind, v.collection_type
    );
  }

  let library = views
    .items
    .iter()
    .find(|v| v.collection_type.is_some_and(CollectionType::is_video))
    .expect("at least one video library");
  let query = ItemsQuery {
    parent_id: Some(library.id.clone()),
    include_item_types: vec![ItemKind::Movie, ItemKind::Series],
    recursive: true,
    sort_by: vec!["SortName".into()],
    limit: Some(20),
    fields: vec!["Overview".into(), "Genres".into()],
    ..Default::default()
  };
  let page = client.items(&query).await.expect("items");
  let with_blurhash = page
    .items
    .iter()
    .filter(|i| {
      i.blur_hash(jellyfin_api::models::ImageType::Primary)
        .is_some()
    })
    .count();
  println!(
    "library {:?}: {} of {} items, {} with a primary blurhash",
    library.collection_type,
    page.items.len(),
    page.total_record_count,
    with_blurhash
  );

  if let Some(first) = page.items.first() {
    let detail = client.item(&first.id).await.expect("item detail");
    println!(
      "detail ok: kind {:?}, has user data {}",
      detail.kind,
      detail.user_data.is_some()
    );
  }

  if std::env::var("PEEBY_CAPTURE_FIXTURES").as_deref() == Ok("1") {
    capture_fixtures(&client, &library.id.to_string()).await;
  }

  client.logout().await.expect("logout");
  println!("logged out");
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see the module docs"]
async fn wrong_password_is_rejected() {
  let mut client = client();
  let (user, _) = credentials();
  let err = client
    .authenticate_by_name(&user, "definitely-not-the-password")
    .await
    .unwrap_err();
  println!("rejected as expected: {err}");
  assert!(matches!(
    err,
    jellyfin_api::Error::Status { status: 401, .. }
  ));
}

/// Fetches raw responses and writes anonymized copies to `tests/fixtures/`.
async fn capture_fixtures(client: &Client, library_id: &str) {
  let session = client.session().expect("session");
  let user = session.user_id.to_string();
  let http = reqwest::Client::new();
  let get = |path: String, query: Vec<(&'static str, String)>| {
    let http = http.clone();
    let url = client.base_url().join(&path).expect("url");
    let auth = client.authorization_header();
    async move {
      let response = http
        .get(url)
        .header("Authorization", auth)
        .query(&query)
        .send()
        .await
        .expect("send");
      assert!(
        response.status().is_success(),
        "{path}: {}",
        response.status()
      );
      response.json::<Value>().await.expect("json")
    }
  };

  let fixtures = [
    (
      "public_system_info.json",
      get("System/Info/Public".into(), vec![]).await,
    ),
    (
      "user_views.json",
      get("UserViews".into(), vec![("userId", user.clone())]).await,
    ),
    (
      "items_library_page.json",
      get(
        "Items".into(),
        vec![
          ("userId", user.clone()),
          ("parentId", library_id.to_owned()),
          ("includeItemTypes", "Movie,Series".into()),
          ("recursive", "true".into()),
          ("sortBy", "SortName".into()),
          ("limit", "5".into()),
          ("fields", "Overview,Genres,PrimaryImageAspectRatio".into()),
          ("enableImageTypes", "Primary,Backdrop,Thumb,Logo".into()),
        ],
      )
      .await,
    ),
  ];

  let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
  std::fs::create_dir_all(&dir).expect("fixtures dir");
  for (name, mut value) in fixtures {
    anonymize(&mut value, &user, &mut 0);
    let text = serde_json::to_string_pretty(&value).expect("serialize");
    std::fs::write(dir.join(name), text + "\n").expect("write fixture");
    println!("wrote fixture {name}");
  }
}

/// Replaces anything personal: user/server ids, tokens, paths, addresses and titles.
fn anonymize(value: &mut Value, user_id: &str, counter: &mut u32) {
  match value {
    Value::Object(map) => {
      for (key, v) in map.iter_mut() {
        match key.as_str() {
          "AccessToken" | "LocalAddress" | "Path" | "Paths" => {
            *v = Value::String("<redacted>".into())
          }
          "ServerId" | "Id" if v.as_str() == Some(user_id) => {
            *v = Value::String("00000000000000000000000000000001".into())
          }
          "ServerId" => *v = Value::String("0000000000000000000000000000beef".into()),
          "ServerName" => *v = Value::String("Test Server".into()),
          "Name" | "SortName" | "OriginalTitle" | "SeriesName" | "ForcedSortName" => {
            *counter += 1;
            *v = Value::String(format!("Title {counter}"));
          }
          "Overview" | "Taglines" => *v = Value::String("Synopsis placeholder.".into()),
          _ => anonymize(v, user_id, counter),
        }
      }
    }
    Value::Array(items) => items
      .iter_mut()
      .for_each(|v| anonymize(v, user_id, counter)),
    Value::String(s) if s == user_id => *s = "00000000000000000000000000000001".into(),
    _ => {}
  }
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see the module docs"]
async fn home_rows_series_and_images() {
  use jellyfin_api::ImageOptions;
  use jellyfin_api::models::ImageType;

  let mut client = client();
  let (user, password) = credentials();
  client
    .authenticate_by_name(&user, &password)
    .await
    .expect("login");

  let resume = client.resume_items(12).await.expect("resume");
  let next_up = client
    .next_up(12, &Default::default())
    .await
    .expect("next up");
  println!(
    "resume {} items, next up {} items",
    resume.items.len(),
    next_up.items.len()
  );

  let views = client.user_views().await.expect("views");
  for view in &views.items {
    let latest = client.latest(&view.id, 12).await.expect("latest");
    println!(
      "latest in {:?}: {} items",
      view.collection_type,
      latest.len()
    );
  }

  let shows = views
    .items
    .iter()
    .find(|v| v.collection_type == Some(CollectionType::TvShows));
  if let Some(shows) = shows {
    let query = ItemsQuery {
      parent_id: Some(shows.id.clone()),
      include_item_types: vec![ItemKind::Series],
      recursive: true,
      limit: Some(1),
      ..Default::default()
    };
    let series = client.items(&query).await.expect("series").items.remove(0);
    let seasons = client.seasons(&series.id).await.expect("seasons");
    println!("series has {} seasons", seasons.items.len());
    if let Some(season) = seasons.items.first() {
      let episodes = client
        .episodes(&series.id, &season.id)
        .await
        .expect("episodes");
      println!("first season has {} episodes", episodes.items.len());
    }
    let url = client
      .image_url(
        &series.id,
        ImageType::Primary,
        &ImageOptions {
          max_width: Some(300),
          tag: series.image_tag(ImageType::Primary).map(Into::into),
          format: Some("Webp"),
          ..Default::default()
        },
      )
      .expect("image url");
    let bytes = client.fetch_bytes(&url).await.expect("image");
    println!(
      "poster: {} bytes, starts with {:?}",
      bytes.len(),
      &bytes[..12.min(bytes.len())]
    );
  }

  client.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see the module docs"]
async fn direct_play_negotiation() {
  use jellyfin_api::playback::StreamKind;

  let mut client = client();
  let (user, password) = credentials();
  client
    .authenticate_by_name(&user, &password)
    .await
    .expect("login");
  let views = client.user_views().await.expect("views");
  for view in views
    .items
    .iter()
    .filter(|v| v.collection_type.is_some_and(CollectionType::is_video))
  {
    let query = ItemsQuery {
      parent_id: Some(view.id.clone()),
      include_item_types: vec![ItemKind::Episode, ItemKind::Movie],
      recursive: true,
      limit: Some(3),
      exclude_missing: true,
      ..Default::default()
    };
    for item in client.items(&query).await.expect("items").items {
      let info = match client.playback_info(&item.id, 0).await {
        Ok(info) => info,
        Err(e) => {
          println!(
            "{:?} (runtime {:?}, has user data {}): {e}",
            item.kind,
            item.run_time_ticks,
            item.user_data.is_some()
          );
          continue;
        }
      };
      let source = &info.media_sources[0];
      let subs: Vec<_> = source
        .streams(StreamKind::Subtitle)
        .map(|s| {
          format!(
            "{}{}",
            s.codec.as_deref().unwrap_or("?"),
            if s.is_external { "(ext)" } else { "" }
          )
        })
        .collect();
      println!(
        "{:?} {:?}: direct play {} | {} audio | subs {:?} | {} font attachments | default audio {:?} sub {:?}",
        item.kind,
        source.container,
        source
          .ensure_direct_play()
          .map(|_| "OK".to_owned())
          .unwrap_or_else(|e| e.to_string()),
        source.streams(StreamKind::Audio).count(),
        subs,
        source
          .media_attachments
          .iter()
          .filter(|a| a.is_font())
          .count(),
        source.default_audio_stream_index,
        source.default_subtitle_stream_index,
      );
      source.ensure_direct_play().expect("direct play");
      let url = client
        .stream_url(&item.id, source, info.play_session_id.as_deref())
        .expect("url");
      assert!(url.query().unwrap_or_default().contains("static=true"));
    }
  }
  client.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see the module docs"]
async fn playback_extras() {
  let mut client = client();
  let (user, password) = credentials();
  client
    .authenticate_by_name(&user, &password)
    .await
    .expect("login");
  let query = ItemsQuery {
    include_item_types: vec![ItemKind::Episode],
    recursive: true,
    limit: Some(10),
    exclude_missing: true,
    sort_by: vec!["Random".into()],
    ..Default::default()
  };
  for episode in client.items(&query).await.expect("items").items {
    let item = client.item(&episode.id).await.expect("item");
    let segments = client.media_segments(&episode.id).await.expect("segments");
    let adjacent = match &episode.series_id {
      Some(series) => client
        .adjacent_episodes(series, &episode.id)
        .await
        .expect("adjacent episodes"),
      None => Default::default(),
    };
    let number = |e: &Option<BaseItem>| {
      (
        e.as_ref().and_then(|e| e.parent_index_number),
        e.as_ref().and_then(|e| e.index_number),
      )
    };
    println!(
      "S{:?}E{:?}: {} segments {:?} | trickplay sources {} | previous {:?} | next {:?}",
      episode.parent_index_number,
      episode.index_number,
      segments.len(),
      segments.iter().map(|s| s.kind).collect::<Vec<_>>(),
      item.trickplay.as_ref().map_or(0, |t| t.len()),
      number(&adjacent.previous),
      number(&adjacent.next),
    );
    for side in [&adjacent.previous, &adjacent.next].into_iter().flatten() {
      assert_ne!(side.id, episode.id);
    }
    if let (Some(previous), Some(next)) = (&adjacent.previous, &adjacent.next) {
      assert_ne!(previous.id, next.id);
    }
  }

  // Walking back from a random episode: each previous episode has the current one as its next,
  // and the walk ends on a first episode whose previous is none.
  let start = client
    .items(&ItemsQuery {
      include_item_types: vec![ItemKind::Episode],
      recursive: true,
      limit: Some(1),
      exclude_missing: true,
      sort_by: vec!["Random".into()],
      ..Default::default()
    })
    .await
    .expect("items")
    .items;
  if let Some(mut current) = start.into_iter().next()
    && let Some(series) = current.series_id.clone()
  {
    for step in 0..40 {
      let adjacent = client
        .adjacent_episodes(&series, &current.id)
        .await
        .expect("adjacent episodes");
      let Some(previous) = adjacent.previous else {
        println!(
          "first episode S{:?}E{:?} after {step} steps",
          current.parent_index_number, current.index_number
        );
        break;
      };
      let back = client
        .adjacent_episodes(&series, &previous.id)
        .await
        .expect("adjacent episodes");
      assert_eq!(back.next.map(|n| n.id), Some(current.id.clone()));
      current = previous;
    }
  }
  client.logout().await.expect("logout");
}
