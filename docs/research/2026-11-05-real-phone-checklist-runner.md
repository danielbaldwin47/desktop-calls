# The Real-Phone Checklist runner

Date: 2026-11-05. Ticket #24. Checklist content from spec #1 and tickets #28 to #31, including their comments.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** `just real-phone-checklist [GROUP]` walks the user through the checklist one step at a time, against the real Phone or, with `--demo`, against the Simulated Phone. It measures what the control interface and the app's log show (Banner latency, audio latency, codec, Audio Location, the hang-up path, keyboard focus) and asks only what a person must judge. Today 8 of 24 steps are runnable: 2.1 to 2.4, 2.8, 3.2, 3.3 and 7.2. The rest wait on feature tickets and are recorded as "Not yet runnable". Nothing here ran against the real Pixel; the user's first sitting will be the first real run.

## How it works

- **Observation.** The runner subscribes to `PropertiesChanged` on the control interface and keeps a timeline of every published state, timestamped when each signal arrived. Each step looks for the first state that satisfies a condition, so latencies are measured between signals, however slowly the runner itself reacts. **[V]** (`crates/automation/src/checklist/watch.rs`)
- **Banner latency** runs from the app's `call-added` log line (its Phone link seeing the Call) to the control interface listing the `banner` surface. When the log has no such line, it falls back to the Call appearing on the control interface, and the evidence says so. In demo runs, the log path measured 33 ms where the control-interface path measured 8 ms: the app publishes the Call about 25 ms after its Phone link sees it. **[V]** (demo run, group 7)
- **Audio latency** runs from Answer to `AudioLocation` becoming `desktop`. "Answer pressed" is the moment `Ringing` goes false while the Phone still lists the Call as ringing: the Call state machine stops ringing as soon as Answer or Decline is pressed, before the Phone confirms. **[V]** (`answer` in `crates/core/src/machine.rs`; demo runs)
- **Hang-up path.** `phone-command-fallback` is now logged at info, so the journal shows when the Phone needed `org.ofono.VoiceCall.Hangup`. Step 2.1 records `hang_up` as `org.ofono.VoiceCall` or `Call1`. The control interface can't tell a hang-up on the Desktop from one by the helper, so `Call1` means "no fallback was logged". **[V]** for the fallback (demo with the `call1-hangup-missing` quirk); **[I]** that the Pixel needs it.
- **Results.** One JSON file per sitting in `docs/real-phone-checklist/`, named by its UTC start time, rewritten after every step. A failed step also keeps the app's info, warning and error lines from its time, for the agent who files it. Debug lines are never read, because they carry phone numbers. `--demo` results go under `target/` and are marked `"mode": "demo"`; the scorecard ignores them. **[V]** (tests in `crates/automation/tests/real_phone_checklist.rs`)
- **Resuming.** A rerun of the same groups continues the latest file if it holds any of those steps and not all of them passed, and runs only the steps that didn't pass. `--fresh` starts a new file. **[V]** (test)
- **Scorecard.** Item 3 reads the codec from the latest real-Phone result (PASS when wideband). Item 5 shows that result's pass, fail and skipped counts, and stays FAIL until #27 counts the scenarios. **[V]** (test)

## Findings

1. **Escape needs another window to give focus to.** In a headless sway with no other window open, Escape on the focused Banner left `focused` true for more than 120 s: the compositor had nowhere to move keyboard focus. With a client window open, as on any real Desktop, focus returned at once. The demo therefore opens a test-client window as the user's own. **[V]** on headless sway; **[I]** that Hyprland behaves the same.
2. **The microphone volume the Pixel reports at Call start (#29, comment b) can't be observed yet.** It is neither on the control interface nor logged at info. **[V]** (`control.rs`, `phone_link.rs`)
3. **The Simulated Phone can't place a Call from the Phone's side**, so step 3.3 (keypad tones to a voicemail menu the user calls from the Pixel) can't be played in `--demo`; its demo hint is `skip`. **[V]** (`SimulatedPhone1` has no such method)
4. **"Answer pressed" relies on `Ringing`.** If #17 makes `Ringing` false under Do Not Disturb, step 2.7's script must detect Answer differently, for example by the Banner closing while the Call still rings. **[I]**

## Follow-ups for tickets

- #12: add the Missed Call question to step 2.4 and write the scripts for 2.5 and 2.6.
- Every feature ticket named in a step's "lands with" writes that step's script in `crates/automation/src/checklist/steps.rs`.
- #29 (b): expose or log the Pixel's microphone volume at Call start, so the runner can record it.
- The glossary has no term for the "helper", the person with a second phone who calls the user. The runner uses the word as the spec does.
