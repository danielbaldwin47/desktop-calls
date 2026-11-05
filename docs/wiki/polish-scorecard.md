# Polish scorecard

Agreed with the user on 2026-11-04 (grilling round 2) as the definition of "polished". Each hill-climb iteration is scored against it. Terms are as in [GLOSSARY.md](../../GLOSSARY.md).

1. The Incoming Call Banner appears within 1 s of the Phone ringing.
2. Audio flows within 1.5 s of pressing Answer.
   - 2026-11-05 (#80): the 1.5 s is the app's own part, from the request reaching it to the audio on the Desktop, as its log records; Handoffs are held to it too. A client's start-up, such as the CLI a key binding runs, is held to a looser 3 s round trip in tests, because a loaded CI runner starts processes far slower than the Desktop.
3. Wideband speech is negotiated with the Pixel 9a.
4. From a fresh install, pairing through to the first Call takes 3 minutes or less.
5. 20 of 20 scripted Simulated Phone scenarios pass, and the Real-Phone Checklist passes.
6. Reconnects within 10 s of the Phone returning to range, and survives suspend/resume.
7. Every action is reachable by keyboard alone.
8. A screenshot-review agent finds no visual defects.
