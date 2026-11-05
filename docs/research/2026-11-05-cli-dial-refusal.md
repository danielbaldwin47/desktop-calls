# Dial from the command line waits for the Phone's verdict

Date: 2026-11-05. Ticket #65. Simulated Phone only; no real Phone was used.
Legend: **[V]** = verified on this machine. **[S]** = read in the zbus 6.2.1 source. **[I]** = inferred, not tested.

**Bottom line:** the control interface's `Dial` now replies once the Call state machine knows what became of the dial attempt: placed (the Phone reported the Call) or not placed, with the same failure the Call Window shows. `desktop-calls dial` and `tel:` links print that sentence and exit 1 on a refusal, and exit 0 about 10 ms after the Phone reports the Call. The deadline is the Desktop's existing 15 s limit for the Phone to report a dialled Call (`DIAL_TIMEOUT_MS`), so the CLI and the Call Window can never disagree.

## 1. How the outcome reaches the CLI

- The Call state machine emits `Effect::DialProgress` for every dial attempt: `NotPlaced(attempt, failure)` at once for a refusal the Desktop makes itself, or `Asked(attempt)` and later `Placed(attempt)` or `NotPlaced(attempt, failure)`. `NotPlaced` comes from the same code that sets the Call Window's notice, so the sentence is the same. **[V]** (`crates/core/tests/outgoing_call.rs`)
- Placed means the first outgoing Call the Phone reports after the dial, in any state. The Pixel reports it straight as `alerting` ([place-a-call note](2026-11-04-place-a-call.md) §1), and the Simulated Phone's `Alert` does the same. **[V]**
- Not placed covers: the Desktop's own refusals (not connected, a Call in progress, undialable), each refusal the Phone gives `Dial`, the 15 s timeout, hands-free dropping, and the Phone being unpaired while the dial waits. **[V]**
- The service's decide loop keeps each waiting `Dial` request by attempt number and answers it when its outcome arrives. Dials from the Dialer and the Missed Call notification's Call back have no request waiting, and the loop ignores their outcomes. **[V]**
- If the user hangs up in the Call Window before the Phone reports the Call, the CLI still waits. It exits 0 when the Call appears (and is hung up at once), or 1 if the Phone refuses. **[I]**: follows from the code, not covered by a scenario.

## 2. zbus holds the interface's lock while a method runs

- zbus 6.2.1 calls an interface method with a read lock on the interface held until the method's future finishes (`dispatch_call_to_iface`, `object_server/mod.rs:424`). **[S]**
- `Publisher::publish` took the write lock (`get_mut`). Once `Dial` waited for the Phone, publishing the dial's own state waited for `Dial`, and `Dial` waited for that state: the first run of the refusal scenario hung, and even the 15 s timeout couldn't end it. **[V]**
- The fix: the published state sits behind `Mutex`es inside the interface, and publishing takes only the read lock (`get`). zbus spawns a task for each method call by default (`spawn_tasks_for_methods`), so a waiting `Dial` doesn't hold up other calls. **[S]** for the spawning; **[V]** for the scenarios passing.
- Any later control method that awaits something must not rely on `get_mut` anywhere. **[I]**

## 3. Test harness changes

- `Harness::start_cli` and `Harness::start_dial` start a dial and return at once, so a scenario can act as the Phone (`Alert`) while the app waits. Scenarios that dial and then `Alert` use them; `harness.cli` and `harness.dial` would wait for the Phone to report the Call and fail after 15 s.
- `Harness::open_link` gave `gio open` pipes for its output. The `tel:` handler inherits them, so `gio open` didn't return until the handler exited, 15 s later. It now writes to files in the scenario directory (`gio-open.stdout`, `gio-open.stderr`). **[V]**
- Pre-existing race, now guarded: "Calling…" shows as soon as the Call state machine decides to dial, slightly before the Phone link sends `Dial`. An `Alert` sent in that gap fails with "the Desktop hasn't dialled anything". The gallery's `call-window-ringing` state hit it once. The gallery and the Call Window scenarios now wait until the Simulated Phone records `Dial` before they `Alert`. **[V]**

## 4. Compatibility

- `Dial` keeps its signature and error names. It only replies later, and with more refusals. The only clients that depended on the old timing were the test harness and the gallery, both updated. No `DesktopCalls2` interface was added. **[I]**: an outside script that called `Dial` and expected an immediate reply would now wait up to 15 s.

## 5. For the Real-Phone Checklist

- `desktop-calls dial` to the Test Number should exit 0 within about a second (the Pixel reported the Call 0.74 s after `Dial` was sent). **[I]**
- With the Phone in airplane mode, `desktop-calls dial` should print the same sentence as the Call Window and exit 1. **[I]**
