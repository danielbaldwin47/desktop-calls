# The Error and MissedCall signals follow the Call state they describe

Date: 2026-11-05. Ticket #116. Closes the follow-up in [2026-11-05-publish-order.md](2026-11-05-publish-order.md). Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** `Publisher::report_error` and `Publisher::missed_call` now take the `Published` proof that only `Publisher::publish` makes, so neither signal can be sent before the state it describes. The Missed Call's notification waits for the proof too.

## What was wrong

- `Shell::carry_out` awaited the `Error` signal while it carried out the step's effects, before `decide` published the step's state. Every `Error` therefore reached a client before the state change of its own step. **[V]**: with the fix removed, the new `publish_order` Error scenario failed with no delay added ("the Error \"phone-out-of-range\" came while ConnectionState was \"connected\"").
- A Missed Call's effect queued its notification, and the notifier task signalled `MissedCall` once the notification server had answered. That is a D-Bus round trip, so on this Desktop it usually lands after the publish, but nothing ordered them. **[V]** that it can come first: with the fix removed and a 300 ms wait put before the publish, the MissedCall scenario failed ("MissedCall told of Call 1 while Calls still listed it"). **[I]** that a loaded runner or a slow bus shows it without an added wait.

## The fix

- `carry_out` no longer sends either signal. It hands back `Announcement`s, and `decide` carries them out through `Shell::announce` after the publish, the surfaces and the Indicator, so the surfaces the user waits to see aren't delayed. **[V]** (`just check`; the scenarios below)
- `Publisher::report_error` and `Publisher::missed_call` take `&Published`; `Notifier::notify` takes it and queues it with the notice. Sending either signal before the publish no longer compiles. **[V]**
- The Missed Call's desktop notification waits for the publish as well, because the notice is queued only once the state is published. **[V]** by reading `service.rs`; no scenario asserts the notification's time against the state.
- `Published` is a proof that some state was published, as in #107, not that the state of this very step was. The one caller passes the proof `decide` just got. **[I]** that this is enough.

## How the test sees the order

- `Harness::published` now records each step from one subscription to every signal on the app's object path, so the bus's order is kept: `Calls`, `Surfaces` and `ConnectionState` after each `PropertiesChanged` that carries one, with every `Error` and `MissedCall` sent so far. It waits until the record has caught up with the state and with the other collectors' `Error` and `MissedCall` lists. **[V]**
- Two scenarios in `crates/test-harness/tests/publish_order.rs`:
  - A Call rings out with Contacts denied, so there is no Call History refresh to wait for and the Missed Call is told in the same step the Call ends. `MissedCall` must come once `Calls` no longer lists the Call, and `Calls` must have listed it before.
  - A dial is waiting for the Phone to report the Call, and the Phone goes out of range. `Error` `phone-out-of-range` must come once `ConnectionState` is `disconnected`.
- Both fail on the old order (the Missed Call one with the 300 ms wait above) and pass on the new, also with the wait. **[V]** (one run each)

## Follow-ups

- Other `Error` kinds (`answer-failed`, `hang-up-failed`, `mute-failed`) follow state no property shows, so no scenario can see their order. They go through the same `announce`. **[I]** that they are right.
