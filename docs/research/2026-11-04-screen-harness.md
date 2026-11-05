# Screen harness: headless compositor, synthetic input and desktop stand-ins

Date: 2026-11-04. Ticket #5. Builds on the [test seam note](2026-11-04-test-seam.md).
Legend: **[V]** = verified on this machine. **[I]** = inferred, not tested.

**Bottom line:** sway 1.12 from Arch `extra` is the screen harness's compositor, headless for tests and nested for `just demo`. The harness types and clicks through its own virtual keyboard and pointer, captures screens with `grim`, and serves the Notifications, tray watcher and Settings portal stand-ins in-process. The suite (29 tests) passes with two copies in parallel. This machine has no sway yet: `sudo pacman -S --needed sway grim` before running `just test` or `just demo` locally.

## 1. Choosing the compositor

| Compositor | Arch repo | Headless | layer-shell | screencopy | virtual keyboard | virtual pointer | Verdict |
|---|---|---|---|---|---|---|---|
| sway 1.12 (wlroots 0.20) | `extra` | `WLR_BACKENDS=headless` | yes | yes | yes | yes | chosen |
| Hyprland (installed) | `extra` | no headless-only mode (aquamarine, not wlroots) | yes | yes | yes | yes | no |
| cage 0.3 | `extra` | yes | no **[I]** | yes **[I]** | no **[I]** | no **[I]** | no: a kiosk, one fullscreen client |
| labwc 0.20 | `extra` | yes | not checked | not checked | not checked | not checked | not chosen: sway was checked first and covers every need |

- sway's globals include `zwlr_layer_shell_v1`, `zwlr_screencopy_manager_v1`, `zwp_virtual_keyboard_manager_v1` and `zwlr_virtual_pointer_manager_v1`. **[V]** (sway 1.12-4 extracted from the `extra` package, sha256 checked against the local sync database; sway is not installed here.)
- `wlrctl` (pointer and keyboard from the shell) is AUR-only, so the harness speaks the protocols itself with `wayland-client` 0.31. `wtype` is installed but has no pointer. **[V]**
- Headless environment: `WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1`. The output is configured like this Desktop's monitor: `HEADLESS-1` at 3840x2160, scale 1.6, so the layout is 2400x1350 logical pixels. **[V]** (Desktop monitor: `DP-3` 3840x2160@1.6 from `hyprctl monitors`.)
- Nested (`just demo`): `WLR_BACKENDS=wayland` with `WAYLAND_DISPLAY` set to the parent's absolute socket path. The window is titled `wlroots - WL-1`. **[V]** (inside a private headless sway standing in for the Desktop, never on the real screen.)
- A nested sway does **not** exit when its window is closed: it loses its only output and carries on. The harness treats "no `wl_output` global left" as closed. **[V]**
- Readiness: the config runs `exec printf '%s\n' "$WAYLAND_DISPLAY" > FIFO`, and the harness reads the FIFO with a 10 s deadline. **[V]**
- Unix socket paths are limited to 108 bytes, so the compositor's runtime directory must stay short. `/tmp/desktop-calls-test-*` fits; the agent scratchpad path does not. **[V]**

## 2. Synthetic input

- A headless sway with no input devices has seat capabilities `0`, so clients get no `wl_keyboard` or `wl_pointer`. The harness creates one virtual keyboard and one virtual pointer as soon as the compositor is ready and keeps them for the whole scenario, which gives the seat both capabilities. **[V]**
- The virtual keyboard's keymap must be a NUL-terminated XKB string (format 1); includes like `pc+us+inet(evdev)` resolve on the compositor side. **[V]**
- Virtual keyboard key events do not change modifier state. Typing `H` needs Shift pressed, then an explicit `modifiers(1, 0, 0, 0)`, the key, Shift released and `modifiers(0, 0, 0, 0)`. **[V]** (`synthetic_keys_reach_the_focused_client`.)
- A click sent before the client has drawn at its tiled size misses: the client still has its first 0x0-sized surface. The test client prints `configured W H` only after drawing at the compositor's size, and pointer tests wait for it. **[V]** (failed, then 10 of 10 runs passed after the fix.)
- The virtual pointer moves in absolute layout coordinates (`motion_absolute` with extent 2400x1350, scaled by 100 for sub-pixel precision). **[V]**
- `wayland-backend` prints its own errors to stderr unless its `log` feature is on; `just demo` printed `Io error: Broken pipe` on Ctrl-C. The workspace turns `log` on, with no logger. **[V]**

## 3. Screenshots

- `grim` captures through wlr-screencopy at physical size: 3840x2160 PNGs. **[V]**
- Scenario screenshots go to `target/screenshots/scenarios/<test name>/`; `just screenshot` writes `target/screenshots/gallery/` with `index.html`, one light and one dark PNG per state. Today the gallery has one state, `bare-output`. **[V]**

## 4. Desktop stand-ins

- All three are served on one zbus connection to the scenario's session bus: `org.freedesktop.Notifications`, `org.kde.StatusNotifierWatcher` and `org.freedesktop.portal.Desktop`. **[V]**
- Notifications: the stand-in reports Omarchy's capabilities, `persistence body body-markup body-hyperlinks actions icon-static`, and server information `quickshell`. Omarchy's server, on a click, invokes only the `default` action and then closes the notification with reason 2; the stand-in's `invoke_default_action` does the same. **[V]** (Omarchy shell source; `notify-send --action=default=...` prints `default` in the test.)
- Tray watcher: `RegisterStatusNotifierItem` takes either a bus name or an object path; for a path, the item's bus name is the caller's unique name. The stand-in reads `Status` and `IconName` live from the item. **[V]**
- Do Not Disturb: Omarchy keeps it in `~/.local/state/omarchy/notifications.json` (hardcoded `$HOME/.local/state`, not `XDG_STATE_HOME`), written atomically as `JSON.stringify({version: 3, dnd}, null, 2)` plus a newline. The harness writes the same file under the scenario's `HOME`. **[V]** (Omarchy shell source.)
- Theme: the Settings portal stand-in answers `org.freedesktop.appearance color-scheme` with `1` (dark) or `2` (light). GTK 4.24 follows it (`gtk-interface-color-scheme` 2 for dark, 3 for light) and so does libadwaita (`AdwStyleManager:dark`). **[V]** (throwaway probe against the stand-in.)

## 5. Scenario files

- The Simulated Phone takes `--scenario FILE`: one step per line (`connect`, `disconnect`), `#` comments, run in order after it is serving. An unknown step stops it before it serves, naming the line. **[V]**
- `just demo` defaults to `crates/simulated-phone/scenarios/demo.scenario`. It reports `ConnectionState: connected (Simulated Pixel).` once the app is connected, then waits; closing the window or Ctrl-C stops everything and removes the scenario directory. **[V]**

## 6. CI

- Arch's sway `post_install` runs `setcap cap_sys_nice+ep /usr/bin/sway`. In the CI container that capability is outside the bounding set, so starting sway fails with `Operation not permitted (os error 1)`. **[V]** (run 41940032873.) CI removes it with `setcap -r /usr/bin/sway`; sway only loses realtime scheduling. On the Desktop the capability is harmless. **[I]**
- CI installs `sway grim libnotify`, runs `just screenshot` and `just scorecard` after `just test`, and uploads `target/screenshots` and `scorecard.txt` as the `screenshots` and `scorecard` artifacts. **[I]** until the PR's first run.
