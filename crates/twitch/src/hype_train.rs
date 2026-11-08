//! A channel's Hype Train (ADR 0032): the state on screen from raw GQL, its moves from Twitch's
//! PubSub (`hype-train-events-v1.<channel id>`, anonymous, beside the raid topic: [`crate::pubsub`]).
//!
//! GQL, checked on 2026-11-08 against its validation, anonymous (`twitch_capture -- gql-probe`),
//! and saved from a live with a train (`gql_hype_train.json`, `hype_probe`):
//! `user(login) { channel { hypeTrain { approaching execution } } }`.
//! - `execution` is the last train (null before the first): `id`, `isActive`, `startedAt`,
//!   `expiresAt` (the level's end, 5 minutes after its last level-up), `updatedAt`, `endedAt`,
//!   `endReason`, `isGoldenKappaTrain`, `isTreasureTrain`, `isFastMode`, `config {
//!   primaryHexColor willUseCreatorColor difficulty calloutEmote }`, `progress { goal progression
//!   total remainingSeconds level { value goal rewards } }` (`goal` and `progression` are the
//!   level's, `level.goal` the train's total at the level's end), `allTimeHigh { level { value } }`
//!   (the channel's record), `conductors { source user }`, `participations`.
//! - `approaching` (null without one): `id`, `expiresAt`, `goal` (contributions needed),
//!   `participants` (user ids so far), `levelOneRewards`, `isGoldenKappaTrain`,
//!   `isTreasureTrain`, `creatorColor`.
//!
//! PubSub, listened to on 500 lives on 2026-11-08 (`raid_probe --topic hype-train-events-v1`):
//! `hype-train-approaching` (`goal`, `participants`, `expires_at`, `level_one_rewards`),
//! `hype-train-start` (the whole train, snake case, dates in Unix milliseconds),
//! `hype-train-progression` (a contribution: `progress`, `expires_at` as a date),
//! `hype-train-level-up` (`progress`, `time_to_expire` in Unix milliseconds, and the train as GQL
//! gives it), `hype-train-conductor-update` (`id` is `<train id>-<source>`), and
//! `hype-train-end` (`ended_at` in Unix milliseconds, `ending_reason` `COMPLETED` or `EXPIRED`
//! when level 1 was not completed, and `rewards`: every emote unlocked, as GQL gives them). The
//! messages carry no channel id: the topic does. A train's `level` is the one under way: it
//! ended with the levels before it completed.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::gql::gql_error;
use crate::time::epoch_millis;

const HYPE_TRAIN_QUERY: &str = "query HypeTrain($login: String!) { user(login: $login) { id \
  channel { hypeTrain { \
  approaching { id expiresAt goal participants isGoldenKappaTrain isTreasureTrain creatorColor \
  levelOneRewards { type ... on HypeTrainEmoteReward { emote { id token } } } } \
  execution { id isActive startedAt expiresAt updatedAt endedAt endReason isGoldenKappaTrain \
  isTreasureTrain config { primaryHexColor willUseCreatorColor } \
  progress { goal progression total remainingSeconds level { value goal \
  rewards { type ... on HypeTrainEmoteReward { emote { id token } } } } } \
  allTimeHigh { level { value } } \
  conductors { source user { id login displayName profileImageURL(width: 50) } } } } } } }";

/// How long a level lasts without a new one, when Twitch does not say (`level_duration`, seen
/// at 5 minutes on every train).
pub const LEVEL_SECONDS: u64 = 300;

/// A Hype Train under way, or just ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HypeTrain {
  /// Id: the same in every message of a train.
  pub id: String,
  /// Where it is.
  pub progress: Progress,
  /// Start, milliseconds since the epoch.
  pub started_at: Option<i64>,
  /// The level's end, milliseconds since the epoch: the train ends then unless it levels up.
  pub expires_at: Option<i64>,
  /// How long a level lasts.
  pub level_seconds: u64,
  /// The channel's color, `RRGGBB`, when it uses one.
  pub color: Option<String>,
  /// A Golden Kappa Train: its contributors get the Golden Kappa for a day.
  pub golden_kappa: bool,
  /// A Treasure Train: subscriptions at a discount.
  pub treasure: bool,
  /// The top contributor by source (subs, bits).
  pub conductors: Vec<Conductor>,
  /// The channel's highest level, before this train.
  pub record: Option<u32>,
  /// When it ended (milliseconds since the epoch), once ended.
  pub ended_at: Option<i64>,
  /// Every emote it unlocked, once ended (its end message's; empty from GQL).
  pub unlocked: Vec<Emote>,
}

