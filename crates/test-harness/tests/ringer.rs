//! Scenarios: the Ringer, the sound the Desktop plays while a Call rings
//! here, heard through the player stand-in (`pw-play`). By default it plays
//! a soft Ring Sound the app makes itself, at 60 % of full stream volume,
//! faded in over its first repeat, with a pause of about 2 s between
//! repeats; and stopping it stops the player at once and leaves nothing
//! behind (issue #432).

use std::time::Duration;

use test_harness::{Harness, Play};

/// How long the app may take to notice a change on the Phone.
const PHONE_DEADLINE: Duration = Duration::from_secs(5);
/// How long the app may take to show a Call ringing.
const ROUND_TRIP: Duration = Duration::from_secs(3);
/// How long two repeats and the pause between them may take: two Ring
/// Sounds of under 2 s and a 2 s pause, with room for a loaded runner.
const TWO_REPEATS: Duration = Duration::from_secs(15);
/// The least pause between repeats: 2 s, less the stand-in's own
/// millisecond rounding. A loaded runner only makes it longer.
const LEAST_PAUSE: Duration = Duration::from_millis(1_950);
/// How long the player may take to stop once the ring stops.
const STOP_DEADLINE: Duration = Duration::from_secs(2);
/// How long the app is watched for a warning after the player stopped.
const HOLD: Duration = Duration::from_millis(500);

const NUMBER: &str = "+447700900123";

/// A scenario whose Phone rings with one Call, once the Desktop shows it.
fn ringing() -> Harness {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", PHONE_DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let _call = harness.simulated_phone().ring(NUMBER);
    harness.wait_for_app("the Desktop rings", ROUND_TRIP, |state| state.ringing);
    harness
}

#[test]
fn a_ringing_call_plays_the_soft_ring_sound_quietly_with_a_pause_between_repeats() {
    let harness = ringing();

    let plays = harness.wait_for_plays(2, TWO_REPEATS);

    let [first, second] = [&plays[0], &plays[1]];
    for play in [first, second] {
        assert_eq!(play.volume, Some(0.6), "at 60 % of full volume: {play:?}");
        assert_eq!(play.media_role.as_deref(), Some("Notification"), "{play:?}");
        assert!(
            play.properties
                .as_deref()
                .is_some_and(|properties| properties.contains("Desktop Calls ringer")
                    && properties.contains("state.restore-props = false")),
            "a named stream whose volume WirePlumber leaves alone: {play:?}"
        );
        assert!(
            play.file
                .starts_with(harness.directory().join("cache/desktop-calls")),
            "a Ring Sound the app made, not the sound theme's: {play:?}"
        );
        assert!(play.peak() <= 0.5, "soft: peaks below -6 dBFS: {play:?}");
    }
    let finished = first.finished.expect("the first repeat played to its end");
    let pause = second
        .started
        .duration_since(finished)
        .expect("the second repeat starts after the first ends");
    assert!(pause >= LEAST_PAUSE, "a pause of about 2 s, not {pause:?}");
    assert_fades_in(first, second);
}

/// The first repeat starts quieter than the repeats after it.
fn assert_fades_in(first: &Play, repeat: &Play) {
    let opening = Duration::from_millis(300);
    let (faded, full) = (first.peak_within(opening), repeat.peak_within(opening));
    assert!(
        full > 0.0 && faded <= full / 2.0,
        "the first repeat fades in: it opens at {faded:.3}, the next at {full:.3}"
    );
}

#[test]
fn a_ring_stopped_mid_sound_stops_the_player_and_reaps_it_once() {
    let harness = ringing();
    let first = harness.wait_for_plays(1, TWO_REPEATS);
    assert!(first[0].finished.is_none(), "still playing: {first:?}");

    let call = harness
        .wait_for_app("one Call", ROUND_TRIP, |state| state.calls.len() == 1)
        .calls[0]
        .id;
    harness.decline(call);
    harness.wait_for_app("the Desktop stops ringing", ROUND_TRIP, |state| {
        !state.ringing
    });

    let plays = harness.wait_until_nothing_plays(STOP_DEADLINE);
    assert!(plays[0].finished.is_none(), "stopped mid-sound: {plays:?}");
    harness.app_holds("the Call has gone", HOLD, |state| state.calls.is_empty());
    let stderr = harness.app_stderr();
    assert!(
        !stderr.contains("waitid"),
        "the player was reaped twice: GLib warned\n{stderr}"
    );
    assert_eq!(
        harness.plays().len(),
        plays.len(),
        "no repeat after the stop"
    );
}
