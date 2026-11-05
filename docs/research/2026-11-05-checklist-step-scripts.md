# Real-Phone Checklist: scripts for steps whose tickets closed

Date: 2026-11-05. Ticket #120, scripts part. Only the Simulated Phone was used (`--demo`). Nothing ran against the real Pixel, and the real WirePlumber was never restarted.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** seven steps waited on tickets that had already closed (#10, #11, #13, #14, #19, #20, #21): 3.1, 5.1, 5.2, 6.1, 6.2, 6.3 and 7.1. Each now has a script, and each passes in `--demo`. A new check keeps any step from waiting on a closed ticket again. `just check` runs it against GitHub, and `just merge` runs it with the tickets the PR closes.

## The scripts

- **3.1, Handoff twice.** Each Handoff is timed from the app's `handoff-requested` log line to `AudioLocation` changing. The user confirms the earpiece by ear each time. Before asking for the way back, the runner waits for the Call Window to offer it, because a Handoff to the Phone bounces hands-free (ADR-0005). In the demo, Move audio here sent during the bounce never moved the audio. **[V]** (demo)
- **5.1 and 5.2, earbuds.** These follow #13's expectations, posted on #29. 5.1 records `audio_location` as `desktop` or `phone`. When the Phone kept the audio, the runner asks whether the Call Window said "Your phone kept the audio", then has the user try Move audio here once more (`moved_on_second_try`). 5.2 checks the audio stays with the Phone for 3 s after answering on the earbuds. In the demo, the Simulated Phone refuses Move audio here while the earbuds are active, as AOSP does, and the step records that refusal as told plainly. **[V]** (demo); **[I]** what the Pixel does.
- **6.1, out of range.** Waits up to 180 s for the Desktop to notice the Phone has gone, which covers the link supervision timeout and the walk. It then asks whether the Desktop stayed quiet, and times the reconnect from Enter, against a 10 s target. **[V]** (demo)
- **6.2, suspend.** Times the reconnect from the app's `desktop-resumed` line. A Phone that stays connected through the suspend counts as 0 ms. The log line can arrive after Enter, so the runner waits up to 2 s for it; a demo run failed once before that wait was added. **[V]** (demo)
- **6.3, WirePlumber.** The user restarts it in another terminal. The runner times the reconnect from the Phone leaving to its return. **[V]** (demo, `RestartTelephony` with a 500 ms gap)
- **7.1, keyboard only.** The keybinding opens the Dialer, the user types the Test Number and presses Enter, the keybinding focuses the Call Window, and Ctrl+H hangs up. The runner checks the Dialer and then the Call Window each had keyboard focus, and asks whether the mouse was needed. This step needs the Test Number, not a helper. **[V]** (demo)

## The closed-ticket check

- `tools/checklist_tickets.py` reads every `Step { … }` in `steps.rs`. A step without a script must name at least one ticket, and only open ones. A step with a script must name none. **[V]** (offline run with the real open set {16, 22}: origin/main showed 16 problems, this branch none)
- It fails when it can't reach GitHub, so a skipped check never passes silently. `DESKTOP_CALLS_OFFLINE=1` skips it on purpose. CI's `just check` step gets `GH_TOKEN` from `github.token`. That is a one-line ci.yml change; #118 owns ci.yml. **[I]** that the token reads issues: the repository was mid-migration, and its API returned 404, so the live check never ran.
- `just merge` runs the same check at the PR head, with the PR's `closingIssuesReferences`. So the PR that closes a ticket can't merge while a step still names that ticket. **[I]** (not run: no PR could be merged during the migration)
