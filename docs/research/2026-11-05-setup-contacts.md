# The Setup Window's Contacts step, the finish page and the keybinding

For #22. Labels: **[V]** verified here, against the Simulated Phone and simulated BlueZ; **[I]** inferred.

## Pacing the sync like the Pixel

- The Pixel reads Call History quickly and the phonebook slowly. From the ticket comment: 200 Calls in under 2 s, and 319 phonebook entries in 9.7 s, about 30 ms each. The Simulated Phone's new `SetPullDelayPerVcard(phonebook_ms, call_history_ms)` adds that much to each pull, per vCard pulled. The harness sets 30 ms and 5 ms, so 312 Contacts take about 9.4 s. **[V]**
- The sync now pulls Call History first and reports it as soon as it lands. Then it selects `pb` and asks `GetSize` before pulling it, so the Setup Window shows "✓ Recent Calls" and "312 Contacts… about N seconds left" during the slow part. The estimate is `GetSize` × 30 ms from the moment it is known. **[V]** by `granted_the_window_warns_counts_down_with_the_phone_and_reports_what_was_synced`. **[I]** that the Pixel's pace is steady enough for the estimate to hold.

## The prompt

- The Simulated Phone's `SetContactsPrompt("ask")` shows a prompt with a 30 s deadline when the first session opens, then holds each OBEX request until it is answered or obexd's 10 s timeout passes. `AnswerContactsPrompt("grant" | "deny")` answers it and `ContactsPrompts()` lists each prompt with its shown and deadline times and its outcome. A fresh session after a timed-out prompt shows a new prompt. **[V]**
- The countdown is the app's 30 s deadline from the first `PermissionNeeded`. The harness checks it within 1.5 s of the Simulated Phone's deadline. **[V]**
- **[I]** Android's real prompt: it lasts about 30 s, a dismissed or unanswered prompt asks again on the next session, and "Don't ask again" is not offered. The Real-Phone Checklist's step 1.3, which lets the prompt time out, is what confirms this.

## The keybinding

- Omarchy's personal bindings are in `$XDG_CONFIG_HOME/hypr/bindings.lua`, as top-level `o.bind("KEYS", "Description", "command")` statements, so a line appended at the end takes effect. The app writes `o.bind("SUPER + ALT + C", "Desktop Calls", "<path to desktop-calls>")`, and only after the user has seen that exact line and chosen Confirm. **[V]** against the file layout on the user's Desktop (read only). **[V]** that Confirm appends exactly the line shown and Cancel writes nothing.
- If the keys are already bound in that file, the page names the clash and the command to bind by hand. With no `bindings.lua` (another Hyprland setup, or not Hyprland at all), it only names the command. A `hyprland.conf`-only setup isn't offered the line. **[V]**: the clash by `keys_the_user_already_bound_are_left_alone_and_the_finish_page_names_the_command`, no config in the core tests.
- **[I]** Hyprland reloads the Lua config on save, as it does `hyprland.conf`, so the binding works without logging out. Scenario 1's harness test runs the line's command itself rather than pressing the keys, because the harness compositor is not Hyprland.

## Real-Phone Checklist

Group 1 already has the steps for this. Step 1.2 pairs, allows Contacts, adds the keybinding and places a first Call to the Test Number. Step 1.3 turns Contacts access off, uses Try again, lets the prompt time out once, then allows it. Running them settles the **[I]** items above. The preflight line "the Setup Window offers one once #22 lands" can drop its "once #22 lands" after this merges.
