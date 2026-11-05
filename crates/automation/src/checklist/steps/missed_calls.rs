//! Group 2's scripts for Calls that ring out: Missed Call notifications,
//! withheld numbers and calling back from the Dialer.

use std::time::{Duration, Instant};

use super::{
    ACT_DEADLINE, LOG_LOOKBACK_US, TALK_DEADLINE, call_ends, helper_calls, helper_calls_as, millis,
    no_call_left, stopped_ringing, unix_micros_at,
};
use crate::checklist::sitting::{Seen, Sitting, Stop, seconds};
use crate::checklist::watch::{AppMissedCall, AppState};

/// How long after the Call History refresh a Missed Call notification may
/// take to show.
pub(super) const NOTIFY_WINDOW: Duration = Duration::from_secs(2);

/// How long the Call History refresh may take after a Call ends: one PBAP
/// pull from the Phone.
const REFRESH_DEADLINE: Duration = Duration::from_secs(30);

/// How the Call History refresh that follows a Call went, as the app's log
/// tells it.
#[derive(Clone, Copy)]
pub(super) enum Refresh {
    Done,
    Failed,
    /// The log said neither within [`REFRESH_DEADLINE`].
    Unseen,
}

/// Waits for the app to log how the Call History refresh after `ended`
/// went.
pub(super) fn call_history_refresh(sitting: &mut Sitting, ended: &Seen) -> Result<Refresh, Stop> {
    let since = unix_micros_at(ended.at).saturating_sub(LOG_LOOKBACK_US);
    let end = Instant::now() + REFRESH_DEADLINE;
    loop {
        let logged =
            sitting
                .log_since(since)
                .into_iter()
                .find_map(|line| match line.event_kind.as_str() {
                    "call-history-refreshed" => Some(Refresh::Done),
                    "call-history-refresh-failed" => Some(Refresh::Failed),
                    _ => None,
                });
        if let Some(refresh) = logged {
            return Ok(refresh);
        }
        if Instant::now() >= end {
            return Ok(Refresh::Unseen);
        }
        // Nothing to wait for on the control interface: this only passes
        // the time while still reading what the user types.
        sitting.happens_within("the Call History refresh", LOG_POLL, |_| false)?;
    }
}

/// How often to read the app's log while waiting for a line.
const LOG_POLL: Duration = Duration::from_millis(250);

/// Waits for the helper's Call to ring out, then for its Missed Call
/// notification. Returns the Missed Call, or `None` if the step can't go
/// on, with the reason in the findings.
fn rings_out(sitting: &mut Sitting, call: u32) -> Result<Option<AppMissedCall>, Stop> {
    sitting.say(
        "Don't answer or decline, on either device: let it ring until it stops.",
        &["ring-out"],
    );
    let ended = sitting.wait_for("the Call ringing out", TALK_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    if ended.state.call(call).is_some() {
        sitting.check(
            false,
            "The Call was answered instead of ringing out.".to_owned(),
        );
        call_ends(sitting, call, "Hang up, on either device.", &["hang-up"])?;
        return Ok(None);
    }
    let refresh = call_history_refresh(sitting, &ended)?;
    let notified =
        sitting.happens_within("the Missed Call notification", NOTIFY_WINDOW, |state| {
            state.missed_call(call).is_some()
        })?;
    let Some(missed) = notified
        .as_ref()
        .and_then(|seen| seen.state.missed_call(call).cloned())
    else {
        sitting.check(
            false,
            match refresh {
                Refresh::Done => "No Missed Call was notified: the Pixel's missed Call History, \
                                  refreshed as the Call ended, doesn't list it."
                    .to_owned(),
                Refresh::Failed | Refresh::Unseen => "No Missed Call was notified.".to_owned(),
            },
        );
        return Ok(None);
    };
    if let Some(seen) = &notified {
        let after = seen.at - ended.at;
        sitting.measure("missed_call_notified_ms", millis(after));
        sitting.check(
            true,
            format!(
                "A Missed Call was notified {} after the Call stopped ringing{}.",
                seconds(after),
                match refresh {
                    Refresh::Done => ", confirmed against the Pixel's missed Call History",
                    Refresh::Failed => ", unconfirmed: the Call History couldn't be refreshed",
                    Refresh::Unseen => "",
                }
            ),
        );
    }
    Ok(Some(missed))
}

/// Asks the user to click the Missed Call notification, and waits for the
/// Dialer to open with keyboard focus. Returns the Dialer's selected row,
/// or `None` if it didn't open, with the reason in the findings.
fn notification_opens_the_dialer(sitting: &mut Sitting) -> Result<Option<String>, Stop> {
    sitting.say(
        "Click the notification itself, not one of its buttons.",
        &["click-notification"],
    );
    let opened = sitting.happens_within("the Dialer opening", ACT_DEADLINE, |state| {
        state.focused("dialer")
    })?;
    let selected = opened
        .as_ref()
        .and_then(|seen| seen.state.dialer_selection())
        .map(str::to_owned);
    sitting.check(
        selected.is_some(),
        if selected.is_some() {
            "Clicking the notification opened the Dialer with keyboard focus.".to_owned()
        } else {
            "Clicking the notification didn't open the Dialer with a row selected.".to_owned()
        },
    );
    Ok(selected)
}

/// 2.5: a Call that rings out is notified as missed; clicking the
/// notification opens the Dialer on it, and Enter calls back.
pub(super) fn ring_out(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(
        sitting,
        "Ask your helper to call you, and to let it ring until it stops.",
    )?;
    let Some(missed) = rings_out(sitting, call)? else {
        return Ok(());
    };
    let shown = sitting.ask(
        "Does a notification say you missed a call from your helper, by name or number, with the time?",
    )?;
    sitting.check(
        shown,
        if shown {
            "The notification names the caller and gives the time.".to_owned()
        } else {
            "The notification didn't name the caller with the time.".to_owned()
        },
    );
    let Some(selected) = notification_opens_the_dialer(sitting)? else {
        return Ok(());
    };
    // The row reads the caller's name or number, which stay out of the
    // result file.
    let on_the_missed_call = selected.contains("Missed");
    sitting.check(
        on_the_missed_call,
        if on_the_missed_call {
            "The Dialer opened with the Missed Call selected.".to_owned()
        } else {
            "The Dialer opened with another row than the Missed Call selected.".to_owned()
        },
    );
    sitting.say("Press Enter.", &["enter", "phone-dials"]);
    let placed = sitting.happens_within("the Call back", ACT_DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|known| known.direction == "outgoing")
    })?;
    let Some(back) = placed.and_then(|seen| {
        seen.state
            .calls
            .iter()
            .find(|known| known.direction == "outgoing")
            .cloned()
    }) else {
        sitting.check(false, "Enter placed no Call.".to_owned());
        return Ok(());
    };
    let same_number = !missed.withheld && back.number == missed.number;
    sitting.check(
        same_number,
        if same_number {
            "Enter called back the number that rang.".to_owned()
        } else {
            "Enter called a different number from the one that rang.".to_owned()
        },
    );
    call_ends(
        sitting,
        back.id,
        "Your helper's phone rings: hang up, on either device.",
        &["hang-up"],
    )?;
    Ok(())
}

