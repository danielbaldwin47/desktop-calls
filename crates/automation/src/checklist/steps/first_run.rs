//! Group 1's first run: forgetting the pairing and installing afresh (1.1),
//! the Setup Window from pairing to the first Call, timed (1.2), and the
//! Phone's Contacts prompt left to time out, then allowed (1.3). Each step
//! follows the Setup Window through the `setup-window` surface it reports.
//!
//! On 2026-11-15 the real Pixel 9a, paired afresh through the Setup Window,
//! shared Contacts as it paired: the window went from `contacts-warning`
//! straight to `contacts-syncing`, with no prompt (#380). Its Pair dialog
//! carries "Allow access to your contacts and call history", ticked, and
//! forgetting the pairing forgets a Contacts switch turned off before it.
//! So 1.2 leaves the box ticked and 1.3 has it unticked; whether the Pixel
//! then asks, or refuses outright, is inferred until 1.3 is run.

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

const FORGET_ON_THE_PIXEL: &str = "Straight away, on the Pixel: open Settings, then Connected devices, tap the settings icon next to this computer, and tap Forget.";
const REMOVE_ON_THE_DESKTOP: &str = "On this computer: remove the Pixel in Bluetooth settings, or with `bluetoothctl remove` and its address.";
const STILL_PAIRED: &str = "The Pixel is still paired, and this step pairs it afresh.";
const GO_BACK_TO_1_1: &str = "Press Enter to go back to step 1.1, which forgets the pairing on both sides, or type skip or quit.";
const PAIRED_AT_INSTALL: &str = "Desktop Calls found the Pixel still paired when it started again, so this computer kept the pairing through the install.";
const DO_1_1_AGAIN: &str = "Press Enter to do step 1.1 again, or type skip or quit.";
/// The box on the Pixel's Pair dialog, as the Pixel words it (inferred).
const CONTACTS_BOX: &str = "\"Allow access to your contacts and call history\"";

/// What the user does with [`CONTACTS_BOX`] while pairing.
#[derive(Clone, Copy)]
enum ContactsBox {
    /// Leave it ticked: Contacts are shared as the Pixel pairs.
    Ticked,
    /// Untick it, so the Pixel doesn't share Contacts as it pairs.
    Unticked,
}

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

/// What the finish page's keybinding card says, without its title.
fn keybinding_said(state: &AppState) -> Option<&str> {
    window_field(state, "keybinding")
        .map(|card| card.split_once('\n').map_or(card, |(_, text)| text))
}

fn already_added(state: &AppState) -> bool {
    window_field(state, "keybinding")
        .is_some_and(|text| text.contains("Place your first call: press Super+Alt+C"))
}

/// Forgets the pairing on this computer first, then on the Pixel. The other
/// way round, the Desktop still follows the Pixel when it forgets, asks it
/// for hands-free, and the Pixel reports a wrong PIN (ticket #373).
fn forget_on_both_sides(sitting: &mut Sitting) -> Result<(), Stop> {
    sitting.say(REMOVE_ON_THE_DESKTOP, &["forget-pairing"]);
    sitting.wait_for(
        "Desktop Calls letting go of the Pixel",
        ACT_DEADLINE,
        |state| state.phone_name.is_empty(),
    )?;
    sitting.say(FORGET_ON_THE_PIXEL, &[]);
    Ok(())
}

/// 1.1: the pairing forgotten on both sides, then `just install`, which
/// starts the app again: with no Phone, the Setup Window opens by itself.
/// Only the app started by the install counts, and only with no Phone: a
/// Setup Window opened by hand, or an app that found the Pixel still
/// paired, doesn't pass (#380).
pub(super) fn forget_and_install(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    forget_on_both_sides(sitting)?;
    sitting.observed("Desktop Calls let go of the Pixel once it was removed.".to_owned());
    let starts = sitting.now().app_starts;
    sitting.say(
        "Now install and start Desktop Calls: run `just install` in your Desktop Calls checkout.",
        &["restart-app"],
    );
    let started = Instant::now();
    let opened = sitting.wait_for(
        "the Setup Window opening by itself after the install",
        INSTALL_DEADLINE,
        |state| {
            state.app_starts > starts && (at_step(state, "choose") || !state.phone_name.is_empty())
        },
    )?;
    if !opened.state.phone_name.is_empty() {
        sitting.check(false, PAIRED_AT_INSTALL.to_owned());
        return Err(sitting.offer_to_go_back(
            "1.1",
            "This step needs the pairing gone on both sides before the install.",
            DO_1_1_AGAIN,
        ));
    }
    sitting.observed(format!(
        "The Setup Window opened by itself at its first step, {} after you were asked to install.",
        seconds(opened.at.saturating_duration_since(started))
    ));
    Ok(())
}

