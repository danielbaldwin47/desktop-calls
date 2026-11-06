//! Reading the app's log file while the app may still be writing it.
//!
//! The app appends one JSON object and a newline per record, and a reader
//! can land between the first byte and the last of one: no promise says a
//! regular file's write is seen whole. A line that doesn't end in `\n` isn't
//! finished, so it isn't read yet; the next read, which starts from the top
//! of the file again, finds it finished (ticket #180).

use std::fs;
use std::path::Path;

/// The app's log at `path` so far, as far as its last finished line: empty
/// before the app has logged anything.
pub(crate) fn read(path: &Path) -> String {
    let mut log = fs::read_to_string(path).unwrap_or_default();
    log.truncate(log.rfind('\n').map_or(0, |last| last + 1));
    log
}

/// Every event in `log`, as [`read`] returns it.
///
/// # Panics
///
/// If a finished line isn't JSON.
pub(crate) fn events(log: &str) -> Vec<serde_json::Value> {
    log.lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{line:?} isn't JSON: {error}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINISHED: &str = "{\"event_kind\":\"service-started\"}\n";
    const PARTIAL: &str = "{\"command\":\"KeepA";

    #[test]
    fn a_line_the_app_has_not_finished_is_not_read_yet() {
        let directory = tempfile::tempdir().expect("make a directory");
        let path = directory.path().join("desktop-calls.log");
        fs::write(&path, format!("{FINISHED}{PARTIAL}")).expect("write the log");

        let events = events(&read(&path));

        assert_eq!(events.len(), 1, "only the finished line is an event");
        assert_eq!(events[0]["event_kind"], "service-started");
    }

    #[test]
    fn the_unfinished_line_is_read_once_the_app_finishes_it() {
        let directory = tempfile::tempdir().expect("make a directory");
        let path = directory.path().join("desktop-calls.log");
        fs::write(&path, format!("{FINISHED}{PARTIAL}")).expect("write the log");
        let _ = read(&path);

        fs::write(
            &path,
            format!("{FINISHED}{{\"command\":\"KeepAudioOnPhone\"}}\n"),
        )
        .expect("the app finishes the line");
        let events = events(&read(&path));

        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["command"], "KeepAudioOnPhone");
    }

    #[test]
    #[should_panic(expected = "isn't JSON")]
    fn a_finished_line_that_isnt_json_still_panics() {
        let directory = tempfile::tempdir().expect("make a directory");
        let path = directory.path().join("desktop-calls.log");
        fs::write(&path, format!("{FINISHED}not json\n{PARTIAL}")).expect("write the log");

        let _ = events(&read(&path));
    }
}