/// 2.6: a withheld number rings as "Withheld number", and ringing out is
/// notified as missed; the Dialer says it can't be called back, and Enter
/// on it explains instead of calling.
pub(super) fn withheld_ring_out(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls_as(
        sitting,
        "Ask your helper to call you with their number withheld: on most phones, #31# before \
         the number. They let it ring until it stops.",
        "call-withheld",
    )?;
    let caller = sitting
        .now()
        .surfaces
        .get("banner")
        .and_then(|banner| banner.get("caller"))
        .cloned()
        .unwrap_or_default();
    let shows_withheld = caller == "Withheld number";
    sitting.check(
        shows_withheld,
        if shows_withheld {
            "The Banner showed \"Withheld number\".".to_owned()
        } else if sitting.now().call(call).is_none() {
            "The Call stopped ringing before the runner read the Banner.".to_owned()
        } else {
            // A number in the evidence would be the helper's: say only
            // that one was shown.
            "The Banner showed a caller, not \"Withheld number\": the helper's number wasn't \
             withheld."
                .to_owned()
        },
    );
    let Some(missed) = rings_out(sitting, call)? else {
        return Ok(());
    };
    sitting.check(
        missed.withheld,
        if missed.withheld {
            "The Missed Call was notified as from a withheld number.".to_owned()
        } else {
            "The Missed Call was notified with a number.".to_owned()
        },
    );
    let shown = sitting.ask(
        "Does a notification say you missed a call from a withheld number, and that it can't be called back?",
    )?;
    sitting.check(
        shown,
        if shown {
            "The notification says the withheld number can't be called back.".to_owned()
        } else {
            "The notification didn't say the withheld number can't be called back.".to_owned()
        },
    );
    let Some(selected) = notification_opens_the_dialer(sitting)? else {
        return Ok(());
    };
    let says_so = selected.contains("Withheld number") && selected.contains("can't be called back");
    sitting.check(
        says_so,
        if says_so {
            format!("The Dialer opened on the Missed Call, reading {selected:?}.")
        } else {
            "The Dialer's selected row isn't the withheld Missed Call.".to_owned()
        },
    );
    sitting.say("Press Enter.", &["enter"]);
    let answered = sitting.happens_within("the Dialer explaining", ACT_DEADLINE, |state| {
        dialer_notice(state).is_some()
            || state
                .calls
                .iter()
                .any(|known| known.direction == "outgoing")
    })?;
    let notice = answered
        .as_ref()
        .and_then(|seen| dialer_notice(&seen.state))
        .map(str::to_owned);
    let dialled = answered.as_ref().is_some_and(|seen| {
        seen.state
            .calls
            .iter()
            .any(|known| known.direction == "outgoing")
    });
    sitting.check(
        notice.is_some() && !dialled,
        match (&notice, dialled) {
            (Some(notice), false) => format!("Enter explained instead of calling: {notice:?}"),
            (_, true) => "Enter placed a Call, though the number was withheld.".to_owned(),
            (None, false) => "Enter neither explained nor called.".to_owned(),
        },
    );
    if dialled {
        no_call_left(sitting)?;
    }
    sitting.say("Press Escape to close the Dialer.", &["escape"]);
    sitting.wait_for("the Dialer closing", ACT_DEADLINE, |state| {
        !state.shows("dialer")
    })?;
    Ok(())
}

/// The Dialer's notice, such as why a row can't be called, while it shows
/// one.
fn dialer_notice(state: &AppState) -> Option<&str> {
    state
        .surfaces
        .get("dialer")
        .and_then(|dialer| dialer.get("notice"))
        .map(String::as_str)
}
