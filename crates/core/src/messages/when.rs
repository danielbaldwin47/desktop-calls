//! When a Call was: in the Missed Call notification, and in the Dialer's
//! right-hand column.

use crate::call::Timestamp;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// 1970-01-01 was a Thursday.
const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// When a Call was, as the Missed Call notification says it: the time if
/// today, "Yesterday" and the time, otherwise the date, with the year if not
/// this year. `utc_offset_seconds` is the Desktop's time zone's offset now.
#[must_use]
pub fn when(at: Timestamp, now: Timestamp, utc_offset_seconds: i64) -> String {
    shown(at, now, utc_offset_seconds, |days_ago, _, time| {
        (days_ago == 1).then(|| format!("Yesterday {time}"))
    })
}

/// When a Call was, in the Dialer's right-hand column: the time if today,
/// the weekday and the time within the last week ("Tue 14:05"), otherwise
/// the date, with the year if not this year. `utc_offset_seconds` is the
/// Desktop's time zone's offset now.
#[must_use]
pub fn dialer_time(at: Timestamp, now: Timestamp, utc_offset_seconds: i64) -> String {
    shown(at, now, utc_offset_seconds, |days_ago, day, time| {
        (1..=6).contains(&days_ago).then(|| {
            let weekday = usize::try_from(day.rem_euclid(7)).unwrap_or(0);
            format!("{} {time}", WEEKDAYS[weekday])
        })
    })
}

/// When `at` was: the time if today, `recent(days ago, day, time)` if it
/// says, otherwise the date, with the year if not this year.
fn shown(
    at: Timestamp,
    now: Timestamp,
    utc_offset_seconds: i64,
    recent: impl FnOnce(i64, i64, &str) -> Option<String>,
) -> String {
    let local = |moment: Timestamp| {
        i64::try_from(moment.0 / 1_000).unwrap_or(i64::MAX) + utc_offset_seconds
    };
    let (at, now) = (local(at), local(now));
    let time = format!(
        "{:02}:{:02}",
        at.rem_euclid(86_400) / 3_600,
        at.rem_euclid(3_600) / 60
    );
    let (day, today) = (at.div_euclid(86_400), now.div_euclid(86_400));
    if today == day {
        return time;
    }
    if let Some(recent) = recent(today - day, day, &time) {
        return recent;
    }
    let (year, month, day_of_month) = civil_date(day);
    let month = MONTHS[month - 1];
    if year == civil_date(today).0 {
        format!("{day_of_month} {month}")
    } else {
        format!("{day_of_month} {month} {year}")
    }
}

/// The year, month (1 to 12) and day of the month of the day `days` after
/// 1970-01-01, in the proleptic Gregorian calendar (Howard Hinnant's
/// `civil_from_days`).
fn civil_date(days: i64) -> (i64, usize, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, usize::try_from(month).unwrap_or(1), day)
}
