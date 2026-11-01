//! Dates as Twitch writes them.

/// Milliseconds since the epoch of an ISO 8601 UTC date as Twitch writes them
/// (`2026-11-01T20:10:51.158Z`); `None` for any other shape.
pub fn epoch_millis(date: &str) -> Option<i64> {
  let date = date.trim().strip_suffix('Z')?;
  let (day, time) = date.split_once('T')?;
  let mut day = day.split('-').map(|p| p.parse::<i64>().ok());
  let (year, month, mday) = (day.next()??, day.next()??, day.next()??);
  let (hms, fraction) = time.split_once('.').unwrap_or((time, "0"));
  let mut hms = hms.split(':').map(|p| p.parse::<i64>().ok());
  let (hour, minute, second) = (hms.next()??, hms.next()??, hms.next()??);
  let millis = format!("{fraction:0<3}").get(..3)?.parse::<i64>().ok()?;
  // Days from the civil date (Howard Hinnant's algorithm).
  let y = if month <= 2 { year - 1 } else { year };
  let era = y.div_euclid(400);
  let yoe = y - era * 400;
  let mp = (month + 9) % 12;
  let doy = (153 * mp + 2) / 5 + mday - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  let days = era * 146_097 + doe - 719_468;
  Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}
