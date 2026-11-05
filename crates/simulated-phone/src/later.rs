//! What the Phone does once a delay has passed, on threads of their own so
//! the service keeps answering meanwhile, and how it answers a method call.

use zbus::{Connection, Message};

use crate::objects::Reply;
use crate::{SharedWorld, lock, obex, telephony};

/// Has the Phone act on an `Activate` once its delay has passed, on a
/// thread of its own so the service keeps answering meanwhile.
pub(crate) fn run_activate_later(
    world: SharedWorld,
    connection: Connection,
    later: telephony::ActivateLater,
) {
    std::thread::spawn(move || {
        std::thread::sleep(later.after);
        let signals = {
            let mut world = lock(&world);
            let world = &mut *world;
            telephony::activate_later(
                &mut world.telephony,
                world.audio_gateway.as_ref(),
                &mut world.behaviour,
                &later,
            )
        };
        zbus::block_on(async {
            for signal in signals {
                if signal.emit(&connection).await.is_err() {
                    break;
                }
            }
        });
    });
}

/// Sends `reply` to `message`, then emits `signals`: as the real service,
/// which replies once the Phone confirms, and then reports the change the
/// Phone makes.
pub(crate) async fn answer(
    connection: &Connection,
    message: &Message,
    reply: Reply,
    signals: Vec<telephony::Signal>,
) -> zbus::Result<()> {
    reply.send(connection, &message.header()).await?;
    for signal in signals {
        signal.emit(connection).await?;
    }
    Ok(())
}

/// Does what obexd does `after` a method call, and after that, on a thread
/// of its own so the service keeps answering meanwhile: replies to
/// `message` if a reply is still owed, and sends the signals.
pub(crate) fn run_obex_later(
    world: SharedWorld,
    connection: Connection,
    message: zbus::Message,
    after: std::time::Duration,
    later: obex::Later,
) {
    std::thread::spawn(move || {
        let mut next = Some((after, later));
        while let Some((after, later)) = next.take() {
            std::thread::sleep(after);
            let done = {
                let mut world = lock(&world);
                let world = &mut *world;
                obex::run_later(&mut world.obex, &world.phone_address, later)
            };
            next = done.later;
            if next.is_none() {
                zbus::block_on(answer(&connection, &message, done.reply, done.signals))?;
            }
        }
        Ok::<_, zbus::Error>(())
    });
}
