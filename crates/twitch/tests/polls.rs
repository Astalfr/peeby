//! Poll parsers against real GQL answers, captured anonymous on 2026-11-04
//! (`twitch_capture -- polls-scan --watch`, then `-- poll <login>`): a public channel's poll
//! while it took votes (`gql_poll.json`) and once completed (`gql_poll_completed.json`), and a
//! vote sent anonymous with made-up ids (`gql_vote_unauthenticated.json`). Request ids removed.

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::Error;
use twitch::polls;

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_running_poll_is_read() {
  let poll = polls::parse_poll(&json("gql_poll.json")).unwrap().unwrap();
  assert_eq!(poll.id, "a06589b9-97ed-44b2-ac54-b52a9942965b");
  assert_eq!(poll.title, "Après le challenge vous allez vous laver ?");
  assert!(poll.active);
  assert_eq!(poll.started_at, Some(1_793_824_272_242));
  assert_eq!(poll.ended_at, None);
  assert_eq!(poll.duration_seconds, 180);
  let choices: Vec<(&str, u64)> = poll
    .choices
    .iter()
    .map(|c| (c.title.as_str(), c.votes))
    .collect();
  assert_eq!(
    choices,
    [("Oui", 0), ("Non", 1), ("Hmmm je sens pas, pk ?", 5)]
  );
  assert!(poll.multichoice);
  assert!(!poll.subscriber_only);
}

#[test]
fn a_completed_poll_has_its_end_and_final_votes() {
  let poll = polls::parse_poll(&json("gql_poll_completed.json"))
    .unwrap()
    .unwrap();
  assert!(!poll.active);
  assert_eq!(poll.ended_at, Some(1_793_824_452_242));
  let votes: Vec<u64> = poll.choices.iter().map(|c| c.votes).collect();
  assert_eq!(votes, [5, 8, 31]);
}

#[test]
fn no_poll_unknown_channels_and_broken_choices() {
  let mut answer = json("gql_poll.json");
  answer["data"]["user"]["viewablePoll"] = Value::Null;
  assert_eq!(polls::parse_poll(&answer).unwrap(), None);
  answer["data"]["user"] = Value::Null;
  assert!(matches!(polls::parse_poll(&answer), Err(Error::NotFound)));
  // A choice without an id is skipped; fewer than two left is no poll.
  let mut answer = json("gql_poll.json");
  let choices = &mut answer["data"]["user"]["viewablePoll"]["choices"];
  choices[0]["id"] = Value::Null;
  assert_eq!(
    polls::parse_poll(&answer).unwrap().unwrap().choices.len(),
    2
  );
  answer["data"]["user"]["viewablePoll"]["choices"][1]["id"] = Value::Null;
  assert_eq!(polls::parse_poll(&answer).unwrap(), None);
}

#[test]
fn a_vote_sends_its_ids_and_reads_its_refusals() {
  let request = polls::vote_request("p", "c", "42", "v");
  assert_eq!(
    request["variables"]["input"],
    json!({ "pollID": "p", "choiceID": "c", "userID": "42", "voteID": "v" })
  );
  // No paid votes, ever.
  assert!(request["variables"]["input"].get("tokens").is_none());
  assert!(matches!(
    polls::parse_vote(&json("gql_vote_unauthenticated.json")),
    Err(Error::NotLoggedIn)
  ));
  let refused = json!({ "data": { "voteInPoll": { "error": { "code": "POLL_NOT_ACTIVE" } } } });
  assert!(
    matches!(polls::parse_vote(&refused), Err(Error::Refused(code)) if code == "POLL_NOT_ACTIVE")
  );
  let done = json!({ "data": { "voteInPoll": { "error": null } } });
  assert!(polls::parse_vote(&done).is_ok());
}
