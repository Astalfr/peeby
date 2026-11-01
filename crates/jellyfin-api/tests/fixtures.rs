//! Deserialization tests on anonymized responses captured from a Jellyfin 12.3 server
//! (regenerate with `PEEBY_CAPTURE_FIXTURES=1`, see `tests/live.rs`).

// Test helpers panic on purpose.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use jellyfin_api::models::{
  BaseItem, CollectionType, ImageType, ItemKind, PublicSystemInfo, QueryResult,
};

fn load<T: serde::de::DeserializeOwned>(name: &str) -> T {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
  serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn public_system_info() {
  let info: PublicSystemInfo = load("public_system_info.json");
  assert_eq!(info.major_version(), Some(12));
  assert!(info.is_supported());
  assert_eq!(info.startup_wizard_completed, Some(true));
}

#[test]
fn user_views() {
  let views: QueryResult<BaseItem> = load("user_views.json");
  assert!(!views.items.is_empty());
  assert!(
    views
      .items
      .iter()
      .all(|v| v.kind == ItemKind::CollectionFolder)
  );
  assert!(
    views
      .items
      .iter()
      .any(|v| v.collection_type == Some(CollectionType::Movies))
  );
  assert!(
    views
      .items
      .iter()
      .any(|v| v.collection_type == Some(CollectionType::TvShows))
  );
}

#[test]
fn library_page() {
  let page: QueryResult<BaseItem> = load("items_library_page.json");
  assert!(page.total_record_count >= page.items.len() as u32);
  assert!(!page.items.is_empty());
  for item in &page.items {
    assert!(matches!(item.kind, ItemKind::Series | ItemKind::Movie));
    assert!(
      item.image_tag(ImageType::Primary).is_some(),
      "primary image tag"
    );
    assert!(
      item.blur_hash(ImageType::Primary).is_some(),
      "primary blurhash"
    );
    assert!(item.user_data.is_some(), "user data");
  }
}
