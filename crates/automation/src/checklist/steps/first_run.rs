//! Group 1's first run: forgetting the pairing and installing afresh (1.1),
//! the Setup Window from pairing to the first Call, timed (1.2), and
//! Contacts not shared as the Pixel pairs, then turned on later on the
//! Pixel (1.3). Each step follows the Setup Window through the
//! `setup-window` surface it reports.
//!
//! On 2026-11-15 the real Pixel 9a, paired afresh through the Setup Window,
//! shared Contacts as it paired: the window went from `contacts-warning`
//! straight to `contacts-syncing`, with no prompt (#380). Its Pair dialog
//! carries "Allow access to your contacts and call history", ticked, and
//! forgetting the pairing forgets a Contacts switch turned off before it.
//! So 1.2 leaves the box ticked and 1.3 has it unticked. With it unticked,
//! the Pixel refused Contacts at once, with no prompt (#392), so the
//! prompt left to time out is the Simulated Phone's scenario only.

use std::time::{Duration, Instant};

use super::keyboard_and_look::{test_number_or_why_not, went_to};
use super::{
    ACT_DEADLINE, APP_DEADLINE, CALL_DEADLINE, SYNC_DEADLINE, Seen, Sitting, Stop, call_ends,
    millis, no_call_left, seconds,
};
use crate::checklist::results::unix_micros;
use crate::checklist::watch::AppState;

/// Spec #1's target: from a fresh install to the first Call in 3 minutes.
const FIRST_CALL_TARGET: Duration = Duration::from_secs(180);
/// How long `just install` may take to build, install and start the app.
const INSTALL_DEADLINE: Duration = Duration::from_secs(600);
/// How long the Phone's prompt may last before it gives up: Android's is
/// about 30 s (inferred, RPU-14).
const PROMPT_DEADLINE: Duration = Duration::from_secs(90);
/// How long Desktop Calls may take to pick up Contacts turned on later on
/// the Pixel: it asks again every 10 s for about 10 minutes after the Pixel
/// connects, then every 5 minutes (#394), and a sync takes moments.
pub(super) const SHARED_LATER_DEADLINE: Duration = Duration::from_secs(330);
/// When a wait for Contacts turned on later says it is still waiting.
const STILL_WAITING_AFTER: Duration = Duration::from_secs(60);
/// What the "Contacts not shared" Problem says to do, as the runner says it.
pub(super) const SHARE_ON_THE_PIXEL: &str = "With the Pixel still connected, on the Pixel open Settings, then Connected devices, tap the settings icon next to this computer, and turn on Contacts and call history. Don't pair again. Then wait here: Desktop Calls notices by itself, usually within a minute.";

const FORGET_ON_THE_PIXEL: &str = "Straight away, on the Pixel: open Settings, then Connected devices, tap the settings icon next to this computer, and tap Forget.";
const REMOVE_ON_THE_DESKTOP: &str = "On this computer: remove the Pixel in Bluetooth settings, or with `bluetoothctl remove` and its address.";
const STILL_PAIRED: &str = "The Pixel is still paired, and this step pairs it afresh.";
const GO_BACK_TO_1_1: &str = "Press Enter to go back to step 1.1, which forgets the pairing on both sides, or type skip or quit.";
const PAIRED_AT_INSTALL: &str = "Desktop Calls found the Pixel still paired when it started again, so this computer kept the pairing through the install.";
const DO_1_1_AGAIN: &str = "Press Enter to do step 1.1 again, or type skip or quit.";
/// How the finish page's summary starts when the Phone refused Contacts,
/// as against "…isn't sharing contacts yet." when nobody answered or the
/// sync failed.
const REFUSED_SUMMARY: &str = "Your phone isn't sharing contacts. ";
/// What the finish page adds when Contacts aren't shared.
const CALLS_STILL_WORK: &str = "Calls still work";
/// The box on the Pixel's Pair dialog, as the Pixel words it (inferred,
/// RPU-15).
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

