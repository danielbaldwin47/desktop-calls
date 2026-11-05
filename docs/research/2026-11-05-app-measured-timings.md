# Scorecard timings measured by the app, up to the frame drawn

Date: 2026-11-05. Ticket #83. Builds on #80 (PR #76), which did the same for Handoffs.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the Banner, Answer and Dialer scenarios now assert the app's own log, from the input reaching the app to the frame drawn (or the audio on the Desktop), against the scorecard targets. The tests' own round trip gets a looser 3 s. Under `just test-ci-like` all of them pass. A 100 ms delay added to each Dialer keystroke fails its scenario, and a 1 s delay added before the Banner shows fails the Banner scenarios. Measured honestly, the Dialer's first frame misses its 150 ms target on a cold shader cache. #47 tracks that, and until it lands the scenario asserts only GTK's map.

## 1. What the app logs

| Event | When | Level |
|---|---|---|
| `answer-requested` (`call_id`) | Answer reaches the Call state machine, from the control interface, the Banner's button or its A key | info |
| `activate-requested` | `Activate` reaches the Call state machine (the keybinding, the app launcher) | info |
| `surface-mapped` (`surface`) | GTK maps a surface: the moment the control interface's `Surfaces` lists it | debug |
| `surface-shown` (`surface`) | the surface's first frame since it was mapped has been drawn and handed to the compositor (the window's frame clock emits `after-paint`) | info |
| `dialer-query-typed` (`query_chars`) | the Dialer's entry changed because the user typed | debug |
| `dialer-query-shown` (`query_chars`) | the frame showing the rows for that query has been drawn | debug |

`query_chars` is the query's length, so the log never holds what was typed. The harness pairs events with `Harness::app_took(from, to)`: it takes the first `from`, then the first `to` at or after it, and subtracts their `timestamp_us`. `assert_banner_within` and `assert_answered_audio_within` wrap the two scorecard items. **[V]**

`after-paint` is the end of GTK's paint phase, after the frame has been committed to the compositor. It is not the compositor's presentation time, which would need `wp_presentation` feedback. **[I]** from GTK's frame-clock phases.

## 2. Figures under `just test-ci-like` (CPU quota 100%), app-measured

From one run of the touched scenario files: 90 scenarios, all passing. **[V]**

| Timing | Target | Observed |
|---|---|---|
| Call appeared to the Banner's first frame | 1 s | 32 to 704 ms (26 Banners) |
| Answer to the audio on the Desktop | 1.5 s | 4 to 96 ms (24 Answers); 788 ms in an earlier run |
| Keystroke to the frame with its rows (5000 Contacts) | 50 ms | 2 to 12 ms (worst per scenario) |
| `Activate` to the Dialer mapped | 150 ms | passes |
| `Activate` to the Dialer's first frame (not asserted) | 150 ms | 380 to 991 ms; 333 to 1393 ms in an earlier run |

On the Desktop without a quota: the Banner took 126 to 144 ms, and the Dialer's first frame took 306 to 319 ms after `Activate`, of which mapping took about 5 to 12 ms. **[V]**

## 3. The Dialer's first frame

- #11 measured the Dialer shown 12 ms after `Activate`. That figure was GTK's map, because `Surfaces` reports a surface when it is mapped, before anything is drawn. **[V]**
- Frame-clock phases for the first opening: map at 5 ms, layout at 14 ms, the end of paint at 307 ms. The next frame painted in 1.4 ms. So the time goes on the first paint. **[V]**
- With `GSK_RENDERER=cairo`, the scenario passed. With a persistent `MESA_SHADER_CACHE_DIR`, the first run took 313 ms and the next two passed. So the cost is the GPU renderer compiling its shaders, on a Mesa cache that is cold in every scenario because each one has a fresh `XDG_CACHE_HOME`. **[V]** The user's Desktop keeps its cache, so it is cold only after an install or a driver update. **[I]**
- Tried and removed: rendering a representative node tree (colour, text, border, shadows, gradient, under each kind of clip) offscreen at startup, through another surface's renderer. The Dialer's first frame dropped to 160 to 164 ms, so GTK's compiled pipelines are shared across windows, but the warm-up held up startup for 1.2 s. **[V]**
- Decision (chief of staff, 2026-11-05): assert the map against 150 ms, log the first frame beside it, and leave the rest to #47.

## 4. The Banner under heavier load

- At a 50% CPU quota, the Banner's first frame took about 1.2 s in some `incoming_call_surfaces` scenarios and failed. The Dialer scenarios passed. The old assertion (map plus the round trip, 1 s) passed at 50% in #43's calibration, because it did not count the first paint. **[V]**
- An app-measured time can't absorb load on the app itself: the app shares the starved CPU. It only removes the tests' process start-up and D-Bus scheduling. At 100%, the calibrated match for CI, the slowest Banner was 704 ms. **[V]** That is the closest margin left on CI. #47 (first-frame work at startup) is what widens it. **[I]**

## 5. Load-alone failures of the old assertions

- The old 50 ms keystroke round trip did not fail locally at 100% or 50% (12 of 12 passed at 50%). At 25%, unrelated focus waits fail first. So the CI failure in the ticket (run 41998137454) was not reproduced. **[V]**
