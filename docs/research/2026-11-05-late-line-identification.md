# A caller's number reported after the Call

Date: 2026-11-05. Ticket #86. Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the app now follows `LineIdentification` changes on a Call. A Phone that reports the number after the Call appears shows the right Contact on the Incoming Call Banner, in the Missed Call notification, and in the Dialer the notification opens.

## How PipeWire 1.6.9 reports a late number

- **Without `+CLCC`, a ringing Call appears with no number.** On `+CIEV: callsetup,1`, PipeWire registers the Call `incoming` with `line_identification` unset, which D-Bus shows as `""`. The `+CLIP` after the first `RING` then sets it, and PipeWire sends `PropertiesChanged` on `Call1` with `LineIdentification` alone, then `org.ofono.VoiceCall.PropertyChanged`. **[V]** (PipeWire 1.6.9 source: `spa/plugins/bluez5/backend-native.c`, the `CIND_CALLSETUP_INCOMING` and `+CLIP` branches; `telephony.c`, `telephony_call_notify_updated_props`)
- **`+CLIP` only counts while the Call is `incoming`.** A Call answered before its `+CLIP` arrives keeps no number. **[V]** (same source)
- **With `+CLCC`, a later list can also fill in a number** that was empty, in the same `PropertiesChanged` as a state change. **[V]** (same source, the `+CLCC:` branch)
- **A withheld caller's `+CLIP` (`"",129`) fails the parse,** so that Call never gets a number and stays withheld. **[V]** (same source; see the [Missed Calls note](2026-11-05-missed-calls.md), finding 3)

## What changed

- **Core:** a new input, `CallerChanged { call, caller }`, replaces the caller of a Call the Desktop still has, and of its entry among the recent Calls the Dialer selects from. The Banner, Call Window and Missed Call all read the caller from the Call, so they follow. A report for a Call already over changes nothing. **[V]** (`crates/core/src/machine/calls.rs`; 3 new tests in `crates/core/tests/missed_calls.rs`)
- **Phone link:** a `LineIdentification` change on a followed Call is reported before any state change in the same signal, so a Call that ends in that signal ends with the right caller. The log event is `caller-changed`, with the number at debug only. **[V]** (`crates/desktop-calls/src/phone_link.rs`)
- **Simulated Phone:** the `late-line-identification` quirk rings with an empty `LineIdentification` and sends the number 1 s later, only while the Call still rings, as PipeWire does. **[V]** (`crates/simulated-phone/src/telephony/phone.rs`, `identify_caller`)
- **Scenarios:** `crates/test-harness/tests/late_line_identification.rs`, 2 of 2 pass, 3 runs in a row. Both failed before the fix, with the Banner reading "Withheld number". **[V]**

## Follow-ups

- **Until the number arrives, the Banner says "Withheld number".** On a real Phone without `+CLCC` the gap should be one `RING` at most, so it should only flicker. Whether the Pixel 9a ever reports a number late was not checked. **[I]**
- **A Call that vanishes and comes back is matched by its number.** If the number arrived late and the returning Call is reported without it again, the app treats it as a new Call. **[I]**
