//! Group 6's scripts: out of range and back, suspend and resume, a
//! `WirePlumber` restart while idle, and calls off until turned on. Each
//! times how long the Phone takes to be connected again, against the
//! polish scorecard's 10 s (#19, #20), from two lines of the app's own log:
//! the runner's clock and the user's Enter would add their own delays (#80,
//! #83, #99).

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
/// How long the user may take to click through the Indicator's menu.
const MENU_DEADLINE: Duration = Duration::from_secs(120);
/// How long calls stay off with the Pixel's Bluetooth back on, to show
/// nothing connects it: the Pixel's pages, `WirePlumber` and the app's
/// reconnect waits have all had their chance by then.
const STAYS_OFF: Duration = Duration::from_secs(60);
/// How long the restarted app may take to find the Phone and show its
/// state.
const APP_SETTLES: Duration = Duration::from_secs(10);
/// How long the runner waits for BlueZ to answer a `Connect` on the blocked
/// Phone: on the Pixel it gave no answer within 25 s (the probe note of
/// 2026-11-13).
const BLOCKED_CONNECT_DEADLINE: Duration = Duration::from_secs(60);

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

/// Waits, for up to [`BACK_DEADLINE`], for the app to log the Phone
/// connected after `after_us` (the Phone having left by then), and times
/// how long it took from `start` (an app-logged line), recording it as
/// `name`. `since` says what `start` was, such as "the Desktop resuming",
/// for the evidence. Reading the log,
/// not the app's state, means the state from before the Phone left can't
/// pass for it being back.
fn connected_again(
    sitting: &mut Sitting,
    start: &LogLine,
    after_us: u64,
    name: &str,
    since: &str,
) -> Result<(), Stop> {
    let Some(connected) = logged_since(
        sitting,
        after_us,
        BACK_DEADLINE,
        connection_logged(|state| state == "connected"),
        |_| false,
    )?
    else {
        sitting.check(
            false,
            format!(
                "The Phone wasn't connected again within {} of {since}.",
                seconds(BACK_DEADLINE)
            ),
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
        present.at_us,
        "reconnect_ms",
        "the Phone's Bluetooth link coming back",
    )
}

/// 6.2: suspend the Desktop for a minute, then resume. The Phone leaves
/// with the Desktop's Bluetooth, and the reconnect is timed from the app's
/// `desktop-resumed` line to the first connection it logs after the Phone
/// left. Real `BlueZ` may tell the app the Phone left only after resume,
/// so a leaving logged either side of `desktop-resumed` counts.
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
    let Some(left) = logged_since(
        sitting,
        from_us,
        BACK_DEADLINE,
        connection_logged(|state| state != "connected"),
        |_| false,
    )?
    else {
        sitting.check(
            false,
            "Couldn't measure the reconnect: the app never logged the Phone leaving as the Desktop slept and woke."
                .to_owned(),
        );
        return Ok(());
    };
    connected_again(
        sitting,
        &resumed,
        left.at_us.max(resumed.at_us),
        "reconnect_after_resume_ms",
        "the Desktop resuming",
    )
}

/// 6.3: restart `WirePlumber` while idle. The Phone leaves with Telephony
/// and is connected again, timed from the app logging the Phone gone.
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
        gone.at_us,
        "reconnect_after_wireplumber_ms",
        "the app logging the Phone gone",
    )
}

/// 6.4: calls off until turned on, from the Indicator. The Phone is
/// disconnected and stays so through its Bluetooth going off and on and an
/// app restart; turned on, it is connected again, timed from the app's
/// `calls-turned-on` line against the scorecard's 10 s. Records any
/// `Connect` the app made while calls were off (it should make none), and
/// what BlueZ said to each failed `Connect` after turning on, such as
/// `br-connection-busy`, which the app tries again.
pub(super) fn calls_off_until_turned_on(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "Click the Indicator, choose Turn calls off, then Until I turn them on.",
        &["calls-off"],
    );
    sitting.wait_for("calls turning off", MENU_DEADLINE, |state| {
        state.calls_off == "off" && state.connection_state != "connected"
    })?;
    sitting.observed("Calls are off and the Phone is disconnected.".to_owned());
    stays_off_through_bluetooth(sitting)?;
    still_off_after_a_restart(sitting)?;

    let on_us = unix_micros();
    let connects_while_off = sitting
        .log_since(from_us)
        .iter()
        .filter(|line| line.at_us < on_us && line.event_kind == "phone-connect-requested")
        .count();
    sitting.measure("connects_while_calls_off", connects_while_off);
    sitting.check(
        connects_while_off == 0,
        format!(
            "The app asked BlueZ to connect the Phone {connects_while_off} times while calls were off."
        ),
    );
    blocked_connect(sitting);
    turned_on_and_connected(sitting, on_us)
}

