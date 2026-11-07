//! Parsers against real InnerTube responses (tests/fixtures/, captured 2026-11-01, fr/FR,
//! public IP and signatures redacted).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use innertube::{
  ChatAction, ChatBadgeKind, ChatMessage, ChatRun, Error, LikeStatus, LinkTarget, NotificationKind,
  NotificationLevel, Progress, Related, SearchResult, Watchtime, hls, hls_manifest_url, is_mix,
  parse_channel_search, parse_feed, parse_members_only, parse_mix, parse_mixes, parse_oembed_title,
  parse_playlist, parse_playlist_choices, parse_playlist_page, parse_playlists, parse_search,
  parse_shorts_seed, parse_shorts_sequence, parse_tracking, parse_watch,
};
use serde_json::Value;

fn fixture(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn anonymous_home_is_empty_with_a_message() {
  let feed = parse_feed(&fixture("home_anonymous.json"));
  assert!(!feed.logged_in);
  assert!(feed.items.is_empty());
  assert_eq!(
    feed.message.as_deref(),
    Some("Votre historique YouTube est désactivé")
  );
}

#[test]
fn channel_videos_grid() {
  let feed = parse_feed(&fixture("channel_videos.json"));
  assert_eq!(feed.items.len(), 30);
  assert!(feed.continuation.is_some());
  let first = &feed.items[0];
  assert_eq!(first.id, "PXC_PYeB6F8");
  assert_eq!(
    first.title,
    "Rick Astley - Angels On My Side (Live at The O2, London, April 2026)"
  );
  assert_eq!(first.duration.as_deref(), Some("3:51"));
  // A channel tab has a single views · date row and no channel row.
  assert_eq!(first.channel, None);
  assert_eq!(first.metadata.len(), 2);
  assert!(first.metadata[1].starts_with("il y a"));
  assert!(first.thumbnails.iter().any(|t| t.width >= 720));
  assert!(feed.items.iter().all(|v| v.id.len() == 11));
  assert!(feed.items.iter().all(|v| !v.live));
}

#[test]
fn channel_videos_sort_chips() {
  let feed = parse_feed(&fixture("channel_videos.json"));
  let labels: Vec<_> = feed.chips.iter().map(|c| c.label.as_str()).collect();
  assert_eq!(
    labels,
    ["Les plus récentes", "Populaires", "Les plus anciennes"]
  );
  assert!(feed.chips[0].selected && !feed.chips[1].selected);
  assert!(feed.chips.iter().all(|c| !c.token.is_empty()));
}

#[test]
fn channel_videos_by_popularity() {
  // The answer to the "Populaires" chip, captured 2026-11-03, trimmed to three videos: it
  // replaces the grid, its own chips showing the new order.
  let feed = parse_feed(&fixture("channel_videos_popular.json"));
  let ids: Vec<_> = feed.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["dQw4w9WgXcQ", "yPYZpwSpKmA", "eYuUAGXN0KM"]);
  assert!(feed.continuation.is_some());
  let selected: Vec<_> = feed.chips.iter().filter(|c| c.selected).collect();
  assert_eq!(selected.len(), 1);
  assert_eq!(selected[0].label, "Populaires");
}

#[test]
fn channel_search_results() {
  // Rick Astley's channel searched for "never", captured 2026-11-03, trimmed to three videos.
  let feed = parse_channel_search(&fixture("channel_search.json"));
  let ids: Vec<_> = feed.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["dQw4w9WgXcQ", "RrESvSRNpeo", "DD70oKDlemE"]);
  let first = &feed.items[0];
  assert_eq!(first.duration.as_deref(), Some("3:34"));
  assert!(!first.metadata.is_empty());
  assert!(feed.chips.is_empty());
  assert!(feed.continuation.is_some());

  let more = parse_channel_search(&fixture("channel_search_continuation.json"));
  assert_eq!(more.items.len(), 2);
  assert!(more.continuation.is_some());
}

#[test]
fn channel_streams_are_live() {
  // Lofi Girl's "En direct" tab, captured 2026-11-02, trimmed to its first three lives.
  let feed = parse_feed(&fixture("channel_streams.json"));
  assert_eq!(feed.items.len(), 3);
  assert_eq!(feed.items[0].id, "0sfDth4lMLc");
  assert!(feed.items.iter().all(|v| v.live && v.duration.is_none()));
}

#[test]
fn members_only_cards() {
  // Okayu Ch.'s "En direct" tab, captured 2026-11-06 anonymously, trimmed to a public past live
  // and a members-only one, whose badge is a metadata row of its own after views · date.
  let feed = parse_feed(&fixture("channel_streams_members.json"));
  assert_eq!(feed.items.len(), 2);
  let (public, members) = (&feed.items[0], &feed.items[1]);
  assert_eq!(public.id, "U1x0E4tXVk4");
  assert!(!public.members_only);
  assert_eq!(members.id, "0XfSSfCjakE");
  assert!(members.members_only);
  // The badge row is not taken for the channel's row on the channel's own tab.
  assert_eq!(members.channel, None);
  assert_eq!(members.metadata, ["Diffusé il y a 1 mois"]);
  assert!(members.duration.is_some());
}

#[test]
fn members_only_player() {
  // WEB `player` of a members-only video, anonymous, captured 2026-11-06: the membership offer.
  let player = fixture("player_web_members_only.json");
  assert!(parse_members_only(&player));
  assert!(matches!(
    hls_manifest_url(&player),
    Err(Error::Unplayable { ref status, .. }) if status == "UNPLAYABLE"
  ));
  assert!(!parse_members_only(&fixture("player_login_required.json")));
  assert!(!parse_members_only(&fixture("player_web_signed_in.json")));
}

