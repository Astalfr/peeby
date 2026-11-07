//! A live's chat source: `live_chat/get_live_chat` polled at the pace YouTube asks (capped, see
//! [`MAX_POLL`]), its messages turned into `ui::chat`'s. No UI of its own: the watch page owns
//! the panel and the task, and dropping the task stops the polling.
//!
//! A page holds what was said since the previous one: its messages are spread over the time to
//! the next page, as they were sent, so the chat flows rather than jumps. The first page (the
//! last minutes of the chat) shows at once. The chat shows the present even when the player is
//! behind the live edge (DVR).
//!
//! YouTube draws the owner's, moderators' and verified badges as icons, not images: their names
//! take YouTube's colors instead (the owner's gold, moderators' blue). A member's badge is the
//! channel's own image. Unicode emojis stay text; the channel's emojis are images.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, Entity, Global, Hsla, SharedString, Task};
use innertube::{ChatAction, ChatBadgeKind, ChatRun, Client, SearchResult};
use peeby_core::settings::Settings;
use ui::chat::{
  Author, Badge, ChatEvent, ChatMessage, ChatPanel, Emote, Fragment, UserLookup, UserProfile,
  UserQuery, same_user, split_text,
};

use crate::convert;
use ui::i18n::t;
use ui::picture::Picture;
use ui::{runtime, tf};

/// YouTube asks for the next page after up to 10 s; sooner keeps the chat closer to the live
/// at one request every few seconds.
const MAX_POLL: Duration = Duration::from_secs(5);
const MIN_POLL: Duration = Duration::from_secs(1);
/// After a failed request, the wait grows by this much each time, up to [`MAX_RETRY`].
const RETRY_STEP: Duration = Duration::from_secs(5);
const MAX_RETRY: Duration = Duration::from_secs(30);

/// YouTube's name colors for the owner and moderators.
const OWNER_COLOR: u32 = 0xffd600;
const MODERATOR_COLOR: u32 = 0x5e84f1;

