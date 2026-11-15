# Screenshot review

The latest verdict of a screenshot-review agent on the gallery (`just screenshot`), for polish-scorecard item 8 (#26). `just scorecard` reads the lines below: it reports PASS for "no defects", FAIL with the open defects otherwise, and NEEDS-REVIEW once any path under "Reviewed tree" has a different git object id at `HEAD`, because what the review judged has changed.

To record a new review: render the gallery, have a screenshot-review agent judge every state in every look, file each defect as a sub-issue of #1, then replace the lines below with the new commit, date, verdict, object ids (`git rev-parse <commit>:<path>`) and one `Defect:` line per open issue.

Reviewed commit: 5a93755
Date: 2026-11-15
Reviewer: two Opus screenshot-review agents, one for the light looks (stock and Omarchy) and one for the dark, from 248 screenshots of 62 states and the Indicator's 13 drawings
Verdict: defects found

Reviewed tree: crates/desktop-calls/src 327e6e08be222f978c1ea8511d10aa628c9379f9
Reviewed tree: crates/core/src/messages 76f4ed6741d8059ae6406b3e62344cacdb834a7e
Reviewed tree: crates/core/src/messages.rs 72f8623518b0d8cf64c9f563b8bb2388c55043a8
Reviewed tree: crates/automation/src/gallery a48f83fbe2e44a72a795eb281f0647888e0a458c
Reviewed tree: crates/automation/src/gallery.rs 5b6dad717b7d1921f9a34e4e26a351be491abe3e

## Defects

Defect: #313 Indicator menu labels are cut off (high)
Defect: #314 Quiet Hours Window says "Saved" beside its refusal (medium)
Defect: #315 Setup Window says it is looking for phones while Bluetooth is off (medium)
Defect: #316 Setup Window's contacts pages: a leftover "0", and the countdown's label far from it (medium)
Defect: #317 The finish page after Contacts were refused has no keybinding step (medium)
Defect: #318 Copy-to-terminal commands wrap mid-command (medium)
Defect: #319 "Calls off after" reads as cut off, and runs into the tag before it (medium)
Defect: #320 Primary buttons' text colour differs (medium)
Defect: #321 Keycaps on Hang up and Swap fill the button's height (low)
Defect: #322 Key notation varies across surfaces (low)
Defect: #323 Layouts jump between closely related states (low)
Defect: #324 Wording differs for the same thing across surfaces (low)
Defect: #325 Gallery: long-number states use a short number; the third-Call caption contradicts its picture (low)
Defect: #326 Small alignment and contrast defects (low)

## Not filed

- The dark navy title bar with monospace text on the Setup Window and the Quiet Hours Window: sway draws it in the gallery's compositor; Hyprland doesn't, on the Desktop.
- "jose" also finding Joanna Smithers in the Dialer: the search matches fuzzily by design (`crates/core/src/matching.rs`), and the closer match is listed first.
