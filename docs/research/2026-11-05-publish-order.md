# The control interface publishes a Call's state before its surfaces

Date: 2026-11-05. Ticket #107. Follows the catch in [2026-11-05-do-not-disturb-silent.md](2026-11-05-do-not-disturb-silent.md). Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** rendering the surfaces and the Indicator now takes a `Published` proof that only `Publisher::publish` makes, so nothing that follows from a state can show before the control interface has signalled that state. Checklist step 2.7 now asks for a second Call answered on the Pixel, and its runner tells it apart from Decline on the Desktop.

## Why the order could go wrong

- `decide` in `service.rs` rendered the surfaces, then awaited the Indicator's D-Bus signals, then published the state. Hiding a GTK window unmaps it at once, and the unmap queues a `Surfaces` report for its own task. If anything awaited between render and publish yielded, that task signalled `Surfaces` first. **[V]** by reading the code; **[V]** that it shows: with a 30 ms wait put before the publish, all 4 `publish_order` scenarios failed ("the banner named Call 1 while Calls listed it as None").
- With no wait added, the 4 scenarios passed on the old order on this Desktop: the Indicator's signals usually go out without yielding, so the race needs a busy bus or a slow write to show. **[V]** (one run each way). **[I]** that it happens on a loaded CI runner or a real Desktop.

## The fix

- `Publisher::publish` returns `Published` and logs a signal it couldn't send itself, so the surfaces still show when the bus fails. `Surfaces::render` and `Indicator::render` each take `&Published`. Rendering before publishing no longer compiles. **[V]** (`just check`)
- `prepare_first_frames` still runs before the publish, so no first-frame work starts while the publish awaits. **[V]** by reading `first_frames.rs`.
- With a 30 ms wait put between publish and render, all 4 scenarios still passed: a surface can't overtake a state that is already signalled. **[V]**
- The cost to the Banner: rendering now waits for the state's signals to be written, a socket write per changed property. **[I]** that this is well under a millisecond; the Banner-latency scenarios passed unchanged.

## How the test sees the order

- The harness now records `Calls` and `Surfaces` after each `PropertiesChanged`, in the order the app sent them (`Harness::published`). `surfaces_seen` is derived from it. `published` waits until the recorded signals match the state the app publishes now, so a check never runs on a short record. **[V]**
- `crates/test-harness/tests/publish_order.rs` checks two things, in four scenarios: a surface starts naming a Call only once `Calls` lists it (the Banner, as ringing), and the Banner closes only once `Calls` says why. The scenarios are: answered on the Phone (with and without Do Not Disturb), declined on the Phone, and the caller hanging up. **[V]**

## Step 2.7

- 2.7 now has two Calls under Do Not Disturb: the user declines the first on the Desktop, as before, and answers the second on the Pixel. The runner fails the step if the second looks handled on the Desktop. `Ringing` must stay false through both Calls. The one question is unchanged. **[V]** (`crates/automation/tests/real_phone_checklist.rs`, demo)
- On the real Pixel the user's next sitting will be the first run. **[I]**

## Follow-ups

- `Error` and `MissedCall` signals are still sent while effects run, before the state. No ticket asked for them to follow the state, so they were left as they were. **[I]** that no client depends on their order.
