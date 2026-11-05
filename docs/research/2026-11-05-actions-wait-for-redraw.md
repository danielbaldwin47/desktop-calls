# Harness actions wait for the surface to redraw

Date: 2026-11-05. Ticket #53. Follows [the gallery "Call ended" note](2026-11-05-gallery-call-ended.md) and [the live theme note](2026-11-04-live-theme.md), §7.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** every harness action that clicks, types or takes a screenshot now waits by itself until the screen has finished redrawing. "Finished" means two things: the compositor used no CPU for 100 ms, and no thread of the compositor, the app or a test client is runnable. Both come from the kernel's per-thread counters in `/proc`, with no change to the app. A slowly redrawing test client proves it: a screenshot and a click issued straight after a state change fail without the wait and pass with it.

## Why a still-screen timer wasn't enough

- The app publishes a state change after it has told GTK what to show, but before GTK draws it (`decide` in `service.rs`: `render`, then `publish`). So `wait_for_app` returns while the old frame is still on screen. **[V]** (code)
- PR #55 made `screenshot_when` return the latest matching frame once no frame came for 100 ms (`SETTLED`). That still failed in CI on PR #71 (run 41987450440). `clicking_a_keypad_button_sends_its_tone` clicked "4" because the Call Window was still growing. **[V]** (issue comment). Under a CPU quota, every process in the cgroup can be frozen for up to a whole quota period (100 ms), the harness included. So 100 ms of wall time with no new frame proves nothing. **[I]**

## Signals measured

Probed under `just test-ci-like` with a live Call and the Call Window focused, sampling every 10 ms. **[V]**

- **The text cursor's blink fades over many frames.** With the tone entry focused, the screen changed every 40 to 90 ms for as long as the cursor blinked. GTK stops blinking 10 s after the last key. No timer-based or CPU-based "still" check could pass during that time. The harness now turns blinking off as a Desktop setting in each scenario's home: `GSETTINGS_BACKEND=keyfile` with `org/gnome/desktop/interface cursor-blink=false`, plus `gtk-4.0/settings.ini` where the schema is missing. Afterwards the only frames were the Call timer's, once a second.
- **The app is never idle.** The stall watch (`stall_watch.rs`) ticks every 20 ms while a Call or surface is up, so "the app used no CPU" never holds. Its threads are asleep (`S`) between ticks, though.
- **The compositor is idle unless something reaches the screen.** sway's runtime (`/proc/<pid>/task/*/schedstat`, field 1) did not move at all between frames. Every redraw ends in a client commit that sway handles and renders, so a sway that used no CPU has had nothing new to show.
- **Keypad growth in one step.** Here the Call Window grew from 288 to 451 layout pixels between two consecutive frames. CI's mid-growth frame was not reproduced locally.

## The rule (`crates/test-harness/src/redraw.rs`)

The screen has finished redrawing when, over 100 ms (six frames of the headless output's 60 Hz), sway's runtime did not change. In addition, at the end, no thread of sway, the app or any test client is in state `R` (running or waiting for a CPU) or `D`.

- A client starved mid-redraw is `R`, so it fails the check, and the same goes for one busy drawing.
- A frame sway had already scheduled renders within the 100 ms and moves its runtime. If sway itself is starved, it is `R`.
- The wait gives up after 10 s, saying what kept the screen busy. Nothing on screen today animates forever, so this never fired in the runs below. **[V]**
- Not covered: an app waiting on another process (a D-Bus reply, a debounce timer) before it starts redrawing looks asleep. A screenshot polled for a theme switch (`omarchy_theme`'s `time_until`) still loops until the colour arrives. **[I]**

`Pointer::click` waits before moving and again before pressing, because moving onto a button redraws its hover state. `screenshot_when` still records every frame, so brief states are caught. Once a frame matches, it waits for the redraw and returns a fresh capture of the redrawn screen if that still matches. Otherwise the state has already gone, and it returns the latest matching frame.

## Proof

- New: `a_screenshot_straight_after_a_state_change_shows_the_redrawn_surface` and `a_click_straight_after_a_state_change_acts_on_the_redrawn_surface` (`tests/screen.rs`). They use `test-client --slow-redraw`, which reports each key as `state <n>` at once, then keeps the CPU busy for 300 ms and draws 20 frames before the state's own colour.
- With the wait switched off: both failed under `just test-ci-like`. The click reported `showing state 0`, and the screenshot showed state 0's grey. With it on: both pass. **[V]**

## Gallery: one state, repeatedly

`just screenshot --only <state> --repeat N` renders one state in all four looks N times. It stops at the first failure and keeps the rest of the gallery. `--only call-window-muted --repeat 2` took about a minute here. **[V]**