/// 6.4's middle: the runner itself asks BlueZ once to connect the blocked
/// Phone, as nothing in the app may, and records BlueZ's answer: what a
/// stray `Connect` would meet while calls are off (spec #198 left it
/// unobserved). It doesn't pass or fail the step.
fn blocked_connect(sitting: &mut Sitting) {
    let address = sitting.now().phone_address;
    sitting.say(
        &format!(
            "The runner now asks BlueZ once to connect the blocked Pixel, to record its answer. This can take up to {}.",
            seconds(BLOCKED_CONNECT_DEADLINE)
        ),
        &[],
    );
    let started = std::time::Instant::now();
    let answer = sitting.connect_the_phone(&address, BLOCKED_CONNECT_DEADLINE);
    let took = started.elapsed();
    let said = match &answer {
        Ok(()) => "connected".to_owned(),
        Err(error) => error.clone(),
    };
    sitting.measure("blocked_connect_answer", said.clone());
    sitting.measure("blocked_connect_ms", millis(took));
    sitting.observed(format!(
        "BlueZ answered a Connect on the blocked Phone after {}: {said}.",
        seconds(took)
    ));
}

/// 6.4's middle: the Pixel's Bluetooth goes off and on, and the Phone
/// stays disconnected for [`STAYS_OFF`] after.
fn stays_off_through_bluetooth(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(
        "Turn the Pixel's Bluetooth off, then on again. The runner then waits a minute.",
        &["bluetooth-off-on"],
    );
    sitting.ready("Press Enter once the Pixel's Bluetooth is on again.")?;
    let mark = sitting.mark();
    // Half way, the runner says how it stands, so a quiet minute doesn't
    // look stuck.
    sitting.pause(STAYS_OFF / 2)?;
    sitting.say(
        &format!(
            "{} so far; the Phone is {}.",
            seconds(STAYS_OFF / 2),
            sitting.now().connection_state
        ),
        &[],
    );
    sitting.pause(STAYS_OFF / 2)?;
    let reconnected = sitting
        .seen_since(mark, |state| state.connection_state == "connected")
        .is_some();
    let off = sitting.now().calls_off == "off";
    sitting.check(
        !reconnected && off,
        if reconnected {
            "The Phone connected again while calls were off.".to_owned()
        } else {
            format!(
                "The Phone stayed disconnected for {} with its Bluetooth back on.",
                seconds(STAYS_OFF)
            )
        },
    );
    Ok(())
}

/// 6.4's middle: the app restarts, reads that calls are off, and keeps the
/// Phone disconnected.
fn still_off_after_a_restart(sitting: &mut Sitting) -> Result<(), Stop> {
    let restart_us = unix_micros();
    sitting.say(
        "In another terminal, run `systemctl --user restart desktop-calls`.",
        &["restart-app"],
    );
    let loaded = logged_since(
        sitting,
        restart_us,
        RESTART_DEADLINE,
        |line| line.event_kind == "calls-off-loaded",
        |_| false,
    )?;
    sitting.check(
        loaded.is_some(),
        if loaded.is_some() {
            "After the restart the app read that calls are off.".to_owned()
        } else {
            "After the restart the app logged no calls-off-loaded line.".to_owned()
        },
    );
    sitting.pause(APP_SETTLES)?;
    let after = sitting.now();
    sitting.check(
        after.calls_off == "off" && after.connection_state != "connected",
        format!(
            "After the restart calls are {} and the Phone is {}.",
            after.calls_off, after.connection_state
        ),
    );
    Ok(())
}

