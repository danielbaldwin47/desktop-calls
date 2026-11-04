# Foner as prior art for Desktop Calls (2026-11-04)

Terms are as in [GLOSSARY.md](../../GLOSSARY.md). Source read at `AminBlg/kaller` commit
`fc25b13592561c6236f191256752171a4bdeb44f` (2026-10-07). Citations are `path:line` at that commit.
[V] = verified by reading code or running a command here; [I] = inferred, not run. Foner was not built or run.

**Verdict: (c) build Desktop Calls in Rust/GTK as planned, and borrow Foner's D-Bus and PBAP findings.**
Coverage against the 21 v1 decisions and 8 scorecard items (29 in all): **7 have, 15 partial, 7 missing**.

## 1. Project facts [V]

- Canonical repo is `github.com/AminBlg/kaller`. `AminBlg/foner` redirects to it, so the rename went foner → kaller. The code and docs still say "foner".
- License: GPL-3.0 (`LICENSE`, SPDX `GPL-3.0-or-later` in every file). Desktop Calls is also GPL-3.0, so copying code would be license-compatible.
- Activity: 21 commits, 2026-09-01 to 2026-10-07, all by one author (AminBlg). The history was flattened before publication (`docs/STATUS.md:9-11`). One tag, v0.1.0 (2026-10-06). 48 stars, 0 forks.
- Open issues: #1 (POCO X8 Pro: needed `wpctl set-default` to choose the microphone) and #2 (Fedora 44 install script refuses to run). Neither has a maintainer reply.
- Size: about 3,800 lines of C++ (sources and headers) plus 1,540 of QML. Tests: unit tests for vCard parsing, search and call-end tracking (`tests/test_contacts.cpp`, `tests/test_callend.cpp`). There is no fake Phone.
- Tested phones: one Samsung (`docs/M0-selfio-notes.md:15`, mSBC codec 2) and one iPhone. **No Pixel.** Tested only on Arch with Plasma.

## 2. Feature map

| v1 decision ([product-shape](../wiki/product-shape.md)) | Foner | Evidence |
|---|---|---|
| Incoming Call Banner, top right | partial | A KNotification with Answer/Decline actions (`src/core/notifications/incomingcallnotifier.cpp:58-74`), plus the main Kirigami window shown and raised with an OverlaySheet (`src/app/qml/main.qml:1365-1460`). No layer-shell. Waiting calls get no notification, because `incomingCall` fires only for state `incoming` (`src/core/telephony/telephonyclient.cpp:358`). |
| Desktop rings while the Phone rings | partial | notifyrc `Sound=phone-incoming-call` (`packaging/foner.notifyrc:10-14`). It plays once and does not loop: no `LoopSound` flag [I]. |
| Answer / Decline | has | `org.ofono.VoiceCall.Answer`/`Hangup` (`src/core/telephony/call.cpp:62-70`). Escape declines (`main.qml:1378-1383`). |
| Hang up | has | Per Call or `HangupAll` (`main.qml:586,636`; `agdevice.cpp:123`) |
| Mute | has | No mute method exists, so it writes `MicrophoneVolume=0` and restores the old value (`telephonyclient.cpp:591-609`). Whether every Phone honours this is unverified. |
| Keypad / DTMF | has | `SendTones` (`agdevice.cpp:129`; `main.qml:816`) |
| Dialer with Contacts search | partial | Keypad field in the main window with accent-folded name, T9 and number search (`contactsmodel.cpp:368-401`). The search does not include Call History. There is no separate Dialer window, but there is a `tel:` handler plus single-instance argument forwarding (`main.cpp:34-52,140,217-227`). |
| Call History | has | PBAP `ich`/`och`/`mch` (`recentsmodel.cpp:23-27,112-136`) |
| Call Window (timer, mute, keypad, Handoff, hang up) | partial | All of these are present, but inside the 420×640 main window (`main.qml:540-685`), not in a small window |
| Audio Location follows the device that answered | missing | Never sets `RejectSCO=true`. Only clears it (`agdevice.cpp:140`), so Android opens SCO to the Desktop for every Call (hfp-stack §5.1). |
| Handoff Phone → Desktop | has | `RejectSCO=false`, then `AudioGatewayTransport1.Activate()` (`agdevice.cpp:136-157`) |
| Handoff Desktop → Phone | missing | Removed on purpose (`docs/STATUS.md:52-57`; `main.qml:670-677`) |
| Audio on the Desktop's default devices | has | Implicit, via WirePlumber. No picker (issue #1). |
| Missed-call notification with Call back and Dismiss, marked in Call History | partial | Notification has no actions (`eventnotifier.cpp:23-29`). Marked "Missed" (`main.qml:1165-1202`). Declined Calls also count as missed (`main.cpp:162-166`). Withheld-number Calls are dropped entirely (`pendingendtracker.cpp:9`). |
| Call waiting: Hold & answer / End & answer / Decline, plus swap and merge | partial | `HoldAndAnswer`, `ReleaseAndAnswer`, `SwapCalls`, `CreateMultiparty` (`agdevice.cpp:124-128`; `main.qml:1415-1447,566-581`). Never exercised on a real Phone (`STATUS.md:39-41`). |
| DnD: Banner shows, no ring | missing | No DnD awareness |
| Out of range: quiet indicator, reconnects on its own | partial | Watches the service and ObjectManager, polls every 1.5 s (`telephonyclient.cpp:135-150`). Tray shows disconnected (`trayicon.cpp:84-86`) but also raises a "Phone disconnected" popup (`eventnotifier.cpp:89-92`). Never calls `Device1.Connect`. No suspend/resume handling. |
| First-run "Connect your phone" pairing | missing | Only launches an external Bluetooth settings app (`telephonyclient.cpp:425-489`). None of its candidates works here: `kcmshell6` is present, but bluedevil is not installed. |
| Contacts and Call History cached (user-only), refreshed on connect | partial | Contacts are cached at `~/.local/share/foner/phonebook.vcf` with default permissions, not user-only (`contactsmodel.cpp:125-149`). They are re-pulled only when the list is empty (`main.cpp:78`). Call History is not cached. |
| Earbuds connected to the Phone | missing | Not addressed |
| Autostart + top-bar indicator, click opens Dialer | partial | KStatusNotifierItem with ringing, in-call and disconnected states, and never Passive (`trayicon.cpp:70-90`). No autostart entry. Clicking the tray icon shows the main window. |

