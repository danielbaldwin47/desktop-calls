//! The Quiet Hours Window (ticket #209): a switch and two times, opened
//! from the Indicator's Quiet Hours… entry. Each change is saved at once,
//! as `SetQuietHours` or `ClearQuietHours` would: there is no confirm.
//!
//! - Switched off, it keeps the last times for as long as it is open, and
//!   offers 22:00 to 07:00 the first time.
//! - Up and Down step a time by 15 minutes, round midnight. Digits typed
//!   fill `HH:MM` from the left, a first digit over 2 being the hour's
//!   second; four save the time, and leaving the field saves fewer, the
//!   rest read as zeros ("22" is 22:00, "223" is 22:30) and one digit
//!   alone as an hour ("2" is 02:00). A time that can't be read is never
//!   saved: the field shows the last time again.
//! - A start equal to the end is shown with a notice and not saved.

use super::State;
use super::input::{Effect, Input};
use crate::call::Timestamp;
use crate::clock::{Deadline, QuietHours, TimeOfDay};

/// What the window offers the first time it is switched on.
const OFFERED_START: TimeOfDay = TimeOfDay {
    hour: 22,
    minute: 0,
};
const OFFERED_END: TimeOfDay = TimeOfDay { hour: 7, minute: 0 };
/// How far Up and Down step a time, in minutes.
const STEP_MINUTES: u16 = 15;
const DAY_MINUTES: u16 = 24 * 60;
/// From this hour on, a start later today is "tonight".
const EVENING_HOUR: u8 = 17;

/// One of the window's two times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuietHoursField {
    Start,
    End,
}

/// The window, while open, and how many times it has opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct QuietHoursWindows {
    open: Option<Window>,
    openings: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    opened: u32,
    start: TimeOfDay,
    end: TimeOfDay,
    /// Digits typed into a field, not yet saved as its time.
    typing: Option<(QuietHoursField, String)>,
}

/// What the Quiet Hours Window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietHoursWindowView {
    /// The switch: whether Quiet Hours are set.
    pub on: bool,
    /// The times shown: the Quiet Hours set, or while switched off the
    /// last ones, dimmed.
    pub start: TimeOfDay,
    pub end: TimeOfDay,
    /// Digits typed into a field, shown in place of its time.
    pub typing: Option<(QuietHoursField, String)>,
    /// The start shown equals the end, so nothing was saved.
    pub refused: bool,
    /// What the sentence under the times says.
    pub sentence: QuietHoursSentence,
    /// The spans of the day the 24-hour bar fills, in minutes after
    /// midnight: the Quiet Hours saved, as the sentence says them; two
    /// pieces when they cross midnight, and none while switched off.
    pub off_spans: Vec<(u16, u16)>,
    /// Different each time the window is asked for, so the shell raises it
    /// and gives the switch keyboard focus.
    pub opened: u32,
}

/// The sentence under the times: what was saved, and when calls next go
/// off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuietHoursSentence {
    /// Switched off.
    Off,
    /// Saved: calls next go off at `start` on `day`, once the clock has
    /// read the Quiet Hours, and come back at `end`.
    Saved {
        start: TimeOfDay,
        end: TimeOfDay,
        day: Option<QuietHoursDay>,
    },
    /// Saved, and calls are off for them now, until `end`.
    OffNow { start: TimeOfDay, end: TimeOfDay },
    /// Saved, and calls go off for them when the last Call ends.
    Pending { start: TimeOfDay, end: TimeOfDay },
}

/// When Quiet Hours next begin, as the sentence says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuietHoursDay {
    Today,
    Tonight,
    Tomorrow,
}

/// The spans of the day the 24-hour bar fills for `hours`, in minutes
/// after midnight: one, or two for Quiet Hours crossing midnight.
fn off_spans(hours: QuietHours) -> Vec<(u16, u16)> {
    let (start, end) = (hours.start().minutes(), hours.end().minutes());
    if start < end {
        vec![(start, end)]
    } else {
        [(0, end), (start, DAY_MINUTES)]
            .into_iter()
            .filter(|(from, to)| from < to)
            .collect()
    }
}

impl State {
    /// What the Quiet Hours Window shows, while it is open.
    pub(super) fn quiet_hours_window_view(&self) -> Option<QuietHoursWindowView> {
        let window = self.quiet_hours_windows.open.as_ref()?;
        let on = self.quiet_hours.is_some();
        Some(QuietHoursWindowView {
            on,
            start: window.start,
            end: window.end,
            typing: window.typing.clone(),
            refused: on && window.start == window.end,
            sentence: self.quiet_hours.map_or(QuietHoursSentence::Off, |hours| {
                self.quiet_hours_sentence(hours)
            }),
            off_spans: self.quiet_hours.map(off_spans).unwrap_or_default(),
            opened: window.opened,
        })
    }

    fn quiet_hours_sentence(&self, hours: QuietHours) -> QuietHoursSentence {
        let (start, end) = (hours.start(), hours.end());
        if self.calls_off_for_quiet_hours() {
            return if self.calls_off_pending() {
                QuietHoursSentence::Pending { start, end }
            } else {
                QuietHoursSentence::OffNow { start, end }
            };
        }
        let day = self
            .quiet_hours_clock()
            .zip(self.clock)
            .map(|(quiet, clock)| day_of(quiet.next_start, clock.now));
        QuietHoursSentence::Saved { start, end, day }
    }
}

