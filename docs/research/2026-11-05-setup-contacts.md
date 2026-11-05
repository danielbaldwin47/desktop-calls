# The Setup Window's Contacts step, the finish page and the keybinding

For #22. Labels: **[V]** verified here, against the Simulated Phone and simulated BlueZ, or by the core tests named; **[I]** inferred.

## Pacing the sync like the Pixel

- The Pixel gives Call History quickly and Contacts slowly. From the ticket comment: 200 Calls in under 2 s, and 319 Contacts in 9.7 s, about 30 ms each. The Simulated Phone's new `SetPullDelayPerVcard(contacts_ms, call_history_ms)` adds that much to each pull, per vCard pulled. The harness sets 30 ms and 5 ms, so 312 Contacts take about 9.4 s. **[V]**
- The sync's order:
  1. It pulls only the first page of Call History, the newest 50 Calls, and reports them as soon as they land.
  2. It selects `pb` and asks `GetSize`, then pulls Contacts.
  3. It pulls the older Call History pages.

  So the Setup Window shows "✓ Recent Calls" and "312 Contacts… about N seconds left" during the slow part, and Contacts never wait on the older pages. If an older page fails, the sync keeps the pages it already has, so Contacts are never skipped. A sync caches nothing until Contacts are read: a cache with the newest Calls and no Contacts would read back after a restart as a sync that finished. If the Phone has Contacts but the pull fails, and none are held from before, the sync fails rather than ending synced with none; with some held, they are kept. (A Call History refresh, a separate path older than this ticket, still saves what it reads.) Call History progress is logged as `call-history-read`, and `contacts-synced` is logged once, when Contacts land. The estimate is `GetSize` × 30 ms, counted from the moment the count is known.
  - **[V]** by `granted_the_window_warns_counts_down_with_the_phone_and_reports_what_was_synced` and `call_history_past_the_phones_50_entry_pulls_is_read_page_by_page`. The second one shows Contacts are ready with 50 Calls, the older Calls follow up to 120, and the log has `call-history-read` twice and `contacts-synced` once.
  - **[V]** by `older_call_history_pages_the_phone_fails_leave_contacts_read`, with the Simulated Phone's new `FailHistoryPullsFrom(offset)`.
  - **[V]** by `a_phone_leaving_during_the_contacts_pull_leaves_contacts_not_synced_across_a_restart`. The Simulated Phone fails the Contacts transfer under way when it goes out of range. The test finds the cache directory empty, restarts the app, and watches for a second after its cache read that Contacts stay `not-synced`. Both checks fail without the fix.
  - **[I]** that the Pixel's pace is steady enough for the estimate to hold.

## The prompt

- The Simulated Phone's `SetContactsPrompt("ask")` shows a prompt with a 30 s deadline when the first session opens. It then holds each `CreateSession` until the prompt is answered or obexd's 10 s timeout passes. **[V]**
  - `AnswerContactsPrompt("grant" | "deny")` answers the prompt.
  - `ContactsPrompts()` lists each prompt with its shown and deadline times and its outcome.
  - A fresh session after a timed-out prompt shows a new prompt.
- The Setup Window reports the deadline it counts down to as `deadline`, in milliseconds since the epoch. The harness checks that it is within 1.5 s of the Simulated Phone's own deadline. **[V]**
- **[I]** Android's real prompt: it lasts about 30 s, a dismissed or unanswered prompt asks again on the next session, and "Don't ask again" is not offered. The Real-Phone Checklist's step 1.3, which lets the prompt time out, is what confirms this.

## The Phone leaving during the Contacts step

- If the Phone disconnects before Contacts are read, the window says "My Pixel disconnected". This applies during the warning, the prompt, or while recent Calls have landed and Contacts are still being read. The window never says "Synced", and Contacts stay `unavailable`/`not-synced`: recent Calls from a first sync that was cut short don't make them `ready`. Once the Phone connects again, the window says "My Pixel is back" and offers Continue. The sync waits until the user chooses it, and then the Phone asks again.
  - **[V]** by the core tests in `crates/core/tests/setup_contacts.rs`.
  - **[V]** by the harness tests `a_phone_leaving_while_it_asks_is_waited_for_and_continue_asks_again` and `a_phone_leaving_during_the_contacts_pull_leaves_contacts_not_synced_across_a_restart`.
