//! Fills the shared Mesa shader cache the scenario recipes give every
//! scenario, as on a Desktop that has run the app before (#298).
//!
//! Each scenario's own cache starts empty. On a machine with no GPU, a
//! surface whose first frame the app had no quiet moment to prepare (a Call
//! that rings within a second of the Phone connecting) then compiles its
//! shader programs on the main loop, and held it 0.6 to 1.4 s in CI runs
//! 43681715594, 43682175818, 43684777099 and 43686065117. That delayed the
//! Banner and the Handoff behind it, past bounds a user's warm Desktop
//! meets. `just ci-scenarios`, `just test` and `just test-ci-like` run this
//! first, with `SCENARIO_SHADER_CACHE` naming the cache, and then every
//! scenario with the same. `just ci-first-frame-timings` still reports the
//! first frames with a cold cache.

use std::time::Duration;

use test_harness::Harness;

/// How long the app may take to notice the Phone connecting.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long the app's first-frame work may take on a cold cache, on a
/// loaded machine with no GPU.
const FIRST_FRAMES_DEADLINE: Duration = Duration::from_secs(60);

/// Not a check of the app: it lets the app draw every kind of thing its
/// surfaces draw once, while quiet, as it does after every start, so that
/// Mesa keeps the compiled programs in the shared cache.
#[test]
#[ignore = "fills the shared shader cache; the scenario recipes run it before the rest"]
fn the_app_prepares_every_surfaces_first_frame_into_the_shared_shader_cache() {
    assert!(
        std::env::var_os(test_harness::SCENARIO_SHADER_CACHE).is_some(),
        "{} names no cache to fill",
        test_harness::SCENARIO_SHADER_CACHE
    );
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness.wait_for_first_frames_prepared(FIRST_FRAMES_DEADLINE);
}
