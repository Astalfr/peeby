//! A live's poll, through raw GQL (no persisted hash), checked on 2026-11-04 against GQL's
//! validation, anonymous (`twitch_capture -- gql-probe`; introspection is disabled), and saved
//! from a live with a poll (`gql_poll.json`, `twitch_capture -- polls-scan --watch`):
//! - `user(login) { viewablePoll }` is the channel's poll on screen (null without one): `id`,
//!   `title`, `status` (`ACTIVE` seen; any other reads as ended), `startedAt`, `endedAt`,
//!   `durationSeconds`, `remainingDurationMilliseconds`, `choices { id title totalVoters votes {
//!   total base bits communityPoints } }`, `settings { communityPointsVotes bitsVotes
//!   subscriberOnly multichoice { isEnabled } }`. It answers anonymous. `self` (`PollSelfEdge`)
//!   answers no guessed field: the viewer's own votes are not read back.
//! - `voteInPoll(input: { pollID, choiceID, voteID, userID })` votes, `{ error { code } }`
//!   (anonymous: `unauthenticated`, `gql_vote_unauthenticated.json`). `tokens: { bits,
//!   channelPoints }` are paid votes: never sent.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::gql::gql_error;
use crate::points::parse_spend;

const POLL_QUERY: &str = "query Poll($login: String!) { user(login: $login) { id viewablePoll { \
  id title status startedAt endedAt durationSeconds remainingDurationMilliseconds totalVoters \
  votes { total } choices { id title totalVoters votes { total base bits communityPoints } } \
  settings { communityPointsVotes { isEnabled cost } bitsVotes { isEnabled cost } \
  subscriberOnly { isEnabled } multichoice { isEnabled } } } } }";

const VOTE_MUTATION: &str = "mutation Vote($input: VoteInPollInput!) { voteInPoll(input: $input) \
  { error { code } } }";

/// A channel's poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poll {
  /// Id (a vote's `pollID`).
  pub id: String,
  /// Question.
  pub title: String,
  /// Takes votes (`ACTIVE`); else its votes are final.
  pub active: bool,
  /// Start, milliseconds since the epoch.
  pub started_at: Option<i64>,
  /// End, milliseconds since the epoch, once ended.
  pub ended_at: Option<i64>,
  /// How long it takes votes, from the start.
  pub duration_seconds: u64,
  /// The answers, two or more.
  pub choices: Vec<Choice>,
  /// A viewer may vote for several choices.
  pub multichoice: bool,
  /// Only subscribers vote.
  pub subscriber_only: bool,
}

/// One answer of a poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
  /// Id (a vote's `choiceID`).
  pub id: String,
  /// Answer.
  pub title: String,
  /// Every vote (free ones, and paid ones when the channel allows them).
  pub votes: u64,
}

/// The request for `login`'s poll on screen.
pub fn poll_request(login: &str) -> Value {
  json!({
    "operationName": "Poll",
    "query": POLL_QUERY,
    "variables": { "login": login },
  })
}

/// Reads the poll on screen: `None` without one; an unknown channel is [`Error::NotFound`]. A
/// choice without an id is skipped; a poll with fewer than two choices reads as none.
pub fn parse_poll(answer: &Value) -> Result<Option<Poll>> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["user"];
  if user.is_null() {
    return Err(Error::NotFound);
  }
  let poll = &user["viewablePoll"];
  if poll.is_null() {
    return Ok(None);
  }
  let choices: Vec<Choice> = poll["choices"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|c| {
      Some(Choice {
        id: c["id"].as_str()?.to_owned(),
        title: c["title"].as_str().unwrap_or_default().to_owned(),
        votes: c["votes"]["total"].as_u64().unwrap_or(0),
      })
    })
    .collect();
  let Some(id) = poll["id"].as_str().filter(|_| choices.len() >= 2) else {
    return Ok(None);
  };
  let enabled = |key: &str| {
    poll["settings"][key]["isEnabled"]
      .as_bool()
      .unwrap_or(false)
  };
  Ok(Some(Poll {
    id: id.to_owned(),
    title: poll["title"].as_str().unwrap_or_default().to_owned(),
    active: poll["status"].as_str() == Some("ACTIVE"),
    started_at: poll["startedAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    ended_at: poll["endedAt"].as_str().and_then(crate::time::epoch_millis),
    duration_seconds: poll["durationSeconds"].as_u64().unwrap_or(0),
    choices,
    multichoice: enabled("multichoice"),
    subscriber_only: enabled("subscriberOnly"),
  }))
}

/// The request for a free vote by `user_id` for `choice_id` in `poll_id`; `vote_id` is new for
/// each vote, as a spend's transaction id.
pub fn vote_request(poll_id: &str, choice_id: &str, user_id: &str, vote_id: &str) -> Value {
  json!({
    "operationName": "Vote",
    "query": VOTE_MUTATION,
    "variables": { "input": {
      "pollID": poll_id,
      "choiceID": choice_id,
      "userID": user_id,
      "voteID": vote_id,
    } },
  })
}

/// Reads a vote's answer: Twitch's refusals keep their code ([`Error::Refused`]); without the
/// auth-token it is [`Error::NotLoggedIn`].
pub fn parse_vote(answer: &Value) -> Result<()> {
  if answer["errors"][0]["message"].as_str() == Some("unauthenticated") {
    return Err(Error::NotLoggedIn);
  }
  parse_spend(answer, "voteInPoll")
}
