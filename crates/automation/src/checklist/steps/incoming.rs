//! Group 2's scripts for answering and declining an incoming Call, on the
//! Desktop and on the Pixel, and with the keybinding.

use super::missed_calls::{NOTIFY_WINDOW, Refresh, call_history_refresh};
use super::{
    ACT_DEADLINE, APP_DEADLINE, AUDIO_TARGET, STAYS_ON_PHONE, call_ends, handled_on_desktop,
    helper_calls, millis, stopped_ringing,
};
use crate::checklist::sitting::{Seen, Sitting, Stop, seconds};

/// Waits until the Call's audio reaches the Desktop after Answer at
/// `answered`, and checks the audio latency and the codec.
pub(super) fn audio_reaches_the_desktop(
    sitting: &mut Sitting,
    call: u32,
    answered: &Seen,
) -> Result<bool, Stop> {
    let audio = sitting.wait_for(
        "the Call audio reaching the Desktop",
        APP_DEADLINE,
        |state| state.audio_location == "desktop" || state.call(call).is_none(),
    )?;
    if audio.state.audio_location != "desktop" {
        sitting.check(
            false,
            "The Call ended before its audio reached the Desktop.".to_owned(),
        );
        return Ok(false);
    }
    let latency = audio.at - answered.at;
    sitting.measure("audio_latency_ms", millis(latency));
    sitting.check(
        latency <= AUDIO_TARGET,
        format!(
            "Audio on the Desktop {} after Answer (target {}).",
            seconds(latency),
            seconds(AUDIO_TARGET)
        ),
    );
    let codec = if audio.state.codec.is_empty() {
        sitting
            .wait_for("the codec being reported", APP_DEADLINE, |state| {
                !state.codec.is_empty()
            })?
            .state
    } else {
        audio.state
    };
    sitting.measure("codec", codec.codec.clone());
    sitting.measure("wideband", codec.wideband);
    sitting.observed(format!(
        "Codec: {} ({}).",
        codec.codec,
        if codec.wideband {
            "wideband"
        } else {
            "narrowband"
        }
    ));
    Ok(true)
}

/// 2.1: the helper calls, the user answers on the Desktop. Observes the
/// Banner latency, the audio latency, the codec and the hang-up path; asks
/// whether the helper heard clearly.
pub(super) fn answer_on_the_desktop(sitting: &mut Sitting) -> Result<(), Stop> {
    let started_us = crate::checklist::results::unix_micros();
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("Answer on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Answer on the Desktop (answered on the Pixel, or ended).".to_owned(),
        );
        return Ok(());
    }
    if !audio_reaches_the_desktop(sitting, call, &answered)? {
        return Ok(());
    }
    let heard =
        sitting.ask("Does your helper hear you clearly, with no echo of their own voice?")?;
    sitting.check(
        heard,
        if heard {
            "Your helper heard you clearly, with no echo.".to_owned()
        } else {
            "Your helper did not hear you clearly, or heard an echo.".to_owned()
        },
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;
    let fallback = sitting
        .log_since(started_us)
        .iter()
        .any(|line| line.event_kind == "phone-command-fallback");
    if fallback {
        sitting.measure("hang_up", "org.ofono.VoiceCall");
        sitting.observed(
            "Hang-up: Call1.Hangup doesn't exist here, so the org.ofono.VoiceCall fallback hung up.".to_owned(),
        );
    } else {
        sitting.measure("hang_up", "Call1");
        sitting.observed(
            "Hang-up: no fallback was logged, so Call1.Hangup worked, if you hung up on the Desktop.".to_owned(),
        );
    }
    Ok(())
}

/// 2.2: answered on the Pixel, the Banner closes and the audio stays there.
pub(super) fn answer_on_the_pixel(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say(
        "Answer on the Pixel, not on the Desktop.",
        &["answer-on-phone"],
    );
    let answered = sitting.wait_for("the Call being answered", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call was handled on the Desktop, not on the Pixel.".to_owned(),
        );
        return Ok(());
    }
    if answered.state.call(call).is_none() {
        sitting.check(false, "The Call ended without being answered.".to_owned());
        return Ok(());
    }
    let closed = sitting.wait_for("the Banner closing", APP_DEADLINE, |state| {
        !state.shows("banner")
    })?;
    sitting.check(
        true,
        format!(
            "The Banner closed {} after the Pixel answered.",
            seconds(closed.at - answered.at)
        ),
    );
    let moved =
        sitting.happens_within("the audio moving to the Desktop", STAYS_ON_PHONE, |state| {
            state.audio_location == "desktop"
        })?;
    sitting.check(
        moved.is_none(),
        if moved.is_none() {
            format!(
                "The audio stayed on the Pixel for {} after answering.",
                seconds(STAYS_ON_PHONE)
            )
        } else {
            "The audio moved to the Desktop, though the Call was answered on the Pixel.".to_owned()
        },
    );
    call_ends(
        sitting,
        call,
        "Hang up, on either device.",
        &["hang-up-on-phone"],
    )?;
    Ok(())
}

