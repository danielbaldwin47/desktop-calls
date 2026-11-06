# Docs layout

Decided 2026-11-04 by Claude, after the user asked for the mattpocock-skills approach ("grill-with-docs, or however is best").

- `GLOSSARY.md`: domain language only, with no implementation detail. Format per the `domain-modeling` skill.
- `docs/adr/`: decisions that are hard to reverse, surprising, and the result of a real trade-off.
- `docs/wiki/`: chief-of-staff memory, meaning dated history, preferences, environment facts and process. Pages cite ADRs, research and commits rather than restating them.
- `docs/research/`: raw subagent research reports, dated and never edited after filing.
- GitHub Issues on `danielbaldwin47/desktop-calls`: specs and tickets (see `docs/agents/issue-tracker.md`).
- 2026-11-04: Integration branch is `main`. Ticket branches merge via PR once checks pass (user Q28). Every implementer follows [`docs/agents/implementer-brief.md`](../agents/implementer-brief.md).
- 2026-11-05: changes to `docs/wiki/` go through a PR and `just ci-local`, like code, and are not merged by their author ([chief-of-staff-practice](chief-of-staff-practice.md)).
