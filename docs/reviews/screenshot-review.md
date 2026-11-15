# Screenshot review

The latest verdict of a screenshot-review agent on the gallery (`just screenshot`), for polish-scorecard item 8 (#26). `just scorecard` reads the lines below:

- **NEEDS-REVIEW** once any "Reviewed path" has a different git object id at `HEAD`, or no longer exists there, because what the review judged has changed. The ids depend only on content, so they survive a squash merge, which the commit doesn't.
- Otherwise **PASS** when the verdict is `no defects`, and **FAIL** when it is `defects found`. The verdict decides, not whether the `Defect:` issues are still open: a defect is gone only once a new review sees it gone.

To record a new review: render the gallery, have a screenshot-review agent judge every state in every look, file each defect as a sub-issue of #1, then replace the lines below with the new commit, date, verdict (`no defects` or `defects found`), each path's object id (`git rev-parse <commit>:<path>`) and one `Defect:` line per open issue.

Reviewed commit: 24300ac
Date: 2026-11-15
Reviewer: two Opus screenshot-review agents, one for the light looks (stock and Omarchy) and one for the dark, from 248 screenshots of 62 states and the Indicator's 13 drawings at 5a93755; the three states changed since then (`setup-window-done-contacts-denied`, `call-window-held-third-call-waiting`, and the new `call-window-phone-kept-audio`) rechecked at 24300ac, where the app's code is unchanged
Verdict: defects found

Reviewed path: crates/desktop-calls/src 327e6e08be222f978c1ea8511d10aa628c9379f9
Reviewed path: crates/core/src f277c0a208462cb3c22ebfdbf17b2e5bf34f40f6
Reviewed path: data/icons 6c7fcbb884ce0c15c638989e44ab75c4daec9fca
Reviewed path: crates/automation/src/gallery eabcc7fad2aa545cd551c4dedaafe73d8e18cc7e
Reviewed path: crates/automation/src/gallery.rs 3a7deba3bed3b878636356ee284bc24c84e64369
Reviewed path: crates/test-harness/src 82278e85af8970c11fb1812a5200ff8b7b59589a
Reviewed path: crates/test-harness/fixtures/omarchy-themes 611945734f7bbac1c7ed24b6e3cc5d1348d765d1

## Defects

Defect: #313 Indicator menu labels are cut off (high)
Defect: #314 Quiet Hours Window says "Saved" beside its refusal (medium)
Defect: #315 Setup Window says it is looking for phones while Bluetooth is off (medium)
Defect: #316 Setup Window's contacts pages: a leftover "0", and the countdown's label far from it (medium)
Defect: #318 Copy-to-terminal commands wrap mid-command (medium)
Defect: #319 "Calls off after" reads as cut off, and runs into the tag before it (medium)
Defect: #320 Primary buttons' text colour differs (medium)
Defect: #321 Keycaps on Hang up and Swap fill the button's height (low)
Defect: #322 Key notation varies across surfaces (low)
Defect: #323 Layouts jump between closely related states (low)
Defect: #324 Wording differs for the same thing across surfaces (low)
Defect: #325 Gallery: the long-number states use a short number (low)
Defect: #326 Small alignment and contrast defects (low)

## Not filed

- The dark navy title bar with monospace text on the Setup Window and the Quiet Hours Window: sway draws it in the gallery's compositor; Hyprland doesn't, on the Desktop.
- "jose" also finding Joanna Smithers in the Dialer: the search matches fuzzily by design (`crates/core/src/matching.rs`), and the closer match is listed first.

## Withdrawn

- #317 (the finish page after Contacts were refused had no keybinding step): the gallery state ran without a Hyprland config. With one, the page offers Add keybinding as the usual finish page does.
