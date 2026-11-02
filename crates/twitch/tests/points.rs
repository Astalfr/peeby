//! Channel points parsers against real GQL answers (tests/fixtures/gql_points_*, gql_predict*,
//! gql_redeem_*, captured 2026-11-02 with the user's auth-token; the balance and request ids
//! redacted). The spends were sent with made-up ids, so nothing was spent.

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::Error;
use twitch::points::{self, PredictionStatus};

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn balance_and_rewards_are_read() {
  let points = points::parse_context(&json("gql_points_context.json")).unwrap();
  assert_eq!(points.channel_id, "51653974");
  assert!(points.enabled);
  assert_eq!(points.balance, 12345);
  assert_eq!(points.claim, None);
  assert!((points.multiplier - 1.0).abs() < f64::EPSILON);
  assert_eq!(points.name, None);
  assert_eq!(points.rewards.len(), 5);
  let first = &points.rewards[0];
  assert_eq!(first.title, "tanzen 1min");
  assert_eq!(first.cost, 300_000);
  assert_eq!(first.prompt.as_deref(), Some("Coach tanzt für dich."));
  assert_eq!(first.color.as_deref(), Some("#8B3EF7"));
  assert!(first.input_required && first.available && !first.sub_only);
  // No image of its own: Twitch's default one.
  assert_eq!(
    first.image.as_deref(),
    Some("https://static-cdn.jtvnw.net/custom-reward-images/default-2.png")
  );
  // An empty prompt is none.
  assert_eq!(points.rewards[1].prompt, None);
}

#[test]
fn a_bonus_chest_and_multipliers_are_read() {
  // The fixture had no chest waiting: one is put where Twitch puts it.
  let mut answer = json("gql_points_context.json");
  let mine = &mut answer["data"]["user"]["channel"]["self"]["communityPoints"];
  mine["availableClaim"] = json!({ "id": "claim-1" });
  mine["activeMultipliers"] = json!([{ "factor": 1.2 }, { "factor": 2.0 }]);
  let points = points::parse_context(&answer).unwrap();
  assert_eq!(points.claim.as_deref(), Some("claim-1"));
  assert!((points.multiplier - 2.4).abs() < 1e-9);
}

#[test]
fn without_a_balance_the_viewer_is_not_signed_in() {
  let mut answer = json("gql_points_context.json");
  answer["data"]["user"]["channel"]["self"] = Value::Null;
  assert!(matches!(
    points::parse_context(&answer),
    Err(Error::NotLoggedIn)
  ));
  answer["data"]["user"] = Value::Null;
  assert!(matches!(
    points::parse_context(&answer),
    Err(Error::NotFound)
  ));
}

#[test]
fn an_open_prediction_is_read() {
  let list = points::parse_predictions(&json("gql_predictions_active.json")).unwrap();
  assert_eq!(list.len(), 1);
  let p = &list[0];
  assert_eq!(p.title, "Will They Win Or Lose this game?");
  assert_eq!(p.status, PredictionStatus::Active);
  assert_eq!(p.window_seconds, 300);
  assert_eq!(p.created_at, Some(1_793_578_005_498));
  assert_eq!(p.bet, None);
  let titles: Vec<&str> = p.outcomes.iter().map(|o| o.title.as_str()).collect();
  assert_eq!(titles, ["Win", "Loss"]);
  assert_eq!(p.outcomes[0].color, "BLUE");
  assert_eq!(p.outcomes[0].points, 8_246_935);
  assert_eq!(p.outcomes[1].users, 87);
}

#[test]
fn a_locked_prediction_and_the_viewers_bet_are_read() {
  let mut answer = json("gql_predictions_locked.json");
  let list = points::parse_predictions(&answer).unwrap();
  assert_eq!(list.len(), 1);
  assert_eq!(list[0].status, PredictionStatus::Locked);
  assert_eq!(list[0].bet, None);
  // The fixture's viewer had not bet: a bet is put where Twitch puts it.
  let outcome = list[0].outcomes[1].id.clone();
  answer["data"]["channel"]["lockedPredictionEvents"][0]["self"]["prediction"] =
    json!({ "points": 250, "outcome": { "id": outcome } });
  let list = points::parse_predictions(&answer).unwrap();
  assert_eq!(list[0].bet, Some((outcome, 250)));
}

#[test]
fn refused_spends_keep_their_code() {
  for name in ["gql_redeem_not_found.json", "gql_predict_not_found.json"] {
    let answer = json(name);
    let result = if name.contains("redeem") {
      points::parse_redeem(&answer)
    } else {
      points::parse_predict(&answer)
    };
    assert!(
      matches!(&result, Err(Error::Refused(code)) if code == "NOT_FOUND"),
      "{name}: {result:?}"
    );
  }
  let done =
    json!({ "data": { "makePrediction": { "error": null, "prediction": { "id": "p" } } } });
  assert!(points::parse_predict(&done).is_ok());
}

#[test]
fn a_redemption_sends_the_reward_and_the_text() {
  let points = points::parse_context(&json("gql_points_context.json")).unwrap();
  let reward = &points.rewards[0];
  let request = points::redeem_request(&points.channel_id, reward, Some("salut"), "ab12");
  let input = &request["variables"]["input"];
  assert_eq!(input["channelID"], "51653974");
  assert_eq!(input["rewardID"], reward.id.as_str());
  assert_eq!(input["cost"], 300_000);
  assert_eq!(input["textInput"], "salut");
  assert_eq!(input["transactionID"], "ab12");
  let bet = points::predict_request("e", "o", 100, "cd34");
  assert_eq!(bet["variables"]["input"]["points"], 100);
}
