# Destination

Where the hill climb ends, and what is still unknown on the way. Named 2026-11-13 by the chief of staff from the user's words "get the v1 to completion, and when it's just my checklist items left we can pause". Terms are as in [GLOSSARY.md](../../GLOSSARY.md). This page is a dated record: when the destination changes, add a new dated entry and mark the old one superseded.

## Destination, 2026-11-13

v1 is done, and the work pauses, when all five hold:

1. Every item of the [polish-scorecard](polish-scorecard.md) passes on the Simulated Phone.
2. `main` is green on GitHub.
3. The Call Pill (spec #197) and Calls Off with Quiet Hours (spec #198) have shipped. The user lifted the feature freeze of 2026-11-05 for these two features and the UI pass below, and for nothing else ([session-log](session-log.md)).
4. The UI pass the user chooses has shipped.
5. Only the user's Real-Phone Checklist groups remain ([testing](testing.md)): the groups that need the Pixel and the user, which no agent may run.

Where each stood on 2026-11-13 at about 19:05 EST (checked on GitHub then; the full list is in the session-log entry):

- Item 2: the GitHub runs on `c3e05dd` (run 43528683478) and `40411a0` (run 43529718982), the two pushes to `main` that day which changed code, passed every job. Before them `main` had been red since `1139783` ([testing](testing.md)). Two docs-only pushes, `81dcfe0` just before them and `13bece2` after them, skipped the test jobs.
- Item 3: the Call Pill's tickets #200 and #201 and Calls Off's tickets #202 to #207, #209 and #215 were open, with the reconnect fixes #212, #213 and #216 to land first. None had a PR open.
- Items 1, 4 and 5: not checked on 2026-11-13. The scorecard's three-tier report ([chief-of-staff-practice](chief-of-staff-practice.md)) was not redone that day.

## Where the five stood, 2026-11-14 (checked against `main` at `32c46f0`; supersedes the 2026-11-13 list above where they differ)

Sources: the [scorecard audit](../research/2026-11-14-scorecard-audit.md), `gh run list --branch main` and `gh issue list`, read 2026-11-14. Nothing ran on the Pixel.

- Item 1 (scorecard on the Simulated Phone): **not met, and the tool cannot show it.** Of 12 audit rows, 6 are covered (Banner within 1 s, audio within 1.5 s, the 20 scenarios, reconnect, keyboard reach of each surface, the 50 ms keystroke), 4 are owner-only (wideband speech, fresh install in 3 minutes, the Real-Phone Checklist, the Dialer's 150 ms first frame) and 2 are gaps: item 8 (no screenshot-review agent's verdict for the current UI; #26 and #262 open) and item 7's completeness (no suite that lists every action and fails on one without a key path, #25; opening the Quiet Hours Window has no command or key). `just scorecard` returns FAIL for every item and its test pins `0 of 8 items pass`; counting the 20 scenarios and showing MANUAL for the Phone-only items is #27. #25 to #27 carry the `after-v1` label, so closing these gaps is not scheduled, and whether they gate v1 is the user's call.
- Item 2 (`main` green on GitHub): **not met.** After `52d001f`, 6 pushes passed, 8 failed and 13 were cancelled (cause not found); the run on `32c46f0` was in progress when checked. Each failed PR had shown "CI passed" from `just ci-local` before it merged ([testing](testing.md)).
- Item 3 (Call Pill and Calls Off): **met on the Simulated Phone.** Specs #197 and #198 closed 2026-11-14 with every ticket merged ([product-shape](product-shape.md)).
- Item 4 (the UI pass): **built.** Spec #221's ten tickets, #222 to #231, merged. The user's judgement of it waits for their sitting; the after-the-sitting tickets #232 to #237 and #262 are open.
- Item 5 (only the user's Checklist remains): **not yet**, because items 1 and 2 are open. The Checklist has no recorded result: its state folder did not exist on 2026-11-14.

## Fog of war, 2026-11-13

Unknown, and not to be planned as if known:

- **What the UI pass will contain.** The user asked for a UI pass after the fixes and before their checklist sitting ([user-preferences](user-preferences.md)). A Fable design review was in progress on 2026-11-13, at the user's request, to propose it. Until the user has chosen from it, the pass has no scope.
- **Whether a "calls only, keep media audio" mode follows v1.** Probe B of the Calls Off probe ([note](../research/2026-11-13-calls-off-mechanism-probe.md)) suggests it is feasible as a best effort: the Phone's hands-free profile can be dropped while A2DP stays up, but it must be dropped again after every reconnect (observed on the Desktop with the Pixel; the reconnect half from BlueZ and PipeWire source, not tested). The user agreed on 2026-11-13 that blocking is the only mode in v1 and that this mode is an after-v1 concept ([product-shape](product-shape.md)).
- **The Companion App.** The user wanted it "soon" after v1 on 2026-11-04 ([product-shape](product-shape.md)). No later decision on it is recorded in this wiki.

## Out of scope, 2026-11-13

- SMS, notification mirroring and file transfer: out of v1 since 2026-11-04 ([product-shape](product-shape.md)).
- The software factory, issue #140: after v1 ([chief-of-staff-practice](chief-of-staff-practice.md), "Merging").
