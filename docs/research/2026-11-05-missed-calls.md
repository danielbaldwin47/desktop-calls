# Missed Calls and withheld numbers

Date: 2026-11-05. Ticket #12, with its comments. Simulated Phone only: #70 held the real Phone in this batch.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** a Call that rings out is notified as missed only once the Phone's Call History, refreshed as the Call ends, lists a new missed Call from that caller. A Call declined on either device is never notified. When the refresh can't run (Contacts denied, a PBAP failure), the Call is notified as missed. One notification per caller counts repeats in place. Clicking it opens the Dialer on that Call; its Call back button dials. A withheld number shows as "Withheld number", is notified, and can't be called back. The Phone's other line is left out, because PipeWire never reports it.

## How it works

- **Classification lives in the core.** Each `RefreshCallHistory` effect is numbered. A Call that rang out (incoming, never connected, not declined or hung up on the Desktop) waits for the outcome of the refresh asked when it ended, either `CallHistoryRefreshed` or the new `CallHistoryRefreshFailed`. It is missed if the refresh failed, or if the refreshed Call History has a Missed entry from the same caller that the snapshot taken when the Call ended didn't have. A Contacts sync that lands in between doesn't decide. **[V]** (`crates/core/src/missed.rs`, 18 tests in `crates/core/tests/missed_calls.rs`)
- **Notification.** `org.freedesktop.Notifications.Notify` with app name "Desktop Calls", icon `call-missed-symbolic`, category `call.unanswered`, `desktop-entry`, normal urgency and no timeout. Actions: `default` ("Show in Dialer"), `call-back` ("Call back", only when there is a number), `dismiss` ("Dismiss"). A repeat from the same caller reuses the notification's server id through `replaces_id`. Clicking the notification, Call back, Dismiss, or closing it by hand (reason 2) ends the group, so the next Missed Call from that caller starts a new notification. Expiry keeps counting. **[V]** (`crates/desktop-calls/src/notifications.rs`; `crates/test-harness/tests/missed_calls.rs`, 8 of 8 pass)
- **Wording.** "Missed call from Alice (mobile)", "Missed call from +44 7700 900123", "Missed call from a withheld number", "3 missed calls from Alice", "2 missed calls from withheld numbers". The body is the time ("08:30", "Yesterday 11:00"), "Latest 08:30" for repeats, and " · can't be called back" when withheld. No word of Omarchy 4.5's verification-code list appears, so the notification never bypasses Do Not Disturb. **[V]** (`crates/core/tests/messages.rs`)
- **`MissedCall` signal** on `io.github.danielbaldwin47.DesktopCalls1`: `(call_id u, number s, display_name s, count u, rang_at_ms t)`. The number is empty when withheld. It is sent after `Notify` returns, whether or not it succeeded, so a listener that acts on the notification finds the app already knows its id. **[V]** (`control.rs`; harness tests)
- **Withheld numbers.** An empty `LineIdentification`, or `"withheld"` in any case, is a withheld caller. The Simulated Phone's new `RingWithCallerId` reads a raw `+CLIP` caller ID as PipeWire does, so `"",129` is tested end to end. **[V]** (scenario 9)

## Findings

1. **PipeWire 1.6.9 never sets `IncomingLine`.** The Telephony API declares the property, but the HFP backend never assigns `incoming_line`. Showing the Phone's other line is therefore left out. **[V]** (PipeWire 1.6.9 source, `spa/plugins/bluez5/backend-native.c` and `telephony.c`)
2. **`IncomingLine` is not a caller fallback.** Spec #1 (line 241) says withheld caller ID falls back to `IncomingLine`. In oFono, whose API PipeWire's mirrors, `IncomingLine` is the line the Call came in on: the Phone's own number on a dual-line Phone, not the caller's. Showing it as the caller would put the user's own number on the Banner. The app doesn't use it. This deviates from the spec. **[V]** (oFono `doc/voicecall-api.txt`; PipeWire source as above)
3. **`"",129` arrives as an empty number.** PipeWire parses `+CLIP` with `"%16[^\"]",%u`, which matches nothing between empty quotes, so the number stays empty. **[V]** (PipeWire source; Simulated Phone `number_from_caller_id`)
4. **The refresh must count as failed when the Phone serves no Call History.** With `cch` gone, the refresh fell back to empty `mch`, `ich` and `och` folders and "succeeded" with nothing new. A Phone serving no Call History at all now reports `CallHistoryRefreshFailed`. **[V]** (harness test)
5. **Whether the Pixel lists a Call it declined among its missed Calls is unknown.** Android logs a declined Call as rejected; whether its PBAP server puts rejected Calls in `mch` decides whether 2.4 passes on the real Phone. **[I]** — the Real-Phone Checklist now checks it.

## Real-Phone Checklist

- **2.4** now also checks that no Missed Call is notified after the Pixel declines. It waits for the Call History refresh in the app's log, then 2 s for a `MissedCall` signal. The machine can see this, so it asks no question.
- **2.5** (new script): the Call rings out, the runner sees the `MissedCall` signal and times it, asks whether the notification names the helper with the time, has the user click it, checks that the Dialer opens focused on the Missed Call, has the user press Enter, and checks that the Call placed goes to the number that rang. Numbers are compared through an in-memory fingerprint and never written.
- **2.6** (new script): a withheld Call, Banner reads "Withheld number", rings out, `MissedCall` with an empty number, the user confirms the notification, the Dialer row says it can't be called back, and Enter explains instead of dialling.
- Demo actions added: `call-withheld`, `ring-out` (the demo's Phone adds a Missed Call to its Call History, then the caller gives up), `click-notification`, `enter`, `phone-dials`. 4 of 4 runner tests pass; group 2 now asks 3 questions. Not run against the Pixel. **[V]** for the demo only.

## Follow-ups

- If the refresh fails and the Call History has other rows, clicking a withheld Missed Call leaves row 0 selected rather than the Missed Call. **[I]**
- Phones without `+CLCC` may report `LineIdentification` after the Call appears; the app reads it once. **[I]**
- Android may write its call log after the refresh pulls it, which would turn a real Missed Call into "declined on the Phone". 2.5 on the real Phone checks this. **[I]**
- A Contact named, say, "Pin" next to a 4-to-8 digit number in the summary could trip Omarchy's Do Not Disturb bypass. **[I]**
