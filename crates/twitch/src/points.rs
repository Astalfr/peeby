//! Channel points through GQL, with the web `auth-token`: the balance of the signed-in viewer, the
//! channel's custom rewards and its predictions, read with raw queries (no persisted hash), and the
//! two spends, redeeming a reward and betting on a prediction (docs/twitch.md, "Channel points").
//!
//! The bonus chest (`availableClaim`) is only read: its mutation, `claimCommunityPoints`, fails
//! Twitch's integrity check outside the browser (checked 2026-11-02), so it is claimed on twitch.tv.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::gql::gql_error;

const CONTEXT_QUERY: &str = "query ChannelPoints($login: String!) { currentUser { id }   user(login: $login) { id stream { id } channel { self { communityPoints { balance availableClaim { id }   activeMultipliers { factor } } } communityPointsSettings { isEnabled name image { url url2x }   customRewards { id title cost prompt backgroundColor isEnabled isPaused isInStock isSubOnly   isUserInputRequired cooldownExpiresAt image { url url2x } defaultImage { url url2x } } } } } }";

const PREDICTIONS_QUERY: &str = "query Predictions($login: String!) { channel(name: $login) { \
  activePredictionEvents { ...E } lockedPredictionEvents { ...E } } } fragment E on \
  PredictionEvent { id title status createdAt predictionWindowSeconds outcomes { id title color \
  totalPoints totalUsers } self { prediction { points outcome { id } } } }";

const REDEEM_MUTATION: &str = "mutation Redeem($input: RedeemCommunityPointsCustomRewardInput!) { \
  redeemCommunityPointsCustomReward(input: $input) { error { code } redemption { id } } }";

const PREDICT_MUTATION: &str = "mutation Predict($input: MakePredictionInput!) { \
  makePrediction(input: $input) { error { code } prediction { id } } }";

/// The viewer's points on a channel, and what they can spend them on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Points {
  /// The channel's user id (the mutations' `channelID`).
  pub channel_id: String,
  /// The viewer's user id (the watch-time report's `user_id`).
  pub viewer_id: Option<String>,
  /// The live's id, when the channel is on air (the watch-time report's `broadcast_id`).
  pub broadcast_id: Option<String>,
  /// Whether the channel has channel points at all.
  pub enabled: bool,
  /// The viewer's balance.
  pub balance: u64,
  /// The bonus chest waiting to be claimed, by its claim id.
  pub claim: Option<String>,
  /// The product of the active multipliers (a sub's 1.2…), 1 without any.
  pub multiplier: f64,
  /// The channel's name for its points, when it renamed them.
  pub name: Option<String>,
  /// The channel's icon for its points, when it has one.
  pub icon: Option<String>,
  /// Its custom rewards, in the channel's order.
  pub rewards: Vec<Reward>,
}

/// A channel's custom reward.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reward {
  /// Id (the redemption's `rewardID`).
  pub id: String,
  /// Title.
  pub title: String,
  /// Cost in points.
  pub cost: u64,
  /// The channel's text about it.
  pub prompt: Option<String>,
  /// Background color, `#RRGGBB`.
  pub color: Option<String>,
  /// Its image, else Twitch's default one.
  pub image: Option<String>,
  /// The viewer must type a text with it.
  pub input_required: bool,
  /// For subscribers only.
  pub sub_only: bool,
  /// Whether it can be redeemed now: enabled, not paused, in stock, no cooldown running.
  pub available: bool,
  /// End of its cooldown, milliseconds since the epoch.
  pub cooldown_until: Option<i64>,
}

/// A channel's prediction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prediction {
  /// Id (the bet's `eventID`).
  pub id: String,
  /// Question.
  pub title: String,
  /// `ACTIVE` takes bets; `LOCKED` waits for its result.
  pub status: PredictionStatus,
  /// Start, milliseconds since the epoch.
  pub created_at: Option<i64>,
  /// How long bets are taken, from the start.
  pub window_seconds: u64,
  /// The answers, two to ten.
  pub outcomes: Vec<Outcome>,
  /// The viewer's bet, when they placed one: the outcome's id and the points.
  pub bet: Option<(String, u64)>,
}

