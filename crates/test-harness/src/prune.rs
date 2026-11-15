//! Removing the scenario folders earlier test processes left behind.
//!
//! A failing scenario keeps its folder as evidence, and a killed test
//! process never reaches `Harness::drop`, so nothing else removes them.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Removes the folders in `directory` whose names start with `prefix` and
/// that were last modified more than `max_age` before `now`. A folder that
/// can't be read or removed, because another user owns it or it vanished
/// meanwhile, is skipped: pruning never fails a scenario.
pub(crate) fn prune_older_than(directory: &Path, prefix: &str, max_age: Duration, now: SystemTime) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let is_ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(prefix));
        // `file_type` doesn't follow a symbolic link, so a link to a folder
        // is not removed along with its target.
        let is_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        let is_old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age > max_age));
        if is_ours && is_folder && is_old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::*;

    const DAY: Duration = Duration::from_hours(24);

    /// Makes a folder called `name` in `directory`, last modified `age` ago.
    fn folder_aged(directory: &std::path::Path, name: &str, age: Duration) {
        let path = directory.join(name);
        fs::create_dir(&path).expect("make a folder");
        fs::File::open(&path)
            .and_then(|folder| folder.set_modified(SystemTime::now() - age))
            .expect("set the folder's modification time");
    }

    #[test]
    fn only_an_old_folder_with_the_prefix_is_removed() {
        let directory = tempfile::tempdir().expect("make a directory");
        folder_aged(directory.path(), "desktop-calls-test-old", 2 * DAY);
        folder_aged(directory.path(), "desktop-calls-test-fresh", DAY / 2);
        folder_aged(directory.path(), "unrelated-old", 2 * DAY);
        fs::write(directory.path().join("desktop-calls-test-file"), "x").expect("make a file");

        prune_older_than(
            directory.path(),
            "desktop-calls-test-",
            DAY,
            SystemTime::now(),
        );

        let mut left: Vec<_> = fs::read_dir(directory.path())
            .expect("list the directory")
            .map(|entry| entry.expect("read an entry").file_name())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "desktop-calls-test-file",
                "desktop-calls-test-fresh",
                "unrelated-old"
            ]
        );
    }

    #[test]
    fn a_directory_that_is_not_there_is_left_alone() {
        let directory = tempfile::tempdir().expect("make a directory");
        prune_older_than(
            &directory.path().join("missing"),
            "desktop-calls-test-",
            DAY,
            SystemTime::now(),
        );
    }
}
