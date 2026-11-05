# A Contact's name and Do Not Disturb

Date: 2026-11-05. Ticket #85. Simulated Phone and the notification stand-in only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the bypass the #12 note suspected is real, but not for a Contact named just "Pin". Omarchy 4.5 lets a notification through Do Not Disturb when it finds a "verification code": a keyword and, near it, 4 to 8 digits (or 5 to 8 uppercase letters and digits). Our own wording has no keyword and no such digits, so the caller's text has to supply both. A Contact named "Gate code 4821" or "PIN A9F3K", or a custom number label such as "verify 4821", did it. The app now puts a zero-width non-joiner (U+200C) after the first letter of every keyword in the notification's summary and body, so Omarchy finds no keyword. The text looks the same, pixel for pixel.

## Omarchy's rule

Read from `/usr/share/omarchy/shell/plugins/notifications/` (omarchy 4.5.0-2), read-only.

- `Store.js` `snapshot` runs `Detect.scan(summary, body)` on the raw summary and the raw body: entities are not decoded and nothing is stripped. Only a browser's body has its origin anchor lifted off first. **[V]**
- `Detect.js` `codes`: each whole-word match of `CODE_WORDS` (`code|otp|one[\s-]?time|verification|verify|passcode|pin|2fa|two[\s-]factor|security key|confirm(?:ation)?`, ignoring case) opens a window from 24 code units before it to 72 after. A capitalised "Code" after a capitalised word ("Claude Code") opens none. In the windows, `CODE_SHAPE` finds `482913`, `482 913`, `482-913` or `G-482913` with 4 to 8 digits, not a year. If none is found, `CODE_ALNUM` finds 5 to 8 uppercase letters and digits with at least one of each. **[V]**
- `Delivery.js` `quietReason`: under Do Not Disturb a notification shows if its app name is `omarchy-action`, if it is `notify-send` at critical urgency, or if a code was found. Everything else, including any other app at critical urgency, is silenced. **[V]**
- The regular expressions have no `u` flag, so `\w`, `\b` and case folding are ASCII only. Any non-ASCII character ends a word. **[V]**

## Findings

1. **"Pin" alone doesn't bypass.** Omarchy's own scripts, run under Node on our texts: "Missed call from Pin (mobile)" / "08:30" is silenced. The time's colon keeps "08:30" from being a code. **[V]** (scratch run of the real `Detect.js` and `Delivery.js`)
2. **A name or label with a keyword and digits does bypass.** "Missed call from Gate code 4821 (mobile)", "… Pin 4821 …", "… PIN A9F3K …", "… one-time 482913", "2 missed calls from Pin 4821" all showed through Do Not Disturb and offered a "copy code" button. "1234 missed calls from Pin" would too. **[V]** (same run; and end to end, see the scenario below, which failed before the fix with `verification_codes: ["4821"], silenced: false`)
3. **The fix:** a zero-width non-joiner after a keyword's first letter (`c\u{200c}ode`) makes the rest a different word, so `CODE_WORDS` never matches and no window opens. The app applies this to the whole summary and body, so it covers the name, a custom number label, and any keyword that might span our text and the caller's. Our own wording contains no keyword. Every fixed text in the core test is silenced by Omarchy's real scripts. **[V]** (`crates/core/tests/messages.rs`; scratch run)
4. **Why U+200C and not the word joiner (U+2060).** In Omarchy's card font (Liberation Sans bold, plain text, as `Toast.qml` draws the summary), the word joiner comes from a fallback font and makes each line 4 px taller (17 to 21 px). The zero-width non-joiner keeps the line's size and renders pixel for pixel the same as the text without it, including where a narrow card wraps it. **[V]** (`qml6` offscreen render of both, compared with `magick compare -metric AE`: 0 differing pixels)
5. **What a reader might still notice:** copying the summary copies the invisible character, and searching notification history for "code" misses that card. **[I]**

## The notification stand-in

- `crates/test-harness/src/omarchy_quiet.rs` ports `Detect.js` `codes` and `Delivery.js` `quietReason`. Each recorded `Notification` now carries `verification_codes` and `silenced`. `Harness::set_do_not_disturb` also tells the stand-in, which decides each notification as it arrives. **[V]**
- The port agrees with Omarchy's real scripts on 19 texts covering every shape, the product-name rule, years, too many digits, distance and whole-word matching. Those verdicts were produced by running Omarchy's scripts, and are the table in `crates/test-harness/tests/stand_ins.rs`. **[V]**
- The port treats every sender as not a browser, so it doesn't lift a browser's origin anchor off the body. **[V]** (by design)

## Scenario

`a_missed_call_from_a_contact_named_like_a_verification_code_is_silenced_under_do_not_disturb` (`crates/test-harness/tests/missed_calls.rs`): with Do Not Disturb on, a Missed Call from Alice, then two from the Contact "Gate code 4821". All three notifications read as before ("Missed call from Gate code 4821 (mobile)", "2 missed calls from Gate code 4821"), are silenced and offer no code. **[V]**

## Follow-ups

- If Omarchy adds a keyword, the core's `KEYWORDS` list must follow. The stand-in's port would catch it only once ported too. **[I]**
- The Do Not Disturb state file the harness writes is still Omarchy 4.0.6's `notifications.json`. Omarchy 4.5 moved it to `notifications/quiet.json` (see the 4.5 recheck). Nothing reads it yet. **[V]** (by reading `stand_ins.rs`)