- If Contacts were already read, only older Calls are missing. The window finishes, with the heading "My Pixel is set up" rather than "is connected". **[V]** by the core tests.
- When the Simulated Phone goes out of range, a `CreateSession` waiting on the prompt fails with `Failed` and the prompt goes away. **[I]** that real obexd fails the same call when the link drops, and that Android takes the prompt down. The app ignores a sync failure that arrives while the Phone is gone. A failure that arrives just before hands-free goes down shows the finish page for a moment. If the Phone leaves within 2 s of that, the window says the Phone disconnected instead, and Contacts are `not-synced`. A Phone leaving later leaves the finish page as it is. **[V]** by the core tests; **[I]** that 2 s covers the gap on real hardware.

## The keybinding

- Omarchy's personal bindings are in `$XDG_CONFIG_HOME/hypr/bindings.lua`, as top-level `o.bind("KEYS", "Description", "command")` statements. **[V]** against the file layout on the user's Desktop (read only).
- The app writes `o.bind("SUPER + ALT + C", "Desktop Calls", "<path to desktop-calls>")`, and only after the user has seen that exact line and chosen Confirm. **[V]** that Confirm writes exactly the line shown and Cancel writes nothing.
- The adapter (`crates/desktop-calls/src/keybinding.rs`) only reads files. It passes the core the text of `bindings.lua` and of each file it names with `require`, `pcall(require, …)` or `dofile`. It looks up a module name under the config directory, then under `$OMARCHY_PATH` (by default `/usr/share/omarchy`). The core (`crates/core/src/keybinding.rs`) decides the outcome. **[V]** by `crates/core/tests/hyprland_keybinding.rs` and the adapter's unit tests.
  - Keys match in any order or case, and with modifier aliases (`MOD4` for `SUPER`, `MOD1` for `ALT`).
  - A commented-out line doesn't take the keys, and neither does a binding followed by `hl.unbind` of the same keys.
  - A binding in a required file counts at the point where it is required.
  - An earlier Desktop Calls line counts as Added, even when it runs `desktop-calls` from another path.
  - The line goes before a final top-level `return`, and gets a newline first if the file lacks one.
- Only `bindings.lua` and the files it names are read, and the offer, the clash and an unreadable file each name the file checked. **[I]** that bindings Omarchy loads from elsewhere, such as `hyprland.lua`'s other requires, don't use these keys. Omarchy's own defaults under `/usr/share/omarchy/default/hypr/bindings/` bind other `SUPER + ALT` keys but not `C`.
- The adapter re-reads the file on Confirm, and writes only if the file still holds the text the line was offered against. If the file changed, it writes nothing, and the page shows what the file can take now: the keys taken, the line already there, or a fresh offer. **[V]** by `keys_taken_after_the_offer_refresh_the_offer_instead_of_asking_to_try_again` and the adapter's unit tests.
- If the file can't be read, the page names it and the command to bind by hand. With no `bindings.lua` (another Hyprland setup, or not Hyprland at all), it names only the command, and a setup with only `hyprland.conf` isn't offered the line. **[V]**: an unreadable file and no config by the core and adapter tests; a clash by `keys_the_user_already_bound_are_left_alone_and_the_finish_page_names_the_command`.
- **[I]** Hyprland reloads the Lua config on save, as it does `hyprland.conf`, so the binding works without logging out. Scenario 1's harness test runs the line's command itself rather than pressing the keys, because the harness compositor is not Hyprland.

## Real-Phone Checklist

Group 1 already has the steps for this:
- Step 1.2 pairs, allows Contacts, adds the keybinding and places a first Call to the Test Number.
- Step 1.3 turns Contacts access off, uses Try again, lets the prompt time out once, then allows it.

Running them settles the **[I]** items above. After this merges, the preflight line "the Setup Window offers one once #22 lands" can drop "once #22 lands".
