//! Group 1's first run: forgetting the pairing and installing afresh (1.1),
//! the Setup Window from pairing to the first Call, timed (1.2), and the
//! Phone's Contacts prompt left to time out, then allowed (1.3). Each step
//! follows the Setup Window through the `setup-window` surface it reports.

use std::time::{Duration, Instant};

use super::keyboard_and_look::{test_number_or_why_not, went_to};
use super::{
    ACT_DEADLINE, APP_DEADLINE, CALL_DEADLINE, Sitting, Stop, call_ends, millis, no_call_left,
    seconds,
};
use crate::checklist::watch::AppState;

/// Spec #1's target: from a fresh install to the first Call in 3 minutes.
const FIRST_CALL_TARGET: Duration = Duration::from_secs(180);
/// How long `just install` may take to build, install and start the app.
const INSTALL_DEADLINE: Duration = Duration::from_secs(600);
/// How long reading Contacts and Call History may take.
const SYNC_DEADLINE: Duration = Duration::from_secs(180);
/// How long the Phone's prompt may last before it gives up: Android's is
/// about 30 s (inferred).
const PROMPT_DEADLINE: Duration = Duration::from_secs(90);
/// How long the Phone may take to ask again after Try again.
const ASK_AGAIN_DEADLINE: Duration = Duration::from_secs(30);

const FORGET_ON_THE_PIXEL: &str = "On the Pixel: open Settings, then Connected devices, tap the settings icon next to this computer, and tap Forget.";
const REMOVE_ON_THE_DESKTOP: &str = "On this computer: remove the Pixel in Bluetooth settings, or with `bluetoothctl remove` and its address.";

/// One of the Setup Window's reported fields, while it shows.
fn window_field<'state>(state: &'state AppState, name: &str) -> Option<&'state str> {
    state
        .surfaces
        .get("setup-window")?
        .get(name)
        .map(String::as_str)
}

fn at_step(state: &AppState, step: &str) -> bool {
    window_field(state, "step") == Some(step)
}

fn offers_the_keybinding(state: &AppState) -> bool {
    window_field(state, "buttons") == Some("Add keybinding\nDone")
}

fn already_added(state: &AppState) -> bool {
    window_field(state, "instructions")
        .is_some_and(|text| text.starts_with("Place your first Call: press Super + Alt + C"))
}

/// 1.1: the pairing forgotten on both sides, then `just install`, which
/// starts the app again: with no Phone, the Setup Window opens by itself.
pub(super) fn forget_and_install(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(FORGET_ON_THE_PIXEL, &[]);
    sitting.say(REMOVE_ON_THE_DESKTOP, &["forget-pairing"]);
    sitting.wait_for(
        "Desktop Calls letting go of the Pixel",
        ACT_DEADLINE,
        |state| state.phone_name.is_empty(),
    )?;
    sitting.observed("Desktop Calls let go of the Pixel once it was removed.".to_owned());
    sitting.say(
        "Now install and start Desktop Calls: run `just install` in your Desktop Calls checkout.",
        &["restart-app"],
    );
    let started = Instant::now();
    let opened = sitting.wait_for(
        "the Setup Window opening by itself",
        INSTALL_DEADLINE,
        |state| at_step(state, "choose"),
    )?;
    sitting.observed(format!(
        "The Setup Window opened by itself at its first step, {} after you were asked to install.",
        seconds(opened.at.saturating_duration_since(started))
    ));
    Ok(())
}

/// Pairs the Pixel through the Setup Window, from its first step to
/// connected.
fn pair_through_the_window(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(
        "On the Pixel: open Settings, then Connected devices, and tap Pair new device. Keep that screen open.",
        &[],
    );
    sitting.wait_for(
        "the Pixel listed in the Setup Window",
        ACT_DEADLINE,
        |state| {
            at_step(state, "choose")
                && window_field(state, "phones").is_some_and(|phones| !phones.is_empty())
                && state.focused("setup-window")
        },
    )?;
    sitting.say("In the Setup Window, choose your Pixel.", &["enter"]);
    sitting.wait_for("the pairing code showing", ACT_DEADLINE, |state| {
        at_step(state, "confirm")
    })?;
    sitting.say(
        "Check the Pixel shows the same code, tap Pair on the Pixel, and choose Confirm here.",
        &["enter"],
    );
    sitting.wait_for("the Pixel connected", ACT_DEADLINE, |state| {
        state.connection_state == "connected"
    })?;
    sitting.observed("The Pixel paired through the Setup Window and connected.".to_owned());
    Ok(())
}

