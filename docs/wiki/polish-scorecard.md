# Polish scorecard

Agreed with the user on 2026-11-04 (grilling round 2) as the definition of "polished". Each hill-climb iteration is scored against it. Terms are as in [GLOSSARY.md](../../GLOSSARY.md).

1. The Incoming Call Banner appears within 1 s of the Phone ringing.
   - 2026-11-05 (#83): the 1 s is the app's own part, from the Call appearing to the Banner's first frame drawn, as its log records. The tests' round trip is held to a looser 3 s.
   - 2026-11-05 (#47, `3def480`): with the first-frame work done while idle, the first Banner drew in 63 to 64 ms under `just test-ci-like`, one scenario at a time (before: 122 to 140 ms, with main-loop stalls over 100 ms in 2 of 3 runs). See [first frames](../research/2026-11-05-first-frames.md).
   - 2026-11-05 (#103, `f6ecaae`): a Call with no number shows "Identifying caller…" for up to 2 s; the Banner appears no later for it (inferred, the latency scenarios pass unchanged). See [identifying caller](../research/2026-11-05-identifying-caller.md).
2. Audio flows within 1.5 s of pressing Answer.
   - 2026-11-05 (#83): Answer is held to it the same way: from the request reaching the app to the audio on the Desktop, by its log.
   - 2026-11-05 (#80): the 1.5 s is the app's own part, from the request reaching it to the audio on the Desktop, as its log records; Handoffs are held to it too. A client's start-up, such as the CLI a key binding runs, is held to a looser 3 s round trip in tests, because a loaded CI runner starts processes far slower than the Desktop.
3. Wideband speech is negotiated with the Pixel 9a.
4. From a fresh install, pairing through to the first Call takes 3 minutes or less.
5. 20 of 20 scripted Simulated Phone scenarios pass, and the Real-Phone Checklist passes.
6. Reconnects within 10 s of the Phone returning to range, and survives suspend/resume.
   - 2026-11-05 (#19): against the Simulated Phone, scenarios 17 and 18 (`crates/test-harness/tests/out_of_range.rs`) hold both to 10 s (`6bf11cb`). The Desktop pages the Phone again after 1 s, 2 s, then every 3 s, so the worst case is about 3 + 1.3 + 1 s. Unchecked on the real Pixel; the Real-Phone Checklist items are listed in [out of range](../research/2026-11-05-out-of-range.md). The 3 s paging interval's cost to the Desktop's other Bluetooth devices is unmeasured (#109, needs-info).
7. Every action is reachable by keyboard alone.
8. A screenshot-review agent finds no visual defects.

- 2026-11-14, audit against the Simulated Phone at `32c46f0` ([note](../research/2026-11-14-scorecard-audit.md)): items 1, 2 and 6 and the 50 ms keystroke target are covered by named tests; item 7 is covered surface by surface but not for "every" action (#25); item 8 has a gallery of 49 states in four looks but no recorded review verdict (#26, #262); items 3, 4 and 5's Checklist half, and the 150 ms first-frame target, wait for the user. Item 5's 20 scenarios each have a test, matched by name (read), but the tool that should count them does not (#27). Row-by-row proofs are in the note; this page does not repeat them.
- 2026-11-15 (#336, `5e56a23`; #341, `cf7d97f`): `just scorecard` now reports each item as PASS, FAIL, MANUAL or NEEDS-REVIEW with its evidence, so all eight items have automated evidence or a MANUAL status. **Supersedes** the 2026-11-14 line above on what the tool can show ("the tool that should count them does not"). Main's own scorecard job on `2846987` (run 43709458787): 0 of 8 pass. Items 1 to 6 are MANUAL, with their Simulated Phone halves passing (12 of 12, 11 of 11, 20 of 20 canonical and 26 of 26 quirk, and 7 of 7 scenarios; items 3 and 4 have no real measurement). Item 7 is FAIL (9 of 9 keyboard-only scenarios pass, 6 actions have no key path, #292). Item 8 is NEEDS-REVIEW (code changed since the screenshot review, whose verdict file is `docs/reviews/screenshot-review.md`). Only a whole Real-Phone Checklist run can pass an item. The 2026-11-14 gaps: item 7's completeness is now a suite (#25, #299) that names the 6 missing key paths; item 8 has a gallery of every state (#26, #311) and a verdict file, but its verdict is stale; the 20-scenario count is automated. Items 3's codec and 4's and 6's real timings remain the user's.
- 2026-11-05 (#83): the Dialer's first frame measured 306 to 319 ms after `Activate`, cold, on the Desktop, and misses the 150 ms target (spec story 100, issue #11). The tests hold only GTK's map to 150 ms. Tracked in #47. **Superseded 2026-11-05 by #47 below:** the tests now hold the drawn frame, not GTK's map. Each keystroke is held to 50 ms up to the frame that shows its rows.
- 2026-11-05 (#47, `3def480`): the app now does the first-frame work while idle, once the Phone is Connected. The Dialer's drawn first frame after `Activate`, warm, is held to:
  - 150 ms locally and under `just test-ci-like`, one scenario at a time. Measured 97 to 102 ms there.
  - 250 ms in CI, run alone. CI's runner paints in software and took 150.4 ms even alone. 250 ms still catches shaders compiling cold, which took 306 ms and more.
  - 150 ms on the user's Desktop, which is where the target is judged: Real-Phone Checklist step 7.4 reads the app-logged time.

  The cold first open, without that work, measured 306 to 319 ms on the Desktop and 186 to 320 ms under `just test-ci-like`. See [first frames](../research/2026-11-05-first-frames.md).
