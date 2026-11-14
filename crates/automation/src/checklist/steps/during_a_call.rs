//! Group 3's scripts: Mute, keypad tones and the Call Pill during a Call.

use std::time::Duration;

use super::incoming::audio_reaches_the_desktop;
use super::{
    ACT_DEADLINE, CALL_DEADLINE, EXPAND_FIRST, call_ends, handled_on_desktop, helper_calls,
    no_call_left, stopped_ringing,
};
use crate::checklist::sitting::{Sitting, Stop, seconds};
use crate::checklist::watch::AppState;

/// How long after a Call connects the Call Window may take to collapse to
/// the Call Pill, while the user moves the pointer off it: 5 s, and time
/// to read the instruction.
const COLLAPSE_DEADLINE: Duration = Duration::from_secs(30);

/// 3.2: mute and unmute on a Call answered on the Desktop; the helper says
/// whether they heard silence, which decides #51.
pub(super) fn mute(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    sitting.say(
        "Click the Mute tile in the Call Window, then keep talking to your helper.",
        &["mute"],
    );
    let muted = sitting.wait_for("Mute", ACT_DEADLINE, |state| {
        state.muted || state.call(call).is_none()
    })?;
    if !muted.state.muted {
        sitting.check(false, "The Call ended before Mute.".to_owned());
        return Ok(());
    }
    sitting.observed("The Desktop muted its microphone.".to_owned());
    let silent = sitting.ask("While you were muted, did your helper hear silence?")?;
    sitting.check(
        silent,
        if silent {
            "Your helper heard silence while you were muted.".to_owned()
        } else {
            "Your helper still heard you while muted: the Pixel ignores MicrophoneVolume 0, so #51 is needed.".to_owned()
        },
    );
    sitting.say(
        "Click the Mute tile again: it now says Unmute.",
        &["unmute"],
    );
    let unmuted = sitting.wait_for("Unmute", ACT_DEADLINE, |state| {
        !state.muted || state.call(call).is_none()
    })?;
    if unmuted.state.call(call).is_none() {
        sitting.check(false, "The Call ended before Unmute.".to_owned());
        return Ok(());
    }
    let heard = sitting.ask("Does your helper hear you again?")?;
    sitting.check(
        heard,
        if heard {
            "Your helper heard you again after Unmute.".to_owned()
        } else {
            "Your helper still couldn't hear you after Unmute.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    Ok(())
}

/// 3.3: keypad tones from the Call Window reach a voicemail menu.
pub(super) fn keypad_tones(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    // The Simulated Phone can't yet place a Call from the Phone's side.
    sitting.say(
        "On the Pixel, call your voicemail, or any number whose menu answers keypad tones.",
        &["skip"],
    );
    let connected = sitting.wait_for("your Call connecting", CALL_DEADLINE, |state| {
        state
            .calls
            .iter()
            .any(|call| call.direction != "incoming" && call.state == "active")
    })?;
    let Some(call) = connected
        .state
        .calls
        .iter()
        .find(|call| call.state == "active")
        .map(|call| call.id)
    else {
        return Err(Stop::TimedOut("your Call connecting".to_owned()));
    };
    sitting.observed("Your Call from the Pixel connected.".to_owned());
    sitting.say(
        &format!(
            "When the menu asks: {EXPAND_FIRST} Then click the Keypad tile in the Call Window and press the digits it wants."
        ),
        &["expand"],
    );
    let answered = sitting.ask("Did the menu respond to the tones?")?;
    sitting.check(
        answered,
        if answered {
            "The menu responded to the keypad tones.".to_owned()
        } else {
            "The menu did not respond to the keypad tones.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up.", &["hang-up"])?;
    Ok(())
}

/// 3.4: on a real Call, the Call Window collapses to the Call Pill about 5 s
/// after it connects; a click on the Call Pill expands it with the
/// keyboard, Escape collapses it, and the keybinding expands it again.
pub(super) fn call_pill(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    sitting.say(
        "Move the pointer off the Call Window and leave it: about 5 s after the Call connected, the Call Window collapses to the Call Pill in the bottom-right corner.",
        &[],
    );
    let collapsed = sitting.wait_for("the Call Pill", COLLAPSE_DEADLINE, |state| {
        state.shows("call-pill") || state.call(call).is_none()
    })?;
    if !collapsed.state.shows("call-pill") {
        sitting.check(
            false,
            "The Call ended before the Call Window collapsed.".to_owned(),
        );
        return Ok(());
    }
    sitting.measure(
        "collapsed_after_answer",
        seconds(collapsed.at.duration_since(answered.at)),
    );
    sitting.observed("The Call Window collapsed to the Call Pill.".to_owned());
    let looks_right = sitting.ask(
        "Is the Call Pill in the bottom-right corner, showing the caller, the timer, Mute and Hang up?",
    )?;
    sitting.check(
        looks_right,
        if looks_right {
            "The Call Pill sat bottom right with the caller, the timer, Mute and Hang up."
                .to_owned()
        } else {
            "The Call Pill was out of place, or missing the caller, the timer, Mute or Hang up."
                .to_owned()
        },
    );
    let expanded = acts_on_call_window(
        sitting,
        call,
        Act {
            request: "Click the Call Pill, away from its buttons.",
            demo_action: "pill-click",
            done: expanded_with_keyboard,
            passed: "A click on the Call Pill expanded the Call Window with the keyboard in it.",
        },
    )?;
    if expanded {
        let in_place = sitting.ask(
            "Did the Call Window open where the Call Pill was, in the bottom-right corner, with its bottom-right corner where the Call Pill's was?",
        )?;
        sitting.check(
            in_place,
            if in_place {
                "The Call Window opened where the Call Pill was, in the bottom-right corner."
                    .to_owned()
            } else {
                "The Call Window didn't open where the Call Pill was.".to_owned()
            },
        );
    }
    let _ = expanded
        && acts_on_call_window(
            sitting,
            call,
            Act {
                request: "Press Escape.",
                demo_action: "escape",
                done: |state| state.shows("call-pill") && !state.shows("call-window"),
                passed: "Escape collapsed the Call Window to the Call Pill.",
            },
        )?
        && acts_on_call_window(
            sitting,
            call,
            Act {
                request: "Press your Desktop Calls keybinding.",
                demo_action: "keybinding",
                done: expanded_with_keyboard,
                passed: "The keybinding expanded the Call Window with the keyboard in it.",
            },
        )?;
    call_ends(sitting, call, "Hang up.", &["hang-up"])?;
    Ok(())
}

/// The Call Window shows expanded, with the keyboard in it.
fn expanded_with_keyboard(state: &AppState) -> bool {
    state.focused("call-window") && !state.shows("call-pill")
}

/// One thing step 3.4 has the user do to the Call Window.
#[derive(Clone, Copy)]
struct Act {
    request: &'static str,
    demo_action: &'static str,
    /// What the app shows once it has worked.
    done: fn(&AppState) -> bool,
    /// The finding when it worked.
    passed: &'static str,
}

/// Has the user do `act`, and records whether the Call Window did as it
/// should. Returns whether it did.
fn acts_on_call_window(sitting: &mut Sitting, call: u32, act: Act) -> Result<bool, Stop> {
    sitting.say(act.request, &[act.demo_action]);
    let seen = sitting.happens_within(act.request, ACT_DEADLINE, |state| {
        (act.done)(state) || state.call(call).is_none()
    })?;
    let worked = seen.as_ref().is_some_and(|seen| (act.done)(&seen.state));
    sitting.check(
        worked,
        if worked {
            act.passed.to_owned()
        } else {
            format!(
                "After \"{}\" the Call Window didn't do as the step says.",
                act.request
            )
        },
    );
    Ok(worked)
}