/// 1.3: paired afresh with [`CONTACTS_BOX`] unticked, the Pixel doesn't
/// share Contacts: the finish page says so and that Calls still work, and
/// the app's log says the Pixel refused. Then the user does what the
/// "Contacts not shared" Problem says, on the Pixel, and Desktop Calls
/// syncs by itself, with no pairing again and the Pixel connected all
/// along (#392). The prompt left to time out is the Simulated Phone's
/// scenario only: the Pixel 9a refuses at once, with no prompt.
pub(super) fn contacts_shared_later(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    sitting.say(
        &format!(
            "This step pairs the Pixel again with {CONTACTS_BOX} unticked, then turns on Contacts afterwards. First forget the pairing on both sides:"
        ),
        &[],
    );
    forget_on_both_sides(sitting)?;
    sitting.say(
        "Then open the Setup Window: run `desktop-calls connect`.",
        &["open-setup-window"],
    );
    let paired_us = unix_micros();
    pair_through_the_window(sitting, ContactsBox::Unticked)?;
    let answered = sitting.wait_for("the Setup Window's finish page", SYNC_DEADLINE, |state| {
        at_step(state, "done") || at_step(state, "contacts-prompt")
    })?;
    let finished = if at_step(&answered.state, "contacts-prompt") {
        sitting.say(
            "The Pixel is asking about contacts: tap Don't allow.",
            &["deny-contacts"],
        );
        let finished =
            sitting.wait_for("the Setup Window's finish page", PROMPT_DEADLINE, |state| {
                at_step(state, "done") || at_step(state, "contacts-prompt-timed-out")
            })?;
        if at_step(&finished.state, "contacts-prompt-timed-out") {
            sitting.check(
                false,
                "The Pixel's prompt timed out before Don't allow was tapped.".to_owned(),
            );
            return Ok(());
        }
        finished
    } else {
        answered
    };
    let not_shared = judge_not_shared(sitting, &finished.state, paired_us)?;
    sitting.wait_for("the finish page's buttons", ACT_DEADLINE, |state| {
        state.focused("setup-window") && window_field(state, "buttons").is_some()
    })?;
    close_the_window(sitting)?;
    if not_shared {
        shared_later(sitting)?;
    }
    Ok(())
}

/// 1.3's finish page: the Pixel didn't share Contacts, so it says that,
/// and that Calls still work, and the app's log says the Pixel refused.
/// Returns whether that is so, for the rest of the step: a Pixel that
/// shared Contacts as it paired has nothing to turn on later.
fn judge_not_shared(sitting: &mut Sitting, state: &AppState, paired_us: u64) -> Result<bool, Stop> {
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
                    "This Pixel shares Contacts as it pairs even with {CONTACTS_BOX} unticked, so there is nothing to turn on later. The finish page says: {summary}"
                )
            } else {
                format!(
                    "{CONTACTS_BOX} was left ticked, so the Pixel shared Contacts as it paired. Run this step again and untick it."
                )
            },
        );
        return Ok(false);
    }
    let refused = summary.starts_with(REFUSED_SUMMARY);
    sitting.check(
        refused && summary.contains(CALLS_STILL_WORK),
        format!("The finish page says: {summary}"),
    );
    let logged = sitting
        .log_since(paired_us)
        .iter()
        .any(|line| line.event_kind == "contacts-sync-ended" && line.message.ends_with(": denied"));
    sitting.check(
        logged,
        if logged {
            "The app's log says the Pixel refused Contacts.".to_owned()
        } else {
            "The app's log has no line saying the Pixel refused Contacts.".to_owned()
        },
    );
    Ok(refused)
}

