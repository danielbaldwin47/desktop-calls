//! Group 4's scripts: call waiting (ticket #148). The Test Number's Call is
//! placed from the Dialer and answered by the first helper; the second
//! helper, and for 4.5 and 4.6 a third phone, then call in during it. The
//! runner sees each choice's result in the app's published Calls and in
//! the Call Window's fields, and records what the app shows when the Phone
//! does something else, so a real Phone's answer to each of the unverified
//! points in `docs/research/2026-11-05-call-waiting.md` lands in the result
//! file.
//!
//! Only the first Call goes to a number the runner knows, so it is the one
//! the Test Number guards: any other is hung up, as in 7.1.

use super::keyboard_and_look::{test_number_or_why_not, went_to};
use super::missed_calls::rings_out;
use super::{
    ACT_DEADLINE, APP_DEADLINE, CALL_DEADLINE, TALK_DEADLINE, check_banner_latency, no_call_left,
};
use crate::checklist::fingerprint::Fingerprint;
use crate::checklist::sitting::{Sitting, Stop, seconds};
use crate::checklist::watch::AppState;

/// What the Banner offers for a Waiting Call while one Call is live, and
/// none is on hold.
const OPTIONS_WITH_ONE_LIVE: &str = "Hold & Answer, End & Answer, Decline";
/// With one Call live and another on hold, the Phone can hold no more.
const OPTIONS_WITH_ONE_ON_HOLD_AND_ONE_LIVE: &str = "End & Answer, Decline";
/// With one Call on hold alone, nothing is live to hold or end.
const OPTIONS_WITH_ONE_ON_HOLD: &str = "Answer, Decline";

const TEST_NUMBER_CALL: &str = "the Test Number's Call";
const SECOND_HELPER_CALL: &str = "the second helper's Call";
const THIRD_PHONE_CALL: &str = "the third phone's Call";

/// What a Call Window field says, while the Call Window shows.
fn window_field<'a>(state: &'a AppState, key: &str) -> Option<&'a str> {
    state.surface_field("call-window", key)
}

fn state_of(state: &AppState, call: u32) -> Option<&str> {
    state.call(call).map(|known| known.state.as_str())
}

/// Exactly two Calls: `live` is live and `held` on hold, the Call Window
/// lists them so, and no Banner is up.
fn live_beside_held(state: &AppState, live: u32, held: u32) -> bool {
    state.calls.len() == 2
        && state_of(state, live) == Some("active")
        && state_of(state, held) == Some("held")
        && window_field(state, "call") == Some(live.to_string().as_str())
        && window_field(state, "held-call") == Some(held.to_string().as_str())
        && !state.shows("banner")
}

/// `call` is the only Call, live, and no Banner is up.
fn live_alone(state: &AppState, call: u32) -> bool {
    state.calls.len() == 1
        && state_of(state, call) == Some("active")
        && window_field(state, "call") == Some(call.to_string().as_str())
        && window_field(state, "held-call").is_none()
        && !state.shows("banner")
}

/// The sentence the Call Window shows when the Phone refused a choice.
fn refusal(state: &AppState) -> Option<&str> {
    window_field(state, "notice")
}

/// What the app shows of `calls`, as words, for a finding that says what
/// came of a choice that didn't go as it should.
fn describe(state: &AppState, calls: &[(u32, &str)]) -> String {
    let mut parts: Vec<String> = calls
        .iter()
        .map(|(id, name)| match state_of(state, *id) {
            Some(known) => format!("{name} is {known}"),
            None => format!("{name} is gone"),
        })
        .collect();
    let live = window_field(state, "call")
        .and_then(|shown| calls.iter().find(|(id, _)| id.to_string() == shown));
    parts.push(match live {
        Some((_, name)) => format!("the Call Window shows {name} as the live Call"),
        None => "the Call Window shows none of them as the live Call".to_owned(),
    });
    if state.shows("banner") {
        parts.push("the Banner is still up".to_owned());
    }
    if let Some(notice) = refusal(state) {
        parts.push(format!("the Call Window says: {notice}"));
    }
    parts.join("; ")
}

/// Has the user end every Call still going, one at a time: the Waiting Call
/// is declined first, because the Phone would act on it for any other end.
fn end_every_call(sitting: &mut Sitting) -> Result<(), Stop> {
    loop {
        let state = sitting.now();
        let left = state.calls.len();
        if left == 0 {
            return Ok(());
        }
        let waiting = state.calls.iter().any(|call| call.state == "waiting");
        sitting.say(
            "End the Calls still going, on either device: decline a waiting one first.",
            &[if waiting {
                "decline-waiting"
            } else {
                "hang-up"
            }],
        );
        sitting.wait_for("a Call ending", TALK_DEADLINE, |state| {
            state.calls.len() < left
        })?;
    }
}

