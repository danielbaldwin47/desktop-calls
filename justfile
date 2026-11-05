# The one entry point for Desktop Calls. Run `just --list` to see every recipe.

set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list

# Check formatting, lints, the build and dependencies. CI runs this.
check: && _core-is-pure _dbus-set-is-signed
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo deny --locked check

# Run the full Simulated Phone suite. Scenarios run the built binaries, so build them first.
# Screen scenarios need sway and grim: sudo pacman -S --needed sway grim
test *args:
    cargo build --workspace --bins --locked
    cargo test --workspace --locked {{args}}

# Render every surface in every state, light and dark, into the screenshot
# gallery at target/screenshots/gallery. Needs sway and grim.
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
    @python3 tools/real-phone/test_call.py "$@"

# Walk through the Real-Phone Checklist one step at a time.
[no-exit-message]
real-phone-checklist:
    @echo "just real-phone-checklist: not available yet (lands with ticket #24)." >&2; exit 1

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