/// 1.3 once the Setup Window has closed: the "Contacts not shared" Problem
/// shows; the user does what it says, on the Pixel; and Desktop Calls syncs
/// by itself, with the Pixel connected all along and the app not restarted.
fn shared_later(sitting: &mut Sitting) -> Result<(), Stop> {
    let shown = sitting.wait_for(
        "the \"Contacts not shared\" Problem",
        APP_DEADLINE,
        |state| contacts_problem(state).is_some(),
    )?;
    let sentence = contacts_problem(&shown.state)
        .unwrap_or_default()
        .to_owned();
    // The runner's instruction below must be the one the app gives.
    sitting.check(
        sentence.contains("turn on Contacts and call history"),
        format!("Desktop Calls shows the Contacts not shared Problem: {sentence}"),
    );
    let starts = shown.state.app_starts;
    let mark = sitting.mark();
    sitting.say(
        &format!("Now do what it says. {SHARE_ON_THE_PIXEL}"),
        &["contacts-access-on"],
    );
    let asked = Instant::now();
    let Some(synced) = contacts_synced(sitting, starts)? else {
        sitting.check(
            false,
            format!(
                "Desktop Calls didn't pick up the Contacts you turned on within {}, with the Pixel connected. That is a product bug: file it with this step's log.",
                seconds(SHARED_LATER_DEADLINE)
            ),
        );
        return Ok(());
    };
    if synced.state.connection_state != "connected" {
        sitting.check(
            false,
            "The Pixel disconnected while Desktop Calls waited for Contacts. Reconnect it, and run this step again.".to_owned(),
        );
        return Ok(());
    }
    let took = synced.at.saturating_duration_since(asked);
    sitting.measure("shared_later_synced_ms", millis(took));
    sitting.observed(format!(
        "Desktop Calls synced Contacts {} after you were asked to turn them on, without pairing again.",
        seconds(took)
    ));
    let dropped = sitting
        .seen_since(mark, |state| state.connection_state != "connected")
        .is_some();
    sitting.check(
        !dropped && synced.state.app_starts == starts,
        if dropped {
            "The Pixel disconnected on the way, so the sync may have come from reconnecting."
                .to_owned()
        } else if synced.state.app_starts != starts {
            "Desktop Calls restarted on the way, so the sync may have come from the restart."
                .to_owned()
        } else {
            "The Pixel stayed connected and Desktop Calls didn't restart.".to_owned()
        },
    );
    let gone = contacts_problem(&synced.state).is_none();
    sitting.check(
        gone,
        if gone {
            "The Contacts not shared Problem went away.".to_owned()
        } else {
            "The Contacts not shared Problem still shows, though Contacts synced.".to_owned()
        },
    );
    Ok(())
}

/// Waits for Contacts turned on later on the Pixel to sync: the state
/// that is synced, or that shows the Pixel disconnected or the app
/// restarted, which end the wait too; `None` if neither happened within
/// [`SHARED_LATER_DEADLINE`]. After a minute it says it is still waiting.
pub(super) fn contacts_synced(sitting: &mut Sitting, starts: u32) -> Result<Option<Seen>, Stop> {
    let ended = |state: &AppState| {
        state.contacts_state == "ready"
            || state.connection_state != "connected"
            || state.app_starts != starts
    };
    if let Some(seen) = sitting.happens_within("Contacts synced", STILL_WAITING_AFTER, ended)? {
        return Ok(Some(seen));
    }
    sitting.say(
        "Still waiting. Desktop Calls checks every 10 seconds for 10 minutes after the Pixel connects, then every 5 minutes. Type skip to stop waiting.",
        &[],
    );
    sitting.happens_within(
        "Contacts synced",
        SHARED_LATER_DEADLINE.saturating_sub(STILL_WAITING_AFTER),
        ended,
    )
}

/// The sentence of the "Contacts not shared" Problem, while it shows.
fn contacts_problem(state: &AppState) -> Option<&str> {
    state
        .problems
        .iter()
        .find(|(kind, _)| kind == "contacts-denied")
        .map(|(_, sentence)| sentence.as_str())
}
