//! Answering every method call that arrives for one service, and recording
//! it: each kind of call goes to the module that acts it out.

use futures_util::StreamExt as _;
use zbus::{Connection, Message, MessageStream};

use crate::later::{answer, run_activate_later, run_obex_later};
use crate::objects::{MethodCall, PROPERTIES};
use crate::{
    Service, SharedWorld, TelephonyConnection, blocked, bluez, hands_free, lock, obex, pairing,
    range, telephony,
};

/// Answers every method call that arrives for one service, and records it.
pub async fn serve(
    connection: Connection,
    mut messages: MessageStream,
    service: Service,
    world: SharedWorld,
    telephony: TelephonyConnection,
) -> zbus::Result<()> {
    while let Some(message) = messages.next().await {
        // A Telephony service that left for a restart closed its connection,
        // as the old process exits.
        if connection.is_closed() {
            break;
        }
        let message = message?;
        let Some(call) = MethodCall::from_message(&message) else {
            continue;
        };
        if matches!(service, Service::Bluez)
            && bluez_module_answers(&world, &connection, &telephony, &call, &message)
        {
            continue;
        }
        let (reply, signals, later, activate_later, slow) = {
            let mut world = lock(&world);
            world.recorded.push((service.bus(), call.clone()));
            let world = &mut *world;
            let slow = matches!(service, Service::Telephony)
                && call.interface == PROPERTIES
                && call.member == "Set"
                && world
                    .behaviour
                    .quirks
                    .contains(&telephony::Quirk::SlowPropertyWrites);
            match service {
                Service::Bluez => (
                    bluez::answer(&world.bluez, &call, &message),
                    Vec::new(),
                    None,
                    None,
                    slow,
                ),
                Service::Telephony => {
                    let answer = telephony::answer(
                        &mut world.telephony,
                        world.audio_gateway.as_ref(),
                        &mut world.behaviour,
                        &mut world.dialling,
                        &call,
                        &message,
                    );
                    (answer.reply, answer.signals, None, answer.later, slow)
                }
                Service::Obex => {
                    let answer =
                        obex::answer(&mut world.obex, &world.phone_address, &call, &message);
                    (answer.reply, answer.signals, answer.later, None, slow)
                }
            }
        };
        if slow {
            // The calls that follow are answered meanwhile.
            let connection = connection.clone();
            std::thread::spawn(move || {
                std::thread::sleep(telephony::SLOW_REPLY);
                zbus::block_on(answer(&connection, &message, reply, signals))
            });
            continue;
        }
        match answer(&connection, &message, reply, signals).await {
            Err(_) if connection.is_closed() => break,
            result => result?,
        }
        if let Some((after, later)) = later {
            run_obex_later(world.clone(), connection.clone(), message, after, later);
        }
        if let Some(later) = activate_later {
            run_activate_later(world.clone(), connection.clone(), later);
        }
    }
    Ok(())
}

/// Hands `call` to the BlueZ module that acts it out on a thread of its
/// own, if one does, recording it: whether one took it.
fn bluez_module_answers(
    world: &SharedWorld,
    connection: &Connection,
    telephony: &TelephonyConnection,
    call: &MethodCall,
    message: &Message,
) -> bool {
    type Answer = fn(SharedWorld, Connection, TelephonyConnection, MethodCall, Message);
    let answer: Answer = if blocked::handles(&lock(world), call) {
        blocked::answer
    } else if hands_free::handles(call) {
        hands_free::answer
    } else if pairing::handles(call) {
        |world, connection, _, call, message| pairing::answer(world, connection, call, message)
    } else if range::handles(call) {
        range::answer_connect
    } else {
        return false;
    };
    lock(world)
        .recorded
        .push((Service::Bluez.bus(), call.clone()));
    answer(
        world.clone(),
        connection.clone(),
        telephony.clone(),
        call.clone(),
        message.clone(),
    );
    true
}
