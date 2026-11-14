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
/// from `now`, and the next [`MORNING`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallClock {
    pub now: Timestamp,
    pub for_an_hour: Deadline,
    pub until_morning: Deadline,
}

/// How long "For 1 hour" turns calls off for, in milliseconds.
pub const FOR_AN_HOUR_MS: u64 = 3_600_000;

/// When "Until tomorrow morning" ends: the next 07:00 local time (spec
/// #198).
pub const MORNING: TimeOfDay = TimeOfDay { hour: 7, minute: 0 };
