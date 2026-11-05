//! Group 6's scripts: out of range and back, suspend and resume, and a
//! `WirePlumber` restart while idle. Each times how long the Phone takes to
//! be connected again, against the polish scorecard's 10 s (#19, #20), from
//! two lines of the app's own log: the runner's clock and the user's Enter
//! would add their own delays (#80, #83, #99).

use std::time::Duration;

use super::{LOG_DEADLINE, connection_logged, logged_since, millis, no_call_left, unix_micros_at};
use crate::checklist::app_log::LogLine;
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

/// The first line after `from_us` that `wanted` accepts, read from the
/// app's log for up to [`LOG_DEADLINE`]: the app logs a change just after
/// publishing it.
fn logged(
    sitting: &mut Sitting,
    from_us: u64,
    wanted: impl Fn(&LogLine) -> bool,
) -> Result<Option<LogLine>, Stop> {
    logged_since(sitting, from_us, LOG_DEADLINE, wanted, |_| false)
}

/// Waits for the Phone to be connected again, then times, by the app's
/// log, how long it took from `start` (an app-logged line) to the app
/// logging it connected, recording it as `name`. `since` says what `start`
/// was, for the evidence.
fn connected_again(
    sitting: &mut Sitting,
    start: &LogLine,
    name: &str,
    since: &str,
) -> Result<(), Stop> {
    sitting.wait_for("the Phone connecting again", BACK_DEADLINE, |state| {
        state.connection_state == "connected"
    })?;
    let Some(connected) = logged(
        sitting,
        start.at_us,
        connection_logged(|state| state == "connected"),
    )?
    else {
        sitting.check(
            false,
            format!("Couldn't measure the reconnect: the app logged no connection after {since}."),
        );
        return Ok(());
    };
    let took = Duration::from_micros(connected.at_us.saturating_sub(start.at_us));
    sitting.measure(name, millis(took));
    sitting.check(
        took > Duration::ZERO && took <= RECONNECT_TARGET,
        format!(
            "Connected again {} after {since}, by the app's log (target {}).",
            seconds(took),
            seconds(RECONNECT_TARGET)
        ),
    );
    Ok(())
}

/// 6.1: the Phone goes out of range (or its Bluetooth off) and comes back.
/// The reconnect is timed from the app logging the Phone's Bluetooth link
/// back (`phone-present`) to it logging the Phone connected, so an Enter
/// pressed late can't make it look instant.
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
    let gone_us = unix_micros_at(gone.at);
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
    sitting.ready("Press Enter once you have turned Bluetooth on, or come back in range.")?;
    sitting.wait_for("the Phone connecting again", BACK_DEADLINE, |state| {
        state.connection_state == "connected"
    })?;
    let Some(present) = logged(sitting, gone_us, |line| line.event_kind == "phone-present")? else {
        sitting.check(
            false,
            "Couldn't measure the reconnect: the app logged no phone-present line as the Phone came back."
                .to_owned(),
        );
        return Ok(());
    };
    connected_again(
        sitting,
        &present,
        "reconnect_ms",
        "the Phone's Bluetooth link came back",
    )
}

/// 6.2: suspend the Desktop for a minute, then resume. The Phone leaves
/// with the Desktop's Bluetooth, and the reconnect is timed from the app's
/// `desktop-resumed` line.
pub(super) fn suspend_and_resume(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "Suspend the Desktop (for example from Omarchy's menu), wait a minute, then wake it.",
        &["suspend", "resume"],
    );
    sitting.ready("Press Enter once the Desktop is awake again.")?;
    let Some(resumed) = logged(sitting, from_us, |line| {
        line.event_kind == "desktop-resumed"
    })?
    else {
        sitting.check(
            false,
            "The app's log has no desktop-resumed line: it didn't see the Desktop suspend and resume."
                .to_owned(),
        );
        return Ok(());
    };
    sitting.observed("The app saw the Desktop suspend and resume.".to_owned());
    let left = logged(
        sitting,
        from_us,
        connection_logged(|state| state != "connected"),
    )?;
    if left.is_none_or(|left| left.at_us > resumed.at_us) {
        sitting.check(
            false,
            "Couldn't measure the reconnect: the app never logged the Phone leaving while the Desktop slept."
                .to_owned(),
        );
        return Ok(());
    }
    connected_again(
        sitting,
        &resumed,
        "reconnect_after_resume_ms",
        "the Desktop resumed",
    )
}

/// 6.3: restart `WirePlumber` while idle. The app sees Telephony's audio
/// gateway go and come back, and the Phone is connected again, timed from
/// the app logging the Phone gone.
pub(super) fn wireplumber_restart(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "In another terminal, run `systemctl --user restart wireplumber`.",
        &["restart-wireplumber"],
    );
    sitting.wait_for(
        "the Phone leaving with WirePlumber",
        RESTART_DEADLINE,
        |state| state.connection_state != "connected",
    )?;
    sitting.observed("The Desktop saw Telephony go with WirePlumber.".to_owned());
    let Some(gone) = logged(
        sitting,
        from_us,
        connection_logged(|state| state != "connected"),
    )?
    else {
        sitting.check(
            false,
            "Couldn't measure the reconnect: the app logged no change as the Phone left."
                .to_owned(),
        );
        return Ok(());
    };
    connected_again(
        sitting,
        &gone,
        "reconnect_after_wireplumber_ms",
        "the app logged the Phone gone",
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
