//! One process group for everything a scenario starts, so that nothing
//! outlives the scenario, not even when its test process is killed.
//!
//! The group's leader is a guard: a shell that reads its standard input from
//! a pipe only the harness holds, and kills its whole group when the pipe
//! closes. The pipe closes when the [`ProcessGroup`] is dropped, and also
//! when the harness's process dies for any reason, SIGKILL included, since
//! the kernel closes a dead process's files. Every process the scenario
//! starts joins the guard's group, and so do their own children.
//!
//! A group rather than `PR_SET_PDEATHSIG`: the kernel clears the parent-death
//! signal when a process executes a file with capabilities (Arch's sway has
//! `cap_sys_nice`) and when it enters a user namespace (the app runs under
//! `unshare`), and it fires when the spawning thread exits, not the process.
//! Setting it also needs `pre_exec`, which is `unsafe`.
//!
//! Before #43, a timed-out or killed test left its private buses behind:
//! 46 `dbus-daemon` processes were found orphaned on the Desktop.

use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// What the guard runs: wait for the pipe to close, then kill its group,
/// itself last.
const GUARD: &str = "read -r _ignored; kill -s KILL 0";

/// A process group led by a guard. Dropping it kills every process still in
/// the group.
pub(crate) struct ProcessGroup {
    guard: Child,
    id: i32,
}

impl ProcessGroup {
    /// Starts the guard, in a new process group of its own. Its command
    /// line names the scenario's `directory`, for `ps`.
    pub fn start(directory: &Path) -> Self {
        let guard = Command::new("sh")
            .args(["-c", GUARD, "scenario-guard"])
            .arg(directory)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the scenario's guard process");
        let id = i32::try_from(guard.id()).expect("a process ID fits in an i32");
        Self { guard, id }
    }

    /// Makes `command` start its process in this group.
    pub fn adopt<'command>(&self, command: &'command mut Command) -> &'command mut Command {
        command.process_group(self.id)
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        drop(self.guard.stdin.take());
        let _ = self.guard.wait();
    }
}
