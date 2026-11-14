//! The wall clock, as the shell reads it for the core: instants, and the
//! local times of day they fall at. The core never reads a clock or a time
//! zone. It decides which times it wants (an hour from now, the next
//! morning), and the shell's clock adapter only converts them, across
//! daylight-saving changes.

use std::fmt;

use crate::call::Timestamp;

/// A local time of day, on the Desktop's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
}

impl TimeOfDay {
    /// The time `HH:MM` names, on the 24-hour clock: `"07:00"`, `"21:30"`.
    /// `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (hour, minute) = text.split_once(':')?;
        if hour.len() != 2 || minute.len() != 2 {
            return None;
        }
        let digits = |part: &str| {
            part.bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| part.parse::<u8>().ok())
                .flatten()
        };
        let (hour, minute) = (digits(hour)?, digits(minute)?);
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }
}

impl fmt::Display for TimeOfDay {
    /// `HH:MM`, on the 24-hour clock.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// When calls come back on: an instant, and the local time of day it falls
/// at, as the shell's clock read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    pub at: Timestamp,
    pub shown: TimeOfDay,
}

/// The wall clock now, and the deadlines of the timed choices under Turn
/// calls off, as the shell's clock adapter read them: [`FOR_AN_HOUR_MS`]
/// from `now`, and the next [`MORNING`]. With Quiet Hours set, when they
/// next begin and end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallClock {
    pub now: Timestamp,
    pub for_an_hour: Deadline,
    pub until_morning: Deadline,
    pub quiet_hours: Option<QuietHoursClock>,
}

/// Quiet Hours: the daily times, set by the user, during which calls are
/// off. A start later than the end crosses midnight; the two are never
/// equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QuietHours {
    start: TimeOfDay,
    end: TimeOfDay,
}

impl QuietHours {
    /// From `start` to `end`; `None` if they are the same time, which
    /// would be no time or the whole day.
    #[must_use]
    pub fn new(start: TimeOfDay, end: TimeOfDay) -> Option<Self> {
        (start != end).then_some(Self { start, end })
    }

    /// The Quiet Hours `HH:MM-HH:MM` names, such as `"22:00-07:00"`.
    /// `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (start, end) = text.trim().split_once('-')?;
        Self::new(TimeOfDay::parse(start)?, TimeOfDay::parse(end)?)
    }

    #[must_use]
    pub fn start(&self) -> TimeOfDay {
        self.start
    }

    #[must_use]
    pub fn end(&self) -> TimeOfDay {
        self.end
    }
}

impl fmt::Display for QuietHours {
    /// `HH:MM-HH:MM`, as the configuration file and the command line have
    /// it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}

impl TimeOfDay {
    /// The time `minutes` after midnight; `None` from 24:00 on.
    #[must_use]
    pub fn from_minutes(minutes: u16) -> Option<Self> {
        let hour = u8::try_from(minutes / 60).ok()?;
        let minute = u8::try_from(minutes % 60).ok()?;
        (hour < 24).then_some(Self { hour, minute })
    }

    /// Minutes after midnight.
    #[must_use]
    pub fn minutes(&self) -> u16 {
        u16::from(self.hour) * 60 + u16::from(self.minute)
    }
}

/// When `hours` next begin and next end, as the shell's clock read them.
/// While they are under way, they end before they next begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuietHoursClock {
    pub hours: QuietHours,
    pub next_start: Deadline,
    pub next_end: Deadline,
}

impl QuietHoursClock {
    /// Whether the Quiet Hours are under way.
    #[must_use]
    pub fn under_way(&self) -> bool {
        self.next_end.at < self.next_start.at
    }
}

/// How long "For 1 hour" turns calls off for, in milliseconds.
pub const FOR_AN_HOUR_MS: u64 = 3_600_000;

/// When "Until tomorrow morning" ends: the next 07:00 local time (spec
/// #198).
pub const MORNING: TimeOfDay = TimeOfDay { hour: 7, minute: 0 };
