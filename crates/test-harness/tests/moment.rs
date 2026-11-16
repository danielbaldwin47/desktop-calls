//! Scenarios: the capture of a state the app shows only for a moment
//! (#248, #384).

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

/// The Banner identifying the caller is captured, though something else
/// on screen starts moving with it and never stops, as the audio tile's
/// progress bar does with "Moving…". Before #384 the app was stopped for
/// the capture only once the screen had held still for 100 ms, which a
/// moving screen never does.
#[test]
fn a_moment_is_caught_on_a_screen_that_never_holds_still() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    let busy = OnceCell::new();

    let shows_green = |screenshot: &test_harness::Screenshot| {
        screenshot
            .region_where(|pixel| {
                pixel
                    .iter()
                    .zip(GREEN)
                    .all(|(channel, wanted)| channel.abs_diff(wanted) <= 16)
            })
            .is_some()
    };

    let capture = harness.capture_moment("banner-identifying", shows_green, || {
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
    });

    assert!(
        shows_green(&capture),
        "the capture shows the Banner; see {}",
        capture.path.display()
    );
}

/// A state no frame shows before it could end fails, saying when each
/// frame was grabbed against when the state could end, rather than keep a
/// capture of something else.
#[test]
#[should_panic(expected = "frames grabbed at")]
fn a_moment_no_frame_shows_fails_with_when_each_frame_was_grabbed() {
    let harness = Harness::builder().headless_screen().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });

    let _ = harness.capture_moment(
        "never-shown",
        |screenshot| screenshot.region_where(|pixel| pixel == GREEN).is_some(),
        || Instant::now() + Duration::from_millis(500),
    );
}