#[test]
fn watch_page() {
  let page = parse_watch("dQw4w9WgXcQ", &fixture("next.json"));
  assert_eq!(
    page.title,
    "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
  );
  assert_eq!(page.channel.as_deref(), Some("Rick Astley"));
  assert_eq!(page.channel_id.as_deref(), Some("UCuAXFkgsw1L7xaCfnd5JJOw"));
  assert!(page.subscribers.is_some());
  assert!(page.views.as_deref().is_some_and(|v| v.ends_with("vues")));
  assert!(page.description.starts_with("The official video"));
  assert!(page.comments_continuation.is_some());
  assert!(page.related_continuation.is_some());
  // Signed out: no rating, the endpoints still carry their params (inside the sign-in modal).
  assert_eq!(page.like_status, LikeStatus::Indifferent);
  for status in [
    LikeStatus::Like,
    LikeStatus::Dislike,
    LikeStatus::Indifferent,
  ] {
    assert!(page.like_params.get(status).is_some(), "{status:?}");
  }
  // Signed out, not subscribed: the button still carries its subscribe params and a bell for
  // after a subscription.
  let subscription = page.subscription.as_ref().unwrap();
  assert!(!subscription.subscribed);
  assert!(subscription.params(true).is_some());
  assert!(page.notifications.is_some());

  assert_eq!(page.related.len(), 20);
  let Related::Video(related) = &page.related[0] else {
    panic!("a video first");
  };
  // Related videos carry the channel row, then views · date.
  assert_eq!(related.channel.as_deref(), Some("Retro Pulse"));
  assert!(
    related
      .channel_id
      .as_deref()
      .is_some_and(|id| id.starts_with("UC"))
  );
  assert_eq!(related.metadata.len(), 2);
  assert!(!related.avatar.is_empty());
}

#[test]
fn subscribe_answers_carry_the_new_state() {
  let channel = "UCuAXFkgsw1L7xaCfnd5JJOw";
  let subscribe = fixture("subscription_subscribe.json");
  assert_eq!(innertube::parse_subscribed(&subscribe, channel), Some(true));
  assert_eq!(innertube::parse_subscribed(&subscribe, "UCother"), None);
  let unsubscribe = fixture("subscription_unsubscribe.json");
  assert_eq!(
    innertube::parse_subscribed(&unsubscribe, channel),
    Some(false)
  );
}

#[test]
fn signed_in_watch_page_has_the_channel_bell() {
  let page = parse_watch("0-k6i8tBwAU", &fixture("next_signed_in.json"));
  assert_eq!(page.channel_id.as_deref(), Some("UCQeRaTukNYft1_6AZPACnog"));
  // The like button's count, liked or not (YouTube rounds both the same here).
  assert_eq!(page.likes.as_deref(), Some("4,2\u{a0}k"));
  assert_eq!(page.likes_liked.as_deref(), Some("4,2\u{a0}k"));
  let subscription = page.subscription.as_ref().unwrap();
  assert!(subscription.subscribed);
  assert_eq!(
    subscription.params(true),
    Some("EgIIAxgAIgswLWs2aTh0QndBVQ%3D%3D")
  );
  // The unsubscribe params sit in the confirmation dialog.
  assert_eq!(
    subscription.params(false),
    Some("CgIIAxILMC1rNmk4dEJ3QVUYAA%3D%3D")
  );
  let bell = page.notifications.unwrap();
  assert_eq!(bell.level, NotificationLevel::All);
  let levels = bell.options.iter().map(|(l, _)| *l).collect::<Vec<_>>();
  assert_eq!(
    levels,
    [
      NotificationLevel::All,
      NotificationLevel::Personalized,
      NotificationLevel::None
    ]
  );
  // Params as the menu sends them, still URL-encoded.
  assert!(
    bell
      .params(NotificationLevel::None)
      .unwrap()
      .ends_with("%3D")
  );
}

