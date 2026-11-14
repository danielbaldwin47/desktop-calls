//! Group 7's scripts: the Incoming Call Banner over a notification, the
//! Indicator in the bar, and the Dialer's first frame.

use std::thread;
use std::time::{Duration, Instant};

use super::{
    ACT_DEADLINE, CALL_DEADLINE, DIALER_TARGET, LOG_DEADLINE, call_ends, helper_calls,
    logged_since, millis, no_call_left,
};
use crate::checklist::fingerprint::Fingerprint;
use crate::checklist::results::unix_micros;
use crate::checklist::sitting::{Sitting, Stop, TestNumber, seconds};
use crate::checklist::watch::AppState;

/// How long a Call the runner hung up may take to end.
const HANG_UP_DEADLINE: Duration = Duration::from_secs(2);
/// How long the Phone may take to report a placed Call's number: some
/// report it only after the Call appears.
const NUMBER_DEADLINE: Duration = Duration::from_secs(3);

/// 7.1: place a Call to the Test Number and end it with the keyboard alone:
/// the keybinding opens the Dialer, Enter places the Call, the keybinding
/// focuses the Call Window and Ctrl+H hangs up. A Call to any other number,
/// a digit mistyped, is hung up at once: it would ring a stranger.
pub(super) fn keyboard_only_call(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let mark = sitting.mark();
    sitting.say(
        "From here on, use only the keyboard. Press your Desktop Calls keybinding, type the Test Number, and press Enter.",
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
    // Where the Call Window's focus is looked for: from the Call placed,
    // however late its number comes.
    let focus_mark = sitting.mark();
    if !went_to(sitting, call, &test_number)? {
        return Ok(());
    }
    sitting.observed("The Call went to the Test Number.".to_owned());
    sitting.check(
        sitting
            .seen_since(mark, |state| state.focused("dialer"))
            .is_some(),
        "The keybinding opened the Dialer with the keyboard in it, and Enter placed the Call."
            .to_owned(),
    );
    call_ends(
        sitting,
        call,
        "Once the Call connects, wait for the Call Window to collapse to the Call Pill in the bottom-right corner, about 5 s. Then press the keybinding to expand it, and Ctrl+H to hang up.",
        &[
            "callee-answers",
            "wait-for-call-pill",
            "keybinding",
            "ctrl-h",
        ],
    )?;
    // The keybinding must expand the collapsed Call Window and move the
    // keyboard to it while the Call goes on: focus it had from the moment it
    // showed proves nothing, and neither does the ended Call's window taking
    // it. So the Call Pill must be seen first, then the Call Window with the
    // keyboard. The compositor may hand it the keyboard for a moment as the
    // Dialer closes, so it must also still have it when the Call ends, as
    // Ctrl+H needs.
    let during = |state: &AppState| state.call(call).is_some();
    let expanded_with_keyboard = sitting.seen_in_order(
        focus_mark,
        |state| during(state) && state.shows("call-pill") && !state.shows("call-window"),
        |state| during(state) && state.focused("call-window") && !state.shows("call-pill"),
    ) && sitting
        .last_seen_since(focus_mark, |state| {
            during(state) && state.shows("call-window")
        })
        .is_some_and(|last| last.state.focused("call-window"));
    sitting.check(
        expanded_with_keyboard,
        if expanded_with_keyboard {
            "The keybinding expanded the collapsed Call Window with the keyboard in it, and the Call ended.".to_owned()
        } else {
            "The Call Window never went from collapsed to expanded with the keyboard, keeping it until the Call ended."
                .to_owned()
        },
    );
    let keyboard_only =
        sitting.ask("Did you place and end the Call without touching the mouse?")?;
    sitting.check(
        keyboard_only,
        if keyboard_only {
            "You placed and ended the Call with the keyboard alone.".to_owned()
        } else {
            "Placing or ending the Call needed the mouse.".to_owned()
        },
    );
    Ok(())
}

/// The Test Number, after saying which country code was assumed for it;
/// or `None`, once the step has recorded why it can't be used.
pub(super) fn test_number_or_why_not(sitting: &mut Sitting) -> Option<Fingerprint> {
    match sitting.test_number() {
        TestNumber::Ready { number, assumed } => {
            if let Some(code) = assumed {
                sitting.observed(format!(
                    "Treating your Test Number as +{code} (from your locale)."
                ));
            }
            Some(number)
        }
        TestNumber::NotSetUp => {
            sitting.check(
                false,
                "No Test Number is set up in ~/.config/desktop-calls/test-number, so the runner won't have you place a Call."
                    .to_owned(),
            );
            None
        }
        TestNumber::NoCountry => {
            sitting.check(
                false,
                "Your Test Number has no country code, and your locale doesn't say which country you're in. Add +<country code> to the start of the number in ~/.config/desktop-calls/test-number, such as +1 for the US."
                    .to_owned(),
            );
            None
        }
    }
}

/// Whether `call` went to the Test Number, once the Phone has said which
/// number it went to: some Phones report it only after the Call appears.
/// A Call that didn't, or might not have, is hung up, and the step fails.
pub(super) fn went_to(
    sitting: &mut Sitting,
    call: u32,
    test_number: &Fingerprint,
) -> Result<bool, Stop> {
    let numbered = sitting.happens_within("the Call's number", NUMBER_DEADLINE, |state| {
        state
            .call(call)
            .is_none_or(|placed| placed.number.is_known())
    })?;
    let number = numbered
        .as_ref()
        .and_then(|seen| seen.state.call(call))
        .map(|placed| placed.number);
    match number {
        Some(number) if number.is(test_number) => Ok(true),
        Some(number) if number.is_known() => {
            end_wrong_call(
                sitting,
                call,
                "The Call went to a number that isn't the Test Number",
            )?;
            Ok(false)
        }
        _ if sitting.now().call(call).is_none() => {
            sitting.check(
                false,
                "The Call ended before the Phone said which number it went to.".to_owned(),
            );
            Ok(false)
        }
        _ => {
            end_wrong_call(
                sitting,
                call,
                &format!(
                    "The Phone didn't say which number the Call went to within {}, so it may not be the Test Number",
                    seconds(NUMBER_DEADLINE)
                ),
            )?;
            Ok(false)
        }
    }
}

/// Hangs up `call`, which may ring a stranger, and fails the step: `why`
/// says what is wrong with it. The Call has ended once the app logs it
/// ended, or once it leaves the app's state with the Phone still connected:
/// a Call also leaves when the link drops, and may then carry on on the
/// Phone.
fn end_wrong_call(sitting: &mut Sitting, call: u32, why: &str) -> Result<(), Stop> {
    let from_us = unix_micros();
    let gone_with_the_phone_here =
        |state: &AppState| state.call(call).is_none() && state.connection_state == "connected";
    if let Err(error) = sitting.hang_up(call) {
        sitting.say(&format!("End this Call on the Pixel now. {why}."), &[]);
        sitting.check(
            false,
            format!("{why}, and the app refused the runner's request to hang it up: {error}"),
        );
        return Ok(());
    }
    let logged = logged_since(
        sitting,
        from_us,
        HANG_UP_DEADLINE,
        |line| line.event_kind == "call-ended" && line.call_id == Some(u64::from(call)),
        gone_with_the_phone_here,
    )?;
    if logged.is_some() || gone_with_the_phone_here(&sitting.now()) {
        sitting.check(
            false,
            format!("{why}, so the runner hung it up, and it ended."),
        );
    } else {
        sitting.say(&format!("End this Call on the Pixel now. {why}."), &[]);
        sitting.check(
            false,
            format!(
                "{why}. The runner hung it up, but couldn't confirm it ended within {}.",
                seconds(HANG_UP_DEADLINE)
            ),
        );
    }
    Ok(())
}

/// 7.2: the Banner flush top right over a real notification. While it
/// rings it shows in full above the notification sent meanwhile, and once
/// it goes the notification is in view. That the Banner draws above
/// Omarchy's notification cards is inferred from Hyprland's drawing order
/// (spec #221), so this step is where it is judged.
pub(super) fn banner_over_a_notification(sitting: &mut Sitting) -> Result<(), Stop> {
    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    match sitting.notify(
        "A sample notification",
        "Shown by the Real-Phone Checklist, to compare with the Incoming Call Banner.",
    ) {
        Ok(()) => sitting.observed("Sent a sample notification while the Banner shows.".to_owned()),
        Err(error) => {
            sitting.check(
                false,
                format!("Couldn't show a sample notification: {error}"),
            );
            return Ok(());
        }
    }
    let on_top = sitting.ask(
        "Is the Banner flush in the top-right corner, and does it show in full, above the notification?",
    )?;
    sitting.check(
        on_top,
        if on_top {
            "The Banner sat flush top right, in full, above the notification.".to_owned()
        } else {
            "The Banner wasn't flush top right, or the notification drew over it.".to_owned()
        },
    );
    call_ends(
        sitting,
        call,
        "Decline the Call, on either device.",
        &["decline"],
    )?;
    let kept = sitting.ask("Now that the Banner has gone, is the notification in view?")?;
    sitting.check(
        kept,
        if kept {
            "The notification was in view once the Banner went.".to_owned()
        } else {
            "The notification wasn't in view once the Banner went.".to_owned()
        },
    );
    Ok(())
}

/// Step 7.3: the user pins the Indicator once (Omarchy puts tray icons in
/// a drawer until pinned), then checks its icon and tooltip by eye; the
/// runner sees its left click open the Dialer and its menu mute, unmute
/// and hang up a Call.
pub(super) fn indicator_in_the_bar(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(
        "Right-click the small arrow at the tray end of Omarchy's bar, not the phone \
         icon (that opens Desktop Calls' own menu). A list called Tray icons opens: \
         on the Desktop Calls row, click Pin, unless it says Unpin already.",
        &[],
    );
    let pinned = sitting.ask("Does Desktop Calls' phone icon now stay in the bar?")?;
    sitting.check(
        pinned,
        if pinned {
            "The Indicator is pinned in Omarchy's bar.".to_owned()
        } else {
            "The Indicator couldn't be pinned in Omarchy's bar.".to_owned()
        },
    );
    let phone = sitting.now().phone_name;
    let worded = sitting.ask(&format!(
        "Hover over the icon. Does it say \"{phone} connected\", in the bar's own colour?"
    ))?;
    sitting.check(
        worded,
        if worded {
            "The Indicator's icon follows the bar's colour and its tooltip names the Phone."
                .to_owned()
        } else {
            "The Indicator's icon or tooltip was wrong.".to_owned()
        },
    );

    sitting.say("Left-click the icon.", &["indicator-click"]);
    let opened = sitting.wait_for("the Dialer opening", ACT_DEADLINE, |state| {
        state.shows("dialer")
    })?;
    sitting.check(
        opened.state.shows("dialer"),
        "Left-clicking the Indicator opened the Dialer.".to_owned(),
    );
    // Not waited for: in a headless demo, Escape may have no other window
    // to give the keyboard to (docs/research/2026-11-05-real-phone-checklist-runner.md).
    sitting.say("Press Escape to close the Dialer.", &["escape"]);

    let call = helper_calls(sitting, "Ask your helper to call you. Don't answer yet.")?;
    let ringing = sitting.ask("Does the icon now show a phone between sound waves?")?;
    sitting.check(
        ringing,
        if ringing {
            "The Indicator's icon showed the Call ringing.".to_owned()
        } else {
            "The Indicator's icon didn't show the Call ringing.".to_owned()
        },
    );
    sitting.say("Click Answer on the Banner.", &["answer"]);
    let answered = sitting.wait_for("the audio reaching the Desktop", ACT_DEADLINE, |state| {
        state.audio_location == "desktop" || state.call(call).is_none()
    })?;
    if answered.state.call(call).is_none() {
        sitting.check(
            false,
            "The Call ended before it was answered here.".to_owned(),
        );
        return Ok(());
    }

    sitting.say("Right-click the icon and choose Mute.", &["indicator-mute"]);
    let muted = sitting.wait_for("the microphone muting", ACT_DEADLINE, |state| {
        state.muted || state.call(call).is_none()
    })?;
    sitting.check(
        muted.state.muted,
        "Mute in the Indicator's menu muted the microphone.".to_owned(),
    );
    sitting.say(
        "Right-click the icon and choose Mute again.",
        &["indicator-mute"],
    );
    let unmuted = sitting.wait_for("the microphone unmuting", ACT_DEADLINE, |state| {
        !state.muted || state.call(call).is_none()
    })?;
    sitting.check(
        unmuted.state.call(call).is_some() && !unmuted.state.muted,
        "Mute in the Indicator's menu, ticked, unmuted the microphone.".to_owned(),
    );
    call_ends(
        sitting,
        call,
        "Right-click the icon and choose Hang up.",
        &["indicator-hang-up"],
    )?;
    sitting.observed("Hang up in the Indicator's menu ended the Call.".to_owned());
    Ok(())
}

/// Step 7.4: the Dialer's first frame within 150 ms of being asked, by
/// the app's own log (ticket #47). CI paints in software and holds it only
/// to 250 ms; the user's Desktop is where the 150 ms target is judged.
pub(super) fn dialer_first_frame(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let asked_us = unix_micros();
    sitting.say(
        "Press your Desktop Calls keybinding to open the Dialer, or click the phone icon if the keybinding isn't set up.",
        &["keybinding"],
    );
    sitting.wait_for("the Dialer opening", ACT_DEADLINE, |state| {
        state.shows("dialer")
    })?;
    match dialer_drawn_after(sitting, asked_us) {
        Some(took) => {
            sitting.measure("dialer_first_frame_ms", millis(took));
            sitting.check(
                took <= DIALER_TARGET,
                format!(
                    "The Dialer drew its first frame {} ms after it was asked (target {} ms).",
                    took.as_millis(),
                    DIALER_TARGET.as_millis()
                ),
            );
        }
        None => sitting.check(
            false,
            "The app's log had no activate-requested line followed by the Dialer's surface-shown line."
                .to_owned(),
        ),
    }
    sitting.say("Press Escape to close the Dialer.", &["escape"]);
    Ok(())
}

/// How long the app took, by its log, from the first `activate-requested`
/// since `from_us` to the Dialer's first frame drawn after it. The frame is
/// logged just after the Dialer shows, so the log is read for a while.
fn dialer_drawn_after(sitting: &Sitting, from_us: u64) -> Option<Duration> {
    let end = Instant::now() + LOG_DEADLINE;
    loop {
        let lines = sitting.log_since(from_us);
        let asked = lines
            .iter()
            .find(|line| line.event_kind == "activate-requested")
            .map(|line| line.at_us);
        let drawn = asked.and_then(|asked| {
            lines
                .iter()
                .find(|line| {
                    line.at_us >= asked
                        && line.event_kind == "surface-shown"
                        && line.surface.as_deref() == Some("dialer")
                })
                .map(|line| line.at_us - asked)
        });
        if let Some(drawn) = drawn {
            return Some(Duration::from_micros(drawn));
        }
        if Instant::now() >= end {
            return None;
        }
        thread::sleep(Duration::from_millis(100));
    }
}
