//! Reading the app's log file while the app may still be writing it.
//!
//! The app appends one JSON object and a newline per record, and a reader
//! can land between the first byte and the last of one: no promise says a
//! regular file's write is seen whole. While the app has the file open, a
//! line that doesn't end in `\n` isn't finished, so it isn't read yet; the
//! next read, which starts from the top of the file again, finds it
//! finished (ticket #180).
//!
//! Once nothing has the file open, the app has exited and nothing will
//! finish the last line, so it is read as it is: a kept log read afterwards
//! loses nothing.

use std::fs;
use std::path::Path;

/// The app's log at `path` so far: empty before the app has logged
/// anything. While the app is writing, only as far as its last finished
/// line.
pub(crate) fn read(path: &Path) -> String {
    let mut bytes = fs::read(path).unwrap_or_default();
    if bytes.last().is_some_and(|&last| last != b'\n') {
        if is_open(path) {
            // Cut on bytes: a write cut inside a multi-byte character isn't
            // UTF-8 yet, and the newline byte never occurs inside one.
            bytes.truncate(
                bytes
                    .iter()
                    .rposition(|&byte| byte == b'\n')
                    .map_or(0, |last| last + 1),
            );
        } else {
            // The app left between the read and the question, so the file
            // is as it will stay: read it again, whole.
            bytes = fs::read(path).unwrap_or_default();
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether any process has the file at `path` open. Processes this user
/// can't look into can't be the app, which runs as this user.
fn is_open(path: &Path) -> bool {
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    let Ok(processes) = fs::read_dir("/proc") else {
        return false;
    };
    processes
        .filter_map(Result::ok)
        .filter(|process| {
            process
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|process| fs::read_dir(process.path().join("fd")).ok())
        .flatten()
        .filter_map(Result::ok)
        .any(|descriptor| fs::read_link(descriptor.path()).is_ok_and(|target| target == path))
}

/// Every event in `log`, as [`read`] returns it.
///
/// # Panics
///
/// If a line isn't JSON.
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
    use std::fs::{File, OpenOptions};
    use std::io::Write as _;
    use std::path::PathBuf;

    use super::*;

    const FINISHED: &str = "{\"event_kind\":\"service-started\"}\n";
    const PARTIAL: &str = "{\"command\":\"KeepA";

    /// A log the test holds open for append, as the app does.
    struct Log {
        directory: tempfile::TempDir,
        path: PathBuf,
        file: File,
    }

    fn app_log() -> Log {
        let directory = tempfile::tempdir().expect("make a directory");
        let path = directory.path().join("desktop-calls.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open the log");
        Log {
            directory,
            path,
            file,
        }
    }

    impl Log {
        fn write(&mut self, bytes: &[u8]) {
            self.file.write_all(bytes).expect("write the log");
        }

        /// The app exits.
        /// Its path, and the directory to keep until the log is read, as a
        /// kept scenario's is.
        fn close(self) -> (PathBuf, tempfile::TempDir) {
            drop(self.file);
            (self.path, self.directory)
        }
    }

    #[test]
    fn a_line_the_app_has_not_finished_is_not_read_yet() {
        let mut log = app_log();
        log.write(format!("{FINISHED}{PARTIAL}").as_bytes());

        let events = events(&read(&log.path));

        assert_eq!(events.len(), 1, "only the finished line is an event");
        assert_eq!(events[0]["event_kind"], "service-started");
    }

    #[test]
    fn the_unfinished_line_is_read_once_the_app_appends_the_rest() {
        let mut log = app_log();
        log.write(format!("{FINISHED}{PARTIAL}").as_bytes());
        assert_eq!(events(&read(&log.path)).len(), 1);

        log.write(b"udioOnPhone\"}\n");
        let events = events(&read(&log.path));

        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["command"], "KeepAudioOnPhone");
    }

    #[test]
    fn a_last_line_the_app_left_unterminated_is_read_once_it_has_exited() {
        let mut log = app_log();
        log.write(format!("{FINISHED}{{\"command\":\"KeepAudioOnPhone\"}}").as_bytes());
        assert_eq!(
            events(&read(&log.path)).len(),
            1,
            "while the app has the log open, the last line may be unfinished"
        );

        let (path, _kept) = log.close();
        let events = events(&read(&path));

        assert_eq!(events.len(), 2, "nothing will finish it now");
        assert_eq!(events[1]["command"], "KeepAudioOnPhone");
    }

    #[test]
    fn a_write_cut_inside_a_character_leaves_the_finished_lines_readable() {
        let mut log = app_log();
        let mut bytes = FINISHED.as_bytes().to_vec();
        bytes.extend_from_slice(b"{\"contact_name\":\"Zo");
        bytes.push(0xC3); // the first byte of the two-byte "\u{e9}"
        log.write(&bytes);

        let events = events(&read(&log.path));

        assert_eq!(events.len(), 1, "only the finished line is an event");
    }

    #[test]
    fn a_log_with_no_finished_line_reads_as_empty() {
        let directory = tempfile::tempdir().expect("make a directory");
        let missing = directory.path().join("desktop-calls.log");
        assert_eq!(read(&missing), "", "the app hasn't made the file yet");

        let mut log = app_log();
        assert_eq!(read(&log.path), "");
        log.write(PARTIAL.as_bytes());

        assert_eq!(read(&log.path), "");
    }

    #[test]
    #[should_panic(expected = "isn't JSON")]
    fn a_finished_line_that_isnt_json_still_panics() {
        let mut log = app_log();
        log.write(format!("{FINISHED}not json\n{PARTIAL}").as_bytes());

        let _ = events(&read(&log.path));
    }
}
