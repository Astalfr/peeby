//! Dates and hours in the viewer's time zone and language, for what is planned (a Twitch
//! channel's schedule): "Today", "Tomorrow", else the weekday and the date; the hours as the
//! language writes them.

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};

use crate::i18n::{Lang, lang, t};

/// The day of epoch milliseconds `millis`, from `today`: "Today", "Tomorrow", else "Thursday,
/// November 12" ("Jeudi 12 novembre").
pub fn day(millis: i64, today: NaiveDate) -> String {
  let Some(local) = local(millis) else {
    return String::new();
  };
  let date = local.date_naive();
  if date == today {
    return t("Today", "Aujourd'hui").to_owned();
  }
  if today.succ_opt() == Some(date) {
    return t("Tomorrow", "Demain").to_owned();
  }
  let weekday = date.weekday().num_days_from_monday() as usize;
  let month = date.month0() as usize;
  match lang() {
    Lang::Fr => format!("{} {} {}", FR_DAYS[weekday], date.day(), FR_MONTHS[month]),
    Lang::En => format!("{}, {} {}", EN_DAYS[weekday], EN_MONTHS[month], date.day()),
  }
}

/// The hours from `start` to `end` (epoch milliseconds): "13:00 – 17:00", "1:00 PM – 5:00 PM";
/// the start alone without an end.
pub fn hours(start: i64, end: Option<i64>) -> String {
  let Some(from) = local(start).map(clock) else {
    return String::new();
  };
  match end.and_then(local).map(clock) {
    Some(to) => format!("{from} – {to}"),
    None => from,
  }
}

/// Today in the viewer's time zone.
pub fn today() -> NaiveDate {
  Local::now().date_naive()
}

fn local(millis: i64) -> Option<DateTime<Local>> {
  DateTime::from_timestamp_millis(millis).map(|utc| utc.with_timezone(&Local))
}

fn clock(time: DateTime<Local>) -> String {
  match lang() {
    Lang::Fr => format!("{:02}:{:02}", time.hour(), time.minute()),
    Lang::En => {
      let (pm, hour) = time.hour12();
      format!(
        "{hour}:{:02} {}",
        time.minute(),
        if pm { "PM" } else { "AM" }
      )
    }
  }
}

const EN_DAYS: [&str; 7] = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];
const FR_DAYS: [&str; 7] = [
  "Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche",
];
const EN_MONTHS: [&str; 12] = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const FR_MONTHS: [&str; 12] = [
  "janvier",
  "février",
  "mars",
  "avril",
  "mai",
  "juin",
  "juillet",
  "août",
  "septembre",
  "octobre",
  "novembre",
  "décembre",
];

#[cfg(test)]
mod tests {
  use chrono::{Local, TimeZone};

  use super::{day, hours};

  #[test]
  fn days_from_today_and_hours() {
    // Tests run in English.
    let at = |d: u32, h: u32| {
      Local
        .with_ymd_and_hms(2026, 11, d, h, 0, 0)
        .unwrap()
        .timestamp_millis()
    };
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
    assert_eq!(day(at(11, 13), today), "Today");
    assert_eq!(day(at(12, 13), today), "Tomorrow");
    assert_eq!(day(at(13, 13), today), "Friday, November 13");
    assert_eq!(hours(at(13, 13), Some(at(13, 17))), "1:00 PM – 5:00 PM");
    assert_eq!(hours(at(13, 9), None), "9:00 AM");
  }
}
