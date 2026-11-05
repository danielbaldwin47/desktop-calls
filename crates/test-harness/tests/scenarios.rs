//! Every scenario file in this directory, compiled as one test binary: each
//! file is a module, listed by `build.rs`. A test's full name starts with
//! its file's name, such as `contacts::with_5000_contacts_the_banner_still_shows_within_a_second`.
//!
//! Run one file with `just test-file contacts`, or one scenario with
//! `just test-file contacts with_5000`.

include!(concat!(env!("OUT_DIR"), "/scenario_files.rs"));
