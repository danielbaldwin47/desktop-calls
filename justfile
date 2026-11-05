# The one entry point for Desktop Calls. Run `just --list` to see every recipe.

set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list

# Check formatting, lints, dependencies and file lengths, then build every target.
check: lint
    cargo build --workspace --all-targets --locked

# CI runs this rather than `just check`: its test jobs build every target
# between them, and building them again cost a minute (#118).
# `just check` without the build: formatting, lints, dependencies, file lengths, personal data.
lint: && _core-is-pure _dbus-set-is-signed _real-phone-tools-use-system-python _rust-files-fit _no-personal-data _tools-pass-their-tests _checklist-waits-on-open-tickets
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo deny --locked check

# Run the full Simulated Phone suite. Scenarios run the built binaries, so build them first.
# Screen scenarios need sway, grim and wl-clipboard: sudo pacman -S --needed sway grim wl-clipboard
# A failing scenario keeps its directory under $TMPDIR; KEEP_SCENARIO_LOGS=1 keeps passing ones too.
test *args:
    cargo build --workspace --bins --locked
    cargo test --workspace --locked {{args}}

# CI passes `--partition count:k/N` to run shard k of N; `just ci-local` runs
# them whole.
# Build the binaries the harness's scenarios run, then run them with cargo-nextest.
ci-scenarios *args:
    cargo build --workspace --bins --locked
    cargo nextest run -p test-harness --locked --no-fail-fast {{args}}

# Judged one scenario at a time as on the Desktop, which runs one app (issue
# #47): in the whole run, every scenario's app at once slows each first paint
# past its target. `just test-file` fails if a file has no ignored scenario
# left, rather than run nothing.
# Run the first-frame timing scenarios, one at a time.
ci-first-frame-timings:
    just test-file dialer_surface --ignored --test-threads=1
    just test-file incoming_call_surfaces --ignored --test-threads=1

# GitHub runs full CI only on pushes to main, so run this before every push.
# It tests HEAD in CI's image: lint, tests, gallery, scorecard and every
# scenario. If all pass, it pushes the branch if GitHub lacks the commit and
# posts the status "CI passed", which branch protection requires and `just
# merge` reads. Refuses a dirty tree. Runs from several agents take turns.
# Cache, image and logs: ~/.cache/desktop-calls-ci. `just ci-local --help`.
# Run CI here, and post "CI passed" on HEAD if it passes.
[positional-arguments]
[no-exit-message]
ci-local *args:
    @python3 tools/ci_local.py "$@"

# `just test-file contacts with_5000`; --ci-like runs them under CI's limits.
# Use it rather than `cargo test --test <file>`. `just test-file --help`.
# Run one harness test file, or only its scenarios whose names contain FILTER.
[positional-arguments]
[no-exit-message]
test-file *args:
    @python3 tools/test_file.py "$@"

# Run the tests under CI's limits, to reproduce a CI-only failure before
# pushing: a cgroup CPU quota, four scenarios at a time, no GPU, CI's
# /dev/shm. Takes a machine-wide lock, so runs from several agents take turns.
# Arguments as for `just test`. `just test-ci-like --help`.
[positional-arguments]
[no-exit-message]
test-ci-like *args:
    @python3 tools/test_ci_like.py "$@"

# Render every surface in every state, light and dark, stock and under an
# Omarchy theme, into the screenshot gallery at target/screenshots/gallery.
# Needs sway and grim. `just screenshot --only call-window-muted --repeat 5`
# redoes one state five times. `just screenshot --help` for options.
[positional-arguments]
[no-exit-message]
screenshot *args:
    cargo build --workspace --bins --locked
    target/debug/automation screenshot "$@"

# Report every polish-scorecard item with its status and evidence.
scorecard:
    @cargo build --quiet --workspace --bins --locked
    @target/debug/automation scorecard

# Run the app against the Simulated Phone in a nested compositor window,
# acting out SCENARIO. Close the window or press Ctrl-C to stop. Needs sway.
[no-exit-message]
demo scenario="crates/simulated-phone/scenarios/demo.scenario":
    @cargo build --quiet --workspace --bins --locked
    @target/debug/automation demo {{scenario}}