/// Waits for what the user's choice should show: `wanted`. It stops early,
/// with `false`, once the Call Window says why the Phone refused. Returns
/// whether `wanted` held, and the last state it looked at.
fn after_choice(
    sitting: &mut Sitting,
    what: &str,
    wanted: impl Fn(&AppState) -> bool,
) -> Result<(bool, AppState), Stop> {
    let seen = sitting.happens_within(what, ACT_DEADLINE, |state| {
        wanted(state) || refusal(state).is_some()
    })?;
    Ok(match seen {
        Some(seen) => (wanted(&seen.state), seen.state),
        None => (false, sitting.now()),
    })
}

/// Waits for the app to settle on `wanted`, as it should within moments of
/// the Phone's news. Returns whether it did, and the last state it looked at.
fn settles(
    sitting: &mut Sitting,
    what: &str,
    wanted: impl Fn(&AppState) -> bool,
) -> Result<(bool, AppState), Stop> {
    let seen = sitting.happens_within(what, APP_DEADLINE, wanted)?;
    Ok(match seen {
        Some(seen) => (true, seen.state),
        None => (false, sitting.now()),
    })
}

/// Places a Call to the Test Number from the Dialer, and has the first
/// helper answer it. Returns its id, or `None` once the step has recorded
/// why it can't go on.
fn live_call_to_the_test_number(
    sitting: &mut Sitting,
    test_number: &Fingerprint,
) -> Result<Option<u32>, Stop> {
    // An earlier step may have hung up a wrong Call, whose states are still
    // in the timeline.
    sitting.forget_the_past();
    sitting.say(
        "Press your Desktop Calls keybinding to open the Dialer, type the Test Number, and press Enter.",
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
    if !went_to(sitting, call, test_number)? {
        return Ok(None);
    }
    sitting.observed("The Call went to the Test Number.".to_owned());
    sitting.say(
        "Have the first helper answer on the Test Number's phone.",
        &["callee-answers"],
    );
    sitting.wait_for(
        "the Test Number's Call being answered",
        TALK_DEADLINE,
        |state| state.call(call).is_none_or(|known| known.state == "active"),
    )?;
    if sitting.now().call(call).is_none() {
        sitting.check(
            false,
            "The Test Number's Call ended before the first helper answered it.".to_owned(),
        );
        return Ok(None);
    }
    Ok(Some(call))
}

/// Has `request` call in during the Calls already up (`known`), and checks
/// that the Phone presents the new Call as waiting, and that the Banner
/// shows it with `options`, quickly, and with the Desktop silent. Returns
/// the Call's id, or `None` once the step has recorded why it can't go on
/// and the user has ended the Calls.
fn waiting_call_arrives(
    sitting: &mut Sitting,
    known: &[u32],
    request: &str,
    demo_action: &str,
    options: &str,
) -> Result<Option<u32>, Stop> {
    let mark = sitting.mark();
    sitting.say(request, &[demo_action]);
    let arrived = sitting.wait_for("the helper's Call arriving", CALL_DEADLINE, |state| {
        state.calls.iter().any(|call| !known.contains(&call.id))
    })?;
    let Some(call) = arrived
        .state
        .calls
        .iter()
        .find(|call| !known.contains(&call.id))
        .cloned()
    else {
        return Err(Stop::TimedOut("the helper's Call arriving".to_owned()));
    };
    if call.state != "waiting" {
        sitting.check(
            false,
            format!(
                "The helper's Call arrived as {}, not as a Waiting Call, so the Banner can't offer a choice between the Calls.",
                call.state
            ),
        );
        end_every_call(sitting)?;
        return Ok(None);
    }
    let banner = sitting.happens_within("the Waiting Call's Banner", APP_DEADLINE, |state| {
        state.surface_field("banner", "options").is_some() || state.call(call.id).is_none()
    })?;
    let Some(banner) = banner.filter(|seen| seen.state.call(call.id).is_some()) else {
        sitting.check(
            false,
            format!(
                "The Waiting Call's Banner, with its options, didn't show within {}.",
                seconds(APP_DEADLINE)
            ),
        );
        end_every_call(sitting)?;
        return Ok(None);
    };
    check_banner_latency(sitting, call.id, arrived.at, banner.at);
    let offered = banner.state.surface_field("banner", "options");
    sitting.check(
        offered == Some(options),
        match offered {
            Some(offered) if offered == options => format!("The Banner offered {offered}."),
            Some(offered) => format!("The Banner offered {offered}, not {options}."),
            None => format!("The Banner offered nothing, not {options}."),
        },
    );
    let rang = sitting.seen_since(mark, |state| state.ringing).is_some();
    sitting.check(
        !rang,
        if rang {
            "The Desktop rang for the Waiting Call.".to_owned()
        } else {
            "The Desktop stayed silent for the Waiting Call.".to_owned()
        },
    );
    Ok(Some(call.id))
}

/// Chooses Hold & Answer on the Banner, and checks that the second helper's
/// Call is live and the Test Number's on hold. Has the user end the Calls
/// when it isn't, and returns whether it was.
fn hold_and_answer(sitting: &mut Sitting, first: u32, second: u32) -> Result<bool, Stop> {
    sitting.say("On the Banner, choose Hold & Answer.", &["hold-and-answer"]);
    let (done, state) = after_choice(sitting, "Hold & Answer taking effect", |state| {
        live_beside_held(state, second, first)
    })?;
    sitting.check(
        done,
        if done {
            "Hold & Answer put the Test Number's Call on hold and made the second helper's live, and the Call Window lists both.".to_owned()
        } else {
            format!(
                "Hold & Answer didn't leave the second helper's Call live beside the Test Number's on hold: {}.",
                describe(&state, &[(first, TEST_NUMBER_CALL), (second, SECOND_HELPER_CALL)])
            )
        },
    );
    if done {
        audio_is_on_the_desktop(sitting);
    } else {
        end_every_call(sitting)?;
    }
    Ok(done)
}

/// Notes where the audio is, as a Call is answered during another: the
/// Desktop, where the user took the first Call.
fn audio_is_on_the_desktop(sitting: &mut Sitting) {
    let location = sitting.now().audio_location;
    sitting.check(
        location == "desktop",
        format!("The audio is on the {location} after the Waiting Call was answered."),
    );
}

/// 4.1: Hold & Answer puts the Test Number's Call on hold and connects the
/// second helper's; Swap trades them; hanging up the live one leaves the
/// other live.
pub(super) fn hold_and_answer_then_swap(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let Some(first) = live_call_to_the_test_number(sitting, &test_number)? else {
        return Ok(());
    };
    let Some(second) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you, from their phone, and don't answer.",
        "call",
        OPTIONS_WITH_ONE_LIVE,
    )?
    else {
        return Ok(());
    };
    if !hold_and_answer(sitting, first, second)? {
        return Ok(());
    }

    sitting.say("In the Call Window, choose Swap.", &["swap"]);
    let (swapped, state) = after_choice(sitting, "Swap taking effect", |state| {
        live_beside_held(state, first, second)
    })?;
    sitting.check(
        swapped,
        if swapped {
            "Swap put the Test Number's Call live and the second helper's on hold, and the Call Window lists both.".to_owned()
        } else {
            format!(
                "Swap didn't put the Test Number's Call live beside the second helper's on hold: {}.",
                describe(&state, &[(first, TEST_NUMBER_CALL), (second, SECOND_HELPER_CALL)])
            )
        },
    );
    if !swapped {
        return end_every_call(sitting);
    }

    sitting.say(
        "Hang up the live Call, the Test Number's, from the Call Window.",
        &["hang-up"],
    );
    sitting.wait_for("the live Call ending", TALK_DEADLINE, |state| {
        state.calls.len() < 2
    })?;
    let (alone, state) = settles(sitting, "the other Call going live", |state| {
        live_alone(state, second)
    })?;
    sitting.check(
        alone,
        if alone {
            "Hanging up one Call left the other, the second helper's, live alone.".to_owned()
        } else {
            format!(
                "Hanging up the live Call didn't leave the second helper's Call live alone: {}.",
                describe(
                    &state,
                    &[(first, TEST_NUMBER_CALL), (second, SECOND_HELPER_CALL)]
                )
            )
        },
    );
    end_every_call(sitting)
}

