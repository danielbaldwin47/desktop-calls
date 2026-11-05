# Following the live Omarchy theme

Date: 2026-11-04. Ticket #39.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** the surfaces follow Omarchy's theme and Hyprland's rounding live, with no restart. Under CI-like load a theme switch reaches the screen in 0.19 to 0.34 s. Separately, the screen harness's own PNG decoding was the slowest part of every screen scenario in a debug build, and it now compiles optimised.

## 1. How Omarchy switches theme

Read from `/usr/share/omarchy/bin/omarchy-theme-set` (Omarchy on this Desktop). **[V]**

- The live theme is `$HOME/.local/state/omarchy/current/theme/`. Omarchy writes `$HOME/.local/state` literally and ignores `XDG_STATE_HOME`.
- A switch builds the new theme in `current/next-theme`, then runs `rm -rf current/theme` and `mv current/next-theme current/theme`, then writes `current/theme.name` and `current/theme.generation`. So `theme/` disappears for a moment. A watcher on `theme/colors.toml` would lose its file; the app watches `current/` and rereads 100 ms after the last event.
- Hyprland is reloaded (`omarchy-restart-hyprctl`) after the swap, so its `configreloaded` event comes later than the new `colors.toml`.
- `omarchy-theme-set-gnome` sets the settings portal's `color-scheme` from the same `mode`, but as one of many per-app hooks that run after the swap.

## 2. Light or dark: `mode` in colors.toml, not the portal

- All 23 of the 24 shipped themes that have a `colors.toml` set `mode = "dark"` or `mode = "light"`. **[V]** (`/usr/share/omarchy/themes/*/colors.toml`)
- The app sets AdwStyleManager's colour scheme from `mode` in the same step as the palette, so the palette and the light or dark style change in one frame. Following the portal alone would show a dark palette on light widgets (or the reverse) until the GNOME hook ran. **[I]** (the ordering is from the script; the mismatch itself was not filmed)
- Without `mode`, the app infers dark from the background's luminance. Omarchy also honours a legacy `light.mode` file beside `colors.toml`; the app doesn't yet. **[V]** (`omarchy-theme-set-gnome`, comment on lines 16 and 17)
- Without Omarchy, the colour scheme is left to the portal (stock Adwaita behaviour).

## 3. Which libadwaita variables

libadwaita 1.10 on GTK 4.24 exposes its palette as CSS custom properties, and one provider at APPLICATION priority on the display overrides them for every window. **[V]** (scenario screenshots)

- `background` sets the window, view, headerbar, sidebar, secondary-sidebar, dialog, popover and overview backgrounds (and their backdrops).
- `foreground` sets the matching foregrounds and `--card-fg-color`.
- `accent`, `red`, `green` and `yellow` set the accent, destructive and error, success, and warning colours. Each foreground is whichever readable label colour (below) suits it.
- Answer takes the theme's `green`, and Decline and Hang up its `red`. That is a UX choice: stock Adwaita's green and red clash with most Omarchy palettes.

## 4. Readable labels on themed buttons

- Labels need WCAG contrast of at least 4.5:1. The app prefers the theme's own background or foreground colour when it meets that, and otherwise picks the most readable of background, foreground, white and black.
- Answer and Decline sit side by side, so they share one label colour: the one whose worse contrast across the two buttons is best.

## 5. Corner radius

- Hyprland answers `j/getoption decoration:rounding` on its request socket with `{"option": "decoration:rounding", "int": N, "set": true }`. **[V]** (the real Hyprland, read-only)
- Omarchy's default is 0, so most Omarchy setups have square corners. Some themes set it (solitude sets 6). **[V]** (Omarchy's Hyprland defaults and theme files)
- Omarchy's own shell uses the same radius for its buttons, so the app rounds the surfaces and their buttons alike.
- The app reads the radius again on each `configreloaded>>` line from the event socket `.socket2.sock`. If Hyprland can't answer, the last radius stays.

## 6. Testing it

- The harness writes a fixture theme (`crates/test-harness/fixtures/omarchy-themes/`) into the scenario's own home directory, and switches it exactly as `omarchy-theme-set` does. The real theme is never touched.
- The Hyprland stand-in answers `getoption` and writes `configreloaded>>` to the event socket's listeners.
- Timed checks capture only a small region with `grim -g`. At scale 1.6, surface edges fall on fractional pixels and are blended, so corner probes look one layout pixel inside the edge.

## 7. Timing under CI-like load, and the harness's slow PNG decoding

Measured with the loop from [2026-11-04-ci-banner-latency.md](2026-11-04-ci-banner-latency.md): four scenarios at a time on one CPU, `/dev/dri` hidden, while five other agents loaded the Desktop (load average 12 to 15 on 20 CPUs). **[V]**

- Theme switch to the screen: 0.19 to 0.34 s over four timed runs, against the 1 s target.
- Before the fix, all four `omarchy_theme` scenarios failed in two of five runs with `the screen never showed … within 5s`. Probes in `screenshot_when` showed one full-screen check took 3.5 to 3.9 s, of which grim took 2.5 to 3.1 s and decoding plus scanning the PNG the rest. Pinned to an idle CPU, one check took 0.6 s.
- `png`, its deflate crates, `test-harness` and `automation` (where the generic `region_where` compiles) now build at `opt-level = 3` in the dev profile. A check then takes about 16 ms after the capture.
- grim's PNG compression level was not the problem: `-l 1` saved only about 40 ms per capture.
- `origin/main` without the fix, under the same load: `incoming_call_surfaces` passed 6 of 11. With it: 11 of 11 twice and 10 of 11 once (one Call Window that showed late).

Faster checks exposed a race the slow ones had hidden. The gallery's muted Call Window state (from #50) clicked the window right after muting; with 0.6 s less delay, the click sometimes landed before the window had redrawn, and the keypad shortcut went nowhere (2 of 3 runs). It now waits for the muted strip on screen first (4 of 4 runs). **[V]** Other scenarios that act on a surface straight after a state change may hide the same race.

The gallery now runs 24 harnesses (four looks: light and dark, stock and under an Omarchy theme), six states in each, so it takes about twice as long.
