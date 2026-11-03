use tracing::{info, warn};

use crate::{
  Error, Master, Target, gql,
  usher::{self, Version},
};

pub(crate) async fn fallback(
  error: &Error,
  client: &crate::Client,
  target: &Target,
  version: Version,
) -> Result<Master, Error> {
  if matches!(error, Error::SubscriberOnly)
    && let Target::Vod(vod_id) = target
  {
    info!("Twitch subscriber-only content for {target:?}");
    let response = client
      .gql_with_client_id(
        &gql::video_request(vod_id),
        "kimne78kx3ncx6brgo4mv6wki5h1ko",
      )
      .await?;
    info!("Twitch subscriber-only content for {target:?}: {response:?}");
    if response["data"]["video"].is_null() {
      return Err(Error::SubscriberOnly);
    }

    fn create_serving_id() -> String {
      use rand::RngExt;
      let chars = b"0123456789abcdefghijklmnopqrstuvwxyz";
      let mut rng = rand::rng();

      (0..32)
        .map(|_| {
          let idx = rng.random_range(0..chars.len());
          chars[idx] as char
        })
        .collect()
    }

    let vod_data = response["data"]["video"].clone();
    let channel_data = vod_data["owner"].clone();
    let broadcast_type = response["data"]["video"]["broadcastType"]
      .as_str()
      .unwrap_or_default()
      .to_lowercase();
    let seek_previews_url = response["data"]["video"]["seekPreviewsURL"]
      .as_str()
      .unwrap_or_default()
      .to_string();
    let domain = seek_previews_url
      .split('/')
      .nth(2)
      .unwrap_or_default()
      .to_string();
    let paths = seek_previews_url.split('/').skip(3).collect::<Vec<_>>();
    let mut storyboards_index = 0;
    for (i, path) in paths.iter().enumerate() {
      if path.contains("storyboards") {
        storyboards_index = i;
        break;
      }
    }
    let vod_special_id = paths[storyboards_index - 1];
    let created_at_string = vod_data["createdAt"]
      .as_str()
      .unwrap_or_default()
      .to_string();
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_string)
      .map(|dt| dt.with_timezone(&chrono::Utc))
      .unwrap_or_else(|_| chrono::DateTime::from(chrono::Utc::now()));
    let now = chrono::Utc::now();
    let days_difference = (now - created_at).num_days();

    let resolutions = vec![
      ("chunked", "chunked", "chunked", 60),
      ("1080p60", "1080p60", "1920x1080", 60),
      ("720p60", "720p60", "1280x720", 60),
      ("480p30", "480p", "854x480", 30),
      ("360p30", "360p", "640x360", 30),
    ];

    info!(
      "Twitch subscriber-only content for {target:?}: broadcast_type={broadcast_type}, vod_special_id={vod_special_id}, domain={domain}, days_difference={days_difference}, created_at={created_at}, now={now}"
    );

    let servicing_id = create_serving_id();
    let mut start_quality = 8534030;
    let mut fake_playlist = format!(
      r#"#EXTM3U
#EXT-X-TWITCH-INFO:ORIGIN="s3",B="false",REGION="EU",USER-IP="127.0.0.1",SERVING-ID="{servicing_id}",CLUSTER="cloudfront_vod",USER-COUNTRY="BE",MANIFEST-CLUSTER="cloudfront_vod""#,
    );

    for (resolution_key, resolution_name, resolution_dimensions, resolution_framerate) in
      resolutions
    {
      let playlist_url;
      if broadcast_type == "highlight" {
        playlist_url = format!(
          "https://{}/{}/{}/highlight-{}.m3u8",
          domain, vod_special_id, resolution_key, vod_id
        );
      } else if broadcast_type == "upload" && days_difference > 7 {
        // Only old uploaded VOD works with this method now
        playlist_url = format!(
          "https://{}/{}/{}/{}/{}/index-dvr.m3u8",
          domain, channel_data["login"], vod_id, vod_special_id, resolution_key
        );
      } else {
        // Only old uploaded VOD works with this method now
        playlist_url = format!(
          "https://{}/{}/{}/index-dvr.m3u8",
          domain, vod_special_id, resolution_key
        );
      }

      async fn get_video_quality(http: &reqwest::Client, playlist_url: &str) -> Option<String> {
        let video_quality_result = http.get(playlist_url).send().await;
        if video_quality_result.is_err() {
          warn!(
            "Failed to fetch playlist for playlist {playlist_url}: {:?}",
            video_quality_result.err()
          );
          return None;
        }
        let video_quality_response = video_quality_result.unwrap();
        if !video_quality_response.status().is_success() {
          warn!(
            "Failed to fetch playlist for playlist {playlist_url}: HTTP {}",
            video_quality_response.status()
          );
          return None;
        }
        let data = video_quality_response.text().await;
        if data.is_err() {
          warn!(
            "Failed to read playlist for playlist {playlist_url}: {:?}",
            data.err()
          );
          return None;
        }
        let data = data.unwrap();
        info!("Fetched playlist for playlist {playlist_url}: {}", data);
        if data.contains(".ts") {
          return Some("avc1.4D001E".to_string());
        }
        if data.contains(".mp4") {
          let mp4_request = http
            .get(playlist_url.replace("index-dvr.m3u8", "init-0.mp4"))
            .send()
            .await;
          if mp4_request.is_err() {
            return Some("hev1.1.6.L93.B0".to_string());
          } else {
            let content = mp4_request.unwrap().text().await.unwrap_or_default();
            if content.contains("hev1") {
              return Some("hev1.1.6.L93.B0".to_string());
            } else {
              return Some("avc1.4D001E".to_string());
            }
          }
        }
        return None;
      }

      let video_quality = get_video_quality(&client.http, &playlist_url).await;

      if video_quality.is_none() {
        warn!(
          "Failed to determine video quality for playlist {playlist_url}, skipping this resolution."
        );
        continue;
      }

      let codec = video_quality.unwrap();
      let resolution = resolution_dimensions;
      let frame_rate = resolution_framerate;
      info!("Found codec {codec} for playlist {playlist_url}, adding to variants.");

      if version == Version::V2 {
        let variant_source = if resolution_key == "chunked" {
          "source"
        } else {
          "transcode"
        };
        fake_playlist = format!(
          r#"{fake_playlist}
#EXT-X-STREAM-INF:BANDWIDTH={start_quality},CODECS="{codec},mp4a.40.2",RESOLUTION={resolution},FRAME-RATE={frame_rate},STABLE-VARIANT-ID="{resolution_key}",IVS-NAME="{resolution_name}",IVS-VARIANT-SOURCE="{variant_source}"
{playlist_url}"#
        );
      } else {
        let enabled = if resolution_key == "chunked" {
          "YES"
        } else {
          "NO"
        };
        fake_playlist = format!(
          r#"{fake_playlist}
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="{resolution_key}",NAME="{resolution_key}",AUTOSELECT={enabled},DEFAULT={enabled}
#EXT-X-STREAM-INF:BANDWIDTH={start_quality},CODECS="{codec},mp4a.40.2",RESOLUTION={resolution},VIDEO="{resolution_name}",FRAME-RATE={frame_rate}
{playlist_url}"#
        );
        start_quality -= 100;
      }
    }

    if !fake_playlist.contains("#EXT-X-STREAM-INF") {
      warn!(
        "No valid variants found in the fake playlist for subscriber-only content for {target:?}"
      );
    } else {
      info!("Fake playlist for subscriber-only content for {target:?}: {fake_playlist}");
      let variants = usher::variants(&fake_playlist);
      if variants.is_empty() {
        return Err(Error::Parse {
          endpoint: "usher",
          detail: "no variant in the fake playlist".into(),
        });
      }
      return Ok(Master {
        version,
        text: fake_playlist,
        variants,
      });
    }
  }
  return Err(Error::SubscriberOnly);
}
