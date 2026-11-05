//! The Simulated Phone: its control interface, and the handle scenarios drive
//! it with.

use std::thread;
use std::time::{Duration, Instant};

use zbus::proxy;
use zbus::proxy::CacheProperties;

use super::{Bus, Harness};

#[proxy(
    interface = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone1",
    default_service = "io.github.danielbaldwin47.DesktopCalls.SimulatedPhone",
    default_path = "/io/github/danielbaldwin47/DesktopCalls/SimulatedPhone"
)]
trait SimulatedPhone1 {
    fn connect(&self) -> zbus::Result<()>;

    fn disconnect(&self) -> zbus::Result<()>;

    fn ring(&self, number: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn ring_with_caller_id(&self, caller_id: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn set_codec(&self, codec: u8) -> zbus::Result<()>;

    fn enable_quirk(&self, name: &str) -> zbus::Result<()>;

    fn reregister_call(&self, call: &zbus::ObjectPath<'_>, after_ms: u32) -> zbus::Result<()>;

    fn restart_telephony(&self, gap_ms: u32) -> zbus::Result<()>;

    fn alert(&self) -> zbus::Result<zbus::OwnedObjectPath>;

    fn callee_answers(&self) -> zbus::Result<()>;

    fn dial_on_phone(&self, number: &str) -> zbus::Result<zbus::OwnedObjectPath>;

    fn set_earbuds(&self, connected: bool) -> zbus::Result<()>;

    fn refuse_next_dial(&self, error: &str) -> zbus::Result<()>;
    fn set_pbap_folder(&self, folder: &str, vcards: &str) -> zbus::Result<()>;

    fn remove_pbap_folder(&self, folder: &str) -> zbus::Result<()>;

    fn set_contacts_prompt(&self, answer: &str) -> zbus::Result<()>;

    fn set_pull_delay(&self, delay_ms: u32) -> zbus::Result<()>;

    fn set_activate_delay(&self, delay_ms: u32) -> zbus::Result<()>;

    fn set_microphone_volume(&self, volume: u8) -> zbus::Result<()>;

    fn answer_on_phone(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    fn decline_on_phone(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    fn caller_hangs_up(&self, call: &zbus::ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn audio_gateway(&self) -> zbus::Result<zbus::OwnedObjectPath>;

    #[zbus(property)]
    fn most_pbap_sessions_at_once(&self) -> zbus::Result<u32>;

    fn recorded_method_calls(&self) -> zbus::Result<Vec<(String, String, String, String, String)>>;
}

/// One method call the Simulated Phone received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMethodCall {
    /// `"system"` or `"session"`.
    pub bus: String,
    pub path: String,
    pub interface: String,
    pub member: String,
    /// The arguments in GVariant text form, such as `("org.bluez.Device1",)`.
    pub arguments: String,
}

/// Who the Simulated Phone says it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneIdentity {
    pub name: String,
    pub address: String,
}

impl Harness {
    /// Acts as the Phone.
    #[must_use]
    pub fn simulated_phone(&self) -> SimulatedPhone<'_> {
        SimulatedPhone { harness: self }
    }
}

impl Harness {
    /// Calls a method on the Phone's side of one of the private buses, as any
    /// D-Bus client could, and returns the reply.
    ///
    /// # Panics
    ///
    /// If the call fails or the reply is an error; the panic names the call.
    pub fn call_phone_method<B>(
        &self,
        bus: Bus,
        service: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: &B,
    ) -> zbus::Message
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType,
    {
        let connection = match bus {
            Bus::System => &self.system,
            Bus::Session => &self.session,
        };
        zbus::block_on(connection.call_method(Some(service), path, Some(interface), member, body))
            .unwrap_or_else(|error| panic!("{interface}.{member} on {service} {path}: {error}"))
    }
}

/// Drives the Simulated Phone through its control interface.
pub struct SimulatedPhone<'harness> {
    harness: &'harness Harness,
}

impl SimulatedPhone<'_> {
    /// Brings hands-free up: the AudioGateway appears once it is fully set up.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn connect(&self) {
        zbus::block_on(async { self.proxy().await?.connect().await })
            .expect("tell the Simulated Phone to connect");
    }