/// Records what the finish page says Contacts came to: synced passes.
fn check_synced(sitting: &mut Sitting, state: &AppState) {
    let summary = window_field(state, "summary").unwrap_or_default();
    sitting.check(
        summary.starts_with("Synced"),
        format!("The finish page says: {summary}"),
    );
}

/// Closes the finish page with Done, its default button.
fn close_the_window(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say("Choose Done to close the Setup Window.", &["enter"]);
    sitting.wait_for("the Setup Window closing", ACT_DEADLINE, |state| {
        !state.shows("setup-window")
    })?;
    Ok(())
}

/// 1.2: pairs, allows Contacts, adds the keybinding, and places the first
/// Call to the Test Number with it, timed from the step's start.
pub(super) fn pair_to_first_call(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let started = Instant::now();
    pair_through_the_window(sitting)?;
    sitting.say(
        "When the Pixel asks to allow access to contacts and call history, tap Allow.",
        &[],
    );
    let finished = sitting.wait_for("the Setup Window's finish page", SYNC_DEADLINE, |state| {
        at_step(state, "done") || at_step(state, "contacts-prompt-timed-out")
    })?;
    if at_step(&finished.state, "contacts-prompt-timed-out") {
        sitting.check(
            false,
            "The Pixel's prompt timed out before Allow was tapped.".to_owned(),
        );
        return Ok(());
    }
    check_synced(sitting, &finished.state);
    add_the_keybinding(sitting)?;
    close_the_window(sitting)?;
    sitting.say(
        "Press Super + Alt + C, type the Test Number, and press Enter.",
        &["keybinding", "type-test-number", "enter", "phone-dials"],
    );
    let placed = sitting.wait_for("your first Call being placed", CALL_DEADLINE, |state| {
        state.calls.iter().any(|call| call.direction == "outgoing")
    })?;
    let Some(call) = placed
        .state
        .calls
        .iter()
        .find(|call| call.direction == "outgoing")
        .map(|call| call.id)
    else {
        return Err(Stop::TimedOut("your first Call being placed".to_owned()));
    };
    let took = placed.at.saturating_duration_since(started);
    sitting.measure("pairing_to_first_call_ms", millis(took));
    sitting.check(
        took <= FIRST_CALL_TARGET,
        format!(
            "From the Setup Window's first step to the first Call placed took {} (target {}).",
            seconds(took),
            seconds(FIRST_CALL_TARGET)
        ),
    );
    if !went_to(sitting, call, &test_number)? {
        return Ok(());
    }
    sitting.observed("The first Call went to the Test Number.".to_owned());
    call_ends(
        sitting,
        call,
        "Once it rings or connects, hang up.",
        &["callee-answers", "hang-up"],
    )?;
    Ok(())
}

/// Adds the keybinding from the finish page, if it offers it: the line is
/// shown first, and written once confirmed.
fn add_the_keybinding(sitting: &mut Sitting) -> Result<(), Stop> {
    // The page checks the Hyprland config as it opens: the offer, or the
    // keybinding already there, follows within moments.
    let offered = sitting.happens_within("the keybinding offered", APP_DEADLINE, |state| {
        state.focused("setup-window") && (offers_the_keybinding(state) || already_added(state))
    })?;
    match offered {
        Some(seen) if already_added(&seen.state) => {
            sitting
                .observed("The keybinding was there already: the finish page says so.".to_owned());
            return Ok(());
        }
        Some(_) => {}
        None => {
            let state = sitting.now();
            let said = window_field(&state, "instructions").unwrap_or_default();
            sitting.observed(format!(
                "The finish page offered no keybinding to add: {said}"
            ));
            return Ok(());
        }
    }
    sitting.say(
        "Choose Add keybinding, and check the line it shows.",
        &["enter"],
    );
    let shown = sitting.wait_for("the keybinding's line showing", ACT_DEADLINE, |state| {
        window_field(state, "buttons") == Some("Confirm\nCancel")
    })?;
    let line = window_field(&shown.state, "line").unwrap_or_default();
    sitting.observed(format!("The window showed the line to add: {line}"));
    sitting.say("Choose Confirm to add it.", &["enter"]);
    let added = sitting.wait_for("the keybinding being added", ACT_DEADLINE, |state| {
        window_field(state, "buttons") != Some("Confirm\nCancel")
    })?;
    let instructions = window_field(&added.state, "instructions").unwrap_or_default();
    sitting.check(
        already_added(&added.state),
        format!("Once confirmed, the finish page says: {instructions}"),
    );
    Ok(())
}

