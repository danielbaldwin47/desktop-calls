# Desktop Calls wiki

Chief-of-staff memory for this project: a dated, historical record. Domain terms live in [`GLOSSARY.md`](../../GLOSSARY.md); hard-to-reverse decisions live in [`docs/adr/`](../adr/). Raw subagent reports live in [`docs/research/`](../research/) and are cited, never edited. Wiki pages link to all three rather than restating them.

## People and preferences

- [user-preferences](user-preferences.md): how the user wants work done and reported, including the 2026-11-13 rules on Fable, subagent interactions, concept review before specs, the UI pass and replying to outside requesters (updated 2026-11-13).

## Decisions

- [destination](destination.md): what "v1 is done, pause" means, the fog of war (the UI pass, a media-audio mode, the Companion App) and what is out of scope (added 2026-11-13).

- [polish-scorecard](polish-scorecard.md): the agreed definition of "polished" for the hill climb.

- [product-shape](product-shape.md): v1 scope and UX decisions from grilling, built behaviour as implementers shipped it, and the 2026-11-13 decisions on the Call Pill (spec #197) and Calls Off with Quiet Hours (spec #198), dated (updated 2026-11-13).

## Environment

- [telephony-api-quirks](telephony-api-quirks.md): known PipeWire Telephony and PBAP misbehaviour to simulate and work around, and BlueZ behaviour seen on the real Pixel (blocked devices, profile drops, `Connect` errors) (updated 2026-11-13).

- [hardware-and-stack](hardware-and-stack.md): the user's Desktop, Phone and installed stack, dated (updated 2026-11-13).

- [testing](testing.md): the harness, CI timings against the Desktop, local CI (`just ci-local`, #158) and how it copes with load (#167), the load flakes by cause (#168 to #172, #180, #185), the rule that stand-ins are as strict as the real service (#166), the GitHub CI red from #182 and its fix in #211, and the Real-Phone Checklist, dated (added 2026-11-05; updated 2026-11-13).

## Process

- [session-log](session-log.md): dated checkpoints for resuming the hill climb; read the newest entry first (newest: 2026-11-13 evening, the real Call's log, main's CI green again and the Call Pill and Calls Off specs in flight).

- [retros](retros.md): subagent-session retros and the improvements pursued, including the Fable retro decisions and the Sonnet implementer trial (2026-11-05).

- [chief-of-staff-practice](chief-of-staff-practice.md): dated rules for writing tickets, planning batches and merging, including the feature freeze, the local-CI merge rules, PR-only docs, the #154 triage, merging from behind main, owned PRs, the concept review before specs and probes before building (updated 2026-11-13).

- [implementer-brief](../agents/implementer-brief.md): standing instructions every ticket implementer follows (added 2026-11-04).

- [docs-layout](docs-layout.md): what goes in GLOSSARY, ADRs, wiki, research, and the issue tracker; wiki changes go through PRs (2026-11-05).
