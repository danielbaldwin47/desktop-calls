//! Group 7's scripts: the Incoming Call Banner beside a notification, the
//! Indicator in the bar, and the Dialer's first frame.

use std::thread;
use std::time::{Duration, Instant};

use super::{
    ACT_DEADLINE, DIALER_TARGET, LOG_DEADLINE, call_ends, helper_calls, millis, no_call_left,
};
use crate::checklist::results::unix_micros;
use crate::checklist::sitting::{Sitting, Stop};

/// 7.2: the Banner beside a real notification, neither covering the other.
pub(super) fn banner_beside_a_notification(sitting: &mut Sitting) -> Result<(), Stop> {
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
    let clear = sitting.ask(
        "Do the Banner and the notification both show in full, with neither covering the other?",
    )?;
    sitting.check(
        clear,
        if clear {
            "The Banner and the notification were both in full view.".to_owned()
        } else {
            "The Banner and the notification overlapped.".to_owned()
        },
    );
    call_ends(
        sitting,
        call,
        "Decline the Call, on either device.",
        &["decline"],
    )?;
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
