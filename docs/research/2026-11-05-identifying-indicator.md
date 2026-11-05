# The Indicator agrees with the Banner while the caller is identified

Date: 2026-11-05. Ticket #115, the follow-ups from [2026-11-05-identifying-caller.md](2026-11-05-identifying-caller.md). Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** who a ringing Call is from is now worked out once, in the view model, as `RingingCaller` (`Identifying`, or `Known` with the caller and Contact). The Banner and the Indicator both read that one value, so they can't disagree. While the caller is identified, the Indicator's tooltip and menu status read "Identifying caller…", the Banner's own caller line. The gallery keeps an `incoming-call-banner-identifying` capture only if the app still said "Identifying caller…" when the frame was judged; otherwise the run fails.

## What changed

- **View model:** `BannerView`'s `caller`, `contact` and `awaiting_number` became one field, `caller: RingingCaller`, set by `State::ringing_caller` (`crates/core/src/machine/view.rs`). `IndicatorStatus::Ringing` holds the Banner's `RingingCaller`. Before, the Indicator copied the Banner's caller and Contact but not `awaiting_number`, which is how the two disagreed. **[V]** (`crates/core/tests/indicator.rs`, red before the change)
- **Words:** while identifying, the Indicator's status is `Label::IdentifyingCaller` on its own, not "Call from Identifying caller…", which isn't English. The ringing icon already says a Call rings. Once the wait is over it reads "Call from Withheld number", as before. **[V]**
- **Scenario:** `while_the_caller_is_identified_the_indicator_says_what_the_banner_says` (`crates/test-harness/tests/late_line_identification.rs`) waits for the Banner's caller line to read "Identifying caller…", then for the tray stand-in to read the same line as the tooltip. On the old code it failed, with the tooltip reading "Call from Withheld number". **[V]** (5 of 5 scenarios in the file pass)

## The gallery state

- **Holding the timer isn't possible within the rules.** The 2 s wait is the app's own timer, and pausing it from the control interface would be a test-only code path (`CODING_STANDARDS.md`). The Simulated Phone can't extend it. **[V]** by reading the code.
- **Checking after the capture wasn't enough.** Checking the app's state after `screenshot_when` returned made the run fail correctly. But the capture regularly came too late: with the wait shortened to 1 s, one look of four failed on the first try. On CI that would have been a flaky gallery. **[V]**
- **What works:** each `State` in the gallery now has a `still` check, and every recorded frame is judged by `shown` and by `still` on the app's state read at that moment. A frame is judged after it is taken, and the app reports a change before the screen shows it. So a frame judged while the app still reports "Identifying caller…" shows that text. `screenshot_when` already falls back to the latest frame that passed when the redrawn screen no longer does. So it keeps the last frame taken while the caller was still being identified. If no frame passes, it panics and the gallery fails. **[V]**: 5 of 5 runs (20 captures) at 2 s; 3 of 3 runs (12 captures) with the wait shortened to 1 s; with it shortened to 0.4 s, the run failed with exit 101 once no frame was early enough. One capture was checked by eye.
- The other gallery states use `still: |_| true`, so they behave as before. `call-window-ended` (2 s) might want a real `still` check later. **[I]**

## Follow-ups

- If a frame fails, `screenshot_when` keeps the last frame it judged as `<name>.png` for diagnosis, and that frame may show "Withheld number". The run has failed by then, so no index shows it. **[V]** (`screenshots.rs`)
