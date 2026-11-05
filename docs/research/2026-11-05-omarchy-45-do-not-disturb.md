# Reading Omarchy 4.5's Do Not Disturb

Date: 2026-11-05. Ticket #96. Read-only on Omarchy 4.5.0-2; Simulated Phone and stand-ins only. Nothing under the user's `~/.config` or `~/.local/state` was written.
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** Desktop Calls now reads Do Not Disturb from `~/.local/state/omarchy/notifications/quiet.json`, the way Omarchy 4.5 reads it, and publishes it as the control interface's `DoNotDisturb`. The harness writes that file as Omarchy 4.5's shell does. Omarchy 4.0.6's `notifications.json` is dropped from both, because the spec amendment on #1 names only 4.5's file. Do Not Disturb doesn't silence the ringtone yet: that rule is #17's.

## Omarchy 4.5's file

Read from `/usr/share/omarchy/shell/plugins/notifications/` and `/usr/share/omarchy/migrations/`.

- **Where.** `Service.qml` `stateDir` is `Quickshell.env("HOME") + "/.local/state/omarchy/notifications"`: built from `HOME`, not `XDG_STATE_HOME`. The app reads it from `HOME` too. **[V]**
- **At shell startup.** `storePrepare` runs `umask 077`, `mkdir -p` and `chmod 700` on the directory, and creates `quiet.json` empty if it is missing. It replaces a symlink and sets an existing file to 0600. An empty file means off. The user's file is 0 bytes now. **[V]**
- **On a toggle.** `setDoNotDisturb` calls `saveSoon("quiet")`. A 250 ms timer then writes `JSON.stringify({ dnd: !!state.dnd }) + "\n"` (`Store.js` `serialiseQuiet`) through a `FileView` with `atomicWrites: true`. So the file is replaced by a rename, and a watcher has to watch the directory. **[V]** for the code. **[I]** for the exact temporary name Qt uses.
- **Reading.** The shell reads the file once at startup (`loadQuiet`). `Store.js` `parseQuiet` returns `null` for empty or whitespace-only text, text over 262,144 characters, invalid JSON, or anything that isn't a JSON object. Otherwise it returns `{ dnd: read.dnd === true }`. On `null` the shell keeps its default, off. **[V]**
- **Migration from 4.0.** `1792648436.sh` deletes `~/.local/state/omarchy/notifications.json`. If that file had `dnd: true` and `quiet.json` was empty, the script first writes `{"snoozes":{},"dnd":true,"silencedSince":0,"codesBypassQuiet":true}`. So a reader must allow extra fields. **[V]**

These verdicts come from running Omarchy's own `parseQuiet` under Node, extracted from `Store.js` by its text. They are the table in `crates/test-harness/tests/do_not_disturb.rs`. **[V]**

| `quiet.json` | Do Not Disturb |
| --- | --- |
| empty, or `"\n"` | off |
| `{"dnd":true}` and `{ "dnd" : true }` | on |
| the migration's object with `"dnd":true` | on |
| `{"version":3,"dnd":true}` (4.0's shape) | on |
| `{"dnd":"true"}` and `{"dnd":1}` | off |
| `[true]`, `null`, `true` | off |
| truncated `{"dnd":true` and `not json` | off |

## The app

- `crates/desktop-calls/src/do_not_disturb.rs` watches the directory with a GIO directory monitor (`WATCH_MOVES`) and reads `quiet.json` again on every event in it. It feeds `Input::DoNotDisturbChanged` to the Call state machine only when the value changes, and `DoNotDisturb` is published from the state. A missing file, or no `HOME`, means off. **[V]** (scenarios below)
- **Order matters.** The monitor has to start before the first read. In the first version the read came first, and a toggle written in between was lost every time in the toggle scenario (2 of 2 runs). With the monitor first, 5 of 5 runs passed. **[V]**
- **A directory that isn't there yet.** GLib's inotify backend accepts a monitor on a missing directory and looks for it every 4 s. The "Omarchy installed later" scenario saw the change in about 4 s. **[V]** (scenario timing: the file finishes in 4.0 to 4.7 s)
- **Kept across unpairing.** The state machine resets to its default when the Phone is unpaired. Do Not Disturb is a Desktop setting, so it is carried over. **[V]** (`crates/core/tests/do_not_disturb.rs`)

## The harness

- By default, every scenario starts with Omarchy's notification state as the shell prepares it: the directory at 0700 and an empty `quiet.json` at 0600. `Builder::do_not_disturb()` starts with `{"dnd":true}`, and the notification stand-in silences from the start. `Builder::no_omarchy_notification_state()` leaves the directory out. **[V]** (`crates/test-harness/tests/stand_ins.rs`)
- `Harness::set_do_not_disturb` writes `{"dnd":true}\n` or `{"dnd":false}\n` to `quiet.json.new` at 0600, then renames it over `quiet.json`. `Harness::replace_omarchy_quiet_file` does the same with any text, for the migration's output or a hand edit. Both live in `crates/test-harness/src/omarchy_quiet.rs`, beside the port of Omarchy's Do Not Disturb rules. **[V]**

## Follow-ups

- #17: no Desktop ringtone under Do Not Disturb. Now that `DoNotDisturb` is in the state, this is one condition in `State::ringing`, and its scenarios can use `Builder::do_not_disturb()` and `Harness::set_do_not_disturb`. **[I]**
- The wiki's omarchy 4.0.6 bullet in `hardware-and-stack.md` still names `notifications.json` as readable. The 4.5 bullet below it already supersedes it. **[V]**
- `omarchy-shell notifications isDnd` would give the shell's live value without the 250 ms save delay, but it means starting a process or using IPC on every check. The spec chose the file. **[I]**
