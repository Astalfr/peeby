//! Keeps the session alive: rotates its `__Secure-1PSIDTS` cookies as youtube.com does in a
//! browser ([`innertube::Client::rotate_cookies`], ADR 0004), at startup then at the delay
//! YouTube gives, as long as Peeby runs (hidden in the notification area included).

use std::time::Duration;

use gpui::{App, Global};
use innertube::Error;

use crate::save::Session;
use ui::runtime;

/// Bounds of YouTube's delay, should it give an odd one.
const MIN_DELAY: Duration = Duration::from_secs(60);
const MAX_DELAY: Duration = Duration::from_secs(60 * 60);

/// After a failure other than a closed session (network, server).
const RETRY: Duration = Duration::from_secs(2 * 60);

/// The session the running loop rotates: a new one stops it.
#[derive(Default)]
struct Generation(u64);

impl Global for Generation {}

/// Starts rotating the current session's cookies (`save::Session`), stopping the previous
/// loop. Called at startup and on every session change.
pub fn reset(cx: &mut App) {
  let generation = {
    let current = cx.default_global::<Generation>();
    current.0 += 1;
    current.0
  };
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return;
  }
  let current = move |cx: &App| cx.try_global::<Generation>().map(|g| g.0) == Some(generation);
  cx.spawn(async move |cx| {
    loop {
      let rotation = cx.update(|cx| {
        let client = client.clone();
        runtime::spawn(cx, async move { client.rotate_cookies().await })
      });
      let delay = match rotation.await {
        Some(Ok(delay)) => {
          tracing::debug!("session cookies rotated, next in {} s", delay.as_secs());
          delay.clamp(MIN_DELAY, MAX_DELAY)
        }
        // The pages say it already; rotating again cannot bring it back.
        Some(Err(Error::NotLoggedIn)) => {
          tracing::warn!("cookie rotation: the session is gone");
          return;
        }
        Some(Err(e)) => {
          tracing::warn!("cookie rotation: {e}");
          RETRY
        }
        None => return,
      };
      cx.background_executor().timer(delay).await;
      if !cx.update(|cx| current(cx)) {
        return;
      }
    }
  })
  .detach();
}
