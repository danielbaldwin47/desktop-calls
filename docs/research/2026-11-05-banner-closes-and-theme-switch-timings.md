# Banner-closes and theme-switch timings on the app's own clock

Date: 2026-11-05. Ticket #99. Follows #83 ([app-measured timings](2026-11-05-app-measured-timings.md)). Simulated Phone only.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** "the Banner closes at once" (200 ms) and "an open Banner follows a theme switch" (1 s) now assert the app's own log, not the tests' wall clock across processes. The tests' own round trip gets a looser 3 s. No bound was loosened. Under `just test-ci-like` both pass, and a deliberate slowdown fails each.

## 1. What the app logs now

| Event | When | Level |
|---|---|---|
| `surface-unmapped` (`surface`) | GTK unmaps a surface: the moment the control interface's `Surfaces` stops listing it | debug |
| `theme-shown` (`surface`, `mode`) | after a switch of Omarchy's theme has been applied, each surface on screen has drawn its next frame (the window's frame clock emits `after-paint`) | info |

- `surface-unmapped` is logged from the window's `unmap` handler. GTK's own unmap, which hides the GDK surface, runs before connected handlers, so the compositor has been told by then. **[I]** from GTK's signal order (`unmap` is `G_SIGNAL_RUN_FIRST`); not traced on the wire.
- `theme-shown` comes from a hook `Theme::when_switched`, which runs only when the debounced reread finds a different palette. A Hyprland rounding reload does not log it, so a scenario can't match a rounding frame for a theme one. `mode` is `dark` or `light`, and is left out when there is no Omarchy theme any more. **[V]** (scenario logs)

## 2. What the scenarios assert

- **Banner closes** (`answered_on_phone_surfaces.rs`: declined on the Phone, the caller hanging up): `Harness::assert_banner_closed_within` takes the first `call-ended` for the Call (logged when the Phone says `disconnected`) to the Banner's `surface-unmapped`, against 200 ms. The wait on `Surfaces` is held to 3 s. **[V]**
- **Theme switch** (`omarchy_theme.rs`): the harness notes its wall clock just before it starts the switch, then `assert_app_within_since` takes that to the Banner's `theme-shown` for the new mode, against 1 s. That is the pattern #17 used for Do Not Disturb: the switch is file operations in the test's own process, so only inotify, the app's 100 ms debounce and the app sit between. The screen check is held to 3 s. **[V]**

## 3. Figures

Under `just test-ci-like`, both files, one run each. **[V]**

| Timing | Target | Observed |
|---|---|---|
| `call-ended` to the Banner unmapped | 200 ms | 0.33 and 0.39 ms |
| Switch started to the Banner drawn in the new theme | 1 s | 119 ms (to light), 190 ms (to dark) |

On the Desktop without a quota: the Banner closed 0.3 to 0.4 ms after `call-ended`, and `theme-changed` to `theme-shown` took 25 to 29 ms. **[V]**

## 4. A deliberate slowdown fails each

Local, uncommitted changes, under `just test-ci-like`. **[V]**

- A 250 ms sleep before the Banner hides: both Banner-closes scenarios failed with "the app took 250.4ms for the Banner to close once the Phone said the Call ended, against 200ms". The third scenario in the file passed.
- A 1 s sleep between reading the new theme and applying it: the theme scenario failed with "the app took 1.127451s for the Banner to show the light theme, against 1s". Its screen check had passed within its 3 s.

## 5. Left as it was

- `incoming_call.rs` still holds "the Desktop stops ringing at once" (declined on the Phone, the caller hanging up) to 200 ms on the tests' wall clock, through `Calls` and `Ringing`. It has the same exposure to a loaded runner. `call-ended` to `ringing-stopped` would be its app-logged form: scenarios log `ringing-started`, `ringing-failed` and `ringing-stopped` even though the player can't reach PipeWire. **[V]** in one kept log. Not changed; outside this ticket's two checks, and worth a ticket of its own.
