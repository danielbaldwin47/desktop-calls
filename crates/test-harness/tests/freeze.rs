//! Scenarios: the app stopped for the capture of a state it shows only for
//! a moment (#248, #384).

use std::cell::OnceCell;
use std::time::{Duration, Instant};

use test_harness::Harness;

/// How long the app, or a client, may take to notice a change.
const DEADLINE: Duration = Duration::from_secs(5);

/// How long the Banner says "Identifying caller…" for a withheld number:
/// `desktop_calls_core::NUMBER_LATE_MS`.
const IDENTIFYING_FOR: Duration = Duration::from_secs(2);

/// The Answer button's green.
const GREEN: [u8; 3] = [0x20, 0x86, 0x57];

/// The app is stopped while the Banner identifies the caller, though
/// something else on screen starts moving with it and never stops, as the
/// audio tile's progress bar does with "Moving…". Before #384 the app was
/// stopped only once the screen had held still for 100 ms, which a moving
/// screen never does.
#[test]
fn a_moment_is_caught_on_a_screen_that_never_holds_still() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let busy = OnceCell::new();

    harness.freeze_app_once_shown(
        "the Banner identifying the caller",
        |screenshot| {
            screenshot
                .region_where(|pixel| {
                    pixel
                        .iter()
                        .zip(GREEN)
                        .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
                })
                .is_some()
        },
        || {
            let client = busy.get_or_init(|| harness.start_busy_test_client());
            client.wait_for("the busy client is drawn", DEADLINE, |line| {
                line.starts_with("configured")
            });
            let rang = Instant::now();
            let _path = harness.simulated_phone().ring("");
            harness.wait_for_app("the Banner identifies the caller", DEADLINE, |state| {
                state
                    .surface("banner")
                    .and_then(|banner| banner.get("caller"))
                    .is_some_and(|caller| caller == "Identifying caller…")
            });
            rang + IDENTIFYING_FOR
        },
    );

    assert!(harness.app_is_frozen(), "the app is stopped");
    harness.thaw_app();
}