# Place one real Call to the Test Number, run HOOK during it, always hang up.
# Stops the installed desktop-calls.service for the run and starts it again
# afterwards, so HOOK may start a dev run of the app (ADR-0006).
# `just test-call --help` for options. Exits 2 if a Call may still be live.
[positional-arguments]
[no-exit-message]
test-call *args:
    @/usr/bin/python3 tools/real-phone/test_call.py "$@"

# Run this checkout's app on the real Phone until Ctrl-C: a dev run, in place
# of the installed desktop-calls.service, which is stopped for the run and
# started again afterwards, even if the run is killed (ADR-0006). Refuses
# while the Phone has a Call. `just run --help`.
[positional-arguments]
[no-exit-message]
run *args:
    @cargo build --quiet --locked -p desktop-calls
    @/usr/bin/python3 tools/real-phone/run.py target/debug/desktop-calls "$@"

# Read the Phone's Contacts and Call History over PBAP and report only their
# shape, never a name or number. Places no Call. Have the Phone unlocked and
# in hand. The report goes to ~/.cache/desktop-calls/. `just pbap-probe --help`.
[positional-arguments]
[no-exit-message]
pbap-probe *args:
    @/usr/bin/python3 tools/real-phone/pbap_probe.py "$@"

# The only way to merge: it checks more than branch protection's "CI passed",
# which `just ci-local` posts. `just merge --help` for options and the
# conditions it checks.
# Squash-merge PR once its checks pass on a head containing origin/main.
[positional-arguments]
[no-exit-message]
merge pr *args:
    @python3 tools/merge.py "$@"

# Wait on one GitHub CI run (default: the latest for this branch; only pushes
# to main and `gh workflow run CI` start one). If it passed, print one line. If not, print each failed test's panic and download the
# scenario logs into target/ci/<run-id>/. `just ci-wait --help`.
[positional-arguments]
[no-exit-message]
ci-wait *args:
    @python3 tools/ci_wait.py "$@"

# Walk through the Real-Phone Checklist one step at a time: all groups, or
# one (`just real-phone-checklist 2`). Run it in your own terminal: it asks
# questions. Results go to ~/.local/state/desktop-calls/real-phone-checklist/;
# a rerun resumes at the first step that didn't pass. `--demo` runs it
# against the Simulated Phone instead. `just real-phone-checklist --help`.
[positional-arguments]
[no-exit-message]
real-phone-checklist *args:
    @cargo build --quiet --workspace --bins --locked
    @target/debug/automation real-phone-checklist "$@"

# Install for this user: the binary in ~/.local/bin, a systemd user unit that
# starts with the graphical session and restarts after a crash, D-Bus
# activation and the desktop file (launcher entry, tel: links). Then starts
# it, now and at every login. With DESTDIR=dir, only stages the files there.
[no-exit-message]
install:
    cargo build --release --locked -p desktop-calls
    /usr/bin/python3 tools/install.py target/release/desktop-calls
    if [ -z "${DESTDIR:-}" ]; then \
        systemctl --user daemon-reload && \
        systemctl --user enable desktop-calls.service && \
        systemctl --user restart desktop-calls.service; \
    fi

# Stop Desktop Calls and remove what `just install` installed. Contacts and
# Call History cached in ~/.cache/desktop-calls stay.
[no-exit-message]
uninstall:
    if [ -z "${DESTDIR:-}" ]; then \
        systemctl --user disable --now desktop-calls.service || true; \
    fi
    /usr/bin/python3 tools/install.py --uninstall
    if [ -z "${DESTDIR:-}" ]; then systemctl --user daemon-reload; fi

# A Real-Phone Checklist step without a script names the open tickets it
# waits on. Once one closes, the step needs its script, or the runner tells
# the user it waits on a ticket long closed (ticket #120). Asks GitHub, and
# fails if it can't; offline, DESKTOP_CALLS_OFFLINE=1 skips it knowingly.
# `just merge` also refuses a PR that closes a ticket a step still names.
[private]
[no-exit-message]
_checklist-waits-on-open-tickets:
    @python3 tools/test_checklist_tickets.py --quiet
    @python3 tools/checklist_tickets.py

