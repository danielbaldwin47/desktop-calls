# The one entry point for Desktop Calls. Run `just --list` to see every recipe.

set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list

# Check formatting, lints, the build and dependencies. CI runs this.
check: && _core-is-pure _dbus-set-is-signed _real-phone-tools-use-system-python
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo deny --locked check

# Run the full Simulated Phone suite. Scenarios run the built binaries, so build them first.
# Screen scenarios need sway, grim and wl-clipboard: sudo pacman -S --needed sway grim wl-clipboard
# A failing scenario keeps its directory under $TMPDIR; KEEP_SCENARIO_LOGS=1 keeps passing ones too.
test *args:
    cargo build --workspace --bins --locked
    cargo test --workspace --locked {{args}}

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
# Needs sway and grim.
screenshot:
    cargo build --workspace --bins --locked
    target/debug/automation screenshot

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
# `just test-call --help` for options. Exits 2 if a Call may still be live.
[positional-arguments]
[no-exit-message]
test-call *args:
    @/usr/bin/python3 tools/real-phone/test_call.py "$@"

# Read the Phone's Contacts and Call History over PBAP and report only their
# shape, never a name or number. Places no Call. Have the Phone unlocked and
# in hand. The report goes to ~/.cache/desktop-calls/. `just pbap-probe --help`.
[positional-arguments]
[no-exit-message]
pbap-probe *args:
    @/usr/bin/python3 tools/real-phone/pbap_probe.py "$@"

# The only way to merge: GitHub can't enforce green checks on this repo.
# `just merge --help` for options and the conditions it checks.
# Squash-merge PR once its checks pass on a head containing origin/main.
[positional-arguments]
[no-exit-message]
merge pr *args:
    @python3 tools/merge.py "$@"

# Wait on one CI run (default: the latest for this branch). If it passed,
# print one line. If not, print each failed test's panic and download the
# scenario logs into target/ci/<run-id>/. `just ci-wait --help`.
[positional-arguments]
[no-exit-message]
ci-wait *args:
    @python3 tools/ci_wait.py "$@"

# Walk through the Real-Phone Checklist one step at a time: all groups, or
# one (`just real-phone-checklist 2`). Results go to docs/real-phone-checklist/;
# a rerun resumes at the first step that didn't pass. `--demo` runs it
# against the Simulated Phone instead. `just real-phone-checklist --help`.
[positional-arguments]
[no-exit-message]
real-phone-checklist *args:
    @cargo build --quiet --workspace --bins --locked
    @target/debug/automation real-phone-checklist "$@"

# Install for this user: binary, systemd user unit, D-Bus activation and desktop file.
[no-exit-message]
install:
    @echo "just install: not available yet (lands with ticket #20)." >&2; exit 1

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
