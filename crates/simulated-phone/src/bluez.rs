//! `org.bluez` on the system bus: one adapter and the paired Phone, shaped
//! like BlueZ with the real Pixel 9a paired (D-Bus snapshot, 2026-11-04).
//!
//! BlueZ serves its `ObjectManager` at `/` and, unlike PipeWire, lists child
//! nodes in `Introspect`. The adapter and the Phone answer `Introspect` with
//! the interfaces captured from the real service (`introspection/`, child
//! nodes removed), and serve every property the real ones served, with the
//! same types. Only `Adapter1` and `Device1` behave; the other interfaces are
//! there so the objects look the same, with the values the snapshot had while
//! the Phone was connected.

use zbus::{Array, Message, ObjectPath, OwnedObjectPath, OwnedVariant, Str, Variant};

use crate::objects::{
    INTROSPECTABLE, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree, PROPERTIES, Properties,
    Reply,
};

pub const SERVICE: &str = "org.bluez";
pub const DEVICE: &str = "org.bluez.Device1";
const ADAPTER: &str = "org.bluez.Adapter1";
const ADAPTER_PATH: &str = "/org/bluez/hci0";

/// `Introspect` on the adapter, captured from the real service.
const ADAPTER_INTROSPECTION: &str = include_str!("../introspection/bluez-adapter.xml");

/// `Introspect` on the paired Pixel, captured from the real service.
const DEVICE_INTROSPECTION: &str = include_str!("../introspection/bluez-device.xml");

/// What the paired Pixel 9a advertises (D-Bus snapshot, 2026-11-04),
/// Hands-Free Audio Gateway (`111f`) among them.
const PHONE_UUIDS: [&str; 22] = [
    "00001105-0000-1000-8000-00805f9b34fb",
    "0000110a-0000-1000-8000-00805f9b34fb",
    "0000110c-0000-1000-8000-00805f9b34fb",
    "0000110d-0000-1000-8000-00805f9b34fb",
    "0000110e-0000-1000-8000-00805f9b34fb",
    "00001112-0000-1000-8000-00805f9b34fb",
    "00001115-0000-1000-8000-00805f9b34fb",
    "00001116-0000-1000-8000-00805f9b34fb",
    "0000111f-0000-1000-8000-00805f9b34fb",
    "0000112d-0000-1000-8000-00805f9b34fb",
    "0000112f-0000-1000-8000-00805f9b34fb",
    "00001132-0000-1000-8000-00805f9b34fb",
    "00001200-0000-1000-8000-00805f9b34fb",
    "00001800-0000-1000-8000-00805f9b34fb",
    "00001801-0000-1000-8000-00805f9b34fb",
    "00001855-0000-1000-8000-00805f9b34fb",
    "00001858-0000-1000-8000-00805f9b34fb",
    "0000185f-0000-1000-8000-00805f9b34fb",
    "16bcfd00-253f-c348-e831-0db3e334d580",
    "3e1d50cd-7e3e-427d-8e1c-b78aa87fe624",
    "a82efa21-ae5c-3dde-9bbc-f16da7b16c5a",
    "abbafc00-e56a-484c-b832-8b17cf6cbfe8",
];

/// What the Desktop's adapter advertised in the snapshot: the hands-free
/// unit (`111e`), PBAP client (`112e`), A2DP sink (`110b`) and others.
const ADAPTER_UUIDS: [&str; 19] = [
    "00001133-0000-1000-8000-00805f9b34fb",
    "0000110e-0000-1000-8000-00805f9b34fb",
    "03b80e5a-ede8-4b33-a751-6ce34ec4c700",
    "00001105-0000-1000-8000-00805f9b34fb",
    "00001132-0000-1000-8000-00805f9b34fb",
    "00001200-0000-1000-8000-00805f9b34fb",
    "00001104-0000-1000-8000-00805f9b34fb",
    "00005005-0000-1000-8000-0002ee000001",
    "0000110c-0000-1000-8000-00805f9b34fb",
    "00001801-0000-1000-8000-00805f9b34fb",
    "0000112f-0000-1000-8000-00805f9b34fb",
    "0000110b-0000-1000-8000-00805f9b34fb",
    "0000180a-0000-1000-8000-00805f9b34fb",
    "00001800-0000-1000-8000-00805f9b34fb",
    "0000112e-0000-1000-8000-00805f9b34fb",
    "0000111f-0000-1000-8000-00805f9b34fb",
    "0000110a-0000-1000-8000-00805f9b34fb",
    "00001106-0000-1000-8000-00805f9b34fb",
    "0000111e-0000-1000-8000-00805f9b34fb",
];

