//! The Telephony service coming back after a restart, as `WirePlumber`
//! does.

use std::time::Duration;

use zbus::{MessageStream, OwnedObjectPath};

use crate::telephony::Signal;
use crate::{Service, SharedWorld, TelephonyConnection, lock, serve, telephony};

/// How long after the AudioGateway the restarted service registers its
/// Calls, as on the real Pixel.
const CALLS_BACK: Duration = Duration::from_millis(40);

/// The Telephony service leaves for a restart with the AudioGateway, if
/// one is connected: its Calls stay away until the new service registers
/// them. Returns the signals the old service sends as it goes, and those
/// the new one sends for the AudioGateway.
pub fn leave(world: &SharedWorld) -> zbus::Result<Option<(Vec<Signal>, Vec<Signal>)>> {
    let mut world = lock(world);
    let world = &mut *world;
    let Some(gateway) = world.audio_gateway.clone() else {
        return Ok(None);
    };
    let telephony::Restart {
        gone,
        audio_gateway_back,
        away,
    } = telephony::restart(&mut world.telephony, &gateway)?;
    world.restarting = Some(away);
    Ok(Some((gone, audio_gateway_back)))
}

/// The Telephony service after a restart: takes the name on a new
/// connection, serves the same objects there, and announces
/// `audio_gateway_back`, then the Calls that were away, registering them
/// only then, so a listing in between has none.
pub fn come_back(
    holder: &TelephonyConnection,
    world: &SharedWorld,
    audio_gateway_back: Option<Vec<Signal>>,
) -> zbus::Result<()> {
    let connection = zbus::block_on(async {
        let connection = zbus::connection::Builder::session().build().await?;
        let calls = MessageStream::from(&connection);
        connection.request_name(telephony::SERVICE).await?;
        Ok::<_, zbus::Error>((connection, calls))
    });
    let (connection, calls) = connection?;
    *lock(holder) = Some(connection.clone());
    let serving = connection.clone();
    let serving_world = world.clone();
    let holder = holder.clone();
    std::thread::spawn(move || {
        if let Err(error) = zbus::block_on(serve(
            serving,
            calls,
            Service::Telephony,
            serving_world,
            holder,
        )) {
            eprintln!("simulated-phone: the restarted Telephony service stopped: {error}");
        }
    });
    let Some(audio_gateway_back) = audio_gateway_back else {
        return Ok(());
    };
    zbus::block_on(async {
        for signal in &audio_gateway_back {
            signal.emit(&connection).await?;
        }
        Ok::<_, zbus::Error>(())
    })?;
    let calls_back_after = lock(world).behaviour.calls_back_after(CALLS_BACK);
    std::thread::sleep(calls_back_after);
    let calls_back = {
        let mut world = lock(world);
        let world = &mut *world;
        match world.restarting.take() {
            // Unless the test connected or disconnected the Phone meanwhile.
            Some(away) if world.audio_gateway.as_ref() == Some(&away.audio_gateway) => {
                telephony::calls_back_after_restart(&mut world.telephony, away)
            }
            _ => Vec::new(),
        }
    };
    zbus::block_on(async {
        for signal in &calls_back {
            signal.emit(&connection).await?;
        }
        Ok(())
    })
}

/// Ends the Call at `call` on the Phone if it is away while the service
/// restarts, so the new service never registers it. Returns whether it was.
pub fn end_call_away(world: &SharedWorld, call: &OwnedObjectPath) -> bool {
    let mut world = lock(world);
    let Some(away) = world.restarting.as_mut() else {
        return false;
    };
    let before = away.calls.len();
    away.calls.retain(|(path, _)| path != call);
    away.calls.len() < before
}
