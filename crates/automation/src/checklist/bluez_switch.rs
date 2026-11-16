//! Turning the app's BlueZ method lines on for a sitting against the
//! installed service, at the owner's yes, so they needn't stop it for
//! `just run`. The user manager's environment gets the switch and the
//! service restarts with it; on leaving the checklist, however it is left,
//! Ctrl-C included, the manager's environment is put back as it was. That
//! leaves nothing behind: the manager's environment lives only until the
//! owner's next login. The running app keeps logging the lines until it
//! next starts, since restarting it on the way out could end a Call.

use std::time::{Duration, Instant};

use zbus::Connection;
use zbus::OwnedObjectPath;

use super::app_log;
use super::sitting::{Sitting, Stop, flush};

const UNIT: &str = "desktop-calls.service";
/// The switch, as the user manager's environment holds it.
const SWITCH: &str = "RUST_LOG=info,bluez_dbus=debug";
/// How long the restarted app has to be back with its lines on.
const BACK_DEADLINE: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_millis(100);

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait Manager {
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn set_environment(&self, assignments: &[&str]) -> zbus::Result<()>;
    fn unset_environment(&self, names: &[&str]) -> zbus::Result<()>;
    #[zbus(property)]
    fn environment(&self) -> zbus::Result<Vec<String>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
trait Unit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
}

/// The switch on in the user manager's environment; dropping it puts back
/// what was there, `RUST_LOG`'s earlier value or none, and says so.
pub struct SwitchedOn {
    bus: Connection,
    before: Option<String>,
}

impl Drop for SwitchedOn {
    fn drop(&mut self) {
        let put_back = zbus::block_on(async {
            let manager = ManagerProxy::new(&self.bus).await?;
            match &self.before {
                Some(before) => manager.set_environment(&[before.as_str()]).await,
                None => manager.unset_environment(&["RUST_LOG"]).await,
            }
        });
        match put_back {
            Ok(()) => println!(
                "The app's BlueZ method lines are off again from Desktop Calls' next start; the running app keeps logging them until then, so nothing restarts it under you."
            ),
            Err(error) => println!(
                "Couldn't turn the app's BlueZ method lines off again ({error}). Run `systemctl --user unset-environment RUST_LOG`; they are off anyway after your next login."
            ),
        }
        flush();
    }
}

/// Before the first step against the user's app: if it logs no BlueZ
/// method calls, says so and, when it is the installed service, offers to
/// turn them on. `Some` while they are on through the offer.
pub fn offer(sitting: &mut Sitting) -> Result<Option<SwitchedOn>, Stop> {
    let Some(bus) = sitting.users_app().cloned() else {
        return Ok(None);
    };
    let Some(why) = app_log::bluez_methods_unlogged(&bus) else {
        return Ok(None);
    };
    println!("\nNot captured: {why}.");
    if !installed_service_runs(&bus) {
        println!(
            "To have them in each step's capture, type quit now, run `just run` in another terminal, and run this again.\n"
        );
        return Ok(None);
    }
    let asked = sitting.ask(
        "Turn them on for this sitting? Desktop Calls restarts now, and they go off again when you leave the checklist",
    );
    // `skip` skips the offer, as no would.
    if !matches!(asked, Ok(true) | Err(Stop::Quit)) {
        println!("Carrying on without them; each step's file says so.\n");
        return Ok(None);
    }
    asked?;
    let before = zbus::block_on(async {
        let manager = ManagerProxy::new(&bus).await?;
        let before = manager.environment().await?;
        manager.set_environment(&[SWITCH]).await?;
        zbus::Result::Ok(before)
    });
    let before = match before {
        Ok(environment) => environment
            .into_iter()
            .find(|each| each.starts_with("RUST_LOG=")),
        Err(error) => {
            println!("Couldn't turn them on ({error}); carrying on without them.\n");
            return Ok(None);
        }
    };
    let switched = SwitchedOn { bus, before };
    let restarted = zbus::block_on(async {
        ManagerProxy::new(&switched.bus)
            .await?
            .restart_unit(UNIT, "replace")
            .await
    });
    if let Err(error) = restarted {
        println!("Couldn't restart Desktop Calls ({error}); carrying on without them.\n");
        return Ok(Some(switched));
    }
    let started = Instant::now();
    while app_log::bluez_methods_unlogged(&switched.bus).is_some() {
        if sitting.stopped() {
            return Err(Stop::Quit);
        }
        if started.elapsed() > BACK_DEADLINE {
            println!(
                "Desktop Calls wasn't back with them on within {} s; carrying on, and each step's file says whether they are there.\n",
                BACK_DEADLINE.as_secs()
            );
            return Ok(Some(switched));
        }
        std::thread::sleep(TICK);
    }
    println!("On: Desktop Calls restarted with its BlueZ method calls logged.\n");
    flush();
    Ok(Some(switched))
}

/// Whether the app is the installed service, active, rather than a dev run
/// (`just run` stops the service for its run).
fn installed_service_runs(bus: &Connection) -> bool {
    zbus::block_on(async {
        let path = ManagerProxy::new(bus).await?.get_unit(UNIT).await?;
        UnitProxy::builder(bus)
            .path(path)
            .build()
            .await?
            .active_state()
            .await
    })
    .is_ok_and(|state| state == "active")
}