/// Whether a prediction still takes bets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PredictionStatus {
  /// Bets are open.
  #[default]
  Active,
  /// Bets are closed, the result is not known yet.
  Locked,
}

/// One answer of a prediction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
  /// Id (the bet's `outcomeID`).
  pub id: String,
  /// Answer.
  pub title: String,
  /// `BLUE` or `PINK`.
  pub color: String,
  /// Points bet on it.
  pub points: u64,
  /// Viewers who bet on it.
  pub users: u64,
}

/// The request for the viewer's points on `login`'s channel and its rewards.
pub fn context_request(login: &str) -> Value {
  json!({
    "operationName": "ChannelPoints",
    "query": CONTEXT_QUERY,
    "variables": { "login": login },
  })
}

/// Reads the points; `user: null` is [`Error::NotFound`]. Without an auth-token, `self` is null and
/// so is the balance: [`Error::NotLoggedIn`].
pub fn parse_context(answer: &Value) -> Result<Points> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["user"];
  if user.is_null() {
    return Err(Error::NotFound);
  }
  let channel = &user["channel"];
  let mine = &channel["self"]["communityPoints"];
  let Some(balance) = mine["balance"].as_u64() else {
    return Err(Error::NotLoggedIn);
  };
  let settings = &channel["communityPointsSettings"];
  let multiplier = mine["activeMultipliers"].as_array().map_or(1.0, |list| {
    list
      .iter()
      .filter_map(|m| m["factor"].as_f64())
      .fold(1.0, |product, factor| product * factor)
  });
  Ok(Points {
    channel_id: user["id"].as_str().unwrap_or_default().to_owned(),
    viewer_id: answer["data"]["currentUser"]["id"]
      .as_str()
      .map(str::to_owned),
    broadcast_id: user["stream"]["id"].as_str().map(str::to_owned),
    enabled: settings["isEnabled"].as_bool().unwrap_or(false),
    balance,
    claim: mine["availableClaim"]["id"].as_str().map(str::to_owned),
    multiplier,
    name: settings["name"]
      .as_str()
      .filter(|n| !n.is_empty())
      .map(str::to_owned),
    icon: image(&settings["image"]),
    rewards: settings["customRewards"]
      .as_array()
      .map(|list| list.iter().filter_map(reward).collect())
      .unwrap_or_default(),
  })
}

/// The largest of an image's URLs.
fn image(image: &Value) -> Option<String> {
  image["url2x"]
    .as_str()
    .or_else(|| image["url"].as_str())
    .map(str::to_owned)
}

fn reward(r: &Value) -> Option<Reward> {
  let flag = |key: &str| r[key].as_bool().unwrap_or(false);
  let cooldown_until = r["cooldownExpiresAt"]
    .as_str()
    .and_then(crate::time::epoch_millis);
  Some(Reward {
    id: r["id"].as_str()?.to_owned(),
    title: r["title"].as_str()?.to_owned(),
    cost: r["cost"].as_u64()?,
    prompt: r["prompt"]
      .as_str()
      .filter(|p| !p.is_empty())
      .map(str::to_owned),
    color: r["backgroundColor"].as_str().map(str::to_owned),
    image: image(&r["image"]).or_else(|| image(&r["defaultImage"])),
    input_required: flag("isUserInputRequired"),
    sub_only: flag("isSubOnly"),
    available: flag("isEnabled") && !flag("isPaused") && flag("isInStock"),
    cooldown_until,
  })
}

/// The request for `login`'s open and locked predictions.
pub fn predictions_request(login: &str) -> Value {
  json!({
    "operationName": "Predictions",
    "query": PREDICTIONS_QUERY,
    "variables": { "login": login },
  })
}

