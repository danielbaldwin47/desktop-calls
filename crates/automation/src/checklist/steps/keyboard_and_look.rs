//! Group 7's scripts: the Incoming Call Banner beside a notification, and the
//! Indicator in the bar.

use super::{ACT_DEADLINE, call_ends, helper_calls, no_call_left};
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
        "Right-click the arrow at the tray end of Omarchy's bar. In the list, find \
         Desktop Calls and click Pin, unless it says Unpin already.",
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
