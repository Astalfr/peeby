//! A live's Hype Train, pinned over its chat as Twitch shows it (ADR 0032): a bar in the
//! channel's color, filled with the level's progress, its level in a badge, the share done and
//! the level's time left, and that time along its edge. Unfolded, the level's progress in points,
//! the emotes it gives, the conductors and the channel's record. A train about to start ("3 more
//! contributions…") shows the same way, and an ended one stays [`ENDED_SHOWN`] with the levels it
//! completed and the emotes it unlocked. The state is `twitch::hype_train`'s, held by `points::PointsView`: read from GQL when
//! the page opens (and after each PubSub reconnection), then moved by the PubSub topic.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, App, ElementId, FontWeight, Hsla, Window, div, px, relative};
use gpui_component::{h_flex, v_flex};
use twitch::hype_train::{Approaching, Conductor, Emote, HypeTrain, Source, State};

use ui::assets::Icon;
use ui::chat::{LEAD, Pinned, Unfolded};
use ui::i18n::t;
use ui::picture::{self, Picture};
use ui::{tf, theme};

use super::cards;
use super::points::{eased_share, percent};

/// How long an ended train stays pinned with the level it reached, from its end.
pub const ENDED_SHOWN: Duration = Duration::from_secs(30);
/// How long past a level's end (or an approach's) Peeby waits for Twitch's message before it
/// takes the train as ended (or the approach as gone).
pub const GRACE_MS: i64 = 10_000;
/// How long an approach lasts (Twitch's `kickoff.duration`, 5 minutes on every channel seen),
/// for its time line: Twitch gives only its end.
const APPROACH_MS: i64 = 5 * 60 * 1000;
/// Twitch's purple, for a channel without its own color.
const PURPLE: u32 = 0x9147ff;
/// A Golden Kappa Train's gold.
const GOLD: u32 = 0xe6a700;
/// Side of an emote reward, logical pixels.
const EMOTE: f32 = 28.;
/// Side of a conductor's avatar, logical pixels.
const AVATAR: f32 = 24.;
/// Height of the card's progress bar, logical pixels.
const BAR: f32 = 8.;

/// The pinned item for the channel's train or its approach, if one shows at `now` (milliseconds
/// since the epoch).
pub fn pinned(
  state: &State,
  unfolded: &Unfolded,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Option<Pinned> {
  if let Some(train) = state.train.as_ref().filter(|t| shown(t, now)) {
    return Some(train_item(train, unfolded, now, window, cx));
  }
  let approaching = state.approaching.as_ref()?;
  Some(approaching_item(approaching, unfolded, now, window, cx))
}

/// The pinned item's id, while one shows.
pub fn shown_id(state: &State, now: i64) -> Option<&str> {
  match &state.train {
    Some(train) if shown(train, now) => Some(&train.id),
    _ => state.approaching.as_ref().map(|a| a.id.as_str()),
  }
}

/// A train shows while it runs and [`ENDED_SHOWN`] after its end.
fn shown(train: &HypeTrain, now: i64) -> bool {
  match train.ended_at {
    None => true,
    Some(end) => now - end < ENDED_SHOWN.as_millis() as i64,
  }
}

/// The share left of an ended train's [`ENDED_SHOWN`], 1 to 0.
fn ended_left(end: i64, now: i64) -> f32 {
  let shown = ENDED_SHOWN.as_millis() as f32;
  (1. - (now - end) as f32 / shown).clamp(0., 1.)
}

/// The train's color: gold for a Golden Kappa Train, else the channel's, else Twitch's purple.
fn accent(color: Option<&str>, golden: bool) -> Hsla {
  let rgb = if golden {
    GOLD
  } else {
    color
      .and_then(|hex| u32::from_str_radix(hex, 16).ok())
      .unwrap_or(PURPLE)
  };
  gpui::rgb(rgb).into()
}

/// What a train is called.
fn kind(golden: bool, treasure: bool) -> &'static str {
  if golden {
    "Golden Kappa Train"
  } else if treasure {
    t("Treasure Train", "Train au trésor")
  } else {
    "Hype Train"
  }
}

