# A vanished Call is settled by the Phone's listing once hands-free is back

Date: 2026-11-05. Tickets #169 and #170, decision recorded in [ADR-0007](../adr/0007-a-vanished-call-is-settled-by-the-phones-calls-once-hands-free-is-back.md).
Legend: **[V]** = verified against the Simulated Phone. **[I]** = inferred, not tested. Nothing in this note was tried on the real Pixel.

**Bottom line:** #170 was the app ending a vanished Call when its 2 s grace ran out, 12 ms after hands-free was back and 28 ms before the Phone listed the Call again. Once hands-free is back, the app now waits up to 3 s for the Phone to list the Call. **[V]** #169 was not that race. The app sent the pending Hang up to the Call that came back. Then the Simulated Phone announced the hung-up Call again from a list it made before the restart. That is fixed in the Simulated Phone. **[V]**

## Diagnosis

- **#170** (`handoff_recovery`, CI log `20261105T233812Z-057f5f1`): the Call vanished at 176 ms and hands-free was back at 2164 ms. The grace ran out at 2176 ms with the Phone connected, so the core took it as a real end ("Call ended"). The Call was listed at 2204 ms and became Call 2. If the grace runs out while hands-free is still away, the core already treats the Call as lost track of and keeps its timer (#101), which is why only this narrow ordering failed. **[V]** from the log and the code.
- **#169** (`telephony_restart`, CI log `20261105T231736Z-0ca6501`, timeline `/tmp/desktop-calls-test-COknSE`): the app log has `call-returned: Call 1 came back` before `call-ended`, a line the issue's excerpt left out. The bus shows the `Call1.Hangup` on `ag91/call1`, `disconnected` and `InterfacesRemoved`. Then the Simulated Phone sent `InterfacesAdded` for `ag91/call1` with `State: active` at 1105 ms. Two Simulated Phone faults caused it:
  1. After a restart it kept the Calls in its object tree, so the app's `GetManagedObjects` on the new AudioGateway listed them before they were announced.
  2. It sent the Calls' `InterfacesAdded` 40 ms later from a list made before the restart, whatever had happened to them since.
  On a quiet machine the ghost Call 2 also appeared, about 30 ms after the scenario had passed and stopped watching; load moved it inside the scenario. **[V]** with a kept passing run's app log.

## Reproduced first, by order rather than load

A new Simulated Phone quirk, `calls-listed-late`, registers the Calls 2 s after the AudioGateway, after both a `DisconnectProfile` and a Telephony restart (`CALLS_LISTED_LATE`). Two scenarios use it, and both failed before their fix:

- `handoff_recovery::with_the_calls_listed_late_after_hands_free_returns_the_call_keeps_its_id_and_timer`: hands-free back at 0.6 s, grace over at 2.0 s, Calls listed at 2.6 s. Before: `the Call Window said Call ended`, then Call 2 (app log: `call-ended … it didn't come back` at 2949 ms, `call-added Call 2` at 3549 ms). **[V]**
- `telephony_restart::hang_up_while_the_service_is_away_ends_the_call_even_when_it_is_listed_late`: the scenario now also checks that the Call stays ended for 3 s. Before: `"the Call stays ended" stopped holding`, with Call 2 `active` 2 s after the Hang up. **[V]**

A third scenario covers the bound: `telephony_restart::a_call_that_ends_while_the_service_is_away_ends_here_soon_after_it_is_back`. The caller hangs up during a 2 s restart. Before, the Call stayed until the 10 s service grace ran out, which failed the 5 s wait. Now it goes 3 s after the service is back. **[V]**

## Changes

- Phone link: when the followed Phone's AudioGateway is adopted, every vanished Call gets a new grace of `LISTING_GRACE` (3 s) from that moment, replacing what was left of `GRACE` or `SERVICE_GRACE`. `GRACE` (2 s) and `SERVICE_GRACE` (10 s) are unchanged while hands-free is away. **[V]**
- Simulated Phone restart: the Calls leave the tree with the old service (`World::restarting`) and are registered again, with their state at that moment, only when they are announced. `CallsOnPhone` lists them meanwhile. A caller who hangs up during the restart ends the Call on the Phone, and the new service never registers it. **[V]**
- That real PipeWire, after a restart, doesn't list Calls in `GetManagedObjects` before announcing them is **[I]**: it registers each Call and emits its `InterfacesAdded` together.

## Checks

The touched files pass: `handoff_recovery` 3 of 3, `telephony_restart` 9 of 9, plus `handoff`, `lost_call`, `out_of_range`, `incoming_call`, `crash_restart`, `conformance`, `two_phones` and `call_waiting`. **[V]**

## Follow-up candidates

- The core's `LOST_CALL_BACK_MS` is still 2 s, for a Call the Phone link gave up on while hands-free was away. It could match `LISTING_GRACE` (3 s), so a Call listed late after a long reconnect doesn't lose its lost notice first. That change is to a core constant and its test, so it is left out here.
