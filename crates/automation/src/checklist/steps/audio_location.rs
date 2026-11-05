//! Where a Call's audio goes: Handoff to the Phone and back (3.1), and
//! answering with the earbuds connected to the Pixel (5.1 and 5.2).

use std::time::Duration;

use super::incoming::audio_reaches_the_desktop;
use super::{
    ACT_DEADLINE, LOG_DEADLINE, STAYS_ON_PHONE, call_ends, handled_on_desktop, helper_calls,
    logged_since, stopped_ringing,
};
use crate::checklist::results::unix_micros;
use crate::checklist::sitting::{Seen, Sitting, Stop, seconds};

/// How long a Handoff may take before the step gives up on it: the Phone
/// may refuse twice, 3 s each, before the Call Window says so (#13).
const HANDOFF_DEADLINE: Duration = Duration::from_secs(10);

/// The plain sentence the Call Window shows when the Phone keeps the audio
/// (#13).
const PHONE_KEPT_IT: &str = "Your phone kept the audio";

/// The helper calls and the user answers on the Desktop. Returns the Call
/// and the moment Answer was pressed, or `None` once the step has recorded
/// why it can't go on.
fn answered_here(sitting: &mut Sitting) -> Result<Option<(u32, Seen)>, Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop.".to_owned(),
        );
        return Ok(None);
    }
    Ok(Some((call, answered)))
}

/// Asks the user to move the Call's audio to `location`, waits for the
/// app to ask the Phone, then for the audio to arrive, and records how
/// long the Handoff took by the app's log, from `handoff-requested` to
/// `audio-moved` (#80, #83, #99). Returns whether the audio got there.
fn audio_moves(
    sitting: &mut Sitting,
    call: u32,
    location: &str,
    request: &str,
    demo_action: &str,
) -> Result<bool, Stop> {
    let from_us = unix_micros();
    sitting.say(request, &[demo_action]);
    let requested = logged_since(
        sitting,
        from_us,
        ACT_DEADLINE,
        |line| {
            line.event_kind == "handoff-requested"
                && line.audio_location.as_deref() == Some(location)
        },
        |state| state.call(call).is_none(),
    )?;
    let Some(requested) = requested else {
        sitting.check(
            false,
            "The app never logged a Handoff request (handoff-requested).".to_owned(),
        );
        return Ok(false);
    };
    let moved = sitting.happens_within("the audio moving", HANDOFF_DEADLINE, |state| {
        state.audio_location == location || state.call(call).is_none()
    })?;
    let place = if location == "phone" {
        "Phone"
    } else {
        "Desktop"
    };
    match moved {
        Some(moved) if moved.state.audio_location == location => {
            let arrived = logged_since(
                sitting,
                requested.at_us,
                LOG_DEADLINE,
                |line| {
                    line.event_kind == "audio-moved"
                        && line.audio_location.as_deref() == Some(location)
                },
                |_| false,
            )?;
            match arrived {
                Some(arrived) => sitting.observed(format!(
                    "The audio moved to the {place} {} after the Handoff was asked for, by the app's log.",
                    seconds(Duration::from_micros(
                        arrived.at_us.saturating_sub(requested.at_us)
                    ))
                )),
                None => sitting.check(
                    false,
                    format!(
                        "The audio moved to the {place}, but the app logged no audio-moved line, so the Handoff couldn't be timed."
                    ),
                ),
            }
            // A Handoff to the Phone bounces hands-free (ADR-0005): the
            // Call Window offers the way back only once it has returned.
            let back = if location == "phone" {
                "Move audio here"
            } else {
                "Move audio to phone"
            };
            let settled =
                sitting.happens_within("the Handoff settling", HANDOFF_DEADLINE, |state| {
                    state.connection_state == "connected"
                        && state
                            .surfaces
                            .get("call-window")
                            .and_then(|window| window.get("handoff"))
                            .is_some_and(|label| label == back)
                })?;
            sitting.check(
                settled.is_some(),
                format!("The Call Window offered {back} afterwards."),
            );
            Ok(settled.is_some())
        }
        Some(_) => {
            sitting.check(false, "The Call ended during the Handoff.".to_owned());
            Ok(false)
        }
        None => {
            sitting.observed(format!(
                "The audio didn't reach the {place} within {} of the Handoff request.",
                seconds(HANDOFF_DEADLINE)
            ));
            Ok(false)
        }
    }
}