/// Pairs the Pixel through the Setup Window, from its first step to
/// connected, with [`CONTACTS_BOX`] as `contacts` says.
fn pair_through_the_window(sitting: &mut Sitting, contacts: ContactsBox) -> Result<(), Stop> {
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
    match contacts {
        ContactsBox::Ticked => sitting.say(
            &format!(
                "Check the Pixel shows the same code. Leave {CONTACTS_BOX} ticked on the Pixel, tap Pair there, and choose Confirm here."
            ),
            &["enter"],
        ),
        ContactsBox::Unticked => sitting.say(
            &format!(
                "Check the Pixel shows the same code. Untick {CONTACTS_BOX} on the Pixel, tap Pair there, and choose Confirm here."
            ),
            &["contacts-access-off", "enter"],
        ),
    }
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
    // With the Pixel still paired, the Setup Window never lists it, and the
    // wait for it would run out its two minutes. A run resumed after 1.1
    // passed in an earlier one finds it so: 1.1 is offered there and then,
    // so the pairing isn't done under 1.3 instead (#380).
    if !sitting.now().phone_name.is_empty() {
        return Err(sitting.offer_to_go_back("1.1", STILL_PAIRED, GO_BACK_TO_1_1));
    }
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let started = Instant::now();
    pair_through_the_window(sitting, ContactsBox::Ticked)?;
    sitting.say(
        "If the Pixel asks to allow access to contacts and call history, tap Allow.",
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
    if !add_the_keybinding(sitting)? {
        return Ok(());
    }
    close_the_window(sitting)?;
    sitting.say(
        "Press Super+Alt+C, type the Test Number, and press Enter.",
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
/// shown first, and written once confirmed. Returns whether Super+Alt+C
/// is bound to the Dialer now; if not, the step has failed, and says why.
fn add_the_keybinding(sitting: &mut Sitting) -> Result<bool, Stop> {
    // The page checks the Hyprland config as it opens: the offer, or the
    // keybinding already there, follows within moments.
    let offered = sitting.happens_within("the keybinding offered", APP_DEADLINE, |state| {
        state.focused("setup-window") && (offers_the_keybinding(state) || already_added(state))
    })?;
    match offered {
        Some(seen) if already_added(&seen.state) => {
            sitting
                .observed("The keybinding was there already: the finish page says so.".to_owned());
            return Ok(true);
        }
        Some(_) => {}
        None => {
            let state = sitting.now();
            let said = keybinding_said(&state).unwrap_or_default();
            sitting.check(
                false,
                format!(
                    "The finish page offered no keybinding to add, so Super+Alt+C won't open the Dialer. It says: {said}"
                ),
            );
            return Ok(false);
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
    // Only the added page ends the wait, or the page saying it couldn't add
    // the line: the states between belong to neither.
    let added = sitting.wait_for("the keybinding being added", ACT_DEADLINE, |state| {
        already_added(state) || window_field(state, "notice").is_some()
    })?;
    let added_now = already_added(&added.state);
    let said = if added_now {
        keybinding_said(&added.state)
    } else {
        window_field(&added.state, "notice")
    };
    sitting.check(
        added_now,
        format!(
            "Once confirmed, the finish page says: {}",
            said.unwrap_or_default()
        ),
    );
    Ok(added_now)
}

/// 1.3: paired afresh with [`CONTACTS_BOX`] unticked, the Phone asks
/// about Contacts once connected. The prompt is left to time out, Try
/// again asks again, and Allow then syncs. A Pixel that shares or refuses
/// Contacts without asking has no prompt to time out (#380).
pub(super) fn prompt_times_out_then_allowed(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(
        &format!(
            "The Pixel asks about Contacts only when {CONTACTS_BOX} is unticked as it pairs, so this step pairs it afresh. First forget the pairing on both sides:"
        ),
        &[],
    );
    forget_on_both_sides(sitting)?;
    sitting.say(
        "Then open the Setup Window: run `desktop-calls connect`.",
        &["open-setup-window"],
    );
    pair_through_the_window(sitting, ContactsBox::Unticked)?;
    let asked = sitting.wait_for("the Pixel asking about contacts", ACT_DEADLINE, |state| {
        at_step(state, "contacts-prompt") || at_step(state, "done")
    })?;
    if at_step(&asked.state, "done") {
        return judge_without_a_prompt(sitting, &asked.state);
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

/// 1.3 when the Setup Window finished with no prompt shown: the Pixel
/// shared Contacts as it paired, or refused them. Either way it has no
/// prompt to time out, which passes, unless the box was left ticked.
fn judge_without_a_prompt(sitting: &mut Sitting, state: &AppState) -> Result<(), Stop> {
    let summary = window_field(state, "summary")
        .unwrap_or_default()
        .to_owned();
    if summary.starts_with("Synced") {
        let unticked = sitting.ask(&format!(
            "The Pixel shared Contacts as it paired. Did you untick {CONTACTS_BOX} on the Pixel before tapping Pair?"
        ))?;
        sitting.check(
            unticked,
            if unticked {
                format!(
                    "This Pixel shares Contacts as it pairs even with {CONTACTS_BOX} unticked, so it never shows a prompt that could time out. The finish page says: {summary}"
                )
            } else {
                format!(
                    "{CONTACTS_BOX} was left ticked, so the Pixel shared Contacts as it paired and there was no prompt to time out. Run this step again and untick it."
                )
            },
        );
    } else {
        sitting.observed(format!(
            "This Pixel refuses Contacts when {CONTACTS_BOX} is unticked, rather than asking, so it never shows a prompt that could time out. The finish page says: {summary}"
        ));
        sitting.say(
            "So that Calls show names for the rest of the checklist: on the Pixel, open Settings, then Connected devices, tap the settings icon next to this computer, and switch on Contacts and call history.",
            &["contacts-access-on"],
        );
    }
    close_the_window(sitting)
}
