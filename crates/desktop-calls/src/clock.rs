//! The wall clock, for the core (ticket #203): it reports the time on the
//! minute, at each instant the core asks for, and at once on resume, since
//! monotonic timers stop while the Desktop is suspended. It also converts
//! between instants and local times of day in the Desktop's time zone
//! (`TZ`, or `/etc/localtime`), across daylight-saving changes. It only
//! converts: which times to ask for is the core's choice, such as when the
//! Quiet Hours it names next begin and end.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use desktop_calls_core::clock::{FOR_AN_HOUR_MS, MORNING};
use desktop_calls_core::{
    Deadline, Input, QuietHours, QuietHoursClock, TimeOfDay, Timestamp, WallClock,
};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_util::StreamExt as _;
use futures_util::future::{Either, select};

use crate::logging::EventKind;

const MINUTE_MS: u64 = 60_000;
const DAY_MS: u64 = 24 * 60 * MINUTE_MS;

/// Why the clock should be read again before the next minute.
pub enum Wake {
    /// The core asked for the clock at this instant.
    At(Timestamp),
    /// The Desktop resumed: timers were held while it slept.
    Resumed,
    /// The core asked for when these Quiet Hours next begin and end, or
    /// for none.
    QuietHours(Option<QuietHours>),
}

/// The wall clock now.
pub fn now() -> Timestamp {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(u64::try_from(now).unwrap_or(u64::MAX))
}

/// The wall clock at `now`, with the deadlines of the timed choices, and
/// when `quiet_hours` next begin and end.
pub fn wall_clock(now: Timestamp, quiet_hours: Option<QuietHours>) -> WallClock {
    WallClock {
        now,
        for_an_hour: deadline(Timestamp(now.0.saturating_add(FOR_AN_HOUR_MS))),
        until_morning: deadline(next(MORNING, now)),
        quiet_hours: quiet_hours.map(|hours| QuietHoursClock {
            hours,
            next_start: deadline(next(hours.start(), now)),
            next_end: deadline(next(hours.end(), now)),
        }),
    }
}

/// `at`, with the local time of day it falls at.
pub fn deadline(at: Timestamp) -> Deadline {
    Deadline {
        at,
        shown: local_time(at),
    }
}

/// The local time of day at `at`, to the minute.
pub fn local_time(at: Timestamp) -> TimeOfDay {
    let local = seconds(at).and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok());
    // GLib can't place it: read it as UTC rather than not at all.
    let (hour, minute) = local.map_or_else(
        || {
            read_as_utc(at);
            let minutes = i32::try_from(at.0 % DAY_MS / MINUTE_MS).unwrap_or(0);
            (minutes / 60, minutes % 60)
        },
        |local| (local.hour(), local.minute()),
    );
    TimeOfDay {
        hour: u8::try_from(hour).unwrap_or(0),
        minute: u8::try_from(minute).unwrap_or(0),
    }
}

/// The first instant after `after` at which the local clock reads `time`:
/// later today, or else tomorrow. On a day when `time` doesn't exist (the
/// clocks go forward over it), the instant GLib moves it to.
pub fn next(time: TimeOfDay, after: Timestamp) -> Timestamp {
    let found = seconds(after).and_then(|seconds| {
        let today = glib::DateTime::from_unix_local(seconds).ok()?;
        [0, 1, 2].into_iter().find_map(|days| {
            let day = today.add_days(days).ok()?;
            let candidate = glib::DateTime::new(
                &glib::TimeZone::local(),
                day.year(),
                day.month(),
                day.day_of_month(),
                i32::from(time.hour),
                i32::from(time.minute),
                0.0,
            )
            .ok()?;
            let millis = u64::try_from(candidate.to_unix())
                .ok()?
                .checked_mul(1_000)?;
            (millis > after.0).then_some(Timestamp(millis))
        })
    });
    // GLib can't place it: read the clock as UTC rather than not at all.
    found.unwrap_or_else(|| {
        read_as_utc(after);
        let wanted = (u64::from(time.hour) * 60 + u64::from(time.minute)) * MINUTE_MS;
        let midnight = after.0 - after.0 % DAY_MS;
        let today = midnight + wanted;
        Timestamp(if today > after.0 {
            today
        } else {
            today + DAY_MS
        })
    })
}

/// GLib couldn't place `at` in the Desktop's time zone, so the time shown
/// or chosen is read as UTC.
fn read_as_utc(at: Timestamp) {
    tracing::warn!(
        event_kind = %EventKind::ClockFailed,
        error_kind = "local-time-unreadable",
        at_ms = at.0,
        "Couldn't read the local time; showing UTC instead"
    );
}

fn seconds(at: Timestamp) -> Option<i64> {
    i64::try_from(at.0 / 1_000).ok()
}

/// Reports the wall clock to `inputs` on each minute, at each instant
/// `wakes` asks for, when the Quiet Hours begin and end, at once on each
/// resume, and at once when the Quiet Hours to read change (from
/// `quiet_hours`, as kept at startup), until either closes. Every instant asked for is kept until it passes, so an
/// earlier one, such as a deadline since replaced, can't hide a later one.
pub async fn run(
    mut wakes: UnboundedReceiver<Wake>,
    inputs: UnboundedSender<Input>,
    mut quiet_hours: Option<QuietHours>,
) {
    let mut wake_at: BTreeSet<Timestamp> = BTreeSet::new();
    loop {
        let now = now();
        wake_at.retain(|at| *at > now);
        let clock = wall_clock(now, quiet_hours);
        if inputs.unbounded_send(Input::ClockTicked(clock)).is_err() {
            return;
        }
        // The Quiet Hours' edges are on the minute by the local clock,
        // which needn't be on the minute by UTC's.
        let edges = clock
            .quiet_hours
            .map(|quiet| [quiet.next_start.at, quiet.next_end.at]);
        let next_minute = Timestamp(now.0 - now.0 % MINUTE_MS + MINUTE_MS);
        let until = wake_at
            .iter()
            .copied()
            .chain(edges.into_iter().flatten())
            .fold(next_minute, Timestamp::min)
            .0;
        let timer = glib::timeout_future(Duration::from_millis(until.saturating_sub(now.0)));
        match select(timer, wakes.next()).await {
            Either::Left(_) | Either::Right((Some(Wake::Resumed), _)) => {}
            Either::Right((Some(Wake::At(at)), _)) => {
                wake_at.insert(at);
            }
            Either::Right((Some(Wake::QuietHours(hours)), _)) => quiet_hours = hours,
            Either::Right((None, _)) => return,
        }
    }
}
