//! The pure core of Desktop Calls.
//!
//! It will hold the Call state machine, matching and messages: every policy
//! decision, as functions from inputs to outputs. It performs no I/O, reads no
//! clock or environment, and knows nothing of GTK, D-Bus or logging. Shell
//! adapters in the `desktop-calls` crate turn the outside world into inputs and
//! carry out the effects. See `CODING_STANDARDS.md`.
#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]
