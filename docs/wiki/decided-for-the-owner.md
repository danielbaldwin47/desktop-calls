# Decided for the owner

Design decisions the chief of staff or an agent made for the owner, under the owner's leave of 2026-11-15 to go ahead with decisions that are simple to reverse ([user-preferences](user-preferences.md)). The owner reads this page to go over them; each line says how to overrule. Hard-to-reverse decisions, anything that stores user data, and new design directions go to the owner first and never appear here (the chief of staff's interpretation).

An implementer whose brief marks a decision "decided for the owner" adds its line here in the same PR ([implementer brief](../agents/implementer-brief.md)): date, decision, issue and PR, how to overrule, and the owner's verdict once given.

- 2026-11-15, **"Pair again"**: a Phone that forgot the pairing gets Pair again in the Setup Window, which removes the stale pairing (the app's first use of BlueZ `RemoveDevice`) and pairs afresh. #377, PR #382 (`cd68b35`). To overrule: say so, and the window goes back to reading key-missing as "Phone calls off". **Owner: kept** (2026-11-15).
- 2026-11-15, **"Phone setup…" with a Phone set up opens on its finish page**, with Pair another phone, instead of on choosing a Phone. #389, PR #393 (open when written). To overrule: name the page it should open on.
- 2026-11-15, **the gallery captures short states from the harness only** (the chief of staff's call): it stops the app on the first frame that shows a short state, with no test hook added to the app. #384, PR #391 (`ef9e3a1`). To overrule: ask for the test-only feature (#391's "option A"), which reverses #248.
