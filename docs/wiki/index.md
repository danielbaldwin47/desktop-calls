# Desktop Calls wiki

Chief-of-staff memory for this project: a dated, historical record. Domain terms live in [`GLOSSARY.md`](../../GLOSSARY.md); hard-to-reverse decisions live in [`docs/adr/`](../adr/). Raw subagent reports live in [`docs/research/`](../research/) and are cited, never edited. Wiki pages link to all three rather than restating them.

## People and preferences

- [user-preferences](user-preferences.md): how the user wants work done and reported, including the 2026-11-13 rules on Fable, subagent interactions, concept review before specs, the UI pass and replying to outside requesters, and the 2026-11-14 rule that every agent clears its build cache when done, and the 2026-11-15 instruction to make CI as minimal as is sensible, the postponed reply to #194 (not to be posted until the user says), "merging everything is fine" and the go-ahead for after-v1 work (updated 2026-11-15).

## Decisions

- [destination](destination.md): what "v1 is done, pause" means, the fog of war (the UI pass, a media-audio mode, the Companion App) and what is out of scope; where the five criteria stood on 2026-11-14 and 2026-11-15, including the evening of 2026-11-15 at `2846987`: v1's agent-side work complete, the UI pass finished, the scorecard tool's 0 of 8 (item 7 FAIL on #292, item 8 NEEDS-REVIEW), and only the user's sitting remaining (added 2026-11-13; updated 2026-11-15).

- [polish-scorecard](polish-scorecard.md): the agreed definition of "polished" for the hill climb, with the audit of 2026-11-14 against the Simulated Phone and the 2026-11-15 scorecard tool's four statuses per item (updated 2026-11-15).

- [product-shape](product-shape.md): v1 scope and UX decisions from grilling, built behaviour as implementers shipped it, the 2026-11-13 decisions on the Call Pill (spec #197) and Calls Off with Quiet Hours (spec #198), what the merged PRs built of them and of the UI pass, and, for 2026-11-15, the pass's finish per surface (Missed Call notification and cue, Dialer rows, Setup Window, Call Window, Banner, motion, copy), the lost-contact words, and the design questions left open from #323 and #324 (updated 2026-11-15).

## Environment

- [telephony-api-quirks](telephony-api-quirks.md): known PipeWire Telephony and PBAP misbehaviour to simulate and work around, and BlueZ behaviour seen on the real Pixel (blocked devices, profile drops, `Connect` errors), and the 2026-11-15 `calls-go-apart` quirk (updated 2026-11-15).

- [hardware-and-stack](hardware-and-stack.md): the user's Desktop, Phone and installed stack, dated, including the disk filling on 2026-11-14 and the `/tmp` per-user quota hit on 2026-11-15 (updated 2026-11-15).

- [testing](testing.md): the harness, CI timings against the Desktop, local CI (`just ci-local`, #158) and how it copes with load (#167), the load flakes by cause (#168 to #172, #180, #185), the rule that stand-ins are as strict as the real service (#166), the GitHub CI red from #182 and its fix in #211, GitHub's CI red or cancelled again on 2026-11-14, CI back on GitHub for PRs (#286), the cold review of #287 (#290, #291), and, for 2026-11-15, what gates a PR and its timings, the flakes by cause (#146, #298, #303, #331, #338 and the gallery failure on `2846987`, found to be a product bug in Missed Call counting, #356), the Dialer's keystroke regression (#340, #348, #351), the gallery, keyboard-only suite and scorecard, and the Real-Phone Checklist, dated (added 2026-11-05; updated 2026-11-15).

## Process

- [session-log](session-log.md): dated checkpoints for resuming the hill climb; read the newest entry first (newest: 2026-11-15, morning to afternoon, `main` up to `2846987`, with #357's later fix of that day's gallery failure noted: the after-v1 merges, the review gate, the `/tmp` quota, the UI pass finished, v1's agent-side work complete and what is open for the user; before it, 2026-11-15, CI back on GitHub and green; and 2026-11-14, the Call Pill, Calls Off and the UI pass merged).

- [retros](retros.md): subagent-session retros and the improvements pursued, including the Fable retro decisions, the Sonnet implementer trial (2026-11-05), the disk filling (2026-11-14), and the `/tmp` quota, the review gate and two CI reruns (2026-11-15).

- [chief-of-staff-practice](chief-of-staff-practice.md): dated rules for writing tickets, planning batches and merging, including the feature freeze, the local-CI merge rules, PR-only docs, the #154 triage, merging from behind main, owned PRs, the concept review before specs, probes before building, clearing build caches, the 2026-11-15 after-v1 batches and held items, the review records `just land` requires since #327 (PR #332) and why, the rule that agents never sweep shared `/tmp`, and the reruns against the rule (updated 2026-11-15).

- [implementer-brief](../agents/implementer-brief.md): standing instructions every ticket implementer follows (added 2026-11-04).

- [docs-layout](docs-layout.md): what goes in GLOSSARY, ADRs, wiki, research, and the issue tracker; wiki changes go through PRs (2026-11-05).