/// Polls the chat from `continuation` ([`innertube::WatchPage::live_chat`]) and feeds `panel`
/// until the task is dropped or the chat ends.
pub fn start(
  client: Client,
  continuation: String,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Task<()> {
  let panel = panel.downgrade();
  cx.spawn(async move |cx| {
    let push = |events: Vec<ChatEvent>, cx: &mut gpui::AsyncApp| {
      panel.update(cx, |p, cx| p.push(events, cx)).is_ok()
    };
    if !push(
      vec![ChatEvent::Loading(
        t("Connecting to the chat…", "Connexion au chat…").into(),
      )],
      cx,
    ) {
      return;
    }
    let mut token = continuation;
    let mut failures: u32 = 0;
    let mut first = true;
    loop {
      let fetch = {
        let (client, token) = (client.clone(), token.clone());
        cx.update(|cx| runtime::spawn(cx, async move { client.live_chat(&token).await }))
      };
      let wait = match fetch.await {
        Some(Ok(page)) => {
          let wait = page.poll_after.clamp(MIN_POLL, MAX_POLL);
          let mut events = vec![ChatEvent::Status(None)];
          let mut messages = Vec::new();
          for action in page.actions {
            match action {
              // The backlog at once, rather than squeezed into the first wait.
              ChatAction::Message(m) if first => events.push(ChatEvent::Message(chat_message(m))),
              ChatAction::Message(m) => messages.push(m),
              ChatAction::Remove(id) => events.push(ChatEvent::Delete(id.into())),
            }
          }
          first = false;
          failures = 0;
          if !push(events, cx) {
            return;
          }
          if !pace(messages, wait, &push, cx).await {
            return;
          }
          match page.continuation {
            Some(next) => token = next,
            None => {
              push(
                vec![ChatEvent::Status(Some(
                  t("The chat has ended", "Le chat est terminé").into(),
                ))],
                cx,
              );
              return;
            }
          }
          // `pace` spent the wait already.
          Duration::ZERO
        }
        Some(Err(e)) => {
          failures += 1;
          let wait = (RETRY_STEP * failures).min(MAX_RETRY);
          tracing::info!("live chat: {e}");
          let secs = wait.as_secs();
          let status = tf!(
            "Chat unavailable · retrying in {secs} s",
            "Chat indisponible · nouvel essai dans {secs} s"
          );
          if !push(vec![ChatEvent::Status(Some(status.into()))], cx) {
            return;
          }
          wait
        }
        None => return,
      };
      if !wait.is_zero() {
        cx.background_executor().timer(wait).await;
      }
    }
  })
}

/// Pushes `messages` spread over `span` as they were sent (their gaps scaled down to fit), and
/// waits out the rest of `span`. `false` once the panel is gone.
async fn pace(
  messages: Vec<innertube::ChatMessage>,
  span: Duration,
  push: &impl Fn(Vec<ChatEvent>, &mut gpui::AsyncApp) -> bool,
  cx: &mut gpui::AsyncApp,
) -> bool {
  let times: Vec<u64> = messages.iter().filter_map(|m| m.timestamp_usec).collect();
  let first = times.iter().copied().min().unwrap_or(0);
  let last = times.iter().copied().max().unwrap_or(0);
  let sent_over = Duration::from_micros(last.saturating_sub(first));
  // Slower than sent would let the chat fall behind.
  let scale = if sent_over > span {
    span.as_secs_f64() / sent_over.as_secs_f64()
  } else {
    1.0
  };
  let mut elapsed = Duration::ZERO;
  let mut batch = Vec::new();
  for message in messages {
    let at = message.timestamp_usec.map_or(elapsed, |t| {
      Duration::from_micros(t.saturating_sub(first)).mul_f64(scale)
    });
    // Messages closer than a frame or two go together.
    if at > elapsed + Duration::from_millis(50) {
      if !batch.is_empty() && !push(std::mem::take(&mut batch), cx) {
        return false;
      }
      cx.background_executor().timer(at - elapsed).await;
      elapsed = at;
    }
    batch.push(ChatEvent::Message(chat_message(message)));
  }
  if !batch.is_empty() && !push(batch, cx) {
    return false;
  }
  if span > elapsed {
    cx.background_executor().timer(span - elapsed).await;
  }
  true
}

/// How the chat's user cards find someone mentioned who has not written (an `@handle`): a
/// search of the handle, anonymous so that it stays out of the account's search history, its
/// channel whose handle it is. Answers are kept for the panel's life.
pub fn user_lookup() -> UserLookup {
  let found: Rc<RefCell<HashMap<String, Option<UserProfile>>>> = Rc::default();
  Rc::new(move |query: UserQuery, cx: &mut App| {
    let handle = format!("@{}", query.name.trim_start_matches('@'));
    let key = handle.to_lowercase();
    let found = found.clone();
    if let Some(known) = found.borrow().get(&key).cloned() {
      return cx.spawn(async move |_| known);
    }
    // The chat has their channel already (they wrote): nothing to find.
    if query.channel.is_some() {
      return cx.spawn(async move |_| None);
    }
    let (language, region) = {
      let settings = Settings::get(cx);
      (settings.language.clone(), settings.region.clone())
    };
    let wanted = handle.clone();
    let search = runtime::spawn(cx, async move {
      let client = Client::new(None, None, &language, &region)?;
      let page = client.search(&wanted).await?;
      Ok::<_, innertube::Error>(page.results.into_iter().find_map(|result| {
        match result {
          SearchResult::Channel(channel)
            if channel
              .handle
              .as_deref()
              .is_some_and(|h| same_user(h, &wanted)) =>
          {
            Some(channel)
          }
          _ => None,
        }
      }))
    });
    cx.spawn(async move |_| {
      let channel = match search.await? {
        Ok(channel) => channel,
        Err(e) => {
          tracing::debug!("chat user lookup: {e}");
          return None;
        }
      };
      let profile = channel.map(|channel| UserProfile {
        name: channel.handle.unwrap_or(channel.name).into(),
        avatar: (!channel.avatar.is_empty()).then(|| convert::picture(&channel.avatar)),
        channel: Some(channel.id.into()),
      });
      found.borrow_mut().insert(key, profile.clone());
      profile
    })
  })
}

/// A YouTube message as the panel's.
fn chat_message(message: innertube::ChatMessage) -> ChatMessage {
  let color = message.badges.iter().find_map(|b| match b.kind {
    ChatBadgeKind::Owner => Some(OWNER_COLOR),
    ChatBadgeKind::Moderator => Some(MODERATOR_COLOR),
    ChatBadgeKind::Verified | ChatBadgeKind::Member => None,
  });
  let badges = message
    .badges
    .iter()
    .filter_map(|b| {
      let url = b.image.clone()?;
      Some(Badge {
        title: b.label.clone().into(),
        picture: Picture::sized(move |_, _| url.clone()),
      })
    })
    .collect();
  let author = Author {
    id: message
      .author_id
      .clone()
      .unwrap_or_else(|| message.author.clone())
      .into(),
    name: message.author.into(),
    color: color.map(|rgb| Hsla::from(gpui::rgb(rgb))),
    badges,
    avatar: (!message.author_photo.is_empty()).then(|| convert::picture(&message.author_photo)),
    channel: message.author_id.map(SharedString::from),
  };
  let mut fragments = Vec::new();
  for run in message.runs {
    match run {
      ChatRun::Text(text) => fragments.extend(split_text(&text)),
      ChatRun::Emoji {
        name,
        url: Some(url),
        custom: true,
      } => fragments.push(Fragment::Emote(Emote {
        name: SharedString::from(name),
        picture: Picture::sized(move |_, _| url.clone()),
        // The channel's emojis are square.
        aspect: 1.,
        overlay: false,
        modifiers: Default::default(),
      })),
      ChatRun::Emoji { name, .. } => fragments.push(Fragment::Text(name.into())),
    }
  }
  let mut chat = ChatMessage::new(message.id, Some(author), fragments);
  if let Some(usec) = message.timestamp_usec {
    chat.time = UNIX_EPOCH + Duration::from_micros(usec);
  } else {
    chat.time = SystemTime::now();
  }
  chat
}

/// The signed-in account's handle, as its chat messages show it: read once per session, when a
/// chat first opens, for the red tint of the messages mentioning it.
#[derive(Default)]
struct AccountHandle {
  /// Counts the sessions: a reading from the previous one is dropped.
  generation: u64,
  asked: bool,
  handle: Option<SharedString>,
}

impl Global for AccountHandle {}

/// Forgets the handle: called on every session change.
pub fn reset_account_handle(cx: &mut App) {
  let state = cx.default_global::<AccountHandle>();
  state.generation += 1;
  state.asked = false;
  state.handle = None;
}

/// The viewer for a chat panel ([`ChatPanel::set_viewer`]): the account's handle, read in the
/// background the first time (signed in only).
pub fn viewer(cx: &mut App) -> ui::chat::Viewer {
  let client = cx
    .try_global::<crate::save::Session>()
    .map(|s| s.0.clone())
    .filter(Client::signed_in);
  let state = cx.default_global::<AccountHandle>();
  if let Some(client) = client
    && !state.asked
  {
    state.asked = true;
    let generation = state.generation;
    let task = runtime::spawn(cx, async move { client.account_handle().await });
    cx.spawn(async move |cx| {
      let handle = match task.await {
        Some(Ok(handle)) => handle,
        Some(Err(e)) => return tracing::warn!("account handle: {e}"),
        None => return,
      };
      cx.update(|cx| {
        let state = cx.default_global::<AccountHandle>();
        if state.generation == generation {
          state.handle = handle.map(SharedString::from);
        }
      });
    })
    .detach();
  }
  Rc::new(|cx: &App| cx.try_global::<AccountHandle>()?.handle.clone())
}