/// BlueZ with one powered adapter and the Phone paired but not connected.
pub fn new_tree(phone_name: &str, phone_address: &str) -> zbus::Result<ObjectTree> {
    let mut tree = ObjectTree::new(true);
    tree.insert(
        &path("/")?,
        Interfaces::new(),
        vec![OBJECT_MANAGER, INTROSPECTABLE],
    );
    tree.insert(&path("/org")?, Interfaces::new(), vec![INTROSPECTABLE]);
    tree.insert(
        &path("/org/bluez")?,
        Interfaces::new(),
        vec![INTROSPECTABLE],
    );

    let adapter = path(ADAPTER_PATH)?;
    tree.insert(
        &adapter,
        adapter_interfaces()?,
        vec![PROPERTIES, INTROSPECTABLE],
    );
    tree.capture_introspection(&adapter, ADAPTER_INTROSPECTION);

    let device = device_path(phone_address)?;
    tree.insert(
        &device,
        device_interfaces(phone_name, phone_address, &device)?,
        vec![PROPERTIES, INTROSPECTABLE],
    );
    tree.capture_introspection(&device, DEVICE_INTROSPECTION);
    Ok(tree)
}

fn adapter_interfaces() -> zbus::Result<Interfaces> {
    let mut adapter = Properties::new();
    adapter.insert("Address".into(), text("00:1A:7D:00:00:01"));
    adapter.insert("AddressType".into(), text("public"));
    adapter.insert("Name".into(), text("desktop"));
    adapter.insert("Alias".into(), text("desktop"));
    adapter.insert("Class".into(), OwnedVariant::from(8_126_724_u32));
    adapter.insert("Connectable".into(), OwnedVariant::from(false));
    adapter.insert("Powered".into(), OwnedVariant::from(true));
    adapter.insert("PowerState".into(), text("on"));
    adapter.insert("Discoverable".into(), OwnedVariant::from(false));
    adapter.insert("DiscoverableTimeout".into(), OwnedVariant::from(180_u32));
    adapter.insert("Pairable".into(), OwnedVariant::from(true));
    adapter.insert("PairableTimeout".into(), OwnedVariant::from(0_u32));
    adapter.insert("Discovering".into(), OwnedVariant::from(false));
    adapter.insert("UUIDs".into(), texts(&ADAPTER_UUIDS)?);
    adapter.insert("Modalias".into(), text("usb:v1D6Bp0246d0557"));
    adapter.insert("Roles".into(), texts(&["central", "peripheral"])?);
    adapter.insert("Manufacturer".into(), OwnedVariant::from(2_u16));
    adapter.insert("Version".into(), OwnedVariant::from(11_u8));

    let mut media = Properties::new();
    media.insert(
        "SupportedUUIDs".into(),
        texts(&[
            "0000110a-0000-1000-8000-00805f9b34fb",
            "0000110b-0000-1000-8000-00805f9b34fb",
        ])?,
    );
    media.insert("SupportedFeatures".into(), texts(&["tx-timestamping"])?);

    let mut advertising = Properties::new();
    advertising.insert("ActiveInstances".into(), OwnedVariant::from(0_u8));
    advertising.insert("SupportedInstances".into(), OwnedVariant::from(12_u8));
    advertising.insert(
        "SupportedIncludes".into(),
        texts(&["tx-power", "appearance", "local-name"])?,
    );
    advertising.insert(
        "SupportedSecondaryChannels".into(),
        texts(&["1M", "2M", "Coded"])?,
    );
    advertising.insert(
        "SupportedFeatures".into(),
        texts(&["CanSetTxPower", "HardwareOffload"])?,
    );
    let capabilities = std::collections::HashMap::from([
        ("MinTxPower", Variant::from(-34_i16)),
        ("MaxTxPower", Variant::from(7_i16)),
        ("MaxAdvLen", Variant::from(251_u8)),
        ("MaxScnRspLen", Variant::from(251_u8)),
    ]);
    advertising.insert(
        "SupportedCapabilities".into(),
        OwnedVariant::try_from(Variant::from(capabilities))?,
    );

    Ok(Interfaces::from([
        (ADAPTER.to_owned(), adapter),
        ("org.bluez.Media1".to_owned(), media),
        ("org.bluez.LEAdvertisingManager1".to_owned(), advertising),
    ]))
}

