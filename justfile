# The one entry point for Desktop Calls. Run `just --list` to see every recipe.

set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list

# Check formatting, lints, the build and dependencies. CI runs this.
check: && _core-is-pure
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo build --workspace --all-targets --locked
    cargo deny --locked check

# Run the full Simulated Phone suite.
[no-exit-message]
test:
    @echo "just test: not available yet (lands with ticket #4)." >&2; exit 1

# Render every surface in every state into the screenshot gallery.
[no-exit-message]
screenshot:
    @echo "just screenshot: not available yet (lands with ticket #26)." >&2; exit 1

# Report every polish-scorecard item with its evidence.
[no-exit-message]
scorecard:
    @echo "just scorecard: not available yet (lands with tickets #5 and #27)." >&2; exit 1

# Run the app against the Simulated Phone in a visible nested compositor.
[no-exit-message]
demo:
    @echo "just demo: not available yet (lands with ticket #5)." >&2; exit 1

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