/// 4.2: End & Answer ends the Test Number's Call and connects the second
/// helper's.
pub(super) fn end_and_answer(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let Some(first) = live_call_to_the_test_number(sitting, &test_number)? else {
        return Ok(());
    };
    let Some(second) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you, from their phone, and don't answer.",
        "call",
        OPTIONS_WITH_ONE_LIVE,
    )?
    else {
        return Ok(());
    };
    sitting.say("On the Banner, choose End & Answer.", &["end-and-answer"]);
    let (done, state) = after_choice(sitting, "End & Answer taking effect", |state| {
        live_alone(state, second)
    })?;
    sitting.check(
        done,
        if done {
            "End & Answer ended the Test Number's Call and connected the second helper's, which is live alone.".to_owned()
        } else {
            format!(
                "End & Answer didn't leave the second helper's Call live alone: {}.",
                describe(&state, &[(first, TEST_NUMBER_CALL), (second, SECOND_HELPER_CALL)])
            )
        },
    );
    if done {
        audio_is_on_the_desktop(sitting);
    }
    end_every_call(sitting)
}

/// 4.3: Decline ends only the second helper's Call.
pub(super) fn decline(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let Some(first) = live_call_to_the_test_number(sitting, &test_number)? else {
        return Ok(());
    };
    let Some(second) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you, from their phone, and don't answer.",
        "call",
        OPTIONS_WITH_ONE_LIVE,
    )?
    else {
        return Ok(());
    };
    sitting.say("On the Banner, choose Decline.", &["decline-waiting"]);
    let (done, state) = after_choice(sitting, "Decline taking effect", |state| {
        live_alone(state, first)
    })?;
    sitting.check(
        done,
        if done {
            "Decline ended only the second helper's Call: the Test Number's is still live, alone."
                .to_owned()
        } else {
            format!(
                "Decline didn't leave the Test Number's Call live alone: {}.",
                describe(
                    &state,
                    &[(first, TEST_NUMBER_CALL), (second, SECOND_HELPER_CALL)]
                )
            )
        },
    );
    end_every_call(sitting)
}

