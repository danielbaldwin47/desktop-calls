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
- Item 2 (`main` green on GitHub): **not met.** After `52d001f`, 6 pushes passed, 8 failed and 12 were cancelled (cause not found); the run on `32c46f0` was in progress when first checked and had completed with failure when checked again later on 2026-11-14 (cause not read). Each failed PR had shown "CI passed" from `just ci-local` before it merged ([testing](testing.md)).
- Item 3 (Call Pill and Calls Off): **met on the Simulated Phone.** Specs #197 and #198 closed 2026-11-14 with every ticket merged ([product-shape](product-shape.md)).
- Item 4 (the UI pass): **built.** Spec #221's ten tickets, #222 to #231, merged. The user's judgement of it waits for their sitting; the after-the-sitting tickets #232 to #237 and #262 are open.
- Item 5 (only the user's Checklist remains): **not yet**, because items 1 and 2 are open. The Checklist has no recorded result: its state folder did not exist on 2026-11-14.

## Where v1 stands, 2026-11-15 (checked against `main` at `7be818a`; supersedes the 2026-11-14 list above where they differ)

**v1's agent-side work is complete as of `7be818a`.** That is the chief of staff's statement of 2026-11-15, relayed to the author, and the author checked what GitHub can show of it. What remains for v1 is the user's Real-Phone Checklist (#28 to #31, all open, `ready-for-human`) and the `needs-info` issues it settles: #112, #109 and #51 (open, `needs-info`). Sources: `gh issue list --state open`, `gh run list --branch main`, `gh pr list`, read 2026-11-15. Nothing ran on the Pixel.

- Item 2 (`main` green on GitHub): **met.** The runs on `a65d408` (43680620102) and `7be818a` (43681111286) ended in success. Causes of the red of 2026-11-14 and their fixes (#285, #287) are in the [session-log](session-log.md) and [testing](testing.md).
- Item 1 (scorecard on the Simulated Phone): **not shown by the tool; the gaps named on 2026-11-14 are partly closed.** The Quiet Hours Window now has a key path: the command `desktop-calls quiet-hours` and Alt+Q in the Dialer (#279, PR #281, `a0597b0`). #25 (the keyboard-only suite), #26 and #27 are open and in flight as after-v1 work (**superseded 2026-11-15 evening:** all three merged, see the next entry). #292, filed after `7be818a` and `needs-triage` (**now `ready-for-human`**), lists Indicator entries that still have no key path (scorecard item 7). Whether #25, #27 and #292 gate v1 is not recorded as the user's call: the 2026-11-14 entry left it to them, and the statement above treats them as not gating. Read it as the chief of staff's judgement until the user says so.
- Items 3 and 4: unchanged from 2026-11-14 (met on the Simulated Phone; the UI pass built, the user's judgement of it waiting for their sitting).
- Item 5 (only the user's Checklist remains): **by the chief of staff's statement, yes**; not re-checked whether the Checklist has any recorded result (`~/.local/state/desktop-calls/real-phone-checklist/` was absent on 2026-11-14).

## Where v1 stands, 2026-11-15 evening (checked against `main` at `2846987`; supersedes the entry above where they differ)

**v1's agent-side work is complete as of 2026-11-15** (the chief of staff's statement, relayed to the author). What remains is the user's sitting: the Real-Phone Checklist (#28 to #31, `ready-for-human`), the `needs-info` issues it settles (#112, #109, #51), and the user's judgement of the UI pass. Sources: `gh issue list --state open`, `gh run list --branch main`, main's own scorecard job (run 43709458787), read 2026-11-15 after 11:42Z. Nothing ran on the Pixel.

- Item 2 (`main` green on GitHub): **met, with two reds today.** Of the 37 pushes from `a65d408` to `2846987`, 35 ended in success; `d9b9d29` failed (the held-Call `lost_call` scenario, which is #303's, fixed by #334; and a handoff scenario, `the_transport_has_no_deactivate_and_the_handoff_to_the_phone_needs_none`, whose cause is diagnosed in a comment on #298: a 1147 ms main-loop stall at the Call Window's first frame, run 43686057519; read from that comment, not from the run's log) and `7788ef4` failed (the gallery state `call-window-moving-audio`, fixed by #354). `2846987` failed once in the gallery's part 1 and passed on a rerun with no cause found (the rerun was against the rule; [chief-of-staff-practice](chief-of-staff-practice.md)).
- Item 1 (scorecard on the Simulated Phone): **the tool now shows it, and it does not read as met.** #25, #26 and #27 merged (#299, #311, #336), so the keyboard-only suite, the gallery of every state and the scorecard's four statuses exist. Main's scorecard job on `2846987` printed "0 of 8 items pass (1 fail, 6 manual, 1 need review)": items 1 to 6 MANUAL (their Simulated Phone scenarios pass: 12 of 12, 11 of 11, 20 of 20 canonical and 26 of 26 quirk, 7 of 7; the real half waits for the Checklist), item 7 FAIL ("9 of 9 scenarios pass; 6 actions have no key path yet (#292)"), item 8 NEEDS-REVIEW (code changed since the screenshot review). So item 7 stays red until #292, which needs a design decision (which surface carries the keys), is decided; whether #292 gates v1 is the user's call, and the statement above treats it as not gating (the chief of staff's judgement, as on 2026-11-15 morning). Item 8 reads green only after a new screenshot review of the changed surfaces; no such review is scheduled in anything I read.
- Item 3 (Call Pill and Calls Off): unchanged, met on the Simulated Phone.
- Item 4 (the UI pass): **finished.** The after-the-sitting tickets #232, #233, #235, #236, #237 and the follow-ups #296, #300, #322 merged, and #355 fixed two of #323's jumps. Still open on it: #234 (the app-side fade question), #323 and #324 (design questions), #313 to #326 (the screenshot review's other findings, `ready-for-agent`). The user's judgement of how it looks and feels waits for the sitting.
- Item 5 (only the user's Checklist remains): **by the chief of staff's statement, yes.** Not re-checked whether the Checklist has any recorded result; it had none on 2026-11-14. The Checklist gained scripts for group 4 (#305) and a locale-aware step 7.1 (#297), so the sitting has more to run than on 2026-11-14 ([testing](testing.md)).
- Open for the user: #329, #301, concept reviews for #283, #140 and #292, the design questions from #323 and #324, the Checklist and the `needs-info` issues. The list with labels is in the [session-log](session-log.md).

## Fog of war, 2026-11-13

Unknown, and not to be planned as if known:

- **What the UI pass will contain.** (**Superseded 2026-11-15:** the pass was scoped by spec #221 and is finished; see the entry above.) The user asked for a UI pass after the fixes and before their checklist sitting ([user-preferences](user-preferences.md)). A Fable design review was in progress on 2026-11-13, at the user's request, to propose it. Until the user has chosen from it, the pass has no scope.
- **Whether a "calls only, keep media audio" mode follows v1.** Probe B of the Calls Off probe ([note](../research/2026-11-13-calls-off-mechanism-probe.md)) suggests it is feasible as a best effort: the Phone's hands-free profile can be dropped while A2DP stays up, but it must be dropped again after every reconnect (observed on the Desktop with the Pixel; the reconnect half from BlueZ and PipeWire source, not tested). The user agreed on 2026-11-13 that blocking is the only mode in v1 and that this mode is an after-v1 concept ([product-shape](product-shape.md)).
- **The Companion App.** The user wanted it "soon" after v1 on 2026-11-04 ([product-shape](product-shape.md)). No later decision on it is recorded in this wiki. On 2026-11-15 the user let the chief of staff choose after-v1 work ([user-preferences](user-preferences.md)); the Companion App is not among the issues sent, which are listed in the [session-log](session-log.md).

## Out of scope, 2026-11-13

- SMS, notification mirroring and file transfer: out of v1 since 2026-11-04 ([product-shape](product-shape.md)).
- The software factory, issue #140: after v1 ([chief-of-staff-practice](chief-of-staff-practice.md), "Merging").