#[test]
fn visionos_player_and_master_playlist() {
  let player = fixture("player_visionos.json");
  let url = hls_manifest_url(&player).unwrap();
  assert!(url.starts_with("https://manifest.googlevideo.com/api/manifest/hls_variant/"));

  let path = format!(
    "{}/tests/fixtures/hls_master.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let master = std::fs::read_to_string(path).unwrap();
  let best = hls::best_variant(&master, 1080).unwrap();
  assert_eq!(best.resolution, "1920x1080");
  assert!(best.codecs.starts_with("vp09"));
  let audio = best.audio_url.unwrap();
  assert!(audio.contains("/itag/234/"));
  let small = hls::best_variant(&master, 360).unwrap();
  assert_eq!(small.resolution, "640x360");
}

#[test]
fn login_required_is_unplayable() {
  let err = hls_manifest_url(&fixture("player_login_required.json")).unwrap_err();
  assert!(matches!(err, Error::Unplayable { ref status, .. } if status == "LOGIN_REQUIRED"));
}

#[test]
fn feed_continuation() {
  // Continuations carry `logged_in` too, so the signed-in check applies to every page.
  let feed = parse_feed(&fixture("channel_videos_continuation.json"));
  assert!(!feed.logged_in);
  assert_eq!(feed.items.len(), 30);
  assert!(feed.continuation.is_some());
  assert!(
    feed
      .items
      .iter()
      .all(|v| v.id.len() == 11 && !v.title.is_empty())
  );
}

#[test]
fn subscriptions_feed() {
  // Signed-in response, trimmed to 4 videos, account data removed.
  let feed = parse_feed(&fixture("subscriptions_feed.json"));
  assert!(feed.logged_in);
  assert_eq!(feed.items.len(), 4);
  assert!(feed.continuation.is_some());
  assert!(
    feed
      .items
      .iter()
      .all(|v| v.channel.is_some() && v.channel_id.is_some())
  );
}

#[test]
fn subscriptions_feed_skips_shelves() {
  // Signed-in response, trimmed and stripped of account data: the "Les plus pertinentes" shelf
  // (2 videos out of date order, repeated further down), a live, a video, the Shorts shelf,
  // then 2 videos.
  let response = fixture("subscriptions_feed_shelves.json");
  let feed = innertube::parse_subscriptions(&response);
  assert!(feed.logged_in);
  assert!(feed.continuation.is_some());
  let ids: Vec<_> = feed.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(
    ids,
    ["dGNhfO8t7hM", "TWM1kc5e2dI", "nkW7MtSZBfg", "MGETw9KdRAs"]
  );
  // The generic parser keeps the shelf's videos.
  assert_eq!(parse_feed(&response).items[0].id, "Jl_8B_KGkZU");
}

#[test]
fn subscribed_channels() {
  let channels = innertube::parse_channels(&fixture("subscriptions_channels.json"));
  assert_eq!(channels.len(), 3);
  let first = &channels[0];
  assert!(first.id.starts_with("UC"));
  assert!(!first.name.is_empty() && first.name == first.name.trim());
  assert!(first.handle.as_deref().is_some_and(|h| h.starts_with('@')));
  assert!(
    first
      .subscribers
      .as_deref()
      .is_some_and(|s| s.contains("abonnés"))
  );
  assert!(first.avatar.iter().all(|a| a.url.starts_with("https://")));
  // Bells: the first channel sends every notification, the others are personalized. The menu's
  // "Se désabonner" is not a level.
  let levels = channels
    .iter()
    .map(|c| c.notifications.as_ref().map(|n| n.level))
    .collect::<Vec<_>>();
  assert_eq!(
    levels,
    [
      Some(NotificationLevel::All),
      Some(NotificationLevel::Personalized),
      Some(NotificationLevel::Personalized)
    ]
  );
  for channel in &channels {
    let bell = channel.notifications.as_ref().unwrap();
    assert_eq!(bell.options.len(), 3);
    assert!(bell.options.iter().all(|(_, params)| !params.is_empty()));
  }
}

#[test]
fn comments_page() {
  let page = innertube::parse_comments(&fixture("comments.json"));
  assert_eq!(page.comments.len(), 20);
  assert!(page.continuation.is_some());
  let first = &page.comments[0];
  assert_eq!(first.author, "@bdr8823");
  assert_eq!(
    first.text,
    "My algebra teacher said that this was a link to the test answers"
  );
  assert_eq!(first.published, "il y a 7 ans");
  assert!(!first.likes.is_empty());
  assert!(first.replies_continuation.is_some());
}

#[test]
fn comment_replies() {
  // Replies pages list `commentViewModel`s directly, then "Afficher plus de réponses".
  let page = innertube::parse_comments(&fixture("comment_replies.json"));
  assert_eq!(page.comments.len(), 10);
  assert!(page.continuation.is_some());
  assert_eq!(page.count, None);
  assert_eq!(page.comments[0].author, "@daikerio6830");
  assert!(
    page
      .comments
      .iter()
      .all(|c| c.replies_continuation.is_none())
  );
}

#[test]
fn comment_links() {
  // A page trimmed to three comments (captured 2026-11-03, anonymous): a link, timestamps,
  // plain text.
  let page = innertube::parse_comments(&fixture("comments_links.json"));
  assert_eq!(page.comments.len(), 3);
  let link = page
    .comments
    .iter()
    .find(|c| c.text.contains("simulator.dev"))
    .unwrap();
  // Out of YouTube's redirect.
  assert_eq!(link.links.len(), 1);
  assert_eq!(
    link.links[0].target,
    LinkTarget::Url("https://simulator.dev/".into())
  );
  assert_eq!(
    &link.text[link.links[0].range.clone()],
    "https://simulator.dev/"
  );
  let stamps = page
    .comments
    .iter()
    .find(|c| c.text.starts_with("0:00"))
    .unwrap();
  let first = &stamps.links[0];
  assert_eq!(&stamps.text[first.range.clone()], "0:00");
  // `0:00` has no `startTimeSeconds`.
  assert_eq!(
    first.target,
    LinkTarget::Video {
      id: "rfscVS0vtbw".into(),
      start: None
    }
  );
  let second = &stamps.links[1];
  assert_eq!(&stamps.text[second.range.clone()], "1:45");
  assert_eq!(
    second.target,
    LinkTarget::Video {
      id: "rfscVS0vtbw".into(),
      start: Some(105)
    }
  );
  assert!(page.comments.iter().any(|c| c.links.is_empty()));
}

#[test]
fn description_links() {
  // An emoji (two UTF-16 units) comes before the first link: ranges are bytes of the text.
  let page = parse_watch("dQw4w9WgXcQ", &fixture("next.json"));
  let texts: Vec<&str> = page
    .description_links
    .iter()
    .map(|l| &page.description[l.range.clone()])
    .collect();
  assert_eq!(texts.len(), 16);
  assert_eq!(texts[0], "#RickAstleyNever");
  assert_eq!(texts[1], "https://linktr.ee/rickastleynever");
  assert_eq!(
    page.description_links[0].target,
    LinkTarget::Page("https://www.youtube.com/hashtag/rickastleynever".into())
  );
  assert_eq!(
    page.description_links[1].target,
    LinkTarget::Url("https://linktr.ee/rickastleynever".into())
  );

  // Chips: a channel and a playlist, named by their text.
  let live = parse_watch("bygxS5WGoKE", &fixture("next_live.json"));
  let targets: Vec<&LinkTarget> = live.description_links.iter().map(|l| &l.target).collect();
  assert!(targets.contains(&&LinkTarget::Channel {
    id: "UC_CDTi963WuQ99TnPdCF_Gg".into(),
    name: "@chrystalghost".into()
  }));
  assert!(targets.contains(&&LinkTarget::Playlist {
    id: "PLLnnKogt5Lcs75NirFznyenILhpFJxrIm".into(),
    title: "Livestreams".into()
  }));

  // Another video.
  let signed_in = parse_watch("0-k6i8tBwAU", &fixture("next_signed_in.json"));
  assert_eq!(
    signed_in.description_links[0].target,
    LinkTarget::Video {
      id: "zqDJYN5qppQ".into(),
      start: Some(0)
    }
  );
}

#[test]
fn dubbed_video_plays_original_audio() {
  let path = format!(
    "{}/tests/fixtures/hls_master_dubbed.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let master = std::fs::read_to_string(path).unwrap();
  let ladder = hls::ladder(&master);
  assert!(!ladder.is_empty());
  for variant in &ladder {
    let audio = variant.audio_url.as_deref().unwrap();
    assert!(audio.contains("acont%3Doriginal"), "{}", variant.label());
  }
}

#[test]
fn quality_ladder() {
  let path = format!(
    "{}/tests/fixtures/hls_master.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let master = std::fs::read_to_string(path).unwrap();
  let ladder = hls::ladder(&master);
  let heights: Vec<u32> = ladder.iter().map(|v| v.height).collect();
  assert_eq!(heights, [2160, 1440, 1080, 720, 480, 360, 240, 144]);
  assert_eq!(ladder[2].label(), "1080p60");
  assert!(ladder[2].codecs.starts_with("vp09"));
  assert!(ladder.iter().all(|v| v.audio_url.is_some()));
}

#[test]
fn captions() {
  let captions = innertube::parse_captions(&fixture("player_captions.json"));
  assert_eq!(captions.len(), 6);
  assert_eq!(captions[1].language, "en");
  assert_eq!(captions[1].name, "Anglais");
  assert!(captions[2].auto_generated);
  assert!(captions.iter().all(|c| c.url.ends_with("&fmt=vtt")));
}

#[test]
fn playlists_library() {
  let playlists = parse_playlists(&fixture("playlists_library.json"));
  let ids: Vec<&str> = playlists.iter().map(|p| p.id.as_str()).collect();
  assert_eq!(
    ids,
    [
      "WL",
      "LL",
      "PLREDACTEDplaylist01xxxxxxxxxxxxxx",
      "PLREDACTEDplaylist02xxxxxxxxxxxxxx"
    ]
  );
  let watch_later = &playlists[0];
  assert_eq!(watch_later.title, "À regarder plus tard");
  assert_eq!(watch_later.count.as_deref(), Some("17\u{a0}vidéos"));
  // The "Afficher la playlist complète" link row is left out.
  assert_eq!(
    watch_later.metadata,
    ["Privée", "Playlist", "Mise à jour aujourd'hui"]
  );
  assert!(!watch_later.thumbnails.is_empty());
  assert_eq!(
    playlists[1].count.as_deref(),
    Some("5\u{202f}000\u{a0}vidéos")
  );
}

#[test]
fn watch_later_uses_playlist_video_renderers() {
  let playlist = parse_playlist("WL", &fixture("playlist_watch_later.json"));
  assert!(playlist.title.starts_with("À regarder plus tard"));
  assert_eq!(playlist.count.as_deref(), Some("17\u{a0}vidéos"));
  assert_eq!(playlist.videos.len(), 3);
  let first = &playlist.videos[0];
  assert_eq!(first.id, "tcqah8v8qmo");
  assert_eq!(first.title, "Activision is genuinely desperate now..");
  assert_eq!(first.channel.as_deref(), Some("SmugAlana"));
  assert_eq!(first.duration.as_deref(), Some("23:37"));
  // The " • " separator run is dropped.
  assert_eq!(first.metadata, ["163\u{a0}k\u{a0}vues", "il y a 7 jours"]);
  // "Watch later" can be edited: each entry has its own id for removal.
  assert!(playlist.videos.iter().all(|v| v.set_video_id.is_some()));
  assert_eq!(playlist.continuation, None);
}

#[test]
fn public_playlist_header_and_next_videos_token() {
  let playlist = parse_playlist(
    "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
    &fixture("playlist_public.json"),
  );
  assert!(!playlist.title.is_empty());
  assert!(playlist.owner.is_some());
  assert!(
    playlist
      .count
      .as_deref()
      .is_some_and(|c| c.chars().any(|ch| ch.is_ascii_digit()))
  );
  let ids: Vec<&str> = playlist.videos.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["fOT0BUpITw8", "NFvDHYMzj9U", "8c7KOGxeY3w"]);
  // Another channel's playlist: nothing to remove.
  assert!(playlist.videos.iter().all(|v| v.set_video_id.is_none()));
  // Two continuation items on the page: the one after the videos is the next videos.
  let token = playlist.continuation.unwrap();
  assert!(token.starts_with("4qmFsgKBARIkVkxQTEZncXVM"), "{token}");
}

#[test]
fn home_mixes() {
  // A signed-in home page, captured 2026-11-07, trimmed to a video, a mix and the
  // continuation.
  let response = fixture("home_signed_in_mix.json");
  let mixes = parse_mixes(&response);
  assert_eq!(mixes.len(), 1);
  let mix = &mixes[0];
  assert_eq!(mix.id, "RDVUmovBRfVQI");
  assert!(is_mix(&mix.id));
  assert!(mix.title.starts_with("Mix\u{a0}- "));
  // Its badge reads "Mix" instead of a count; its first row lists artists.
  assert_eq!(mix.count.as_deref(), Some("Mix"));
  assert_eq!(mix.metadata.last().map(String::as_str), Some("Playlist"));
  assert!(!mix.thumbnails.is_empty());
  // The feed keeps the video, and the mix after it.
  let feed = parse_feed(&response);
  assert_eq!(feed.items.len(), 1);
  assert_eq!(feed.playlists.len(), 1);
  assert_eq!(feed.playlists[0].0, 1);
  assert_eq!(feed.playlists[0].1.id, "RDVUmovBRfVQI");
  assert!(feed.continuation.is_some());
}

#[test]
fn related_mix() {
  // Despacito's related list, anonymous, captured 2026-11-07, trimmed to a video and its mix.
  let page = parse_watch("kJQP7kiw5Fk", &fixture("next_related_mix.json"));
  assert_eq!(page.related.len(), 2);
  assert!(matches!(page.related[0], Related::Video(_)));
  let Related::Playlist(mix) = &page.related[1] else {
    panic!("the mix after the video");
  };
  assert_eq!(mix.id, "RDkJQP7kiw5Fk");
  assert!(mix.title.starts_with("Mix\u{a0}- "));
  assert!(!mix.thumbnails.is_empty());
}

#[test]
fn search_mix() {
  // A search for "despacito", anonymous, captured 2026-11-07, trimmed to the first video and
  // the mix.
  let page = parse_search(&fixture("search_mix.json"));
  assert_eq!(page.results.len(), 2);
  assert!(matches!(page.results[0], SearchResult::Video(_)));
  let SearchResult::Playlist(mix) = &page.results[1] else {
    panic!("the mix after the video");
  };
  assert_eq!(mix.id, "RDkJQP7kiw5Fk");
  assert!(mix.title.starts_with("Mix\u{a0}- "));
}

#[test]
fn mix_videos() {
  // `next` with a mix's `playlistId` alone, anonymous, captured 2026-11-07, trimmed to three
  // videos.
  let mix = parse_mix("RDkJQP7kiw5Fk", &fixture("next_mix.json"));
  assert_eq!(
    mix.title,
    "Mix\u{a0}- Luis Fonsi - Despacito ft. Daddy Yankee"
  );
  assert_eq!(mix.owner.as_deref(), Some("YouTube"));
  assert!(mix.continuation.is_none());
  let ids: Vec<&str> = mix.videos.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["NUsoVlDFqZg", "TyHvyGVs42U", "6Of73C7vH54"]);
  let first = &mix.videos[0];
  assert_eq!(
    first.title,
    "Enrique Iglesias - Bailando ft. Descemer Bueno, Gente De Zona"
  );
  assert_eq!(first.channel.as_deref(), Some("Enrique Iglesias"));
  assert!(
    first
      .channel_id
      .as_deref()
      .is_some_and(|id| id.starts_with("UC"))
  );
  assert_eq!(first.duration.as_deref(), Some("4:47"));
  assert!(first.set_video_id.is_none());
  // Its cover is its first video's picture.
  assert_eq!(mix.thumbnails, first.thumbnails);
}

#[test]
fn playlist_continuation_is_the_last_page() {
  let page = parse_playlist_page(&fixture("playlist_public_continuation.json"));
  assert!(page.logged_in);
  let ids: Vec<&str> = page.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["1STyaZA7-BY", "Ke9A6XV2H38", "_db-mfu8y1k"]);
  assert_eq!(page.continuation, None);
}

