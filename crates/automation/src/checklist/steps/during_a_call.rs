//! Group 3's scripts: Mute and keypad tones during a Call.

use super::incoming::audio_reaches_the_desktop;
use super::{
    ACT_DEADLINE, CALL_DEADLINE, call_ends, handled_on_desktop, helper_calls, no_call_left,
    stopped_ringing,
};
use crate::checklist::sitting::{Sitting, Stop};

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
        "Click Mute in the Call Window, then keep talking to your helper.",
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
    sitting.say("Click Unmute.", &["unmute"]);
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
        "When the menu asks, click Keypad in the Call Window and press the digits it wants.",
        &[],
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