| Scorecard item ([polish-scorecard](../wiki/polish-scorecard.md)) | Foner |
|---|---|
| 1. Banner within 1 s | partial. Driven by `InterfacesAdded`, then `GetCalls`, with a 1.5 s poll as fallback (`telephonyclient.cpp:147-150,208-213`). It ignores the `CallAdded` signal its own notes recommend (`M0-selfio-notes.md:61-64`). Never measured. |
| 2. Audio within 1.5 s of Answer | partial. Never measured. |
| 3. Wideband with the Pixel 9a | partial. mSBC was seen on a Samsung. No Pixel tested and no codec shown. |
| 4. Pairing to first Call in 3 min or less | missing (no pairing flow) |
| 5. 20 Simulated Phone scenarios + Real-Phone Checklist | missing. Unit tests only. |
| 6. Reconnect within 10 s, survives suspend | partial (see the reconnect row above) |
| 7. Keyboard only | partial. The author's own audit says the keypad is unreachable by Tab (`STATUS.md:72-80`). |
| 8. No visual defects | partial. Designed against Plasma. Off-theme on Omarchy (§3). |

## 3. Fit with Hyprland/Omarchy

- **Plasma-only APIs: none [V].** It links Kirigami, KCoreAddons, KDBusAddons, KI18n, KContacts, KNotifications, KConfig, KStatusNotifierItem and KItemModels (`CMakeLists.txt:26-44`). These are all standalone frameworks, so the Qt Wayland app should start on Hyprland [I].
- **KNotifications works, but the actions are lost [V via omarchy-shell-integration §3-4].** Omarchy's notification server draws no action buttons, so the Answer and Decline buttons do not appear. The notifyrc's `Urgency=Critical` is silenced under DND: the Banner disappears, which is the opposite of our DND rule.
- **The tray works [V].** quickshell owns `org.kde.StatusNotifierWatcher`, and Foner never sets Passive. Items still land in the hover drawer unless the user pins them.
- **On a ringing Call, the 420×640 main window is shown and raised** (`main.qml:1452-1460`). On a tiling compositor this tiles a new window into the current workspace rather than drawing an overlay [I].
- **Theming [I].** The app forces the `org.kde.desktop` style (`main.cpp:117`). There is no `~/.config/kdeglobals` and `QT_QPA_PLATFORMTHEME=gtk3`, so expect Breeze defaults rather than the Omarchy theme.
- **Packaging [V].** Foner is not in `extra` or the AUR (AUR RPC search for "foner": 0 results). A PKGBUILD ships in `packaging/`. Installed here already: qt6-base 6.12, kirigami, knotifications, kconfig, ki18n, kdbusaddons, kitemmodels, kcoreaddons 6.31, qqc2-desktop-style. Missing: `kcontacts`, `kstatusnotifieritem` and `qt6-tools` (all in `extra`). Because of those, it was not built.

## 4. Reusable knowledge

