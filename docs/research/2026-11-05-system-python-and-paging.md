# Real-Phone tools on the system Python, and the Call History paging probe

For #54. Labels: **[V]** verified here; **[I]** inferred.

## Which Python

- On the user's Desktop, `python3` on `PATH` is mise's shim (`~/.local/share/mise/shims/python3`), which has no `dbus` module. Only `/usr/bin/python3` has dbus-python and PyGObject. **[V]**
- `just` is a mise shim too. Run with a temporary `HOME`, as the Real-Phone Checklist's `--demo` runs everything, it fails with `Config files in ~/.config/mise/config.toml are not trusted`. So the runner runs `/usr/bin/python3 tools/real-phone/pbap_probe.py` itself, which is what `just pbap-probe` runs, rather than `just`. **[V]**
- Every entry point under `tools/real-phone` now starts `#!/usr/bin/python3` and is executable. `just check` rejects any other shebang there, and any bare `python3 tools/real-phone/…` in the justfile or under `tools/real-phone`. **[V]**
- CI's Arch container needs `python-dbus` and `python-gobject` for the checklist's group 1 demo test, which runs the probe. **[V]** for the Desktop; CI is proven by this PR's run.

## `just pbap-probe`

- The report and the D-Bus snapshot go to one file, `~/.cache/desktop-calls/pbap-probe-<time>.json`, under the keys `report` and `dbus snapshot`. obexd writes the pulled folders to a private `pbap-probe-*` directory there, deleted at the end. The terminal shows one line per pull, the paging verdict and the file's path, no longer the whole report. **[V]** against the Simulated Phone.
- obexd's user unit (`/usr/lib/systemd/user/obex.service`) has no sandboxing, so obexd can write under `~/.cache`. **[V]** for the unit; **[I]** that the real pull writes there, because the real Phone was in use by #13.
- After the five folders, the probe pulls `cch` again with `Offset` 50 and `MaxCount` 50 (both `q`), and reports page 2's entry count, how many of its entries are on page 1 too, and whether all of them are older than page 1's (by `X-IRMC-CALL-DATETIME`). The verdict is "honours Offset" unless page 2 is mostly page 1 again, is empty while `GetSize` exceeds 50, or isn't older than page 1. **[V]** against the Simulated Phone (120 Calls: 50 on page 2, none on page 1, all older).

## Real-Phone Checklist step 1.4

- Group 1 gains 1.4, "Read a second page of Call History: the Pixel honours paging". It runs the probe, shows its progress, and passes only if the Phone honours `Offset` counting from the newest Call. It asks the user nothing. **[V]** by `group_1_reads_a_second_page_of_call_history_and_reports_whether_the_phone_honours_paging`.

## Next

- With the Pixel in hand: `just pbap-probe`, or `just real-phone-checklist 1`. This settles the **[I]** in docs/research/2026-11-04-contacts-sync.md on whether `Offset` counts from the newest Call.