    /// Drops hands-free: the AudioGateway disappears.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn disconnect(&self) {
        zbus::block_on(async { self.proxy().await?.disconnect().await })
            .expect("tell the Simulated Phone to disconnect");
    }

    /// Rings the Phone from `number`, as a caller does, and returns the new
    /// Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn ring(&self, number: &str) -> String {
        zbus::block_on(async { self.proxy().await?.ring(number).await })
            .expect("ring the Simulated Phone")
            .to_string()
    }

    /// Rings the Phone with this caller ID as the Phone sends it to the
    /// Desktop, such as `"",129` for a withheld number, read as PipeWire
    /// reads it. Returns the new Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn ring_with_caller_id(&self, caller_id: &str) -> String {
        zbus::block_on(async { self.proxy().await?.ring_with_caller_id(caller_id).await })
            .expect("ring the Simulated Phone")
            .to_string()
    }

    /// Makes the Phone's audio link negotiate the codec with this hands-free
    /// id from now on: 1 CVSD, 2 mSBC, 3 LC3-SWB (the Pixel's, by default).
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_codec(&self, codec: u8) {
        zbus::block_on(async { self.proxy().await?.set_codec(codec).await })
            .expect("set the Simulated Phone's codec");
    }

    /// Switches on a quirk: `"call1-hangup-missing"` (only
    /// `org.ofono.VoiceCall.Hangup` works) or `"hangup-too-few-arguments"`
    /// (`Hangup` fails with "Too few arguments" while the Call ends anyway).
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the quirk.
    pub fn enable_quirk(&self, name: &str) {
        zbus::block_on(async { self.proxy().await?.enable_quirk(name).await })
            .unwrap_or_else(|error| panic!("enable quirk {name:?}: {error}"));
    }

    /// Unregisters the Call at `call` and registers it again `after` later,
    /// in the same state, as PipeWire does when the hands-free profile
    /// briefly drops mid-Call. Returns once the Call is gone.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn reregister_call(&self, call: &str, after: Duration) {
        let after_ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.reregister_call(&call, after_ms).await
        })
        .unwrap_or_else(|error| panic!("reregister the Call at {call}: {error}"));
    }

    /// Restarts the Telephony service, as restarting `WirePlumber` does: the
    /// service unannounces the AudioGateway's Calls and the AudioGateway,
    /// leaves the bus, and `gap` later comes back on a new connection with
    /// the same objects at the same paths, the Calls 40 ms after the
    /// AudioGateway. Returns once the service has left the bus. On the real
    /// Pixel the gap was 0.73 s.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn restart_telephony(&self, gap: Duration) {
        let gap_ms = u32::try_from(gap.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async { self.proxy().await?.restart_telephony(gap_ms).await })
            .expect("restart the Simulated Phone's Telephony service");
    }

    /// Makes the Call the Desktop dialled ring at the other end, as the
    /// Pixel reports it: the Call appears, `alerting`. Returns its path.
    ///
    /// # Panics
    ///
    /// If nothing was dialled.
    #[must_use]
    pub fn alert(&self) -> String {
        zbus::block_on(async { self.proxy().await?.alert().await })
            .expect("make the dialled Call ring at the other end")
            .to_string()
    }

    /// The person called answers the Call the Desktop dialled: it goes
    /// `active`.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn callee_answers(&self) {
        zbus::block_on(async { self.proxy().await?.callee_answers().await })
            .expect("answer the dialled Call at the other end");
    }

    /// Places a Call to `number` on the Phone itself, as the person holding
    /// it does: the Call appears `dialing`, and the audio stays on the Phone.
    /// Returns the Call's object path.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses, as it does while hands-free is down.
    #[must_use]
    pub fn dial_on_phone(&self, number: &str) -> String {
        zbus::block_on(async { self.proxy().await?.dial_on_phone(number).await })
            .expect("place a Call on the Simulated Phone")
            .to_string()
    }

    /// Connects earbuds to the Phone beside the Desktop (`true`), or
    /// disconnects them. Connected earbuds take the audio of Calls answered
    /// on them, and the Phone ignores `Activate` until the Desktop answers.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone can't be reached.
    pub fn set_earbuds(&self, connected: bool) {
        zbus::block_on(async { self.proxy().await?.set_earbuds(connected).await })
            .expect("connect or disconnect the Simulated Phone's earbuds");
    }

    /// Makes the Phone refuse the next `Dial` with a Telephony error:
    /// `"invalid-format"`, `"invalid-state"`, `"not-supported"`,
    /// `"in-progress"`, `"failed"`, or `"cme:N"` for the Phone's `+CME ERROR: N`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the error.
    pub fn refuse_next_dial(&self, error: &str) {
        zbus::block_on(async { self.proxy().await?.refuse_next_dial(error).await })
            .unwrap_or_else(|failure| panic!("refuse the next Dial with {error:?}: {failure}"));
    }

    /// Gives the Phone the PBAP folder `folder` (`"pb"` for Contacts;
    /// `"cch"`, `"ich"`, `"och"` or `"mch"` for Call History) with exactly
    /// this vCard text, which it serves whatever format is asked for. See
    /// [`crate::vcards`] for builders.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_pbap_folder(&self, folder: &str, vcards: &str) {
        zbus::block_on(async { self.proxy().await?.set_pbap_folder(folder, vcards).await })
            .unwrap_or_else(|error| panic!("set the {folder} folder: {error}"));
    }

    /// Makes the Phone lack the folder `folder`, as some Phones lack
    /// `cch`: selecting it fails.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn remove_pbap_folder(&self, folder: &str) {
        zbus::block_on(async { self.proxy().await?.remove_pbap_folder(folder).await })
            .unwrap_or_else(|error| panic!("remove the {folder} folder: {error}"));
    }

    /// How the person holding the Phone answers its "Allow access to
    /// contacts and call history?" prompt for each new PBAP session:
    /// `"grant"` (the default), `"deny"` or `"ignore"`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't know the answer.
    pub fn set_contacts_prompt(&self, answer: &str) {
        zbus::block_on(async { self.proxy().await?.set_contacts_prompt(answer).await })
            .unwrap_or_else(|error| panic!("set the contacts prompt to {answer:?}: {error}"));
    }

    /// Makes every PBAP pull take `delay` before it completes.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_pull_delay(&self, delay: Duration) {
        let delay_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async { self.proxy().await?.set_pull_delay(delay_ms).await })
            .expect("set the Simulated Phone's pull delay");
    }

    /// Makes the Phone take `delay` after each `Activate` before it opens
    /// the audio link, as the Pixel took 62 to 313 ms.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_activate_delay(&self, delay: Duration) {
        let delay_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
        zbus::block_on(async { self.proxy().await?.set_activate_delay(delay_ms).await })
            .expect("set the Simulated Phone's Activate delay");
    }

    /// The most PBAP sessions the Phone has had open at once, counting a
    /// session from its `CreateSession` to its `RemoveSession`.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't answer.
    #[must_use]
    pub fn most_pbap_sessions_at_once(&self) -> u32 {
        zbus::block_on(async { self.proxy().await?.most_pbap_sessions_at_once().await })
            .expect("read the Simulated Phone's PBAP session count")
    }

    /// Makes the Phone report this hands-free microphone gain, as `+VGM`
    /// does: 0 to 15, or 255, which is how PipeWire sends its -1 ("the Phone
    /// hasn't said"). A later AudioGateway starts with it too.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone refuses.
    pub fn set_microphone_volume(&self, volume: u8) {
        zbus::block_on(async { self.proxy().await?.set_microphone_volume(volume).await })
            .expect("set the Simulated Phone's microphone volume");
    }

    /// Answers the ringing Call at `call` on the Phone, as the person holding
    /// it does. Returns once the Phone has reported the Call active.
    ///
    /// # Panics
    ///
    /// If there is no such Call, or it isn't ringing.
    pub fn answer_on_phone(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.answer_on_phone(&call).await
        })
        .unwrap_or_else(|error| panic!("answer the Call at {call} on the Phone: {error}"));
    }

    /// Waits until the method calls the Simulated Phone has received, in
    /// order, satisfy `condition`, and returns them.
    ///
    /// The app sends its Phone commands on a connection of its own, so a
    /// call can reach the Phone after the app's state has moved on. That is
    /// why there is no snapshot of the calls to assert on: every check of
    /// what the Phone was asked waits, here or in [`Self::recorded_holds`].
    ///
    /// # Panics
    ///
    /// If `deadline` passes first; the panic names `description` and every
    /// call received.
    pub fn wait_for_recorded(
        &self,
        description: &str,
        deadline: Duration,
        condition: impl Fn(&[RecordedMethodCall]) -> bool,
    ) -> Vec<RecordedMethodCall> {
        let started = Instant::now();
        loop {
            let recorded = self.recorded_method_calls();
            if condition(&recorded) {
                return recorded;
            }
            assert!(
                started.elapsed() < deadline,
                "\"{description}\" did not happen within {deadline:?}; the Simulated Phone received {recorded:#?}{}",
                self.harness.timeline.render()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Checks that the method calls the Simulated Phone has received satisfy
    /// `condition` for the whole of `window`, for something the Phone must
    /// not be asked, such as to hang up. Returns the calls at the end.
    ///
    /// # Panics
    ///
    /// At the first check that fails `condition`, naming `description` and
    /// every call received.
    pub fn recorded_holds(
        &self,
        description: &str,
        window: Duration,
        condition: impl Fn(&[RecordedMethodCall]) -> bool,
    ) -> Vec<RecordedMethodCall> {
        let started = Instant::now();
        loop {
            let recorded = self.recorded_method_calls();
            assert!(
                condition(&recorded),
                "\"{description}\" stopped holding; the Simulated Phone received {recorded:#?}{}",
                self.harness.timeline.render()
            );
            if started.elapsed() >= window {
                return recorded;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Declines the ringing Call at `call` on the Phone, as the person holding
    /// it does. Returns once the Phone has removed the Call.
    ///
    /// # Panics
    ///
    /// If there is no such Call, or it isn't ringing.
    pub fn decline_on_phone(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.decline_on_phone(&call).await
        })
        .unwrap_or_else(|error| panic!("decline the Call at {call} on the Phone: {error}"));
    }

    /// Ends the Call at `call` from the other end, as the caller hanging up
    /// does, whether it is ringing or live. Returns once the Phone has
    /// removed the Call.
    ///
    /// # Panics
    ///
    /// If there is no such Call.
    pub fn caller_hangs_up(&self, call: &str) {
        zbus::block_on(async {
            let call = zbus::ObjectPath::try_from(call)?;
            self.proxy().await?.caller_hangs_up(&call).await
        })
        .unwrap_or_else(|error| panic!("hang up the Call at {call} as the caller: {error}"));
    }

    /// The object path of the current AudioGateway, or `"/"` while there is
    /// none.
    ///
    /// # Panics
    ///
    /// If the Simulated Phone doesn't answer.
    #[must_use]
    pub fn audio_gateway(&self) -> String {
        zbus::block_on(async {
            self.proxy()
                .await?
                .audio_gateway()
                .await
                .map(|path| path.to_string())
        })
        .expect("read the Simulated Phone's AudioGateway")
    }

    /// Every method call the Simulated Phone received so far, in order.
    /// Private, so no test can assert on a snapshot: see
    /// [`Self::wait_for_recorded`].
    fn recorded_method_calls(&self) -> Vec<RecordedMethodCall> {
        zbus::block_on(async { self.proxy().await?.recorded_method_calls().await })
            .expect("read the Simulated Phone's recorded method calls")
            .into_iter()
            .map(
                |(bus, path, interface, member, arguments)| RecordedMethodCall {
                    bus,
                    path,
                    interface,
                    member,
                    arguments,
                },
            )
            .collect()
    }

    async fn proxy(&self) -> zbus::Result<SimulatedPhone1Proxy<'static>> {
        SimulatedPhone1Proxy::builder(&self.harness.session)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }
}