/// Where a train is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
  /// The level under way, 1 and up.
  pub level: u32,
  /// Points within the level.
  pub value: u64,
  /// Points the level needs.
  pub goal: u64,
  /// Points since the start.
  pub total: u64,
  /// The level's emotes, for its contributors.
  pub rewards: Vec<Emote>,
}

impl Progress {
  /// The share of the level done, 0 to 1.
  pub fn share(&self) -> f32 {
    if self.goal == 0 {
      return 0.;
    }
    (self.value as f32 / self.goal as f32).clamp(0., 1.)
  }
}

/// An emote a level gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Emote {
  /// Id, for its image (`chat::irc::emote_url`).
  pub id: String,
  /// Its code in chat.
  pub token: String,
}

/// What a conductor gave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
  /// Subscriptions, gifted or not.
  Subs,
  /// Bits.
  Bits,
  /// Anything else.
  #[default]
  Other,
}

impl Source {
  fn read(name: &str) -> Self {
    match name {
      "SUBS" => Self::Subs,
      "BITS" => Self::Bits,
      _ => Self::Other,
    }
  }
}

/// A train's top contributor for a source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conductor {
  /// What they gave most of.
  pub source: Source,
  /// Their login.
  pub login: String,
  /// Shown name.
  pub name: String,
  /// Avatar, 50 px.
  pub avatar: Option<String>,
}

/// A train about to start: a few more contributions in time start it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Approaching {
  /// Id of the train it would start.
  pub id: String,
  /// Contributions it needs.
  pub goal: u32,
  /// Contributions so far.
  pub count: u32,
  /// When it is called off without them, milliseconds since the epoch.
  pub expires_at: Option<i64>,
  /// The channel's color, `RRGGBB`.
  pub color: Option<String>,
  /// It would be a Golden Kappa Train.
  pub golden_kappa: bool,
  /// It would be a Treasure Train.
  pub treasure: bool,
  /// Level 1's emotes.
  pub rewards: Vec<Emote>,
}

impl Approaching {
  /// Contributions still needed.
  pub fn left(&self) -> u32 {
    self.goal.saturating_sub(self.count)
  }
}

/// The channel's train on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
  /// A train about to start.
  pub approaching: Option<Approaching>,
  /// The last train, running or ended.
  pub train: Option<HypeTrain>,
}

/// What the train topic says.
#[derive(Clone, Debug, PartialEq)]
pub enum HypeEvent {
  /// Contributions came: a train may start.
  Approaching(Approaching),
  /// A train started.
  Start(HypeTrain),
  /// A contribution (`level_up`: the level changed): where the train is now.
  Progress {
    /// The train's id, when the message gives it.
    id: Option<String>,
    /// Where it is now.
    progress: Progress,
    /// The level's new end, milliseconds since the epoch.
    expires_at: Option<i64>,
    /// The level changed.
    level_up: bool,
  },
  /// A new top contributor for a source; `train` is its train's id.
  Conductor {
    /// The train's id.
    train: String,
    /// The new conductor.
    conductor: Conductor,
  },
  /// The train ended.
  End {
    /// The train's id, when the message gives it.
    id: Option<String>,
    /// When, milliseconds since the epoch.
    ended_at: Option<i64>,
    /// Every emote it unlocked (none when it did not complete level 1).
    unlocked: Vec<Emote>,
  },
}

/// The request for `login`'s train.
pub fn hype_train_request(login: &str) -> Value {
  json!({
    "operationName": "HypeTrain",
    "query": HYPE_TRAIN_QUERY,
    "variables": { "login": login },
  })
}

/// Reads a channel's train: an unknown channel is [`Error::NotFound`]; an ended train is kept
/// with its end (the page shows it for a while), an inactive one without an end is dropped.
pub fn parse_hype_train(answer: &Value) -> Result<State> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["user"];
  if user.is_null() {
    return Err(Error::NotFound);
  }
  let hype = &user["channel"]["hypeTrain"];
  let approaching = &hype["approaching"];
  let approaching = approaching["id"].as_str().map(|id| Approaching {
    id: id.to_owned(),
    goal: count(&approaching["goal"]),
    count: approaching["participants"]
      .as_array()
      .map_or(0, |p| u32::try_from(p.len()).unwrap_or(u32::MAX)),
    expires_at: date(&approaching["expiresAt"]),
    color: hex(&approaching["creatorColor"]),
    golden_kappa: approaching["isGoldenKappaTrain"].as_bool() == Some(true),
    treasure: approaching["isTreasureTrain"].as_bool() == Some(true),
    rewards: gql_rewards(&approaching["levelOneRewards"]),
  });
  let train = gql_train(&hype["execution"])
    .filter(|t| hype["execution"]["isActive"].as_bool() == Some(true) || t.ended_at.is_some());
  Ok(State { approaching, train })
}

