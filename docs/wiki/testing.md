# Testing

Dated facts about how the app is tested: the harness, CI against the Desktop, and the Real-Phone Checklist. Page added 2026-11-05 (batch 3). Earlier testing history lives in [session-log](session-log.md) and [polish-scorecard](polish-scorecard.md); the test seam itself is in [product-shape](product-shape.md) (Q27).

## Timings: CI against the Desktop

- 2026-11-05 (#47, `3def480`): CI's runner paints in software. Run alone there, the Dialer's first frame, warm, took 150.4 ms (6 ms to map, 144 ms for its own first paint), against 97 to 102 ms under `just test-ci-like`. So CI holds it to 250 ms, a regression bound that still catches shaders compiling cold (306 ms and more). Locally and under `just test-ci-like` (which sets `DESKTOP_CALLS_CI_LIKE`) the bound is 150 ms. The 150 ms Desktop target is judged on the real Desktop, at Real-Phone Checklist step 7.4, from the app-logged time between `activate-requested` and the Dialer's `surface-shown`.
- 2026-11-05 (#47, `3def480`): the two first-frame timing tests (`the_first_banner_shows_without_holding_up_the_main_loop` and `with_5000_contacts_the_dialer_shows_and_follows_each_keystroke_promptly`) are `#[ignore]`d in the whole run. CI runs them in a step of their own, one scenario at a time (`-- --ignored --test-threads=1`). With four scenarios at once, every scenario's app compiles shaders together, which no Desktop does. The Dialer, first-Banner and idle scenarios wait for `first-frames-prepared` (`wait_for_first_frames_prepared`) before they act. Source: [first frames](../research/2026-11-05-first-frames.md).
- 2026-11-05 (#17, `417411e`): the Do Not Disturb toggle scenario holds 250 ms from the harness replacing `quiet.json` to the ringer's log line. The app's own part was under 1 ms; the rest is inotify and the main loop. Source: [Do Not Disturb silent](../research/2026-11-05-do-not-disturb-silent.md).

## Harness

- 2026-11-05 (#107, `7032137`): the harness records `Calls` and `Surfaces` after each `PropertiesChanged`, in the order the app sent them (`Harness::published`), and `surfaces_seen` derives from it. `crates/test-harness/tests/publish_order.rs` checks that a surface names a Call only once `Calls` lists it, and closes only once `Calls` says why. In the app, `Surfaces::render` and `Indicator::render` take a `Published` proof that only `Publisher::publish` makes, so rendering before publishing doesn't compile. Source: [publish order](../research/2026-11-05-publish-order.md).
- 2026-11-05 (#19, `6bf11cb`): the Simulated Phone's `Disconnect` means out of range, and it gained `ReturnToRange`, `ForgetPairing`, `CallsOnPhone` and `PrepareForSleep(b)` (it owns `org.freedesktop.login1` on the private system bus). `Battery1` exists only while hands-free is up, as on the Pixel. Source: [out of range](../research/2026-11-05-out-of-range.md).
- 2026-11-05: new gallery states: `call-window-connection-lost` (#101, `f7a35a6`) and `incoming-call-banner-identifying` (#103, `f6ecaae`). The latter is captured inside a 2 s window and can silently show "Withheld number" on a slow runner (#115).

## Real-Phone Checklist

- 2026-11-05 (#17, `417411e`; extended by #107, `7032137`): step 2.7 is scripted. Under Do Not Disturb, the user declines one Call on the Desktop and answers a second on the Pixel; the runner fails the step if the second looks handled on the Desktop, and `Ringing` must stay false throughout. Under Do Not Disturb the runner reads "handled on the Desktop" from the Banner closing while the Call still rings. The user's next sitting is the first real run.
- 2026-11-05 (#19, `6bf11cb`): six out-of-range checks await the user (walk out and back, suspend, a Call to the Test Number out of range, the real `Connect` errors, A2DP and paging cost, the link supervision timeout). Agents must not run them. They are listed in [out of range](../research/2026-11-05-out-of-range.md); the paging cost also gates #109.
- 2026-11-05 (#101, `f7a35a6`): no item added, because nothing safe makes `ConnectProfile` fail on the real Pixel.
