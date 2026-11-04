//! `org.bluez` on the system bus: one adapter and the paired Phone.
//!
//! BlueZ serves its `ObjectManager` at `/` and, unlike PipeWire, lists child
//! nodes in `Introspect`.

use zbus::{Array, Message, ObjectPath, OwnedObjectPath, OwnedVariant, Str, Variant};

use crate::objects::{
    INTROSPECTABLE, InterfaceSchema, Interfaces, MethodCall, OBJECT_MANAGER, ObjectTree,
    PROPERTIES, Properties, Reply,
};

pub const SERVICE: &str = "org.bluez";
pub const DEVICE: &str = "org.bluez.Device1";
const ADAPTER: &str = "org.bluez.Adapter1";
const ADAPTER_PATH: &str = "/org/bluez/hci0";

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

pub static SCHEMAS: [InterfaceSchema; 2] = [
    InterfaceSchema {
        name: ADAPTER,
        members: r#"  <method name="StartDiscovery"/>
  <method name="StopDiscovery"/>
  <method name="RemoveDevice">
   <arg name="device" type="o" direction="in"/>
  </method>
  <property name="Address" type="s" access="read"/>
  <property name="Name" type="s" access="read"/>
  <property name="Alias" type="s" access="readwrite"/>
  <property name="Powered" type="b" access="readwrite"/>
  <property name="Discoverable" type="b" access="readwrite"/>
  <property name="Pairable" type="b" access="readwrite"/>
  <property name="Discovering" type="b" access="read"/>"#,
    },
    InterfaceSchema {
        name: DEVICE,
        members: r#"  <method name="Disconnect"/>
  <method name="Connect"/>
  <method name="ConnectProfile">
   <arg name="UUID" type="s" direction="in"/>
  </method>
  <method name="DisconnectProfile">
   <arg name="UUID" type="s" direction="in"/>
  </method>
  <method name="Pair"/>
  <method name="CancelPairing"/>
  <property name="Address" type="s" access="read"/>
  <property name="AddressType" type="s" access="read"/>
  <property name="Name" type="s" access="read"/>
  <property name="Alias" type="s" access="readwrite"/>
  <property name="Class" type="u" access="read"/>
  <property name="Icon" type="s" access="read"/>
  <property name="Paired" type="b" access="read"/>
  <property name="Bonded" type="b" access="read"/>
  <property name="Trusted" type="b" access="readwrite"/>
  <property name="Blocked" type="b" access="readwrite"/>
  <property name="LegacyPairing" type="b" access="read"/>
  <property name="CablePairing" type="b" access="read"/>
  <property name="Modalias" type="s" access="read"/>
  <property name="Connected" type="b" access="read"/>
  <property name="UUIDs" type="as" access="read"/>
  <property name="Adapter" type="o" access="read"/>
  <property name="ServicesResolved" type="b" access="read"/>"#,
    },
];

/// BlueZ with one powered adapter and the Phone paired but not connected.
pub fn new_tree(phone_name: &str, phone_address: &str) -> zbus::Result<ObjectTree> {
    let mut tree = ObjectTree::new(&SCHEMAS, true);
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

    let mut adapter = Properties::new();
    adapter.insert("Address".into(), text("00:1A:7D:00:00:01"));
    adapter.insert("Name".into(), text("desktop"));
    adapter.insert("Alias".into(), text("desktop"));
    adapter.insert("Powered".into(), OwnedVariant::from(true));
    adapter.insert("Discoverable".into(), OwnedVariant::from(false));
    adapter.insert("Pairable".into(), OwnedVariant::from(true));
    adapter.insert("Discovering".into(), OwnedVariant::from(false));
    tree.insert(
        &path(ADAPTER_PATH)?,
        Interfaces::from([(ADAPTER.to_owned(), adapter)]),
        vec![PROPERTIES, INTROSPECTABLE],
    );

    let mut device = Properties::new();
    device.insert("Address".into(), text(phone_address));
    device.insert("AddressType".into(), text("public"));
    device.insert("Name".into(), text(phone_name));
    device.insert("Alias".into(), text(phone_name));
    // As the Pixel reports it: major class Phone, minor class Smartphone.
    device.insert("Class".into(), OwnedVariant::from(5_915_148_u32));
    device.insert("Icon".into(), text("phone"));
    device.insert("Paired".into(), OwnedVariant::from(true));
    device.insert("Bonded".into(), OwnedVariant::from(true));
    device.insert("Trusted".into(), OwnedVariant::from(true));
    device.insert("Blocked".into(), OwnedVariant::from(false));
    device.insert("LegacyPairing".into(), OwnedVariant::from(false));
    device.insert("CablePairing".into(), OwnedVariant::from(false));
    device.insert("Modalias".into(), text("bluetooth:v00E0p4114d0000"));
    device.insert("Connected".into(), OwnedVariant::from(false));
    device.insert("ServicesResolved".into(), OwnedVariant::from(false));
    device.insert(
        "UUIDs".into(),
        OwnedVariant::try_from(Variant::from(Array::from(PHONE_UUIDS.to_vec())))?,
    );
    device.insert(
        "Adapter".into(),
        OwnedVariant::from(ObjectPath::try_from(ADAPTER_PATH)?),
    );
    tree.insert(
        &device_path(phone_address)?,
        Interfaces::from([(DEVICE.to_owned(), device)]),
        vec![PROPERTIES, INTROSPECTABLE],
    );
    Ok(tree)
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