/// 4.4: the second helper's Call rings out beside the Test Number's: a
/// Missed Call notification appears, and the Test Number's Call carries on.
pub(super) fn ring_out_while_waiting(sitting: &mut Sitting) -> Result<(), Stop> {
    no_call_left(sitting)?;
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let Some(first) = live_call_to_the_test_number(sitting, &test_number)? else {
        return Ok(());
    };
    let Some(second) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you, from their phone, and let it ring until it stops.",
        "call",
        OPTIONS_WITH_ONE_LIVE,
    )?
    else {
        return Ok(());
    };
    if rings_out(sitting, second, "waiting")?.is_none() {
        return end_every_call(sitting);
    }
    let shown = sitting.ask(
        "Does a notification say you missed a call from the second helper, by name or number, with the time?",
    )?;
    sitting.check(
        shown,
        if shown {
            "The notification names the second helper with the time.".to_owned()
        } else {
            "The notification didn't name the second helper with the time.".to_owned()
        },
    );
    let carried_on = live_alone(&sitting.now(), first);
    sitting.check(
        carried_on,
        if carried_on {
            "The Test Number's Call carried on, live, while the second helper's rang out."
                .to_owned()
        } else {
            format!(
                "The Test Number's Call didn't carry on live alone: {}.",
                describe(&sitting.now(), &[(first, TEST_NUMBER_CALL)])
            )
        },
    );
    end_every_call(sitting)
}

/// The Calls 4.5 leaves up, if they are up: the Test Number's on hold and
/// one other live. Returns (the Test Number's, the live one).
fn left_by_4_5(state: &AppState, test_number: &Fingerprint) -> Option<(u32, u32)> {
    let held = state
        .calls
        .iter()
        .find(|call| call.state == "held" && call.number.is(test_number))?;
    let live = state.calls.iter().find(|call| call.state == "active")?;
    live_beside_held(state, live.id, held.id).then_some((held.id, live.id))
}