/// 1.3: with Contacts access off, the Phone asks again once paired
/// afresh. The prompt is left to time out, Try again asks again, and Allow
/// then syncs.
pub(super) fn prompt_times_out_then_allowed(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(
        "On the Pixel: open Settings, then Connected devices, tap the settings icon next to this computer, and switch off Contacts and call history.",
        &["contacts-access-off"],
    );
    sitting.say(
        "So that the Pixel asks again, pair afresh. First forget the pairing on both sides:",
        &[],
    );
    sitting.say(FORGET_ON_THE_PIXEL, &[]);
    sitting.say(REMOVE_ON_THE_DESKTOP, &["forget-pairing"]);
    sitting.wait_for(
        "Desktop Calls letting go of the Pixel",
        ACT_DEADLINE,
        |state| state.phone_name.is_empty(),
    )?;
    sitting.say(
        "Then open the Setup Window: run `desktop-calls connect`.",
        &["open-setup-window"],
    );
    pair_through_the_window(sitting)?;
    let asked = sitting.wait_for("the Pixel asking about contacts", ACT_DEADLINE, |state| {
        at_step(state, "contacts-prompt") || at_step(state, "done")
    })?;
    if at_step(&asked.state, "done") {
        sitting.check(
            false,
            "The Pixel shared Contacts without asking, so its prompt couldn't time out.".to_owned(),
        );
        return close_the_window(sitting);
    }
    sitting.say(
        "Don't answer the Pixel's prompt: let the countdown here reach zero.",
        &[],
    );
    let timed_out =
        sitting.wait_for("the Pixel's prompt timing out", PROMPT_DEADLINE, |state| {
            at_step(state, "contacts-prompt-timed-out") || at_step(state, "done")
        })?;
    if at_step(&timed_out.state, "done") {
        sitting.check(
            false,
            "The prompt was answered before it timed out.".to_owned(),
        );
        return close_the_window(sitting);
    }
    let lasted = timed_out.at.saturating_duration_since(asked.at);
    sitting.measure("prompt_lasted_ms", millis(lasted));
    sitting.observed(format!(
        "The Pixel's prompt timed out {} after it showed.",
        seconds(lasted)
    ));
    let offers = window_field(&timed_out.state, "countdown") == Some("0")
        && window_field(&timed_out.state, "buttons") == Some("Try again\nSkip");
    sitting.check(
        offers,
        "The countdown reached 0, and the window offers Try again and Skip.".to_owned(),
    );
    sitting.say("Choose Try again.", &["enter"]);
    let again = sitting.happens_within("the Pixel asking again", ASK_AGAIN_DEADLINE, |state| {
        at_step(state, "contacts-prompt")
    })?;
    sitting.check(
        again.is_some(),
        if again.is_some() {
            "After Try again, the Pixel asked again.".to_owned()
        } else {
            format!(
                "After Try again, the Pixel didn't ask again within {}.",
                seconds(ASK_AGAIN_DEADLINE)
            )
        },
    );
    if again.is_none() {
        return Ok(());
    }
    sitting.say("Tap Allow on the Pixel.", &["allow-contacts"]);
    let finished = sitting.wait_for("the Setup Window's finish page", SYNC_DEADLINE, |state| {
        at_step(state, "done")
    })?;
    check_synced(sitting, &finished.state);
    sitting.wait_for("the finish page's buttons", ACT_DEADLINE, |state| {
        state.focused("setup-window") && window_field(state, "buttons").is_some()
    })?;
    close_the_window(sitting)
}