# The core does no I/O and knows no GTK, D-Bus or logging, so none of those
# crates may appear anywhere in its dependency tree.
[private]
[no-exit-message]
_core-is-pure:
    #!/usr/bin/env bash
    set -euo pipefail
    forbidden='^(gtk4|gdk4|gsk4|libadwaita|gtk4-layer-shell|glib|gio|zbus|tokio|async-std|tracing|log|libc|nix|rustix) '
    if found=$(cargo tree --locked -p desktop-calls-core -e normal,build --prefix none | grep -E "$forbidden"); then
        echo "desktop-calls-core must stay pure (CODING_STANDARDS.md), but depends on:" >&2
        echo "$found" | sort -u >&2
        exit 1
    fi

# The repo's Python tools pass their own tests, tools/*_test.py.
[private]
_tools-pass-their-tests:
    python3 -B -m unittest discover --start-directory tools --pattern '*_test.py' --quiet

# An agent that reads a whole oversized file spends its context on it: the
# harness's 2,442-line lib.rs took 88k of the 276k characters one agent read
# before its context was compacted (ticket #79). So no .rs file in the repo,
# tracked or new, may be over 1,000 lines. Split one along its seams.
# It walks the tree rather than asking git, because in CI's container git
# refuses a checkout another user owns (exit 128, "dubious ownership").
# It skips build output (target) and hidden directories (.git, and the agent
# worktrees under .claude, which are other checkouts).
[private]
[no-exit-message]
_rust-files-fit:
    #!/usr/bin/env python3
    import os, sys
    LIMIT = 1000
    over = []
    for directory, subdirectories, files in os.walk("."):
        subdirectories[:] = sorted(d for d in subdirectories if d != "target" and not d.startswith("."))
        for name in sorted(files):
            if not name.endswith(".rs"):
                continue
            path = os.path.relpath(os.path.join(directory, name))
            with open(path, encoding="utf-8") as file:
                lines = sum(1 for _ in file)
            if lines > LIMIT:
                over.append(f"  {path}: {lines:,} lines")
    if over:
        print(f"Rust files over {LIMIT:,} lines (CODING_STANDARDS.md); split each along its seams:", file=sys.stderr)
        print("\n".join(over), file=sys.stderr)
        sys.exit(1)

# A D-Bus property Set without an explicit signature lets the library guess
# one. On 2026-11-04 a guessed `ssb` crashed WirePlumber mid-Call (ticket #3).
# So every `Set(` call under tools/ must carry `signature=` inside its
# parentheses. Use `set_property` in tools/real-phone/real_phone.py.
[private]
[no-exit-message]
_dbus-set-is-signed:
    #!/usr/bin/env python3
    import pathlib, re, sys
    unsigned = []
    for path in sorted(pathlib.Path("tools").rglob("*")):
        if not path.is_file():
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        for match in re.finditer(r"\bSet\s*\(", text):
            depth, end = 1, match.end()
            while end < len(text) and depth:
                depth += {"(": 1, ")": -1}.get(text[end], 0)
                end += 1
            if not re.search(r"\bsignature\s*=", text[match.end():end]):
                line = text.count("\n", 0, match.start()) + 1
                unsigned.append(f"{path}:{line}")
    if unsigned:
        print("D-Bus Set( without an explicit signature= (use set_property in", file=sys.stderr)
        print("tools/real-phone/real_phone.py, or pass signature=\"ssv\"):", file=sys.stderr)
        for where in unsigned:
            print(f"  {where}", file=sys.stderr)
        sys.exit(1)

