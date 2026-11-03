//! Sign-in answers (tests/fixtures/auth_*.json, captured 2026-11-01 with Peeby's Client-ID: a
//! device code never approved, the pending poll, a made-up refresh token and access token).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::auth::{self, Poll};
use twitch::{Error, gql};

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn device_code() {
  let device = auth::parse_device(&json("auth_device.json")).unwrap();
  assert_eq!(device.user_code, "ABCDEFGH");
  assert_eq!(
    device.verification_uri,
    "https://www.twitch.tv/activate?device-code=ABCDEFGH"
  );
  assert_eq!((device.expires_in, device.interval), (1800, 5));
  assert!(!format!("{device:?}").contains("REDACTED_DEVICE_CODE"));
}

#[test]
fn pending_poll() {
  assert!(matches!(
    auth::parse_poll(400, &json("auth_token_pending.json")),
    Ok(Poll::Pending)
  ));
}

#[test]
fn other_poll_answers() {
  // Shapes of Twitch's documentation (the same `{status, message}` as the pending answer).
  let message = |m: &str| json!({ "status": 400, "message": m });
  assert!(matches!(
    auth::parse_poll(400, &message("slow_down")),
    Ok(Poll::SlowDown)
  ));
  assert!(matches!(
    auth::parse_poll(400, &message("expired_token")),
    Err(Error::DeviceExpired)
  ));
  assert!(matches!(
    auth::parse_poll(400, &message("access_denied")),
    Err(Error::DeviceDenied)
  ));
  let approved = json!({
    "access_token": "a", "refresh_token": "r", "expires_in": 14400,
    "scope": ["user:read:follows"], "token_type": "bearer"
  });
  match auth::parse_poll(200, &approved) {
    Ok(Poll::Done(tokens)) => {
      assert_eq!(
        (tokens.access.as_str(), tokens.refresh.as_str()),
        ("a", "r")
      );
      assert_eq!(format!("{tokens:?}"), "Tokens(…)");
    }
    other => panic!("{other:?}"),
  }
}

#[test]
fn refused_refresh_ends_the_session() {
  assert!(matches!(
    auth::parse_refresh_error(400, &json("auth_refresh_invalid.json")),
    Error::SessionExpired
  ));
}

#[test]
fn validation() {
  assert!(matches!(
    auth::parse_validation(401, &json("auth_validate_invalid.json")),
    Err(Error::SessionExpired)
  ));
  // Shape of Twitch's documentation.
  let valid = json!({
    "client_id": auth::CLIENT_ID, "login": "peeby_test", "scopes": ["user:read:follows"],
    "user_id": "123", "expires_in": 13000
  });
  let v = auth::parse_validation(200, &valid).unwrap();
  assert_eq!(
    (v.login.as_str(), v.user_id.as_str()),
    ("peeby_test", "123")
  );
  assert_eq!(v.scopes, ["user:read:follows"]);
}

#[test]
fn auth_token_check() {
  assert!(matches!(
    gql::parse_account(&json("gql_current_user_anonymous.json")),
    Err(Error::NotLoggedIn)
  ));
  let signed = json!({ "data": { "currentUser": {
    "id": "1", "login": "peeby_test", "displayName": "Peeby_Test", "profileImageURL": null
  } } });
  let account = gql::parse_account(&signed).unwrap();
  assert_eq!(
    (
      account.id.as_str(),
      account.login.as_str(),
      account.display_name.as_str()
    ),
    ("1", "peeby_test", "Peeby_Test")
  );
  assert_eq!(account.avatar_url, None);
  assert!(gql::is_auth_token_shape("abcdefghij0123456789abcdefghij"));
  assert!(!gql::is_auth_token_shape("too short"));
  assert!(!gql::is_auth_token_shape("abcdefghij0123456789abcdefgh-j"));
}