#[test]
fn unplayable_playlist_entries_are_skipped() {
  let response: Value = serde_json::from_str(
    r#"{"contents": [
      {"playlistVideoRenderer": {"videoId": "aaaaaaaaaaa", "isPlayable": false,
        "title": {"runs": [{"text": "[Vidéo supprimée]"}]}}},
      {"playlistVideoRenderer": {"videoId": "bbbbbbbbbbb", "isPlayable": true,
        "title": {"runs": [{"text": "Une vidéo"}]}}}
    ]}"#,
  )
  .unwrap();
  let page = parse_playlist_page(&response);
  let ids: Vec<&str> = page.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["bbbbbbbbbbb"]);
}

#[test]
fn playlist_choices() {
  // Asked for a video that is in "Watch later" only.
  let choices = parse_playlist_choices(&fixture("add_to_playlist.json"));
  assert_eq!(choices.len(), 7);
  assert_eq!(choices[0].id, "WL");
  assert_eq!(choices[0].title, "À regarder plus tard");
  assert!(choices[0].contains && choices[0].private);
  assert_eq!(choices[2].title, "Ma playlist 1");
  assert!(!choices[2].contains && !choices[2].private);
  assert!(choices[1..].iter().all(|c| !c.contains));
}

#[test]
fn notifications_inbox() {
  let page = innertube::parse_notifications(&fixture("notifications_inbox.json"));
  let titles = page
    .sections
    .iter()
    .map(|s| s.title.as_str())
    .collect::<Vec<_>>();
  assert_eq!(titles, ["Important", "Autres notifications"]);
  assert_eq!(page.sections[0].items.len(), 2);
  assert_eq!(page.sections[1].items.len(), 3);
  assert!(page.continuation.is_some());

  let first = &page.sections[0].items[0];
  assert_eq!(first.video_id.as_deref(), Some("0-k6i8tBwAU"));
  assert!(!first.read && !first.short);
  // The channel's run ends with spaces: collapsed.
  assert!(first.message.starts_with("Asmongold TV a mis en ligne "));
  assert!(first.sent.starts_with("il y a"));
  assert!(!first.avatar.is_empty() && !first.thumbnails.is_empty());
  assert!(first.read_token.is_some());

  let others = &page.sections[1].items;
  assert!(others[0].read);
  // A Short opens through `reelWatchEndpoint`.
  assert!(others[1].short && others[1].video_id.is_some());
  // Every one announces an upload, a Short's too.
  assert!(
    page
      .sections
      .iter()
      .flat_map(|s| &s.items)
      .all(|n| n.kind == NotificationKind::Upload)
  );
}

