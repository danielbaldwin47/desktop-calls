//! Group 6's scripts: out of range and back, suspend and resume, and a
//! `WirePlumber` restart while idle. Each times how long the Phone takes to
//! be connected again: the polish scorecard's 10 s (#19, #20).

use std::time::Duration;

use super::{LOG_DEADLINE, logged_since, millis, no_call_left, unix_micros_at};
use crate::checklist::results::unix_micros;
use crate::checklist::sitting::{Sitting, Stop, seconds};

/// The scorecard's reconnect target (`docs/wiki/polish-scorecard.md`).
const RECONNECT_TARGET: Duration = Duration::from_secs(10);
/// How long the Desktop may take to notice the Phone has gone: Bluetooth's
/// link supervision timeout is up to 20 s, and the user must walk away.
const GONE_DEADLINE: Duration = Duration::from_secs(180);
/// How long the runner waits for the Phone to come back before failing.
const BACK_DEADLINE: Duration = Duration::from_secs(60);
/// How long the user may take to restart `WirePlumber`.
const RESTART_DEADLINE: Duration = Duration::from_secs(300);
/// How long to look for a change the timeline should already hold.
const ALREADY_SEEN: Duration = Duration::from_secs(1);

/// Waits for the Phone to be connected again and checks how long it took
/// from `since_us`, recording it as `name`.
fn connected_again(
    sitting: &mut Sitting,
    since_us: u64,
    name: &str,
    since: &str,
) -> Result<(), Stop> {
    let back = sitting.wait_for("the Phone connecting again", BACK_DEADLINE, |state| {
        state.connection_state == "connected"
    })?;
    let took = Duration::from_micros(unix_micros_at(back.at).saturating_sub(since_us));
    sitting.measure(name, millis(took));
    sitting.check(
        took <= RECONNECT_TARGET,
        format!(
            "Connected again {} after {since} (target {}).",
            seconds(took),
            seconds(RECONNECT_TARGET)
        ),
    );
    Ok(())
}

/// 6.1: the Phone goes out of range (or its Bluetooth off) and comes back.
/// The reconnect is timed from the user pressing Enter as it returns.
pub(super) fn out_of_range_and_back(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(
        "Turn the Pixel's Bluetooth off, or walk it out of range of the Desktop.",
        &["out-of-range"],
    );
    let gone = sitting.wait_for(
        "the Desktop noticing the Phone gone",
        GONE_DEADLINE,
        |state| state.connection_state != "connected",
    )?;
    sitting.observed(format!(
        "The Desktop shows the Phone as {}.",
        gone.state.connection_state
    ));
    let quiet = sitting.ask("Did the Desktop stay quiet, with no notification about it?")?;
    sitting.check(
        quiet,
        if quiet {
            "No notification appeared when the Phone left.".to_owned()
        } else {
            "A notification appeared when the Phone left.".to_owned()
        },
    );
    sitting.say(
        "Now bring it back: turn Bluetooth on, or walk back.",
        &["return-to-range"],
    );
    sitting.ready("Press Enter as you turn Bluetooth on, or as you come back in range.")?;
    let back_us = unix_micros();
    connected_again(
        sitting,
        back_us,
        "reconnect_ms",
        "you brought the Phone back",
    )
}

/// 6.2: suspend the Desktop for a minute, then resume. The reconnect is
/// timed from the app's `desktop-resumed` log line.
pub(super) fn suspend_and_resume(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "Suspend the Desktop (for example from Omarchy's menu), wait a minute, then wake it.",
        &["suspend", "resume"],
    );
    sitting.ready("Press Enter once the Desktop is awake again.")?;
    let resumed = logged_since(sitting, "desktop-resumed", from_us, LOG_DEADLINE, |_| false)?;
    let Some(resumed_us) = resumed else {
        sitting.check(
            false,
            "The app's log has no desktop-resumed line: it didn't see the Desktop suspend and resume."
                .to_owned(),
        );
        return Ok(());
    };
    sitting.observed("The app saw the Desktop suspend and resume.".to_owned());
    // The timeline keeps what happened while the runner waited for Enter:
    // the Phone leaving, then coming back.
    let left = sitting.happens_within("the Phone leaving", ALREADY_SEEN, |state| {
        state.connection_state != "connected"
    })?;
    if left.is_none() {
        sitting.measure("reconnect_after_resume_ms", 0);
        sitting.observed("The Phone stayed connected through the suspend.".to_owned());
        return Ok(());
    }
    connected_again(
        sitting,
        resumed_us,
        "reconnect_after_resume_ms",
        "the Desktop resumed",
    )
}

/// 6.3: restart `WirePlumber` while idle. The app sees Telephony's audio
/// gateway go and come back, and the Phone is connected again.
pub(super) fn wireplumber_restart(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "In another terminal, run `systemctl --user restart wireplumber`.",
        &["restart-wireplumber"],
    );
    let gone = sitting.wait_for(
        "the Phone leaving with WirePlumber",
        RESTART_DEADLINE,
        |state| state.connection_state != "connected",
    )?;
    sitting.observed("The Desktop saw Telephony go with WirePlumber.".to_owned());
    connected_again(
        sitting,
        unix_micros_at(gone.at),
        "reconnect_after_wireplumber_ms",
        "WirePlumber went",
    )?;
    let removed = sitting
        .log_since(from_us)
        .iter()
        .any(|line| line.event_kind == "audio-gateway-removed");
    if removed {
        sitting.observed("The app logged the audio gateway going and coming back.".to_owned());
    }
    Ok(())
}
