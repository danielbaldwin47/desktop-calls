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
- It fails when it can't reach GitHub, so a skipped check never passes silently. `DESKTOP_CALLS_OFFLINE=1` skips it on purpose. CI's `just check` step gets `GH_TOKEN` from `github.token`. That is a one-line ci.yml change; #118 owns ci.yml. Confirmed after the migration (see below).
- `just merge` runs the same check at the PR head, with the PR's `closingIssuesReferences`. So the PR that closes a ticket can't merge while a step still names that ticket. **[I]** (not run: no PR could be merged during the migration)

## After cold review (same day)

- **7.1 and the Test Number.** The runner reads the Test Number's fingerprint before you dial: the last 9 digits, so `07700…` and `+44 7700…` match. The number is never kept or printed. If the placed Call's number doesn't match, the runner hangs it up at once through the app's `HangUp` and fails the step. With no Test Number set up, it never asks you to dial. **[V]** (demo: `step_7_1_hangs_up_at_once_a_call_to_a_number_that_isnt_the_test_number` types a number one digit off.) **[I]** that the Pixel reports an outgoing Call's number in a form whose last 9 digits match.
- **7.1's focus check** now needs the Call Window to go from unfocused to focused after the keybinding. Focus it had all along no longer passes. **[V]** (demo)
- **The app's clock at both ends.** **[V]** (demo, `by the app's log` in each record, and every reconnect between 1 ms and 10 s):
  - 3.1: from `handoff-requested` to `audio-moved`.
  - 6.1: from `phone-present` to `connection-state-changed` (connected). The app now logs `phone-present` at info, as it already logged `phone-absent`. An Enter pressed late can't make the reconnect look instant, and a missing line reports "Couldn't measure".
  - 6.2: from `desktop-resumed` to connected. The demo's Phone now leaves on suspend and returns after resume, as the Pixel does. A Phone that never left reports "Couldn't measure" instead of 0 ms.
  - 6.3: from the app logging the Phone gone to connected.
- **`logged_since` keeps the timeline's cursor.** It polls with `Sitting::pause`, which reads skip and quit without moving where the next wait starts. So a departure or an audio change published while the log was read is still seen in order.
- **1.1** no longer claims to land with #22. `RUNS_WITH` in steps.rs says it runs together with 1.2, and the runner prints "runs together with 1.2, which lands with #22". The closed-ticket rule fails 1.1 once 1.2 has its script and 1.1 doesn't. **[V]** (group 1 demo test)
- **The rule's own tests.** `tools/test_checklist_tickets.py` runs in `just check`, 13 of 13 passing. It covers `steps()`, `problems()`, `--closing`, `RUNS_WITH`, and the `just merge` gate with a stand-in GitHub. The parser now fails on any step block it can't read, instead of a 20-step floor. Offline, the gate prints "Skipped". When GitHub can't be reached, its Next line says to check the network and `gh auth status`. **[V]**
- **CI's token.** CI run 42043342003 on this PR passed `just check` with `GH_TOKEN: ${{ github.token }}` under `permissions: contents: read`. The check asked GitHub about every listed ticket, and any failure would have failed the step, so that token can read issues on this public repository. `issues: read` is not needed. **[V]** (CI log)
