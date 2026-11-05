//! The Desktop's own conditions, which the setup checks look for: its
//! Bluetooth power, its clipboard and its `WirePlumber` drop-ins.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use zbus::proxy;

use super::Harness;
use super::builder::WIREPLUMBER_DROP_INS;

#[proxy(
    interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedDesktop1",
    default_service = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone",
    default_path = "/io/github/danielbaldwin47/DesktopCalls/SimulatedPhone"
)]
trait SimulatedDesktop1 {
    fn set_adapter_power(&self, power: &str) -> zbus::Result<()>;
}

impl Harness {
    /// Turns the Desktop's Bluetooth `"on"` or `"off"`, or blocks it
    /// (`"blocked"`), as another app or the user would.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_bluetooth_power(&self, power: &str) {
        zbus::block_on(async {
            SimulatedDesktop1Proxy::new(&self.session)
                .await?
                .set_adapter_power(power)
                .await
        })
        .unwrap_or_else(|error| panic!("set the Desktop's Bluetooth {power:?}: {error}"));
    }

    /// What a paste would get from the screen's clipboard now.
    ///
    /// # Panics
    ///
    /// If the scenario has no screen, `wl-paste` (from `wl-clipboard`) can't
    /// run, or the clipboard holds no text within 5 s.
    #[must_use]
    pub fn clipboard_text(&self) -> String {
        let started = Instant::now();
        loop {
            let pasted = self
                .external_command("wl-paste")
                .args(["--no-newline", "--type", "text/plain"])
                .stdin(Stdio::null())
                .output()
                .expect("run wl-paste, from wl-clipboard");
            if pasted.status.success() {
                return String::from_utf8_lossy(&pasted.stdout).into_owned();
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the clipboard holds no text; wl-paste said {:?}{}",
                String::from_utf8_lossy(&pasted.stderr),
                self.timeline.render()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// The permission bits of the Desktop's `WirePlumber` drop-in `name`, or
    /// of the file it links to.
    ///
    /// # Panics
    ///
    /// If the drop-in can't be read.
    #[must_use]
    pub fn wireplumber_drop_in_mode(&self, name: &str) -> u32 {
        let path = self
            .directory
            .join("config")
            .join(WIREPLUMBER_DROP_INS)
            .join(name);
        fs::metadata(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            .permissions()
            .mode()
            & 0o7777
    }

    /// The text of the Desktop's `WirePlumber` drop-in `name`, or of the
    /// file it links to.
    ///
    /// # Panics
    ///
    /// If the drop-in can't be read.
    #[must_use]
    pub fn wireplumber_drop_in_text(&self, name: &str) -> String {
        let path = self
            .directory
            .join("config")
            .join(WIREPLUMBER_DROP_INS)
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    /// Whether the Desktop's `WirePlumber` drop-in `name` is a symbolic link.
    ///
    /// # Panics
    ///
    /// If the drop-in isn't there.
    #[must_use]
    pub fn wireplumber_drop_in_is_link(&self, name: &str) -> bool {
        let path = self
            .directory
            .join("config")
            .join(WIREPLUMBER_DROP_INS)
            .join(name);
        fs::symlink_metadata(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            .is_symlink()
    }

    /// Every file in the Desktop's `WirePlumber` drop-in folder, by name,
    /// with its text.
    ///
    /// # Panics
    ///
    /// If the folder can't be read.
    #[must_use]
    pub fn wireplumber_drop_ins(&self) -> BTreeMap<String, String> {
        let folder = self.directory.join("config").join(WIREPLUMBER_DROP_INS);
        fs::read_dir(&folder)
            .unwrap_or_else(|error| panic!("read {}: {error}", folder.display()))
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| {
                let text = fs::read_to_string(entry.path())
                    .unwrap_or_else(|error| panic!("read {}: {error}", entry.path().display()));
                (entry.file_name().to_string_lossy().into_owned(), text)
            })
            .collect()
    }
}
