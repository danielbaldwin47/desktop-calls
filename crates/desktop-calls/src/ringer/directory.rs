//! Where the Ringer writes the Ring Sounds it plays: the Ringer's own
//! folder in the cache, `$XDG_CACHE_HOME/desktop-calls/ringer` (`~/.cache`
//! by default); or, when that can't be written, one in the runtime
//! folder, `$XDG_RUNTIME_DIR/desktop-calls-ringer`, so an unwritable cache
//! never silences a Call.

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

/// The folders to try, in order: those whose variables are set to an
/// absolute path.
pub(super) fn candidates() -> Vec<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let cache =
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")));
    [
        cache.map(|cache| cache.join("desktop-calls/ringer")),
        absolute("XDG_RUNTIME_DIR").map(|runtime| runtime.join("desktop-calls-ringer")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Writes each of `files` (a name and its bytes) into the first of the
/// [`candidates`] that takes them all, and returns that folder. A file
/// already there with the same bytes is left as it is.
pub(super) fn write_all(files: &[(&str, &[u8])]) -> io::Result<PathBuf> {
    let mut failure = io::Error::other("neither XDG_CACHE_HOME, HOME nor XDG_RUNTIME_DIR is set");
    for directory in candidates() {
        let written = DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .and_then(|()| {
                files
                    .iter()
                    .try_for_each(|(name, bytes)| write(&directory.join(name), bytes))
            });
        match written {
            Ok(()) => return Ok(directory),
            Err(error) => {
                tracing::debug!(
                    directory = %directory.display(),
                    error = %error,
                    "Couldn't write the Ring Sound here; trying the next folder"
                );
                failure = error;
            }
        }
    }
    Err(failure)
}

/// Writes `bytes` to `path`, first beside it, then into place, so a player
/// never reads half a file. A copy beside it that couldn't be put in place
/// is removed, so a failed write leaves nothing behind.
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|kept| kept == bytes) {
        return Ok(());
    }
    let partial = path.with_extension(format!(
        "{}.{}",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("part"),
        std::process::id()
    ));
    let written = fs::write(&partial, bytes).and_then(|()| fs::rename(&partial, path));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}