/// Reads the predictions, open ones first; `channel: null` is [`Error::NotFound`].
pub fn parse_predictions(answer: &Value) -> Result<Vec<Prediction>> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let channel = &answer["data"]["channel"];
  if channel.is_null() {
    return Err(Error::NotFound);
  }
  let list = |key: &str, status| {
    channel[key]
      .as_array()
      .map(|list| {
        list
          .iter()
          .filter_map(|p| prediction(p, status))
          .collect::<Vec<_>>()
      })
      .unwrap_or_default()
  };
  let mut predictions = list("activePredictionEvents", PredictionStatus::Active);
  predictions.extend(list("lockedPredictionEvents", PredictionStatus::Locked));
  Ok(predictions)
}

fn prediction(p: &Value, status: PredictionStatus) -> Option<Prediction> {
  let outcomes: Vec<Outcome> = p["outcomes"]
    .as_array()?
    .iter()
    .filter_map(|o| {
      Some(Outcome {
        id: o["id"].as_str()?.to_owned(),
        title: o["title"].as_str().unwrap_or_default().to_owned(),
        color: o["color"].as_str().unwrap_or_default().to_owned(),
        points: o["totalPoints"].as_u64().unwrap_or(0),
        users: o["totalUsers"].as_u64().unwrap_or(0),
      })
    })
    .collect();
  if outcomes.len() < 2 {
    return None;
  }
  let mine = &p["self"]["prediction"];
  let bet = mine["outcome"]["id"]
    .as_str()
    .map(|id| (id.to_owned(), mine["points"].as_u64().unwrap_or(0)));
  Some(Prediction {
    id: p["id"].as_str()?.to_owned(),
    title: p["title"].as_str().unwrap_or_default().to_owned(),
    status,
    created_at: p["createdAt"].as_str().and_then(crate::time::epoch_millis),
    window_seconds: p["predictionWindowSeconds"].as_u64().unwrap_or(0),
    outcomes,
    bet,
  })
}

/// The request redeeming `reward` on `channel_id` with the viewer's `text` (for a reward that
/// asks for one). `transaction_id` is 32 random hex digits, new for each redemption, as the web
/// player sends.
pub fn redeem_request(
  channel_id: &str,
  reward: &Reward,
  text: Option<&str>,
  transaction_id: &str,
) -> Value {
  let mut input = json!({
    "channelID": channel_id,
    "cost": reward.cost,
    "prompt": reward.prompt,
    "rewardID": reward.id,
    "title": reward.title,
    "transactionID": transaction_id,
  });
  if let Some(text) = text {
    input["textInput"] = json!(text);
  }
  json!({
    "operationName": "Redeem",
    "query": REDEEM_MUTATION,
    "variables": { "input": input },
  })
}

/// Reads a redemption's answer: Twitch's refusal (`NOT_ENOUGH_POINTS`, `COOLDOWN`…) is
/// [`Error::Refused`].
pub fn parse_redeem(answer: &Value) -> Result<()> {
  parse_spend(answer, "redeemCommunityPointsCustomReward")
}

/// The request betting `points` on `outcome_id` of the prediction `event_id`; `transaction_id` as
/// for [`redeem_request`].
pub fn predict_request(
  event_id: &str,
  outcome_id: &str,
  points: u64,
  transaction_id: &str,
) -> Value {
  json!({
    "operationName": "Predict",
    "query": PREDICT_MUTATION,
    "variables": { "input": {
      "eventID": event_id,
      "outcomeID": outcome_id,
      "points": points,
      "transactionID": transaction_id,
    } },
  })
}

/// Reads a bet's answer, as [`parse_redeem`].
pub fn parse_predict(answer: &Value) -> Result<()> {
  parse_spend(answer, "makePrediction")
}

fn parse_spend(answer: &Value, key: &str) -> Result<()> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let result = &answer["data"][key];
  if result.is_null() {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: format!("no {key}"),
    });
  }
  match result["error"]["code"].as_str() {
    Some(code) => Err(Error::Refused(code.to_owned())),
    None => Ok(()),
  }
}