fn device_interfaces(
    phone_name: &str,
    phone_address: &str,
    device: &OwnedObjectPath,
) -> zbus::Result<Interfaces> {
    let mut properties = Properties::new();
    properties.insert("Address".into(), text(phone_address));
    properties.insert("AddressType".into(), text("public"));
    properties.insert("Name".into(), text(phone_name));
    properties.insert("Alias".into(), text(phone_name));
    // As the Pixel reports it: major class Phone, minor class Smartphone.
    properties.insert("Class".into(), OwnedVariant::from(5_915_148_u32));
    properties.insert("Icon".into(), text("phone"));
    properties.insert("Paired".into(), OwnedVariant::from(true));
    properties.insert("Bonded".into(), OwnedVariant::from(true));
    properties.insert("Trusted".into(), OwnedVariant::from(true));
    properties.insert("Blocked".into(), OwnedVariant::from(false));
    properties.insert("LegacyPairing".into(), OwnedVariant::from(false));
    properties.insert("CablePairing".into(), OwnedVariant::from(false));
    properties.insert("Modalias".into(), text("bluetooth:v00E0p4114d0000"));
    properties.insert("Connected".into(), OwnedVariant::from(false));
    properties.insert("ServicesResolved".into(), OwnedVariant::from(false));
    properties.insert("UUIDs".into(), texts(&PHONE_UUIDS)?);
    properties.insert(
        "Adapter".into(),
        OwnedVariant::from(ObjectPath::try_from(ADAPTER_PATH)?),
    );

    let network = Properties::from([("Connected".to_owned(), OwnedVariant::from(false))]);
    let media_control = Properties::from([
        ("Connected".to_owned(), OwnedVariant::from(true)),
        (
            "Player".to_owned(),
            OwnedVariant::from(ObjectPath::try_from(format!("{device}/avrcp/player0"))?),
        ),
    ]);
    let battery = Properties::from([("Percentage".to_owned(), OwnedVariant::from(40_u8))]);

    Ok(Interfaces::from([
        (DEVICE.to_owned(), properties),
        ("org.bluez.Network1".to_owned(), network),
        ("org.bluez.MediaControl1".to_owned(), media_control),
        ("org.bluez.Battery1".to_owned(), battery),
    ]))
}

/// BlueZ's path for a device: `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`.
pub fn device_path(address: &str) -> zbus::Result<OwnedObjectPath> {
    path(&format!("{ADAPTER_PATH}/dev_{}", address.replace(':', "_")))
}

/// Answers a method call on BlueZ.
pub fn answer(tree: &ObjectTree, call: &MethodCall, message: &Message) -> Reply {
    match tree.answer(call, message) {
        Reply::Unhandled => Reply::unknown_method(call),
        reply => reply,
    }
}

fn path(path: &str) -> zbus::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(path)
}

fn text(value: &str) -> OwnedVariant {
    OwnedVariant::from(Str::from(value.to_owned()))
}

fn texts(values: &[&str]) -> zbus::Result<OwnedVariant> {
    OwnedVariant::try_from(Variant::from(Array::from(values.to_vec())))
}