/// A train as GQL gives it (camel case), in its answer and in a level-up.
fn gql_train(train: &Value) -> Option<HypeTrain> {
  let id = train["id"].as_str()?;
  let p = &train["progress"];
  let config = &train["config"];
  let color = (config["willUseCreatorColor"].as_bool() != Some(false))
    .then(|| hex(&config["primaryHexColor"]))
    .flatten();
  // A level lasts from the last level-up (`updatedAt`) to `expiresAt`.
  let expires_at = date(&train["expiresAt"]);
  let level_seconds = expires_at
    .zip(date(&train["updatedAt"]))
    .and_then(|(end, start)| u64::try_from((end - start) / 1000).ok())
    .filter(|&s| s > 0)
    .unwrap_or(LEVEL_SECONDS);
  Some(HypeTrain {
    id: id.to_owned(),
    progress: Progress {
      level: count(&p["level"]["value"]).max(1),
      value: p["progression"].as_u64().unwrap_or(0),
      goal: p["goal"].as_u64().unwrap_or(0),
      total: p["total"].as_u64().unwrap_or(0),
      rewards: gql_rewards(&p["level"]["rewards"]),
    },
    started_at: date(&train["startedAt"]),
    expires_at,
    level_seconds,
    color,
    golden_kappa: train["isGoldenKappaTrain"].as_bool() == Some(true),
    treasure: train["isTreasureTrain"].as_bool() == Some(true),
    conductors: train["conductors"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|c| {
        conductor(
          c["source"].as_str().unwrap_or_default(),
          &c["user"],
          "displayName",
          "profileImageURL",
        )
      })
      .collect(),
    record: train["allTimeHigh"]["level"]["value"]
      .as_u64()
      .and_then(|l| u32::try_from(l).ok()),
    ended_at: date(&train["endedAt"]),
    unlocked: Vec::new(),
  })
}

/// Emote rewards as GQL gives them (`{ type emote { id token } }`); other kinds are skipped.
fn gql_rewards(rewards: &Value) -> Vec<Emote> {
  rewards
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|r| {
      Some(Emote {
        id: r["emote"]["id"].as_str()?.to_owned(),
        token: r["emote"]["token"].as_str().unwrap_or_default().to_owned(),
      })
    })
    .collect()
}

/// Emote rewards as PubSub gives them (`{ type id token }`).
fn rewards(rewards: &Value) -> Vec<Emote> {
  rewards
    .as_array()
    .into_iter()
    .flatten()
    .filter(|r| r["type"].as_str().is_none_or(|t| t == "EMOTE"))
    .filter_map(|r| {
      Some(Emote {
        id: r["id"].as_str()?.to_owned(),
        token: r["token"].as_str().unwrap_or_default().to_owned(),
      })
    })
    .collect()
}

/// A train's progress as PubSub gives it (snake case).
fn progress(p: &Value) -> Option<Progress> {
  let level = p["level"]["value"].as_u64()?;
  Some(Progress {
    level: u32::try_from(level).unwrap_or(u32::MAX).max(1),
    value: p["value"].as_u64().unwrap_or(0),
    goal: p["goal"].as_u64().unwrap_or(0),
    total: p["total"].as_u64().unwrap_or(0),
    rewards: rewards(&p["level"]["rewards"]),
  })
}

fn conductor(source: &str, user: &Value, name: &str, avatar: &str) -> Option<Conductor> {
  let login = user["login"].as_str()?.to_owned();
  Some(Conductor {
    source: Source::read(source),
    name: user[name]
      .as_str()
      .filter(|n| !n.is_empty())
      .unwrap_or(&login)
      .to_owned(),
    login,
    avatar: user[avatar].as_str().map(str::to_owned),
  })
}

