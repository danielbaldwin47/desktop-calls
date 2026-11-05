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

- **7.1 and the Test Number.** The runner reads the Test Number's fingerprint before you dial. The number is never kept or printed. If the placed Call's number doesn't match, the runner hangs it up through the app's `HangUp` and fails the step. With no Test Number set up, it never asks you to dial. How numbers match, and what happens when `HangUp` does nothing, is in the second review below.
- **7.1's focus check** now needs the Call Window to go from unfocused to focused after the keybinding. The second review below tightened it.
- **The app's clock at both ends.** **[V]** (demo, `by the app's log` in each record, and every reconnect between 1 ms and 10 s):
  - 3.1: from `handoff-requested` to `audio-moved`.
  - 6.1: from `phone-present` to `connection-state-changed` (connected). The app now logs `phone-present` at info, as it already logged `phone-absent`. An Enter pressed late can't make the reconnect look instant, and a missing line reports "Couldn't measure".
  - 6.2: from `desktop-resumed` to connected. The demo's Phone now leaves on suspend and returns after resume, as the Pixel does. The second review below changed how the runner waits.
  - 6.3: from the app logging the Phone gone to connected.
- **`logged_since` keeps the timeline's cursor.** It polls with `Sitting::pause`, which reads skip and quit without moving where the next wait starts. So a departure or an audio change published while the log was read is still seen in order.
- **1.1** no longer claims to land with #22. `RUNS_WITH` in steps.rs says it runs together with 1.2, and the runner prints "runs together with 1.2, which lands with #22". The closed-ticket rule fails 1.1 once 1.2 has its script and 1.1 doesn't. **[V]** (group 1 demo test)
- **The rule's own tests.** `tools/test_checklist_tickets.py` runs in `just check`. It covers `steps()`, `problems()`, `--closing`, `RUNS_WITH`, and the `just merge` gate with a stand-in GitHub. The parser now fails on any step block it can't read, instead of a 20-step floor. Offline, the gate prints "Skipped". When GitHub can't be reached, its Next line says to check the network and `gh auth status`. **[V]**
- **CI's token.** CI run 42043342003 on this PR passed `just check` with `GH_TOKEN: ${{ github.token }}` under `permissions: contents: read`. The check asked GitHub about every listed ticket, and any failure would have failed the step, so that token can read issues on this public repository. `issues: read` is not needed. **[V]** (CI log)

## After the second review (same day)

- **7.1 compares the whole national number.** A fingerprint is the national number, hashed, plus the country code when the number has one. `+44` or `0044` gives the country code (ITU-T E.164 lengths); `0` alone is a trunk prefix and is dropped. Two numbers match when their national numbers match, and their country codes too where both have one. So `07700 900999` matches `+44 7700 900999`. A slip in the first digit after the country code doesn't match, and neither do the same digits under another country code. The last-9-digits rule matched both. **[V]** (demo: `type-wrong-number`, `type-wrong-first-digit` and `type-wrong-country` each fail 7.1, with the Call ended, and neither number printed)
- **7.1 waits for the hung-up Call to go.** `HangUp` only queues the request, so its reply proves nothing. The runner waits up to 2 s for the Call to leave the app's state. If it's still there, it says "End this Call on the Pixel now" and fails the step. **[V]** (demo: each slip test checks that the Call ended, and that 7.2 never says "A Call is still going". With the `HangUp` call removed, all 3 slip tests fail.)
- **7.1's focus check counts only focus during the Call.** It needs the Call Window shown without the keyboard, then with it, while the Call is still listed. Two things made the earlier check too weak. The app publishes a Call before its Call Window (#107), so the mark's own state always counted as "unfocused". And the ended Call's window takes the keyboard by itself. **[V]** (demo: with the second keybinding replaced by Hang up, the earlier check passed and this one fails)
- **6.2 waits on the app's log, not its state.** The runner used to wait for "connected" first. That wait returned at once on the state from before the suspend, so a Pixel that reconnected more than 2 s after resume was reported as "couldn't measure". Now the runner reads the log for up to 60 s: first for the Phone leaving, which may come on either side of `desktop-resumed`, because real BlueZ may only tell the app after resume. Then it reads for the first connection after both lines. 6.1 and 6.3 use the same log wait. **[V]** (demo: `resume-slowly` has the Phone answer the Desktop's page 4 s after resume, and 6.2 passes with 4 to 10 s.) **[I]** when the Pixel's leaving is logged on real hardware.
- **6.3 no longer cites `audio-gateway-removed`.** The app logs it at debug, so it never reaches the journal the runner reads. **[V]** (source: `phone_link.rs`)
- **Tests added.** The `logged_since` cursor fix: `move-audio-to-phone-and-back` moves the audio while the runner is still reading the log, and 3.1 must still see it arrive. With a cursor-moving poll, that test fails. 3.1's test now requires "by the app's log". In Python, `just merge` must run the closed-ticket gate before it merges: with that call removed, the test fails. **[V]**

## Not verified

- **[I]** The real Test Number read: `$XDG_CONFIG_HOME` (or `~/.config`) `/desktop-calls/test-number`. Only the demo's Test Number has been matched.
- **[I]** The form in which the Pixel reports an outgoing Call's number. If it reports neither `+` nor `00` nor a national `0` form (for example `447700…`), the numbers won't match. The runner then hangs up a Call to the Test Number and fails 7.1. That is the safe direction, never a stranger rung.
