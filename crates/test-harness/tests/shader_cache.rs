//! Fills the shared Mesa shader cache the scenario recipes give every
//! scenario, as on a Desktop that has run the app before (#298).
//!
//! Each scenario's own cache starts empty. On a machine with no GPU, a
//! surface whose first frame the app had no quiet moment to prepare (a Call
//! that rings within a second of the Phone connecting) then compiles its
//! shader programs on the main loop, and held it 0.6 to 1.4 s in CI runs
//! 43681715594, 43682175818, 43684777099 and 43686065117. That delayed the
//! Banner and the Handoff behind it, past bounds a user's warm Desktop
//! meets. `just ci-scenarios`, `just test`, `just test-file` and
//! `just test-ci-like` run this first, with `SCENARIO_SHADER_CACHE` naming
//! the cache, and then every scenario with the same
//! (`tools/scenario_shader_cache.py`). `just ci-first-frame-timings` still
//! reports the first frames with a cold cache.

use std::path::Path;
use std::time::{Duration, Instant};

use test_harness::{Harness, Key};

/// How long the app may take to notice a change on the Phone.
const DEADLINE: Duration = Duration::from_secs(5);
/// How long a first frame, or the app's first-frame work, may take on a
/// cold cache, on a loaded machine with no GPU.
const COLD_DEADLINE: Duration = Duration::from_secs(60);
/// How long the cache must stay the same size to count as written: Mesa
/// writes it from a thread of its own.
const SETTLED: Duration = Duration::from_secs(1);

const NUMBER: &str = "+447700900123";

/// Not a check of the app: it shows each surface once, through the
/// scenarios' usual actions, so that Mesa keeps the programs every window
/// draws with in the shared cache. The app's offscreen first-frame work
/// alone misses those a window's own first draw needs on a software
/// renderer ([testing](../../../docs/wiki/testing.md)).
#[test]
#[ignore = "fills the shared shader cache; the scenario recipes run it before the rest"]
fn the_app_shows_every_surface_once_into_the_shared_shader_cache() {
    let cache = std::env::var(test_harness::SCENARIO_SHADER_CACHE)
        .ok()
        .filter(|cache| !cache.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "{} names no cache to fill",
                test_harness::SCENARIO_SHADER_CACHE
            )
        });
    let harness = Harness::builder().headless_screen().omarchy_bar().start();
    harness.simulated_phone().connect();
    harness.wait_for_app("ConnectionState is connected", DEADLINE, |state| {
        state.connection_state == "connected"
    });
    harness.wait_for_first_frames_prepared(COLD_DEADLINE);

    harness.open_dialer("", 0);
    first_frame(&harness, "dialer");
    // It takes the keyboard apart from drawing; a key before that is lost.
    harness.wait_for_app("the Dialer has the keyboard", DEADLINE, |state| {
        state
            .surface("dialer")
            .and_then(|dialer| dialer.get("focused"))
            .map(String::as_str)
            == Some("true")
    });
    harness.keyboard().press(Key::Escape);
    harness.wait_for_app("the Dialer closed", DEADLINE, |state| {
        state.surface("dialer").is_none()
    });

    let _call = harness.simulated_phone().ring(NUMBER);
    first_frame(&harness, "banner");
    let state = harness.wait_for_app("Call 1 rings", DEADLINE, |state| !state.calls.is_empty());
    let call = state.calls[0].id;
    harness.answer(call);
    first_frame(&harness, "call-window");
    harness.hang_up(call);
    harness.wait_for_app("the Call ended", DEADLINE, |state| state.calls.is_empty());

    settled(Path::new(&cache));
}

/// Waits for the app to draw `surface`'s first frame, by its log.
fn first_frame(harness: &Harness, surface: &str) {
    let _ = harness.wait_for_app_log(
        &format!("Drew the {surface} surface's first frame"),
        COLD_DEADLINE,
    );
}

/// Waits until the cache holds something and its size stays the same for
/// [`SETTLED`]. The cache outlives a run in `target/`, so on a rerun only
/// the steady size says this run's writes are done.
fn settled(cache: &Path) {
    let started = Instant::now();
    let mut last = (size(cache), Instant::now());
    loop {
        let now = size(cache);
        if now != last.0 {
            last = (now, Instant::now());
        } else if now > 0 && last.1.elapsed() >= SETTLED {
            return;
        }
        assert!(
            started.elapsed() < COLD_DEADLINE,
            "the shader cache in {} never settled: {now} bytes",
            cache.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn size(path: &Path) -> u64 {
    std::fs::read_dir(path).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => size(&entry.path()),
                Ok(metadata) => metadata.len(),
                Err(_) => 0,
            })
            .sum()
    })
}
