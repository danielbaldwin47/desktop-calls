//! The Telephony service coming back after a restart, as `WirePlumber`
//! does.

use zbus::MessageStream;

use crate::{Service, SharedWorld, TelephonyConnection, lock, serve, telephony};

/// The Telephony service after a restart: takes the name on a new
/// connection, serves the same objects there, and announces them again.
pub fn come_back(
    holder: &TelephonyConnection,
    world: &SharedWorld,
    restart: Option<telephony::Restart>,
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
    let world = world.clone();
    let holder = holder.clone();
    std::thread::spawn(move || {
        if let Err(error) = zbus::block_on(serve(serving, calls, Service::Telephony, world, holder))
        {
            eprintln!("simulated-phone: the restarted Telephony service stopped: {error}");
        }
    });
    let Some(restart) = restart else {
        return Ok(());
    };
    zbus::block_on(async {
        for signal in &restart.audio_gateway_back {
            signal.emit(&connection).await?;
        }
        Ok::<_, zbus::Error>(())
    })?;
    std::thread::sleep(std::time::Duration::from_millis(40));
    zbus::block_on(async {
        for signal in &restart.calls_back {
            signal.emit(&connection).await?;
        }
        Ok(())
    })
}
