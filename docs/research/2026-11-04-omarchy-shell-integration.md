# Omarchy shell integration facts — 2026-11-04

omarchy 4.0.6-1 · Hyprland 0.56.2 · quickshell (server reports "quickshell" 1.2) · gtk4 4.24.1 · gtk4-layer-shell 1.3.0. Read-only; S = paths under /usr/share/omarchy/shell.

## 1. System tray: yes
- Verified: `busctl --user list` shows `org.kde.StatusNotifierWatcher` and `org.kde.StatusNotifierHost-1404-…`, both owned by quickshell PID 1404.
- Widget `omarchy.tray`: S/plugins/bar/widgets/Tray.qml:5 imports `Quickshell.Services.SystemTray`; :169 reads `SystemTray.items`.
- Enabled in this user's bar: ~/.config/omarchy/shell.json:35 (right section, first). It is also in the shipped default, /usr/share/omarchy/config/omarchy/shell.json:42.
- Caveats (Tray.qml:155-174): items go to a hover-reveal **drawer** by default and are shown inline only if the user pins them (`pinned`/`hidden` settings). Items with `Status.Passive` are skipped (:173). So an SNI icon should set status Active, or NeedsAttention for a live call.

## 2. Bar extension point: third-party plugins
- Documented in S/README.md:46-141. Drop a git repo or directory into `~/.config/omarchy/plugins/<id>/` with `manifest.json` (`kinds: ["bar-widget"]`, optionally `"service"`) and a QML entry point. Then run `omarchy-shell shell rescanPlugins` and `omarchy plugin enable <id>`, or `omarchy plugin add <git-url> --enable --yes`.
- Plugins land disabled and run unsandboxed inside the shell (README.md:112). The schema is in S/services/PluginRegistry.qml.
- Working local example: ~/.config/omarchy/plugins/37signals.hey (`service` + `bar-widget`; it talks to an external CLI). It is in the user's layout at shell.json right section.
- Built-in `omarchy.indicators` (S/plugins/bar/indicators/*.qml) is first-party only. I found no documented way for third parties to add an indicator there (inferred).

## 3. Do Not Disturb
- No D-Bus property. The server interface exposes only the standard signals (ActionInvoked, NotificationClosed, NotificationReplied, ActivationToken, PropertiesChanged).
- Read via IPC: `omarchy-shell notifications isDnd` returns `on`/`off` (verified: `off`). Also `dndState`, `toggleDnd`, `setDnd <v>`: S/plugins/notifications/Service.qml:857-877. The user-facing toggle is `omarchy-toggle-notification-silencing`.
- File to watch: `~/.local/state/omarchy/notifications.json` → `{"version":3,"dnd":false}`. It is rewritten on each toggle (Service.qml:25-26, 832).
- Bypass rule: NotificationLogic.js:118-122. A notification bypasses DND only if `app_name == "omarchy-action"`, or `app_name == "notify-send"` with urgency critical. **`urgency=critical` from any other app_name is silenced** (Service.qml:121-131, 170-173). No `x-*` hint affects DND. The only custom hints read are `omarchy-glyph` and `omarchy-exec-argv` (NotificationLogic.js:140-151) and `transient`, which only decides whether a silenced notification is written to history (Service.qml:139-151).
- A silenced notification never shows a popup; it goes straight to history (Service.qml:173-178).

## 4. Actions
- Verified `GetCapabilities`: `persistence body body-markup body-hyperlinks actions icon-static`. Set by `actionsSupported: true` at Service.qml:935. There is no `inline-reply` capability.
- **No action buttons are drawn.** I found no action rendering in S/plugins/notifications/components/NotificationCard.qml. Clicking a popup invokes only the action with identifier `"default"` (Service.qml:359-395). Without one, the click focuses the sender's window by class. Accept/Decline buttons would therefore be ignored, so do not depend on them.

## 5. GTK4 layer-shell overlay, top-right
- No `layer_rule` in ~/.config/hypr (Hyprland config is Lua; I grepped all files for "layer"). Shipped rules only match `omarchy-bar`, `omarchy-menu|…`, and `selection`: /usr/share/omarchy/default/hypr/apps/omarchy-shell.lua:5,10 and apps/screenshot-selection.lua:2. A custom namespace gets only the global `layers*` fade animations (default/hypr/looknfeel.lua:83-87).
- Expected to work: anchor top+right on the overlay layer. I did not test this with a running window.
- Conflict: notification toasts are a full-screen Overlay layer, namespace `omarchy-notifications`, laid out top-right (Service.qml:961-985). An overlay anchored top-right will overlap toasts. Offset it or pick another corner. The bar is a 26px top-layer surface (`hyprctl layers`), so set a top margin of at least the bar height, or rely on exclusive-zone handling.