/// 3.1: Handoff to the Phone and back, twice, with the audio confirmed by
/// ear on the Pixel's earpiece.
pub(super) fn handoff_twice(sitting: &mut Sitting) -> Result<(), Stop> {
    let Some((call, answered)) = answered_here(sitting)? else {
        return Ok(());
    };
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    for round in 1..=2 {
        if !audio_moves(
            sitting,
            call,
            "phone",
            "Click Move audio to phone in the Call Window.",
            "move-audio-to-phone",
        )? {
            sitting.check(
                false,
                format!("Handoff {round} to the Phone didn't move the audio."),
            );
            return Ok(());
        }
        let earpiece = sitting.ask("Do you hear your helper on the Pixel's earpiece?")?;
        sitting.check(
            earpiece,
            if earpiece {
                format!("Handoff {round}: your helper was on the Pixel's earpiece.")
            } else {
                format!("Handoff {round}: the audio didn't land on the Pixel's earpiece.")
            },
        );
        if !audio_moves(
            sitting,
            call,
            "desktop",
            "Click Move audio here in the Call Window.",
            "move-audio-here",
        )? {
            sitting.check(
                false,
                format!("Handoff {round} back to the Desktop didn't move the audio."),
            );
            return Ok(());
        }
    }
    sitting.measure("handoffs", 4);
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    Ok(())
}

/// Records where the audio went, either way: on the Desktop, or kept by
/// the Phone. When the Phone kept it, checks the Call Window says so
/// plainly with the Call still live, and has the user try Move audio here
/// once more, as #13's expectations ask.
fn record_where_the_audio_went(
    sitting: &mut Sitting,
    call: u32,
    on_the_desktop: bool,
) -> Result<(), Stop> {
    if on_the_desktop {
        sitting.measure("audio_location", "desktop");
        return Ok(());
    }
    if sitting.now().call(call).is_none() {
        sitting.check(false, "The Call ended.".to_owned());
        return Ok(());
    }
    sitting.measure("audio_location", "phone");
    let told = sitting.ask(&format!(
        "Does the Call Window say \"{PHONE_KEPT_IT}\", with the Call still going?"
    ))?;
    sitting.check(
        told,
        if told {
            "The Phone kept the audio, and the Call Window said so plainly.".to_owned()
        } else {
            "The Phone kept the audio, and the Call Window didn't say so.".to_owned()
        },
    );
    let again = audio_moves(
        sitting,
        call,
        "desktop",
        "Click Move audio here once more.",
        "move-audio-here",
    )?;
    sitting.measure("moved_on_second_try", again);
    Ok(())
}

/// 5.1: with the earbuds connected to the Pixel, answer on the Desktop.
/// AOSP makes the device that answers the active one, so the audio is
/// expected here (#13); the Phone keeping it is recorded, not hidden.
pub(super) fn earbuds_answer_on_the_desktop(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(
        "Connect the earbuds to the Pixel and put them in.",
        &["earbuds-on"],
    );
    let Some((call, answered)) = answered_here(sitting)? else {
        return Ok(());
    };
    let reached = sitting.happens_within(
        "the audio reaching the Desktop",
        HANDOFF_DEADLINE,
        |state| state.audio_location == "desktop" || state.call(call).is_none(),
    )?;
    let on_the_desktop = reached
        .as_ref()
        .is_some_and(|seen| seen.state.audio_location == "desktop");
    if let Some(seen) = reached.filter(|_| on_the_desktop) {
        sitting.observed(format!(
            "The audio came to the Desktop {} after Answer.",
            seconds(seen.at.saturating_duration_since(answered.at))
        ));
    }
    record_where_the_audio_went(sitting, call, on_the_desktop)?;
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    Ok(())
}

/// 5.2: answered on the earbuds, the audio stays with the Phone; then Move
/// audio here, which AOSP may refuse while the earbuds are active (#13).
pub(super) fn earbuds_answer_on_the_earbuds(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(
        "Keep the earbuds connected to the Pixel and in your ears.",
        &["earbuds-on"],
    );
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Answer by tapping the earbuds.", &["answer-on-phone"]);
    let answered = sitting.wait_for("the Call being answered", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if handled_on_desktop(&answered.state, call) || answered.state.call(call).is_none() {
        sitting.check(false, "The Call wasn't answered on the earbuds.".to_owned());
        return Ok(());
    }
    let left = sitting.happens_within("the audio leaving the Phone", STAYS_ON_PHONE, |state| {
        state.audio_location == "desktop"
    })?;
    sitting.check(
        left.is_none(),
        format!(
            "The audio {} the Phone for {} after you answered on the earbuds.",
            if left.is_none() {
                "stayed with"
            } else {
                "didn't stay with"
            },
            seconds(STAYS_ON_PHONE)
        ),
    );
    let on_the_desktop = audio_moves(
        sitting,
        call,
        "desktop",
        "Click Move audio here in the Call Window.",
        "move-audio-here",
    )?;
    record_where_the_audio_went(sitting, call, on_the_desktop)?;
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    Ok(())
}
