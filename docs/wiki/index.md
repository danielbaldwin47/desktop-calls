# Desktop Calls wiki

Chief-of-staff memory for this project: a dated, historical record. Domain terms live in [`GLOSSARY.md`](../../GLOSSARY.md); hard-to-reverse decisions live in [`docs/adr/`](../adr/). Raw subagent reports live in [`docs/research/`](../research/) and are cited, never edited. Wiki pages link to all three rather than restating them.

## People and preferences

- [user-preferences](user-preferences.md): how the user wants work done and reported, including the 2026-11-13 rules on Fable, subagent interactions, concept review before specs, the UI pass and replying to outside requesters, and the 2026-11-14 rule that every agent clears its build cache when done, and the 2026-11-15 instruction to make CI as minimal as is sensible, the postponed reply to #194 and the go-ahead for after-v1 work (updated 2026-11-15).

## Decisions

- [destination](destination.md): what "v1 is done, pause" means, the fog of war (the UI pass, a media-audio mode, the Companion App) and what is out of scope; where the five criteria stood on 2026-11-14 and 2026-11-15, when v1's agent-side work was complete and only the user's Checklist and the `needs-info` issues it settles remained (added 2026-11-13; updated 2026-11-15).

- [polish-scorecard](polish-scorecard.md): the agreed definition of "polished" for the hill climb, with the audit of 2026-11-14 against the Simulated Phone (updated 2026-11-14).

- [product-shape](product-shape.md): v1 scope and UX decisions from grilling, built behaviour as implementers shipped it, the 2026-11-13 decisions on the Call Pill (spec #197) and Calls Off with Quiet Hours (spec #198), and what the merged PRs built of them and of the UI pass, including the Quiet Hours Window's key path (#279), the long-number fix (#276) and the README's Calls Off section, dated (updated 2026-11-15).

## Environment

- [telephony-api-quirks](telephony-api-quirks.md): known PipeWire Telephony and PBAP misbehaviour to simulate and work around, and BlueZ behaviour seen on the real Pixel (blocked devices, profile drops, `Connect` errors) (updated 2026-11-13).

- [hardware-and-stack](hardware-and-stack.md): the user's Desktop, Phone and installed stack, dated, including the disk filling on 2026-11-14 (updated 2026-11-14).

- [testing](testing.md): the harness, CI timings against the Desktop, local CI (`just ci-local`, #158) and how it copes with load (#167), the load flakes by cause (#168 to #172, #180, #185), the rule that stand-ins are as strict as the real service (#166), the GitHub CI red from #182 and its fix in #211, GitHub's CI red or cancelled again on 2026-11-14, CI back on GitHub for PRs (#286), the cold review of #287 (#290, #291), and the Real-Phone Checklist, dated (added 2026-11-05; updated 2026-11-15).

## Process

- [session-log](session-log.md): dated checkpoints for resuming the hill climb; read the newest entry first (newest: 2026-11-15, CI back on GitHub and green, v1's agent-side work complete, after-v1 work begun; before it, 2026-11-14, the Call Pill, Calls Off and the UI pass merged).

- [retros](retros.md): subagent-session retros and the improvements pursued, including the Fable retro decisions, the Sonnet implementer trial (2026-11-05) and the disk filling (2026-11-14).

- [chief-of-staff-practice](chief-of-staff-practice.md): dated rules for writing tickets, planning batches and merging, including the feature freeze, the local-CI merge rules, PR-only docs, the #154 triage, merging from behind main, owned PRs, the concept review before specs, probes before building, clearing build caches, the 2026-11-15 after-v1 batches and held items, and the review records `just land` requires since #327 (updated 2026-11-15).

- [implementer-brief](../agents/implementer-brief.md): standing instructions every ticket implementer follows (added 2026-11-04).

- [docs-layout](docs-layout.md): what goes in GLOSSARY, ADRs, wiki, research, and the issue tracker; wiki changes go through PRs (2026-11-05).