#[test]
fn notifications_next_page_is_one_untitled_section() {
  let page = innertube::parse_notifications(&fixture("notifications_inbox_continuation.json"));
  assert_eq!(page.sections.len(), 1);
  assert_eq!(page.sections[0].title, "");
  assert_eq!(page.sections[0].items.len(), 2);
  assert!(page.continuation.is_some());
  // `… est en direct : …`, then an upload, told apart by their tracking label.
  let items = &page.sections[0].items;
  assert!(items[0].message.contains("en direct"));
  assert_eq!(items[0].kind, NotificationKind::Live);
  assert_eq!(items[1].kind, NotificationKind::Upload);
}

#[test]
fn unseen_notifications_count() {
  assert_eq!(
    innertube::parse_unseen_count(&fixture("notifications_unseen.json")),
    Some(0)
  );
  assert_eq!(innertube::parse_unseen_count(&fixture("next.json")), None);
}

#[test]
fn live_player_and_media_playlist() {
  // Lofi Girl's live, captured 2026-11-02; its media playlist trimmed to 8 segments.
  let player = fixture("player_visionos_live.json");
  assert!(hls_manifest_url(&player).is_ok());
  let path = format!(
    "{}/tests/fixtures/hls_live_media.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let media = std::fs::read_to_string(path).unwrap();
  let length = hls::playlist_length(&media);
  assert!((length - 40.0).abs() < 1e-9);
  let live = innertube::parse_live(&player, length).unwrap();
  assert_eq!(live.length, length);
  // DVR: an hour back (`playlist_duration/3600`), whatever the window covers now.
  assert_eq!(live.dvr_window, Some(3600.0));
  // A full window measures a little less: the measure wins.
  let full = innertube::parse_live(&player, 3599.9).unwrap();
  assert_eq!(full.dvr_window, Some(3599.9));
  // A live's storyboard is another kind (`playerLiveStoryboardSpecRenderer`): no seek previews.
  assert!(innertube::parse_storyboard(&player).is_none());
  // A video is not a live.
  assert!(innertube::parse_live(&fixture("player_visionos.json"), 0.0).is_none());
}

#[test]
fn live_dash_formats() {
  // The same live's `adaptiveFormats`: AVC up to 1080p and two AACs, 5 s segments, 12 h back.
  let formats = innertube::parse_live_formats(&fixture("player_visionos_live.json"));
  let itags: Vec<u32> = formats.iter().map(|f| f.itag).collect();
  assert_eq!(itags, [137, 136, 135, 134, 133, 160, 139, 140]);
  let best = &formats[0];
  assert_eq!(
    (best.mime.as_str(), best.codecs.as_str()),
    ("video/mp4", "avc1.640028")
  );
  assert_eq!((best.width, best.height, best.fps), (1920, 1080, Some(30)));
  assert_eq!((best.segment, best.max_dvr), (5.0, 43200.0));
  assert!(best.url.contains("itag=137"));
  let audio = &formats[7];
  assert!(audio.audio());
  assert_eq!((audio.codecs.as_str(), audio.height), ("mp4a.40.2", 0));
  // A video has none: it plays from HLS.
  assert!(innertube::parse_live_formats(&fixture("player_visionos.json")).is_empty());
  // WebM (VP9) is left out: FFmpeg ends its video at the first seek.
  let mut player = fixture("player_visionos_live.json");
  player["streamingData"]["adaptiveFormats"][0]["mimeType"] = "video/webm; codecs=\"vp9\"".into();
  let itags: Vec<u32> = innertube::parse_live_formats(&player)
    .iter()
    .map(|f| f.itag)
    .collect();
  assert_eq!(itags, [136, 135, 134, 133, 160, 139, 140]);
}

#[test]
fn watch_page_chapters() {
  // The player bar's markers of a video whose description has timestamps (captured
  // 2026-11-02, trimmed to a few chapters, one of them made malformed).
  let page = parse_watch("rfscVS0vtbw", &fixture("next_chapters.json"));
  let chapters = page
    .chapters
    .iter()
    .map(|c| (c.start, c.title.as_str()))
    .collect::<Vec<_>>();
  assert_eq!(
    chapters,
    [
      (0.0, "Introduction"),
      (105.0, "Installing Python & PyCharm"),
      (400.0, "Setup & Hello World"),
      (623.0, "Drawing a Shape"),
    ]
  );
  assert!(
    parse_watch("dQw4w9WgXcQ", &fixture("next.json"))
      .chapters
      .is_empty()
  );
}

#[test]
fn live_watch_page_has_its_chat() {
  // A live's `next` (captured 2026-11-02): its chat's first token; a video has none.
  let page = parse_watch("bygxS5WGoKE", &fixture("next_live.json"));
  assert!(
    page
      .live_chat
      .as_deref()
      .is_some_and(|t| t.starts_with("0ofMyAO"))
  );
  assert!(!page.title.is_empty());
  assert_eq!(
    parse_watch("dQw4w9WgXcQ", &fixture("next.json")).live_chat,
    None
  );
}

#[test]
fn live_chat_page() {
  // `live_chat/get_live_chat` of the same live: 30 actions kept, chatters renamed.
  let page = innertube::parse_live_chat(&fixture("live_chat.json"));
  assert_eq!(page.actions.len(), 30);
  assert!(page.continuation.is_some());
  assert_eq!(page.poll_after, std::time::Duration::from_millis(10_000));
  let messages: Vec<&ChatMessage> = page
    .actions
    .iter()
    .filter_map(|a| match a {
      ChatAction::Message(m) => Some(m),
      ChatAction::Remove(_) => None,
    })
    .collect();
  assert_eq!(messages.len(), 28);
  assert_eq!(
    page.actions[17],
    ChatAction::Remove("ChwKGkNNcXFwdGpNNTVjREZRbDFxd0lkT2k4MFhR".into())
  );

  let first = messages[0];
  assert_eq!(first.author, "@viewer1");
  assert_eq!(first.author_id.as_deref(), Some("UCviewer1xxxxxxxxxxxxxxx"));
  assert_eq!(first.author_photo.len(), 1);
  assert!(first.author_photo[0].url.contains("ggpht.com"));
  assert_eq!(first.runs, [ChatRun::Text("Parkour!!!".into())]);
  assert_eq!(first.timestamp_usec, Some(1_793_563_401_559_297));

  // A Unicode emoji: its character, and Noto's image.
  let ChatRun::Emoji { name, url, custom } = &messages[1].runs[1] else {
    panic!("an emoji");
  };
  assert_eq!(name, "\u{1f998}");
  assert!(!custom);
  assert!(url.as_deref().is_some_and(|u| u.starts_with("https://")));

  // The moderator, also verified.
  let kinds: Vec<ChatBadgeKind> = messages[2].badges.iter().map(|b| b.kind).collect();
  assert_eq!(kinds, [ChatBadgeKind::Verified, ChatBadgeKind::Moderator]);
  assert_eq!(messages[2].badges[1].label, "Modérateur");
  assert!(messages[2].badges.iter().all(|b| b.image.is_none()));

  // A member: the channel's badge image, and the channel's own emojis.
  let member = messages.iter().find(|m| m.author == "@viewer5").unwrap();
  assert_eq!(member.badges[0].kind, ChatBadgeKind::Member);
  assert_eq!(member.badges[0].label, "Nouveau membre");
  assert!(member.badges[0].image.is_some());
  assert!(member.runs.iter().any(|r| matches!(
    r,
    ChatRun::Emoji { name, custom: true, url: Some(_) } if name == ":face-blue-smiling:"
  )));
}

#[test]
fn player_storyboard() {
  let board = innertube::parse_storyboard(&fixture("player_visionos.json")).unwrap();
  assert_eq!((board.width, board.height), (160, 90));
  assert_eq!((board.count, board.columns, board.rows), (128, 5, 5));
  assert_eq!(board.interval_ms, 5000);
  assert!(
    board
      .url
      .starts_with("https://i.ytimg.com/sb/aqz-KE-bpKQ/storyboard3_L2/M$M.jpg?")
  );
  assert!(
    board
      .url
      .ends_with("&sigh=rs$AOn4CLC7aItN_RejAPMNuUuVk2lyfgHzJQ")
  );

  // 5 s per frame, 25 frames per sheet: 2 min 12 s is frame 26, the second of sheet 1.
  let frame = board.frame(132.0, 635.0).unwrap();
  assert!(frame.url.contains("/M1.jpg?"));
  assert_eq!((frame.column, frame.row), (1, 0));
  // Past the last frame: the last one (127 = sheet 5, cell 2).
  let last = board.frame(10_000.0, 635.0).unwrap();
  assert!(last.url.contains("/M5.jpg?"));
  assert_eq!((last.column, last.row), (2, 0));
}

#[test]
fn search_results() {
  let page = parse_search(&fixture("search.json"));
  // Videos, the course, the playlist and the channel, in order; the Shorts shelf is left out.
  assert_eq!(page.results.len(), 7);
  assert!(page.continuation.is_some());
  let SearchResult::Video(first) = &page.results[0] else {
    panic!("a video first");
  };
  assert_eq!(first.id, "rQ_J9WH6CGk");
  assert_eq!(first.channel.as_deref(), Some("BekBrace"));
  assert_eq!(
    first.channel_id.as_deref(),
    Some("UC7EVSn5inapL20oPSwAwEUg")
  );
  assert_eq!(first.duration.as_deref(), Some("3:05:04"));
  assert!(!first.live);
  assert_eq!(first.metadata, ["663\u{a0}k\u{a0}vues", "il y a 2 ans"]);
  assert!(first.thumbnails.iter().any(|t| t.width >= 720));
  assert!(!first.avatar.is_empty());
  // A course is a playlist too.
  let SearchResult::Playlist(course) = &page.results[4] else {
    panic!("the course after the videos");
  };
  assert_eq!(course.id, "PLzMcBGfZo4-nyLTlSRBvo0zjSnCnqjHYQ");
  assert_eq!(course.title, "Rust Programming Tutorial");
  assert_eq!(course.metadata, ["Cours"]);
  let SearchResult::Playlist(playlist) = &page.results[5] else {
    panic!("the playlist after the course");
  };
  assert_eq!(playlist.id, "PLai5B987bZ9CoVR-QEIN9foz4QCJ0H2Y8");
  assert_eq!(playlist.title, "The Rust Lang Book");
  assert_eq!(playlist.channel.as_deref(), Some("Let's Get Rusty"));
  assert_eq!(
    playlist.channel_id.as_deref(),
    Some("UCSp-OaMpsO8K0KkOqyBl7_w")
  );
  // Its first videos, its page's link and its description are no details.
  assert_eq!(playlist.metadata, ["Playlist"]);
  assert_eq!(playlist.count.as_deref(), Some("44\u{a0}vidéos"));
  assert!(!playlist.thumbnails.is_empty());
  let SearchResult::Channel(channel) = &page.results[6] else {
    panic!("the channel last");
  };
  assert_eq!(channel.id, "UCaYhcUwRBNscFNUKTjgPFiA");
  assert_eq!(channel.name, "Rust Programming Language");
  assert_eq!(channel.handle.as_deref(), Some("@RustVideos"));
  assert_eq!(
    channel.subscribers.as_deref(),
    Some("70,4\u{a0}k\u{a0}abonnés")
  );
  // Protocol-relative avatars get their scheme.
  assert!(channel.avatar.iter().all(|a| a.url.starts_with("https://")));
}

#[test]
fn search_result_progress() {
  let page = parse_search(&fixture("search.json"));
  let watched: Vec<_> = page
    .results
    .iter()
    .filter_map(|r| match r {
      SearchResult::Video(v) => v.progress.map(|p| (v.id.as_str(), p)),
      SearchResult::Channel(_) | SearchResult::Playlist(_) => None,
    })
    .collect();
  assert_eq!(
    watched,
    [(
      "0y6RKiIk6cs",
      Progress {
        percent: 7,
        resume: Some(57)
      }
    )]
  );
}

#[test]
fn history_lockups_progress() {
  let feed = parse_feed(&fixture("history.json"));
  let progress: Vec<_> = feed
    .items
    .iter()
    .map(|v| (v.id.as_str(), v.progress))
    .collect();
  // Watched to the end: the bar is full and the card starts over.
  assert_eq!(
    progress,
    [
      (
        "RJsycDbdSgo",
        Some(Progress {
          percent: 100,
          resume: None
        })
      ),
      (
        "0s0ltiSqQyU",
        Some(Progress {
          percent: 10,
          resume: Some(10)
        })
      ),
    ]
  );
}

#[test]
fn watch_later_progress() {
  let page = parse_playlist_page(&fixture("playlist_watch_later_progress.json"));
  let progress: Vec<_> = page.items.iter().map(|v| v.progress).collect();
  assert_eq!(
    progress,
    [
      Some(Progress {
        percent: 10,
        resume: Some(69)
      }),
      None
    ]
  );
}

#[test]
fn web_player_tracking_urls() {
  let tracking = parse_tracking(&fixture("player_web_signed_in.json"), "2.20261029.11.00")
    .expect("signed-in WEB player has playbackTracking");
  let playback = tracking.playback_url(12.0);
  assert!(playback.starts_with("https://s.youtube.com/api/stats/playback?"));
  for part in [
    "docid=HH6B2p4Om5Y",
    "len=3529",
    "ver=2",
    "cmt=12.000",
    "el=detailpage",
    "c=WEB",
  ] {
    assert!(playback.contains(part), "{part} in {playback}");
  }
  // `el` replaced, not repeated.
  assert_eq!(playback.matches("el=").count(), 1);
  let watchtime = tracking.watchtime_url(Watchtime {
    from: 12.0,
    to: 42.5,
    paused: true,
    last: true,
  });
  assert!(watchtime.starts_with("https://s.youtube.com/api/stats/watchtime?"));
  for part in [
    "st=12.000",
    "et=42.500",
    "cmt=42.500",
    "state=paused",
    "final=1",
  ] {
    assert!(watchtime.contains(part), "{part} in {watchtime}");
  }
  // One nonce per playback, 16 characters.
  let cpn = |url: &str| {
    url
      .split('&')
      .find_map(|p| p.strip_prefix("cpn="))
      .map(str::to_owned)
  };
  assert_eq!(cpn(&playback), cpn(&watchtime));
  assert_eq!(cpn(&playback).map(|c| c.len()), Some(16));
  // Another playback gets another nonce.
  let other = parse_tracking(&fixture("player_web_signed_in.json"), "2.20261029.11.00").unwrap();
  assert_ne!(cpn(&other.playback_url(0.0)), cpn(&playback));
  // No tracking in the VISIONOS-less response of an unplayable video.
  assert!(parse_tracking(&fixture("player_login_required.json"), "2").is_none());
}

#[test]
fn related_playlist() {
  // The related list of a Rust tutorial, captured anonymously 2026-11-07, trimmed to a playlist
  // between two videos.
  let page = parse_watch("OX9HJsJUDxA", &fixture("next_related_playlist.json"));
  assert_eq!(page.related.len(), 3);
  assert!(matches!(page.related[0], Related::Video(_)));
  assert!(matches!(page.related[2], Related::Video(_)));
  let Related::Playlist(playlist) = &page.related[1] else {
    panic!("the playlist between the videos");
  };
  assert_eq!(playlist.id, "PLai5B987bZ9CoVR-QEIN9foz4QCJ0H2Y8");
  assert_eq!(playlist.title, "The Rust Lang Book");
  assert_eq!(playlist.channel.as_deref(), Some("Let's Get Rusty"));
  assert_eq!(playlist.metadata, ["Playlist"]);
  assert_eq!(playlist.count.as_deref(), Some("44\u{a0}vidéos"));
  assert!(page.related_continuation.is_some());
}

#[test]
fn search_lives() {
  // A search filtered on lives, captured 2026-11-02, trimmed to its first three results.
  let page = parse_search(&fixture("search_live.json"));
  assert_eq!(page.results.len(), 3);
  assert!(page.results.iter().all(|r| match r {
    SearchResult::Video(v) => v.live && v.duration.is_none(),
    _ => false,
  }));
}

#[test]
fn search_next_page() {
  let page = parse_search(&fixture("search_continuation.json"));
  let ids: Vec<&str> = page
    .results
    .iter()
    .map(|r| match r {
      SearchResult::Video(v) => v.id.as_str(),
      SearchResult::Channel(c) => c.id.as_str(),
      SearchResult::Playlist(p) => p.id.as_str(),
    })
    .collect();
  assert_eq!(
    ids,
    [
      "0XFq9K7N9o4",
      "d9ahpl1gHVA",
      "k9zj8i8FBzE",
      "UCaYhcUwRBNscFNUKTjgPFiA",
      "QNPwKMOQIKM",
      "PLPoSdR46FgI412aItyJhj2bF66cudB6Qs"
    ]
  );
  assert!(page.continuation.is_some());
}

#[test]
fn shorts_feed_seed_and_sequence() {
  let seed = parse_shorts_seed(&fixture("reel_seedless.json"));
  assert_eq!(seed.ids, ["vWfSwQU15Fs"]);
  assert!(seed.continuation.unwrap().ends_with("%3D"));
  // The entry without a `reelWatchEndpoint` is skipped.
  let next = parse_shorts_sequence(&fixture("reel_sequence.json"));
  assert_eq!(next.ids, ["7N3EdefgZMY", "nfqebsbIpmE", "BfmZ2c44tS0"]);
  assert!(next.continuation.is_some());
}

#[test]
fn oembed_title_is_the_original() {
  // Captured 2026-11-03: the same video is "Bats Haaland, Gagne 100 000 $" in fr/FR feeds.
  assert_eq!(
    parse_oembed_title(&fixture("oembed.json")).as_deref(),
    Some("Beat Haaland, Win $100,000")
  );
  assert_eq!(
    parse_oembed_title(&serde_json::json!({ "title": "" })),
    None
  );
}
