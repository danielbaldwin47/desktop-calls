# User preferences

- 2026-11-04: User experience beats delivery speed, code purity and convenience in every trade-off (user's global CLAUDE.md).
- 2026-11-04: The user leaves technical details to Claude; product and UX decisions go to the user. Source: kickoff message of this session.
- 2026-11-04: The user prefers the mattpocock-skills workflow: GLOSSARY.md, ADRs, GitHub issues ("grill-with-docs"). See [docs-layout](docs-layout.md).
- 2026-11-04: Agents mostly test against a Simulated Phone. Steps that need the real Phone are grouped into a Real-Phone Checklist for the user. Agents may Call one Test Number on their own (number not yet chosen). See [product-shape](product-shape.md).
- 2026-11-04 (Q28): Agents may commit to feature branches, push them, open pull requests and merge to `main` once checks pass, without per-PR review. The user reviews through Real-Phone Checklists and the [polish-scorecard](polish-scorecard.md).
- 2026-11-04 (Q29): The user declined a recurring wiki/issue checkup "yet". Don't schedule one; offer again later if the wiki drifts.
- 2026-11-04: The user supplied the Test Number. It is stored **outside the repo** at `~/.config/desktop-calls/test-number` (mode 600), because the repo will be published later. Agents read it from there and never commit it.
- 2026-11-04 (Q30): An agent may restart WirePlumber once during a short test Call, to check that hangup survives a restart, but only while no other audio stream is active. The user may be on real Calls at times. When they say so, no agent touches the Phone, Bluetooth or WirePlumber until they say they're done.

## 2026-11-13

- 2026-11-13, user: **no subagent runs on Fable unless the user names it.** They named one that day, for UI design work (the Fable design review in [destination](destination.md)). Agent descriptions end ": <model>" (the existing rule).
- 2026-11-13, user: **keep interactions with subagents minimal**, to protect the chief of staff's context and turns. How the chief of staff did that on the day is in [chief-of-staff-practice](chief-of-staff-practice.md).
- 2026-11-13, user: "when there's a conceptual/bigger design direction decision, you can go over the concept to make sure it's quality before sending off builders. Probably something to be done before writing up the spec in the future." The rule and its evidence are in [chief-of-staff-practice](chief-of-staff-practice.md).
- 2026-11-13, user, on a UI pass: they want one after the fixes and before their checklist sitting. "It should be very polished and have top notch UX as well." What it will contain is not yet chosen ([destination](destination.md)).
- 2026-11-13, user, on replying to the outside requester of #194: the chief of staff shows the user a draft first, "and also we can just implement and let him know it's been implemented". So the reply waits until Calls Off has shipped, and the user sees the draft before it is posted ([session-log](session-log.md)).
- 2026-11-13, user, about the first long real Call: it was incoming, no Incoming Call Banner appeared, they answered on the Phone and then moved the audio from the Call Window, the audio was perfect, and they did not mute. Facts about a Call, not preferences; the source is the chief of staff's conversation, and the log note's questions to the user ([real Call note](../research/2026-11-13-real-call-log.md), "For the user") are answered by them in part. The Banner's absence has a cause found by the triage, in [session-log](session-log.md).