/// What a PubSub frame means for a train topic; `None` for every other frame.
pub fn parse_message(frame: &str) -> Option<HypeEvent> {
  let frame: Value = serde_json::from_str(frame).ok()?;
  if frame["type"] != "MESSAGE"
    || !frame["data"]["topic"]
      .as_str()?
      .starts_with("hype-train-events-v1.")
  {
    return None;
  }
  // The message is JSON in a string.
  let message: Value = serde_json::from_str(frame["data"]["message"].as_str()?).ok()?;
  parse_event(&message)
}

/// A train topic's message (once out of its frame).
pub fn parse_event(message: &Value) -> Option<HypeEvent> {
  let data = &message["data"];
  match message["type"].as_str()? {
    "hype-train-approaching" => Some(HypeEvent::Approaching(Approaching {
      id: data["approaching_hype_train_id"].as_str()?.to_owned(),
      goal: count(&data["goal"]),
      count: data["participants"]
        .as_array()
        .map_or(0, |p| u32::try_from(p.len()).unwrap_or(u32::MAX)),
      expires_at: date(&data["expires_at"]),
      color: hex(&data["creator_color"]),
      golden_kappa: data["is_golden_kappa_train"].as_bool() == Some(true),
      treasure: data["is_treasure_train"].as_bool() == Some(true),
      rewards: rewards(&data["level_one_rewards"]),
    })),
    "hype-train-start" => {
      let config = &data["config"];
      let color = (config["use_creator_color"].as_bool() != Some(false))
        .then(|| hex(&config["primary_hex_color"]))
        .flatten();
      Some(HypeEvent::Start(HypeTrain {
        id: data["id"].as_str()?.to_owned(),
        progress: progress(&data["progress"])?,
        started_at: date(&data["started_at"]),
        expires_at: date(&data["expires_at"]),
        // Nanoseconds.
        level_seconds: config["level_duration"]
          .as_u64()
          .map(|ns| ns / 1_000_000_000)
          .filter(|&s| s > 0)
          .unwrap_or(LEVEL_SECONDS),
        color,
        golden_kappa: data["is_golden_kappa_train"].as_bool() == Some(true),
        treasure: data["is_treasure_train"].as_bool() == Some(true),
        conductors: Vec::new(),
        record: data["all_time_high_progress"]["level"]["value"]
          .as_u64()
          .and_then(|l| u32::try_from(l).ok()),
        ended_at: None,
        unlocked: Vec::new(),
      }))
    }
    "hype-train-progression" => Some(HypeEvent::Progress {
      id: data["id"].as_str().map(str::to_owned),
      progress: progress(&data["progress"])?,
      expires_at: date(&data["expires_at"]),
      level_up: false,
    }),
    "hype-train-level-up" => Some(HypeEvent::Progress {
      id: data["hype_train"]["id"].as_str().map(str::to_owned),
      progress: progress(&data["progress"])?,
      expires_at: date(&data["time_to_expire"]).or_else(|| date(&data["hype_train"]["expiresAt"])),
      level_up: true,
    }),
    "hype-train-conductor-update" => {
      let source = data["source"].as_str().unwrap_or_default();
      let id = data["id"].as_str()?;
      let train = id
        .strip_suffix(source)
        .and_then(|id| id.strip_suffix('-'))
        .unwrap_or(id);
      Some(HypeEvent::Conductor {
        train: train.to_owned(),
        conductor: conductor(source, &data["user"], "display_name", "profile_image_url")?,
      })
    }
    "hype-train-end" => Some(HypeEvent::End {
      id: data["id"].as_str().map(str::to_owned),
      ended_at: date(&data["ended_at"]),
      unlocked: gql_rewards(&data["rewards"]),
    }),
    _ => None,
  }
}

impl HypeTrain {
  /// The share of the level's time left, 1 to 0; `None` once ended or without an end.
  pub fn time_left(&self, now_millis: i64) -> Option<f32> {
    if self.ended_at.is_some() {
      return None;
    }
    let length = i64::try_from(self.level_seconds).ok()? * 1000;
    let end = self.expires_at?;
    (length > 0).then(|| ((end - now_millis) as f32 / length as f32).clamp(0., 1.))
  }

  /// Seconds left in the level; `None` once ended or without an end.
  pub fn remaining(&self, now_millis: i64) -> Option<i64> {
    if self.ended_at.is_some() {
      return None;
    }
    Some(((self.expires_at? - now_millis) / 1000).max(0))
  }