/// 6.4's end: calls turned on from the Indicator after `on_us`, the
/// reconnect timed, and what BlueZ said to each failed `Connect` on the way.
fn turned_on_and_connected(sitting: &mut Sitting, on_us: u64) -> Result<(), Stop> {
    sitting.say(
        "Click the Indicator and choose Turn calls on.",
        &["calls-on"],
    );
    let Some(turned_on) = logged_since(
        sitting,
        on_us,
        MENU_DEADLINE,
        |line| line.event_kind == "calls-turned-on",
        |_| false,
    )?
    else {
        sitting.check(
            false,
            "The app logged no calls-turned-on line: calls weren't turned on.".to_owned(),
        );
        return Ok(());
    };
    connected_again(
        sitting,
        &turned_on,
        turned_on.at_us,
        "reconnect_after_calls_on_ms",
        "turning calls on",
    )?;
    let failures: Vec<String> = sitting
        .log_since(turned_on.at_us)
        .iter()
        .filter(|line| line.event_kind == "phone-connect-failed")
        .map(|line| line.detail.clone().unwrap_or_default())
        .collect();
    sitting.measure("connect_failures_after_calls_on", failures.join(", "));
    sitting.observed(if failures.is_empty() {
        "The first Connect after turning calls on succeeded.".to_owned()
    } else {
        format!(
            "BlueZ failed {} Connect(s) after turning calls on ({}); the app tried again until connected.",
            failures.len(),
            failures.join(", ")
        )
    });
    Ok(())
}

/// 6.5: calls off until a time two minutes ahead, from the CLI, and the
/// Desktop suspended past that time. Calls come back on only once it
/// resumes, and the reconnect is timed from the app's `desktop-resumed`
/// line against the scorecard's 10 s.
pub(super) fn calls_off_until_past_a_suspend(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let from_us = unix_micros();
    sitting.say(
        "In a terminal, run `desktop-calls calls off --until HH:MM`, with HH:MM two minutes from now.",
        &["calls-off-soon"],
    );
    let off = sitting.wait_for("calls turning off until a time", MENU_DEADLINE, |state| {
        state.calls_off == "off" && state.calls_off_until != 0
    })?;
    let until_us = off.state.calls_off_until.saturating_mul(1_000_000);
    let left = Duration::from_micros(until_us.saturating_sub(unix_micros()));
    sitting.observed(format!(
        "Calls are off until the time given, {} from now.",
        seconds(left)
    ));
    sitting.say(
        &format!(
            "Suspend the Desktop now, and wake it once that time has passed: wait at least {}.",
            seconds(left + Duration::from_secs(30))
        ),
        &["suspend-phone-stays", "resume-past-calls-off-deadline"],
    );
    sitting.ready("Press Enter once the Desktop is awake again.")?;
    let suspended = logged(sitting, from_us, |line| {
        line.event_kind == "desktop-suspending"
    })?;
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
    let slept_through =
        suspended.is_some_and(|line| line.at_us < until_us) && resumed.at_us >= until_us;
    sitting.check(
        slept_through,
        if slept_through {
            "The Desktop slept through the time calls were off until.".to_owned()
        } else {
            "The Desktop didn't sleep through the time calls were off until: suspend before it and wake after it."
                .to_owned()
        },
    );
    let Some(passed) = logged(sitting, from_us, |line| {
        line.event_kind == "calls-off-deadline-passed"
    })?
    else {
        sitting.check(
            false,
            "The app logged no calls-off-deadline-passed line: calls didn't come back on."
                .to_owned(),
        );
        return Ok(());
    };
    sitting.check(
        passed.at_us >= resumed.at_us,
        if passed.at_us >= resumed.at_us {
            "Calls came back on once the Desktop resumed.".to_owned()
        } else {
            "Calls came back on before the Desktop resumed.".to_owned()
        },
    );
    connected_again(
        sitting,
        &resumed,
        resumed.at_us,
        "reconnect_after_deadline_resume_ms",
        "the Desktop resuming past the deadline",
    )
}