fn train_item(
  train: &HypeTrain,
  unfolded: &Unfolded,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Pinned {
  let accent = accent(train.color.as_deref(), train.golden_kappa);
  let level = train.progress.level;
  let ended = train.ended_at;
  let record = if train.record_beaten() {
    t(" · new record!", " · nouveau record !")
  } else {
    ""
  };
  let done = train.levels_done();
  let (title, status) = match ended {
    Some(_) if done == 0 => (
      t(
        "The train didn't complete level 1",
        "Le train n'a pas complété le niveau 1",
      )
      .to_owned(),
      t("Ended", "Terminé").to_owned(),
    ),
    Some(_) => (
      tf!(
        "Level {done} completed{record}",
        "Niveau {done} complété{record}"
      ),
      t("Ended", "Terminé").to_owned(),
    ),
    None => {
      let share = percent(format!("{:.0}", train.progress.share() * 100.));
      let left = train
        .remaining(now)
        .map(|s| format!(" · {}", ui::player::format_time(s as f64)))
        .unwrap_or_default();
      (
        tf!("Level {level}{record}", "Niveau {level}{record}"),
        format!("{share}{left}"),
      )
    }
  };
  // The level's progress slides there as contributions come; full once ended.
  let fill = match ended {
    Some(_) => 1.,
    None => eased_share(
      format!("hype-fill-{}", train.id),
      f64::from(train.progress.share()),
      window,
      cx,
    ),
  };
  Pinned {
    id: train.id.clone().into(),
    icon: Icon::Train,
    accent,
    kind: kind(train.golden_kappa, train.treasure).into(),
    title: title.into(),
    status: status.into(),
    body: unfolded
      .is(&train.id)
      .then(|| train_body(train, accent, window, cx)),
    time_left: match ended {
      Some(end) => Some(ended_left(end, now)),
      None => train.time_left(now),
    },
    lead: Some(match ended {
      Some(_) if done == 0 => train_badge(accent),
      Some(_) => level_badge(done, accent),
      None => level_badge(level, accent),
    }),
    fill: Some(fill),
  }
}

fn approaching_item(
  approaching: &Approaching,
  unfolded: &Unfolded,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Pinned {
  let accent = accent(approaching.color.as_deref(), approaching.golden_kappa);
  let left = approaching.left();
  let title = if cards::plural(u64::from(left)) {
    tf!(
      "{left} more contributions start a {}",
      "Encore {left} contributions pour lancer un {}",
      kind(approaching.golden_kappa, approaching.treasure)
    )
  } else {
    tf!(
      "{left} more contribution starts a {}",
      "Encore {left} contribution pour lancer un {}",
      kind(approaching.golden_kappa, approaching.treasure)
    )
  };
  let time = approaching
    .expires_at
    .map(|end| ((end - now) / 1000).max(0))
    .map(|s| format!(" · {}", ui::player::format_time(s as f64)))
    .unwrap_or_default();
  let share = if approaching.goal == 0 {
    0.
  } else {
    approaching.count as f32 / approaching.goal as f32
  };
  Pinned {
    id: approaching.id.clone().into(),
    icon: Icon::Train,
    accent,
    kind: "Hype Train".into(),
    title: title.into(),
    status: tf!("Approaching{time}", "En approche{time}").into(),
    body: unfolded
      .is(&approaching.id)
      .then(|| approaching_body(approaching, accent, window, cx)),
    time_left: approaching
      .expires_at
      .map(|end| ((end - now) as f32 / APPROACH_MS as f32).clamp(0., 1.)),
    lead: Some(train_badge(accent)),
    fill: Some(eased_share(
      format!("hype-fill-{}", approaching.id),
      f64::from(share),
      window,
      cx,
    )),
  }
}

/// The level in a square of the train's color, as Twitch's banner shows it.
fn level_badge(level: u32, accent: Hsla) -> AnyElement {
  badge(accent)
    .text_size(px(if level >= 100 { 10. } else { 13. }))
    .font_weight(FontWeight::BOLD)
    .child(level.to_string())
    .into_any_element()
}

/// The train's icon in a square of its color.
fn train_badge(accent: Hsla) -> AnyElement {
  let ink = ink(accent);
  badge(accent)
    .child(
      gpui_component::Icon::new(Icon::Train)
        .size(px(16.))
        .text_color(ink),
    )
    .into_any_element()
}

fn badge(accent: Hsla) -> gpui::Div {
  div()
    .size(px(LEAD))
    .flex()
    .items_center()
    .justify_center()
    .rounded(px(6.))
    .bg(accent)
    .text_color(ink(accent))
}

/// Text that reads on `color`: black on a light one, white on a dark one.
fn ink(color: Hsla) -> Hsla {
  let rgb = color.to_rgb();
  let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
  if luminance > 0.6 {
    gpui::black()
  } else {
    gpui::white()
  }
}

/// A bar of `share` in `accent`.
fn bar(share: f32, accent: Hsla) -> AnyElement {
  div()
    .h(px(BAR))
    .w_full()
    .rounded_full()
    .bg(theme::surface_hover())
    .overflow_hidden()
    .child(
      div()
        .h_full()
        .w(relative(share.clamp(0., 1.)))
        .rounded_full()
        .bg(accent),
    )
    .into_any_element()
}

fn label(text: impl Into<gpui::SharedString>) -> AnyElement {
  div()
    .text_xs()
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(theme::muted())
    .child(text.into())
    .into_any_element()
}

fn note(text: impl Into<gpui::SharedString>) -> AnyElement {
  div()
    .text_xs()
    .text_color(theme::muted())
    .child(text.into())
    .into_any_element()
}

/// `1 point`, `1,200 points`.
fn points(n: u64) -> String {
  let count = cards::grouped(n);
  if cards::plural(n) {
    format!("{count} points")
  } else {
    format!("{count} point")
  }
}

fn train_body(train: &HypeTrain, accent: Hsla, window: &mut Window, cx: &mut App) -> AnyElement {
  let p = &train.progress;
  let level = p.level;
  let ended = train.ended_at.is_some();
  let share = if ended {
    1.
  } else {
    eased_share(
      format!("hype-bar-{}", train.id),
      f64::from(p.share()),
      window,
      cx,
    )
  };
  let record = match train.record {
    Some(_) if train.record_beaten() => {
      Some(t("New channel record!", "Nouveau record de la chaîne !").to_owned())
    }
    Some(record) => Some(tf!(
      "Channel record: level {record}",
      "Record de la chaîne : niveau {record}"
    )),
    None => None,
  };
  let total = points(p.total);
  v_flex()
    .gap_2()
    .child(
      v_flex()
        .gap_1()
        .child(
          h_flex()
            .justify_between()
            .gap_2()
            .child(
              div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(if ended {
                  let done = train.levels_done();
                  if cards::plural(u64::from(done)) {
                    tf!("{done} levels completed", "{done} niveaux complétés")
                  } else {
                    tf!("{done} level completed", "{done} niveau complété")
                  }
                } else {
                  tf!("Level {level}", "Niveau {level}")
                }),
            )
            .when(!ended, |row| {
              row.child(
                div()
                  .text_xs()
                  .text_color(theme::muted())
                  .child(format!("{} / {}", cards::grouped(p.value), points(p.goal))),
              )
            }),
        )
        .child(bar(share, accent))
        .child(note(match record {
          Some(record) => tf!("Total: {total} · {record}", "Total : {total} · {record}"),
          None => tf!("Total: {total}", "Total : {total}"),
        })),
    )
    // Under way, what the level gives; ended, what the train gave.
    .children(match ended {
      false => (!p.rewards.is_empty()).then(|| {
        rewards(
          tf!("Level {level} rewards", "Récompenses du niveau {level}"),
          &p.rewards,
          window,
          cx,
        )
      }),
      true => (!train.unlocked.is_empty()).then(|| {
        rewards(
          t("Emotes unlocked", "Emotes débloquées").into(),
          &train.unlocked,
          window,
          cx,
        )
      }),
    })
    .children((!train.conductors.is_empty()).then(|| conductors(&train.conductors, window, cx)))
    .when(train.golden_kappa, |el| {
      el.child(note(t(
        "Golden Kappa Train: its contributors get the Golden Kappa emote for 24 hours.",
        "Golden Kappa Train : ses contributeurs reçoivent l'emote Golden Kappa pour 24 heures.",
      )))
    })
    .when(train.treasure, |el| {
      el.child(note(t(
        "Treasure Train: subscriptions cost less while it runs.",
        "Train au trésor : les abonnements coûtent moins cher tant qu'il roule.",
      )))
    })
    .when(!ended, |el| {
      el.child(note(t(
        "Subscribe, gift subs or cheer bits to move it forward: each level unlocks its emotes for the contributors.",
        "Abonnez-vous, offrez des abonnements ou envoyez des bits pour le faire avancer : chaque niveau débloque ses emotes pour les contributeurs.",
      )))
    })
    .into_any_element()
}

fn approaching_body(
  approaching: &Approaching,
  accent: Hsla,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let share = if approaching.goal == 0 {
    0.
  } else {
    approaching.count as f32 / approaching.goal as f32
  };
  let (count, goal) = (approaching.count, approaching.goal);
  v_flex()
    .gap_2()
    .child(
      v_flex()
        .gap_1()
        .child(div().text_xs().text_color(theme::muted()).child(tf!(
          "{count} of {goal} contributions",
          "{count} contributions sur {goal}"
        )))
        .child(bar(share, accent)),
    )
    .children((!approaching.rewards.is_empty()).then(|| {
      rewards(
        t("Level 1 rewards", "Récompenses du niveau 1").into(),
        &approaching.rewards,
        window,
        cx,
      )
    }))
    .child(note(t(
      "Subscribe, gift subs or cheer bits in time to start it.",
      "Abonnez-vous, offrez des abonnements ou envoyez des bits à temps pour le lancer.",
    )))
    .into_any_element()
}

/// A level's emotes under `title`, each named in its tooltip.
fn rewards(title: String, emotes: &[Emote], window: &mut Window, cx: &mut App) -> AnyElement {
  v_flex()
    .gap_1()
    .child(label(title))
    .child(
      h_flex()
        .flex_wrap()
        .gap_1()
        .children(emotes.iter().map(|emote| emote_image(emote, window, cx))),
    )
    .into_any_element()
}

fn emote_image(emote: &Emote, window: &mut Window, cx: &mut App) -> AnyElement {
  // The fake chat's are drawn in memory, under its own scheme.
  let url = if emote.id.contains("://") {
    emote.id.clone()
  } else {
    twitch::chat::irc::emote_url(&emote.id, 2)
  };
  let token: gpui::SharedString = emote.token.clone().into();
  div()
    .id(ElementId::Name(format!("hype-emote-{}", emote.id).into()))
    .size(px(EMOTE))
    .flex()
    .items_center()
    .justify_center()
    .children(picture::logo(
      &Picture::sized(move |_, _| url.clone()),
      EMOTE,
      EMOTE,
      window,
      cx,
    ))
    .tooltip(move |window, cx| {
      gpui_component::tooltip::Tooltip::new(token.clone()).build(window, cx)
    })
    .into_any_element()
}

/// The train's conductors: the top contributor of subscriptions, then of bits.
fn conductors(conductors: &[Conductor], window: &mut Window, cx: &mut App) -> AnyElement {
  v_flex()
    .gap_1()
    .child(label(t("Conductors", "Conducteurs")))
    .children(conductors.iter().map(|c| {
      let avatar = c
        .avatar
        .clone()
        .map(|url| picture::avatar(&Picture::sized(move |_, _| url.clone()), AVATAR, window, cx));
      h_flex()
        .gap_2()
        .items_center()
        .child(div().size(px(AVATAR)).flex_none().children(avatar))
        .child(
          div()
            .flex_1()
            .min_w_0()
            .text_sm()
            .truncate()
            .child(c.name.clone()),
        )
        .child(note(match c.source {
          Source::Subs => t("Subs", "Abonnements"),
          Source::Bits => "Bits",
          Source::Other => t("Other", "Autre"),
        }))
    }))
    .into_any_element()
}