/// 2.3: declined on the Desktop, the Call ends unanswered.
pub(super) fn decline_on_the_desktop(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say("Click Decline on the Banner.", &["decline"]);
    let declined = sitting.wait_for("Decline on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without Decline on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let ended = sitting.wait_for("the declined Call ending", APP_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    let unanswered = ended.state.call(call).is_none();
    sitting.check(
        unanswered,
        if unanswered {
            format!(
                "The Call ended {} after Decline, unanswered.",
                seconds(ended.at - declined.at)
            )
        } else {
            "The Call was answered instead of ending.".to_owned()
        },
    );
    if !unanswered {
        call_ends(sitting, call, "Hang up, on either device.", &["hang-up"])?;
    }
    Ok(())
}

/// 2.4: declined on the Pixel, the Banner closes, and no Missed Call is
/// notified: the Pixel's missed Call History, refreshed as the Call ends,
/// doesn't list a Call it declined.
pub(super) fn decline_on_the_pixel(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you.")?;
    sitting.say(
        "Decline on the Pixel, not on the Desktop.",
        &["decline-on-phone"],
    );
    let declined = sitting.wait_for("Decline on the Pixel", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call was handled on the Desktop, not on the Pixel.".to_owned(),
        );
        return Ok(());
    }
    if declined.state.call(call).is_some() {
        sitting.check(
            false,
            "The Call was answered instead of declined.".to_owned(),
        );
        call_ends(
            sitting,
            call,
            "Hang up, on either device.",
            &["hang-up-on-phone"],
        )?;
        return Ok(());
    }
    let closed = sitting.wait_for("the Banner closing", APP_DEADLINE, |state| {
        !state.shows("banner")
    })?;
    sitting.check(
        true,
        format!(
            "The Call ended unanswered and the Banner closed {} after the Pixel declined.",
            seconds(closed.at - declined.at)
        ),
    );
    let refresh = call_history_refresh(sitting, &declined)?;
    let notified =
        sitting.happens_within("a Missed Call notification", NOTIFY_WINDOW, |state| {
            state.missed_call(call).is_some()
        })?;
    sitting.check(
        notified.is_none(),
        match (notified.is_none(), refresh) {
            (true, Refresh::Done) => {
                "No Missed Call was notified: the Pixel's missed Call History, \
                                      refreshed as the Call ended, doesn't list it."
                    .to_owned()
            }
            (true, Refresh::Failed | Refresh::Unseen) => "No Missed Call was notified.".to_owned(),
            (false, Refresh::Done) => "A Missed Call was notified: the Pixel lists a Call it \
                                       declined among its missed Calls."
                .to_owned(),
            (false, Refresh::Failed) => "A Missed Call was notified, because the Call History \
                                         couldn't be refreshed to tell a declined Call apart."
                .to_owned(),
            (false, Refresh::Unseen) => "A Missed Call was notified.".to_owned(),
        },
    );
    Ok(())
}

/// 2.8: the keybinding focuses the Banner, Escape gives focus back, A
/// answers and, on a second Call, D declines.
pub(super) fn keybinding(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    sitting.say("Press your Desktop Calls keybinding.", &["keybinding"]);
    let focused = sitting.wait_for("the Banner taking keyboard focus", ACT_DEADLINE, |state| {
        state.focused("banner") || stopped_ringing(state, call)
    })?;
    sitting.check(
        focused.state.focused("banner"),
        "The keybinding gave the Banner keyboard focus.".to_owned(),
    );
    sitting.say("Press Escape.", &["escape"]);
    let released = sitting.wait_for(
        "the Banner giving keyboard focus back",
        ACT_DEADLINE,
        |state| !state.focused("banner") || stopped_ringing(state, call),
    )?;
    sitting.check(
        released.state.shows("banner") && !released.state.focused("banner"),
        "Escape gave keyboard focus back, and the Banner stayed.".to_owned(),
    );
    sitting.say("Press the keybinding again, then A.", &["keybinding", "a"]);
    let answered = sitting.wait_for("A answering on the Desktop", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&answered.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without A answering it on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let audio = sitting.wait_for(
        "the Call audio reaching the Desktop",
        APP_DEADLINE,
        |state| state.audio_location == "desktop" || state.call(call).is_none(),
    )?;
    sitting.check(
        audio.state.audio_location == "desktop",
        "A answered the Call on the Desktop.".to_owned(),
    );
    call_ends(sitting, call, "Hang up from the Call Window.", &["hang-up"])?;

    let call = helper_calls(sitting, "Ask your helper to call you again.")?;
    sitting.say("Press the keybinding, then D.", &["keybinding", "d"]);
    let declined = sitting.wait_for("D declining", ACT_DEADLINE, |state| {
        handled_on_desktop(state, call) || stopped_ringing(state, call)
    })?;
    if !handled_on_desktop(&declined.state, call) {
        sitting.check(
            false,
            "The Call stopped ringing without D declining it on the Desktop.".to_owned(),
        );
        return Ok(());
    }
    let ended = sitting.wait_for("the declined Call ending", APP_DEADLINE, |state| {
        stopped_ringing(state, call)
    })?;
    sitting.check(
        ended.state.call(call).is_none(),
        "D declined the Call.".to_owned(),
    );
    Ok(())
}
