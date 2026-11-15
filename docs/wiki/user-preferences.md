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

## 2026-11-14

- 2026-11-14, user: **every agent clears its build cache when its work is done** (`cargo clean` in its worktree, or delete the worktree's `target/`; after its PR merges, or when it stops for good), says so in its final report, and leaves no `target/` in a finished worktree. Unneeded worktrees and caches are cleaned up as work goes. Why: `/home` filled to 100% from 34 worktrees' `target/` folders and all work stopped ([hardware-and-stack](hardware-and-stack.md), [session-log](session-log.md)). The rule is in the [implementer brief](../agents/implementer-brief.md). Whether `just land` should do it for the agent is a tooling ticket, #277; this page has no record of the user weighing in on it.

## 2026-11-15

All as the chief of staff relayed them to the author of this entry, who did not see the user's messages; each is checked against the PR or issue it names.

- 2026-11-15, user, on CI: "make it as minimal as is sensible." A PR's CI runs only what gates a merge. The checklist's group 6 and the scorecard report moved to `main`, and the gallery renders one look on a PR (PR #287, `a65d408`; [testing](testing.md)). Whether that trade went too far for dark and Omarchy looks is #291.
- 2026-11-15, user: **postpone the reply to the outside requester of #194.** **Supersedes** the 2026-11-13 entry above, under which the reply waited until Calls Off had shipped and the user saw a draft first. Calls Off has shipped ([product-shape](product-shape.md)), and #194 is still open, `ready-for-agent`, with no comment (checked 2026-11-15). No new date for the reply is recorded.
- 2026-11-15, user: **the chief of staff may choose and do after-v1 work** without asking first. In flight and held items are in [session-log](session-log.md) and [chief-of-staff-practice](chief-of-staff-practice.md). Items that need a concept review with the user stay held.
- 2026-11-15, worktree and cache clean-up: the user's rule of 2026-11-14 above stands (every agent clears its build cache; unneeded worktrees go as work goes). `just land` now removes the landed worktree's `target/` itself (#277, PR #287), which settles the question that entry left open; this page still has no record of the user weighing in on that choice.
- 2026-11-15, user (as relayed): **merging everything is fine.** Read with Q28 above (agents merge to `main` once checks pass) and the review gate ([chief-of-staff-practice](chief-of-staff-practice.md)); my reading is leave to land the open after-v1 PRs without asking per PR, reviews recorded. The user's exact words are not recorded here.
- 2026-11-15, user, restated (as relayed): **do not post the reply to the outside requester of #194 until the user says.** The same decision as the "postpone" entry above, now with a trigger: the user's say-so. #194 is open, `ready-for-agent`, with no comment (checked 2026-11-15).
- 2026-11-15, user (as relayed): after-v1 work is at the chief of staff's judgement. What is held or open for the user (concept reviews for #283, #140 and #292; #329; #301; the design questions from #323 and #324) is listed in the [session-log](session-log.md), "Open for the user".