1. **The Call1.Hangup anomaly.** `org.pipewire.Telephony.Call1.Hangup` fails with "Method doesn't exist" even though introspection lists it. The same object path also exports `org.ofono.VoiceCall`, whose `Answer` and `Hangup` work. A redundant Hangup mid-teardown returns "Too few arguments"; treat that as already ended (`M0-selfio-notes.md:31-56`; `call.cpp:12-13`). Re-verify on PipeWire 1.6.9 with the Simulated Phone and a real Phone.
2. **Finding Calls.** `GetManagedObjects` lists the AG but not Call objects. Use `org.ofono.VoiceCallManager.GetCalls` on the AG path (`telephonyclient.cpp:308-312`). It briefly returns empty while a Call is dialling or ringing, so subscribe to `CallAdded`/`CallRemoved`/`PropertyChanged` and debounce removals; Foner uses a 3 s grace period (`pendingendtracker.h:21`).
3. **Call path reuse.** Call paths are reused as `callN`, so clear all per-path state on removal (`telephonyclient.cpp:398`).
4. **Call states.** An answered incoming Call reports `active`, so store the direction separately (`telephonyclient.cpp:346-351`). A second incoming Call has state `waiting` (`callmodel.cpp:90`).
5. **Where hold and conference methods live.** They are on the AG interface, not the Call (`M0-selfio-notes.md:42-44`). The AG refuses `ReleaseAndAnswer` with no established Call, `SwapCalls` while the second Call is ringing, and `CreateMultiparty` with a ringing leg (`main.qml:566-581,1431-1436`). A merged Call cannot be split.
6. **Mute.** There is no mute method. Write `MicrophoneVolume=0`, and never remember -1 as the volume to restore (`telephonyclient.cpp:597-604`).
7. **Handoff.** Clear `RejectSCO` before calling `Activate()`, or the link is torn down again (`agdevice.cpp:138-140`). Transport `State` `idle`/`active` is the reliable Audio Location signal (`M0-selfio-notes.md:68`). There is no `Deactivate`. This is our ADR-0002 consequence, confirmed independently.
8. **Withheld caller ID.** PipeWire logs `spa.bluez5.native: Failed to parse number: "",129` and leaves `LineIdentification` empty (issue #1). Fall back to `IncomingLine`, and still record and notify a missed Call from a withheld number, which Foner does not.
9. **PBAP sessions.** Never run two obexd sessions to the same Phone at once; Foner serialises them (`main.cpp:58-63`). `PullAll` only starts a transfer; completion arrives as Transfer `Status` `complete` or `error` through `PropertiesChanged` (`pbapclient.cpp:100-122`).
10. **Call History cards.** History is three books. `X-IRMC-CALL-DATETIME;RECEIVED|DIALED|MISSED` overrides the book's direction. The time is local, `yyyyMMddTHHmmss`, with no zone. The same Call can appear in two books, so dedupe on number plus time. One unsupported book must not abort the others (`recentsmodel.cpp:29-38,54-58,181-185,212-217`).
11. **Number matching.** Foner matches on the last 9 digits, tuned for Algerian numbers (`contactsmodel.cpp:300-302`). Use real normalisation instead. Its accent folding and T9 code is a good test oracle (`contactsmodel.cpp:319-351`, `tests/test_contacts.cpp`).
12. **vCard encoding risk [I].** Its hand-written history parser ignores vCard 2.1 `QUOTED-PRINTABLE` and folded lines (`recentsmodel.cpp:145-187`). Request vCard 3.0, or decode both.
13. **Pixel/Android.** Nothing Pixel-specific. Foner adds that USSD replies appear only on the handset (`STATUS.md:39-41`).

## 5. Reasoning against "UX is king"

- **(a) Use Foner as is: no.** On Omarchy the Banner has no Answer button and vanishes under DND. Ringing tiles a full window into the workspace. Audio jumps to the Desktop for every Call. Handoff back is impossible, there is no pairing flow, and the theme is not Omarchy's. That fails scorecard items 4, 5 and 8 outright, and the v1 Audio Location rule.
- **(b) Contribute: poor fit.** The author tried GTK4/libadwaita, Tauri, Electron and Flutter versions and declared Kirigami "the real application" (`STATUS.md:122-135`). The project is Plasma-first, single-maintainer, with no replies on its issues. Our hardest work would be a layer-shell Banner, Omarchy DND, `RejectSCO` policy, Handoff back, guided pairing and a Simulated Phone. All of it sits outside its design and gets no easier inside it.
- **(c) Build: yes.** Nearly all of Foner's value is the D-Bus knowledge in §4, and that ports to zbus directly. Its UI and Qt plumbing do not port to Rust/GTK anyway.

**Next step:** add §4 items 1, 2, 5 and 8 as scenarios in the Simulated Phone plan, and re-verify item 1 against PipeWire 1.6.9 on the real Phone.