  /// This train beats the channel's record.
  pub fn record_beaten(&self) -> bool {
    self.record.is_some_and(|r| self.progress.level > r)
  }

  /// The levels it completed: the one under way counts once its goal is met.
  pub fn levels_done(&self) -> u32 {
    let p = &self.progress;
    let current = p.goal > 0 && p.value >= p.goal;
    p.level.saturating_sub(1) + u32::from(current)
  }
}

impl State {
  /// Takes a topic's event in. Messages of another train than the one on screen are dropped,
  /// except a start or an approach, which replace it.
  pub fn apply(&mut self, event: HypeEvent) {
    let same =
      |train: &HypeTrain, id: &Option<String>| id.as_ref().is_none_or(|id| *id == train.id);
    match event {
      HypeEvent::Approaching(approaching) => {
        // A train under way has no approach.
        if self.train.as_ref().is_none_or(|t| t.ended_at.is_some()) {
          self.approaching = Some(approaching);
        }
      }
      HypeEvent::Start(train) => {
        self.approaching = None;
        self.train = Some(train);
      }
      HypeEvent::Progress {
        id,
        progress,
        expires_at,
        ..
      } => {
        if let Some(train) = self.train.as_mut().filter(|t| same(t, &id)) {
          // A late message (PubSub, or a GQL reading after it) moves nothing back.
          if progress.total >= train.progress.total {
            train.progress = progress;
            train.expires_at = expires_at.or(train.expires_at);
          }
        }
      }
      HypeEvent::Conductor { train, conductor } => {
        if let Some(t) = self.train.as_mut().filter(|t| t.id == train) {
          t.conductors.retain(|c| c.source != conductor.source);
          t.conductors.push(conductor);
          t.conductors.sort_by_key(|c| c.source as u8);
        }
      }
      HypeEvent::End {
        id,
        ended_at,
        unlocked,
      } => {
        if let Some(train) = self.train.as_mut().filter(|t| same(t, &id)) {
          train.ended_at = Some(ended_at.unwrap_or_else(now_millis));
          train.unlocked = unlocked;
        }
      }
    }
  }

  /// Takes a GQL reading in: it replaces the state, except that the train on screen keeps what
  /// the topic said since, when it is further along.
  pub fn merge(&mut self, read: State) {
    let kept = self.train.take();
    self.approaching = read.approaching;
    self.train = match (kept, read.train) {
      (Some(kept), Some(read))
        if kept.id == read.id
          && kept.progress.total > read.progress.total
          && read.ended_at.is_none() =>
      {
        Some(HypeTrain {
          conductors: if read.conductors.is_empty() {
            kept.conductors.clone()
          } else {
            read.conductors
          },
          ..kept
        })
      }
      (_, read) => read,
    };
    if self.train.as_ref().is_some_and(|t| t.ended_at.is_none()) {
      self.approaching = None;
    }
  }
}

impl State {
  /// At `now` (milliseconds since the epoch): an approach past its end is gone, and a train
  /// `grace_ms` past its level's end ended then (its end message may have been missed). Whether
  /// anything changed.
  pub fn settle(&mut self, now: i64, grace_ms: i64) -> bool {
    let mut changed = false;
    if self
      .approaching
      .as_ref()
      .is_some_and(|a| a.expires_at.is_some_and(|end| now > end + grace_ms))
    {
      self.approaching = None;
      changed = true;
    }
    if let Some(train) = self.train.as_mut()
      && train.ended_at.is_none()
      && let Some(end) = train.expires_at.filter(|&end| now > end + grace_ms)
    {
      train.ended_at = Some(end);
      changed = true;
    }
    changed
  }
}

/// A count as a `u32`.
fn count(value: &Value) -> u32 {
  value
    .as_u64()
    .and_then(|n| u32::try_from(n).ok())
    .unwrap_or(0)
}

/// A date as Twitch writes it in these messages: a string, or Unix milliseconds.
fn date(value: &Value) -> Option<i64> {
  match value {
    Value::String(s) => epoch_millis(s).filter(|&ms| ms > 0),
    Value::Number(n) => n.as_i64().filter(|&ms| ms > 0),
    _ => None,
  }
}

/// A color as `RRGGBB`, with or without `#`; `None` when empty or malformed.
fn hex(value: &Value) -> Option<String> {
  let s = value.as_str()?.trim_start_matches('#');
  (s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_uppercase())
}

fn now_millis() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