/// Brings up the Test Number's Call on hold beside the second helper's
/// first Call live, has the third phone call, and ends the live Call with
/// End & Answer. Returns (the Test Number's Call, the third phone's), the
/// first on hold and the second live, or `None` once the step has
/// recorded why it can't go on and the user has ended the Calls.
fn test_number_held_beside_the_third_phone(
    sitting: &mut Sitting,
    test_number: &Fingerprint,
) -> Result<Option<(u32, u32)>, Stop> {
    no_call_left(sitting)?;
    let Some(first) = live_call_to_the_test_number(sitting, test_number)? else {
        return Ok(None);
    };
    let Some(second) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you, from their phone, and don't answer.",
        "call",
        OPTIONS_WITH_ONE_LIVE,
    )?
    else {
        return Ok(None);
    };
    if !hold_and_answer(sitting, first, second)? {
        return Ok(None);
    }
    let Some(third) = waiting_call_arrives(
        sitting,
        &[first, second],
        "Now have the second helper call you again, from the third phone, and don't answer.",
        "call-third",
        OPTIONS_WITH_ONE_ON_HOLD_AND_ONE_LIVE,
    )?
    else {
        return Ok(None);
    };
    sitting.say("On the Banner, choose End & Answer.", &["end-and-answer"]);
    let (done, state) = after_choice(sitting, "End & Answer taking effect", |state| {
        live_beside_held(state, third, first)
    })?;
    sitting.check(
        done,
        if done {
            "End & Answer ended only the second helper's first Call: the third phone's is live and the Test Number's is still on hold.".to_owned()
        } else {
            format!(
                "End & Answer didn't leave the third phone's Call live beside the Test Number's on hold: {}.",
                describe(
                    &state,
                    &[
                        (first, TEST_NUMBER_CALL),
                        (second, "the second helper's first Call"),
                        (third, THIRD_PHONE_CALL),
                    ]
                )
            )
        },
    );
    if !done {
        end_every_call(sitting)?;
        return Ok(None);
    }
    Ok(Some((first, third)))
}

/// 4.5: with the Test Number's Call on hold and the second helper's live,
/// a Call from the third phone is offered only End & Answer and Decline,
/// and End & Answer ends only the live Call. It leaves the two Calls up for
/// 4.6.
pub(super) fn end_and_answer_with_one_on_hold(sitting: &mut Sitting) -> Result<(), Stop> {
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    if test_number_held_beside_the_third_phone(sitting, &test_number)?.is_some() {
        sitting.say(
            "Leave both Calls as they are: the next step carries on from them.",
            &[],
        );
    }
    Ok(())
}

/// 4.6: once the third phone hangs up, the Test Number's Call stays on
/// hold alone, and a new Call from the second helper's first phone is
/// offered Answer and Decline, and Answer keeps the Test Number's Call on
/// hold. It carries on from the Calls 4.5 left up, and brings them up
/// again if they aren't.
pub(super) fn answer_beside_a_held_call(sitting: &mut Sitting) -> Result<(), Stop> {
    let Some(test_number) = test_number_or_why_not(sitting) else {
        return Ok(());
    };
    let carried_over = left_by_4_5(&sitting.now(), &test_number);
    let Some((first, third)) = (match carried_over {
        Some(calls) => {
            sitting.observed(
                "Carrying on from 4.5: the Test Number's Call is on hold and the third phone's is live."
                    .to_owned(),
            );
            Some(calls)
        }
        None => test_number_held_beside_the_third_phone(sitting, &test_number)?,
    }) else {
        return Ok(());
    };

    sitting.say("Have the third phone hang up.", &["hang-up-on-phone"]);
    sitting.wait_for("the third phone's Call ending", TALK_DEADLINE, |state| {
        state.call(third).is_none()
    })?;
    let (alone, state) = settles(sitting, "the Test Number's Call on hold alone", |state| {
        state.calls.len() == 1 && state_of(state, first) == Some("held")
    })?;
    sitting.check(
        alone,
        if alone {
            "The Test Number's Call stayed on hold, alone, when the third phone hung up."
                .to_owned()
        } else {
            format!(
                "The Test Number's Call didn't stay on hold alone when the third phone hung up: {}.",
                describe(&state, &[(first, TEST_NUMBER_CALL)])
            )
        },
    );
    if !alone {
        return end_every_call(sitting);
    }

    let Some(again) = waiting_call_arrives(
        sitting,
        &[first],
        "Now have the second helper call you again, from their first phone, and don't answer.",
        "call",
        OPTIONS_WITH_ONE_ON_HOLD,
    )?
    else {
        return Ok(());
    };
    sitting.say("On the Banner, choose Answer.", &["answer-waiting"]);
    let (done, state) = after_choice(sitting, "Answer taking effect", |state| {
        live_beside_held(state, again, first)
    })?;
    sitting.check(
        done,
        if done {
            "Answer connected the second helper's new Call, and the Test Number's stayed on hold."
                .to_owned()
        } else {
            format!(
                "Answer didn't leave the second helper's new Call live beside the Test Number's on hold: {}.",
                describe(
                    &state,
                    &[
                        (first, TEST_NUMBER_CALL),
                        (again, "the second helper's new Call")
                    ]
                )
            )
        },
    );
    end_every_call(sitting)
}
