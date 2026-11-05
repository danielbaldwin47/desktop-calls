# "Identifying caller…" until the number may no longer come

Date: 2026-11-05. Ticket #103. Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** a Call that rings with no number shows "Identifying caller…" on the Incoming Call Banner for up to 2 s. If the Phone reports the number in that time, the Banner names the caller and never says "Withheld number". If it doesn't, the Banner says "Withheld number".

## What changed

- **Core:** an incoming Call added with no number starts `Timer::NumberLate(call)` for 2 s (`NUMBER_LATE_MS` in `crates/core/src/machine.rs`). Until it fires, `BannerView::awaiting_number` is true while the caller is still withheld. A number arriving through `CallerChanged` ends the wait at once, because the caller is no longer withheld. **[V]** (4 new tests in `crates/core/tests/incoming_call.rs`)
- **Messages:** `Label::IdentifyingCaller` is "Identifying caller…". The ellipsis matches the other in-progress labels ("Connecting…", "Calling…"). It sits under the heading "Incoming call", so it does not repeat the heading, as the ticket's example wording would have. **[V]**
- **Banner:** while `awaiting_number` is true, the caller line reads the label and the detail line is hidden. The Banner keeps its one-line height, so it does not resize when "Withheld number" replaces the label. **[V]** (gallery state `incoming-call-banner-identifying`, all 4 looks, checked by eye)
- **Appearance time:** the Banner appears exactly as it did before. The wait changes only the caller line's text, and starting the timer is just one more effect in the same step. **[I]** (nothing on the path to showing the Banner changed; the app-measured latency scenarios cover it on CI)

## Why 2 s

- The Simulated Phone's `late-line-identification` quirk sends the number 1 s after the Call appears, and the scenarios need room for CI load on top of that. **[V]** (`crates/test-harness/tests/late_line_identification.rs`, 4 of 4 pass)
- On a real Phone without `+CLCC`, the number comes with the `+CLIP` after the first `RING`, which should come well within 2 s (see the [late line identification note](2026-11-05-late-line-identification.md)). **[I]**
- The cost: a truly withheld Call reads "Identifying caller…" for 2 s before "Withheld number". **[V]**

## Follow-ups

- **The Indicator's tooltip still reads "Call from Withheld number"** while the number may be on its way (`messages::indicator_status`). This ticket covered the Banner only. **[V]** (code read)
- **The gallery state is captured within the 2 s window.** If CI is so slow that the capture after redraw lands after the window, the picture shows "Withheld number" under the caption for "Identifying caller…". The capture does not fail when that happens. **[I]**
