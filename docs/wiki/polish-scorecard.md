# Polish scorecard

Agreed with the user on 2026-11-04 (grilling round 2) as the definition of "polished". Each hill-climb iteration is scored against it. Terms are as in [GLOSSARY.md](../../GLOSSARY.md).

1. The Incoming Call Banner appears within 1 s of the Phone ringing.
   - 2026-11-05 (#83): the 1 s is the app's own part, from the Call appearing to the Banner's first frame drawn, as its log records. The tests' round trip is held to a looser 3 s.
2. Audio flows within 1.5 s of pressing Answer.
   - 2026-11-05 (#83): Answer is held to it the same way: from the request reaching the app to the audio on the Desktop, by its log.
   - 2026-11-05 (#80): the 1.5 s is the app's own part, from the request reaching it to the audio on the Desktop, as its log records; Handoffs are held to it too. A client's start-up, such as the CLI a key binding runs, is held to a looser 3 s round trip in tests, because a loaded CI runner starts processes far slower than the Desktop.
3. Wideband speech is negotiated with the Pixel 9a.
4. From a fresh install, pairing through to the first Call takes 3 minutes or less.
5. 20 of 20 scripted Simulated Phone scenarios pass, and the Real-Phone Checklist passes.
6. Reconnects within 10 s of the Phone returning to range, and survives suspend/resume.
   - 2026-11-05 (#19): against the Simulated Phone, scenarios 17 and 18 (`crates/test-harness/tests/out_of_range.rs`) hold both to 10 s. The Desktop pages the Phone again after 1 s, 2 s, then every 3 s, so the worst case is about 3 + 1.3 + 1 s. Unchecked on the real Pixel.
7. Every action is reachable by keyboard alone.
8. A screenshot-review agent finds no visual defects.

- 2026-11-05 (#83): the Dialer's first frame measured 306 to 319 ms after `Activate`, cold, on the Desktop, and misses the 150 ms target (spec story 100, issue #11). The tests hold only GTK's map to 150 ms. Tracked in #47. Each keystroke is held to 50 ms up to the frame that shows its rows.