/// Which day `start` falls on, seen from `now`: today if it is no further
/// off than its own time is after midnight.
fn day_of(start: Deadline, now: Timestamp) -> QuietHoursDay {
    let minutes_away = start.at.0.saturating_sub(now.0) / 60_000;
    if minutes_away > u64::from(start.shown.minutes()) {
        QuietHoursDay::Tomorrow
    } else if start.shown.hour >= EVENING_HOUR {
        QuietHoursDay::Tonight
    } else {
        QuietHoursDay::Today
    }
}

/// What the window and its user report.
pub(super) fn quiet_hours_window_input(
    state: &mut State,
    input: &Input,
    effects: &mut Vec<Effect>,
) {
    if let Input::OpenQuietHoursWindow = input {
        open(state);
        return;
    }
    let switched_on = state.quiet_hours.is_some();
    let Some(window) = state.quiet_hours_windows.open.as_mut() else {
        return;
    };
    match *input {
        Input::CloseQuietHoursWindow => state.quiet_hours_windows.open = None,
        Input::SwitchQuietHours(on) => {
            window.typing = None;
            if on {
                save(state, effects);
            } else {
                set(state, None, effects);
            }
        }
        // Switched off, the times can't be changed.
        Input::QuietHoursStep { .. }
        | Input::QuietHoursDigit { .. }
        | Input::QuietHoursErase(_)
        | Input::QuietHoursFieldLeft(_)
            if !switched_on => {}
        Input::QuietHoursStep { field, later } => {
            window.typing = None;
            let shown = window.time(field).minutes();
            let stepped = if later {
                (shown + STEP_MINUTES) % DAY_MINUTES
            } else {
                (shown + DAY_MINUTES - STEP_MINUTES) % DAY_MINUTES
            };
            if let Some(time) = TimeOfDay::from_minutes(stepped) {
                window.set_time(field, time);
                save(state, effects);
            }
        }
        Input::QuietHoursDigit { field, digit } => {
            let typing = window.typing_in(field);
            if digit <= 9 && typing.len() < 4 {
                if typing.is_empty() && digit > 2 {
                    typing.push('0');
                }
                typing.push(char::from(b'0' + digit));
            }
            if typing.len() == 4 {
                read_typing(state, effects);
            }
        }
        Input::QuietHoursErase(field) => {
            window.typing_in(field).pop();
        }
        Input::QuietHoursFieldLeft(field)
            if window
                .typing
                .as_ref()
                .is_some_and(|(typed, _)| *typed == field) =>
        {
            read_typing(state, effects);
            if let Some(window) = state.quiet_hours_windows.open.as_mut() {
                window.typing = None;
            }
        }
        _ => {}
    }
}

impl Window {
    fn time(&self, field: QuietHoursField) -> TimeOfDay {
        match field {
            QuietHoursField::Start => self.start,
            QuietHoursField::End => self.end,
        }
    }

    fn set_time(&mut self, field: QuietHoursField, time: TimeOfDay) {
        match field {
            QuietHoursField::Start => self.start = time,
            QuietHoursField::End => self.end = time,
        }
    }

    /// The digits typed into `field`, starting afresh if another field
    /// had them.
    fn typing_in(&mut self, field: QuietHoursField) -> &mut String {
        if self
            .typing
            .as_ref()
            .is_none_or(|(typed, _)| *typed != field)
        {
            self.typing = Some((field, String::new()));
        }
        let (_, digits) = self.typing.get_or_insert_with(|| (field, String::new()));
        digits
    }
}

/// The time the digits typed read as, the missing ones as zeros, saved if
/// it is one; four that aren't one stay shown until the field is left.
fn read_typing(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(window) = state.quiet_hours_windows.open.as_mut() else {
        return;
    };
    let Some((field, digits)) = window.typing.clone() else {
        return;
    };
    // One digit is an hour: 2 is 02:00.
    let digits = if digits.len() == 1 {
        format!("0{digits}")
    } else {
        digits
    };
    let time = (!digits.is_empty())
        .then(|| format!("{digits:0<4}"))
        .and_then(|padded| TimeOfDay::parse(&format!("{}:{}", &padded[..2], &padded[2..])));
    if let Some(time) = time {
        window.typing = None;
        window.set_time(field, time);
        save(state, effects);
    }
}

/// Saves the times shown, while switched on or being switched on, unless
/// they are the same.
fn save(state: &mut State, effects: &mut Vec<Effect>) {
    let Some(window) = &state.quiet_hours_windows.open else {
        return;
    };
    if let Some(hours) = QuietHours::new(window.start, window.end) {
        set(state, Some(hours), effects);
    }
}

fn set(state: &mut State, hours: Option<QuietHours>, effects: &mut Vec<Effect>) {
    let input = hours.map_or(Input::ClearQuietHours, Input::SetQuietHours);
    super::quiet_hours::quiet_hours_input(state, &input, effects);
}

/// Opens the window on the Quiet Hours set, or raises it if it is open.
fn open(state: &mut State) {
    let windows = &mut state.quiet_hours_windows;
    windows.openings += 1;
    let opened = windows.openings;
    if let Some(window) = windows.open.as_mut() {
        window.opened = opened;
        return;
    }
    let (start, end) = state
        .quiet_hours
        .map_or((OFFERED_START, OFFERED_END), |hours| {
            (hours.start(), hours.end())
        });
    state.quiet_hours_windows.open = Some(Window {
        opened,
        start,
        end,
        typing: None,
    });
}

/// Quiet Hours were set, from the window or elsewhere: the open window
/// shows them.
pub(super) fn follow(state: &mut State) {
    if let (Some(window), Some(hours)) =
        (state.quiet_hours_windows.open.as_mut(), state.quiet_hours)
    {
        window.start = hours.start();
        window.end = hours.end();
    }
}