# The real-Phone tools need dbus-python and PyGObject, which only the system
# Python has: a bare `python3` may be mise's or a virtualenv's, and fails with
# `No module named 'dbus'` (ticket #54). So in the justfile and in every file
# under tools/real-phone, a real-Phone tool runs only under /usr/bin/python3,
# and every shebang there is `#!/usr/bin/python3`.
[private]
[no-exit-message]
_real-phone-tools-use-system-python:
    #!/usr/bin/env python3
    import pathlib, re, sys
    # A bare python3 (or python, or env python3) followed by a real-Phone tool:
    # any python run on a real-Phone tool, unless it is /usr/bin/python3.
    bare_run = re.compile(r"(?<!/usr/bin/)(?<![\w/.-])python3?(?:\.\d+)?\s+(?:\S*/)?tools/real-phone/")
    files = [pathlib.Path("justfile"), *sorted(pathlib.Path("tools/real-phone").rglob("*"))]
    found = []
    for path in files:
        if not path.is_file():
            continue
        try:
            lines = path.read_text().splitlines()
        except UnicodeDecodeError:
            continue
        for number, line in enumerate(lines, 1):
            wrong_shebang = (path.parent.name == "real-phone" and number == 1
                             and line.startswith("#!") and line != "#!/usr/bin/python3")
            if wrong_shebang or bare_run.search(line):
                found.append(f"  {path}:{number}: {line.strip()}")
    if found:
        print("Real-Phone tools must run under /usr/bin/python3, the only Python with", file=sys.stderr)
        print("dbus-python (ticket #54). Use `#!/usr/bin/python3` and `/usr/bin/python3 tools/real-phone/...`:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        sys.exit(1)

# The repository is public, so no file in it, tracked or new, may carry:
# - a Bluetooth address that isn't a placeholder. Placeholders are
#   AA:BB:CC:DD:EE:FF, 00:11:22:33:44:55, 11:22:33:44:55:66, all zeros, all
#   Fs, and any address whose 4th and 5th octets are 00 (the Simulated
#   Phone's `XX:XX:XX:00:00:NN`), with `:`, `_` or `-` between octets;
# - a home directory (`/home/<user>`, or `-home-<user>-` in a Claude path);
# - the Test Number, when ~/.config/desktop-calls/test-number exists. It
#   matches the last 7 digits with any separators, and never prints them.
# It asks git for the file list, trusting the checkout so CI's container
# (another user owns it) doesn't refuse with "dubious ownership".
[private]
[no-exit-message]
_no-personal-data:
    #!/usr/bin/env python3
    import os, pathlib, re, subprocess, sys
    names = subprocess.run(
        ["git", "-c", "safe.directory=*", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        capture_output=True, check=True, text=True).stdout.split("\0")
    # Six hex octets joined by one separator, used throughout. Written out,
    # because just reads a doubled brace as its own interpolation.
    octet = "[0-9A-Fa-f][0-9A-Fa-f]"
    address = re.compile(r"(?<![0-9A-Za-z])" + octet + r"([:_-])" + (r"(?:" + octet + r"\1)") * 4 + octet + r"(?![0-9A-Za-z])")
    placeholders = {"AA:BB:CC:DD:EE:FF", "00:11:22:33:44:55", "11:22:33:44:55:66", "00:00:00:00:00:00", "FF:FF:FF:FF:FF:FF"}
    home = re.compile(r"(?<![\w.])/home/(?!user\b|<user>)[A-Za-z0-9_][\w.-]*|-home-[a-z_][a-z0-9_]*-")
    config = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    number_file = pathlib.Path(config) / "desktop-calls" / "test-number"
    test_number = None
    if number_file.is_file():
        digits = re.sub(r"\D", "", number_file.read_text())[-7:]
        if len(digits) == 7:
            test_number = re.compile(r"[\s.()/-]{0,2}".join(digits) + r"(?!\d)")
    found = []
    for name in filter(None, names):
        path = pathlib.Path(name)
        if not path.is_file():
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        def line_of(match):
            return f"{name}:{text.count(chr(10), 0, match.start()) + 1}"
        for match in address.finditer(text):
            octets = re.split(r"[:_-]", match.group(0).upper())
            if ":".join(octets) not in placeholders and octets[3:5] != ["00", "00"]:
                found.append(f"  {line_of(match)}: Bluetooth address {match.group(0)} (use AA:BB:CC:DD:EE:FF)")
        for match in home.finditer(text):
            found.append(f"  {line_of(match)}: home directory (use ~ or /home/<user>)")
        if test_number:
            for match in test_number.finditer(text):
                found.append(f"  {line_of(match)}: the Test Number (never commit it)")
    if found:
        print("Personal data in a public repository; replace each with a placeholder:", file=sys.stderr)
        print("\n".join(found), file=sys.stderr)
        sys.exit(1)
