# User preferences

- 2026-11-04: User experience beats delivery speed, code purity and convenience in every trade-off (user's global CLAUDE.md).
- 2026-11-04: The user leaves technical details to Claude; product and UX decisions go to the user. Source: kickoff message of this session.
- 2026-11-04: The user prefers the mattpocock-skills workflow: GLOSSARY.md, ADRs, GitHub issues ("grill-with-docs"). See [docs-layout](docs-layout.md).
- 2026-11-04: Agents mostly test against a Simulated Phone. Steps that need the real Phone are grouped into a Real-Phone Checklist for the user. Agents may Call one Test Number on their own (number not yet chosen). See [product-shape](product-shape.md).
- 2026-11-04 (Q28): Agents may commit to feature branches, push them, open pull requests and merge to `main` once checks pass, without per-PR review. The user reviews through Real-Phone Checklists and the [polish-scorecard](polish-scorecard.md).
- 2026-11-04 (Q29): The user declined a recurring wiki/issue checkup "yet". Don't schedule one; offer again later if the wiki drifts.
- 2026-11-04: The user supplied the Test Number. It is stored **outside the repo** at `~/.config/desktop-calls/test-number` (mode 600), because the repo will be published later. Agents read it from there and never commit it.
- 2026-11-04 (Q30): An agent may restart WirePlumber once during a short test Call, to check that hangup survives a restart, but only while no other audio stream is active. The user may be on real Calls at times. When they say so, no agent touches the Phone, Bluetooth or WirePlumber until they say they're done.
