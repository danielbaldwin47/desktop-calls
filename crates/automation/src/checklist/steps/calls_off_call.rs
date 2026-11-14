//! 6.7's script: a Call placed from the Dialer while calls are off (ticket
//! #206). Enter connects the Pixel for that one Call and places it; once it
//! ends the Pixel disconnects again and calls are still off. The Call goes
//! only to the Test Number: any other is hung up and fails the step.

use std::time::Duration;

use super::keyboard_and_look::{test_number_or_why_not, went_to};
use super::{CALL_DEADLINE, call_ends, millis, no_call_left};
use crate::checklist::sitting::{Sitting, Stop, seconds};

/// How long the user may take to click through the Indicator's menu.
const MENU_DEADLINE: Duration = Duration::from_secs(120);
/// How long the Pixel may take to disconnect again once the Call ends:
/// "within a few seconds", with BlueZ's block and link teardown.
const DISCONNECTS_AGAIN: Duration = Duration::from_secs(10);

pub(super) fn call_while_calls_off(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    if sitting.now().calls_off != "off" {
        sitting.say(
            "Click the Indicator, choose Turn calls off, then Until I turn them on.",
            &["calls-off"],
        );
        sitting.wait_for("calls turning off", MENU_DEADLINE, |state| {
            state.calls_off == "off" && state.connection_state != "connected"
        })?;
    }
    sitting.observed("Calls are off and the Phone is disconnected.".to_owned());

    let mark = sitting.mark();
    sitting.say(
        "Press your Desktop Calls keybinding to open the Dialer, type the Test Number, and press Enter.",
        &["keybinding", "type-test-number", "enter", "phone-dials"],
    );
    let placed = sitting.wait_for("your Call being placed", CALL_DEADLINE, |state| {
        state.calls.iter().any(|call| call.direction == "outgoing")
    })?;
    let Some(call) = placed
        .state
        .calls
        .iter()
        .find(|call| call.direction == "outgoing")
        .map(|call| call.id)
    else {
        return Err(Stop::TimedOut("your Call being placed".to_owned()));
    };
    if !went_to(sitting, call, &test_number)? {
        return Ok(());
    }
    sitting.observed("The Call went to the Test Number.".to_owned());
    sitting.check(
        sitting
            .seen_since(mark, |state| {
                state.shows("dialer") && state.calls_off == "off"
            })
            .is_some(),
        "The Dialer opened while calls were off.".to_owned(),
    );
    let connected = placed.state.connection_state == "connected";
    sitting.check(
        connected && placed.state.calls_off_pending,
        format!(
            "The Pixel connected and the Call to the Test Number was placed, with calls {}.",
            if placed.state.calls_off_pending {
                "going off again after it"
            } else {
                "not set to go off again after it"
            }
        ),
    );

    let ended = call_ends(
        sitting,
        call,
        "Once the Call connects, hang up from the Call Window.",
        &["callee-answers", "hang-up"],
    )?;
    let off_again = sitting.happens_within(
        "the Pixel disconnecting again",
        DISCONNECTS_AGAIN,
        |state| state.calls_off == "off" && state.connection_state != "connected",
    )?;
    let Some(off_again) = off_again else {
        sitting.check(
            false,
            format!(
                "The Pixel was still connected, or calls not off, {} after the Call ended.",
                seconds(DISCONNECTS_AGAIN)
            ),
        );
        return Ok(());
    };
    let took = off_again.at.saturating_duration_since(ended.at);
    sitting.measure("disconnected_again_ms", millis(took));
    sitting.observed(format!(
        "The Pixel disconnected again {} after the Call ended, and `desktop-calls status` says calls are off.",
        seconds(took)
    ));
    Ok(())
}
