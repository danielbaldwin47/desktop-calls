//! Conformance: the Simulated Phone against the real Pixel 9a.
//!
//! The Simulated Phone must be indistinguishable from the real Phone, so the
//! app cannot come to depend on anything only the Simulated Phone does. These
//! scenarios bring it into the state of the D-Bus snapshot captured from the
//! Pixel behind PipeWire 1.6.9 (`docs/research/2026-11-04-dbus-snapshot/`) and
//! compare, object by object:
//!
//! - its introspection data: every interface, method, signal and property,
//!   with argument names, directions and types, property types and access,
//!   and annotations. Child nodes are compared only by the Pixel-behaviour
//!   scenario, because elsewhere they name other objects (BlueZ lists every
//!   paired device);
//! - the properties `GetAll` returns for each interface that declares any:
//!   the same names as the Pixel's, each with the same type;
//! - the reply signature of every method called here, against the out
//!   arguments the Pixel declares for it;
//! - for the methods whose results the snapshot captured, which objects,
//!   interfaces and properties the reply lists, and with which types.
//!
//! Values are not compared: they differ by design (the Simulated Phone has
//! its own address, and its Call rings rather than being active).
//!
//! A failure lists every difference, naming the object and the member.

// Clippy's exemption for `#[test]` code doesn't reach this file's helpers.
// Like a `#[test]`, a helper that meets a malformed snapshot must fail the
// scenario at once, saying what it expected.
#![expect(
    clippy::expect_used,
    reason = "test code: a malformed snapshot fails the scenario with its message"
)]

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;

use serde_json::Value;
use test_harness::{Bus, Harness};
use zbus::{Message, OwnedObjectPath, OwnedVariant};

const TELEPHONY: &str = "org.pipewire.Telephony";
const BLUEZ: &str = "org.bluez";
const OBEX: &str = "org.bluez.obex";
const PHONEBOOK_ACCESS: &str = "org.bluez.obex.PhonebookAccess1";

const INTROSPECTABLE: &str = "org.freedesktop.DBus.Introspectable";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// Where the snapshot keeps each object, by what it is.
const PIXEL_MANAGER: &str = "/org/pipewire/Telephony";
const PIXEL_AUDIO_GATEWAY: &str = "/org/pipewire/Telephony/ag1";
const PIXEL_CALL: &str = "/org/pipewire/Telephony/ag1/call1";
const PIXEL_ADAPTER: &str = "/org/bluez/hci0";
const PIXEL_DEVICE: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";

/// A made-up caller, from the range reserved for fiction.
const CALLER: &str = "+15555550123";

#[test]
fn telephony_objects_match_the_pixel() {
    let snapshot = Snapshot::live_call();
    let harness = Harness::start();
    harness.simulated_phone().connect();
    let audio_gateway = harness.simulated_phone().audio_gateway();
    let call = harness.simulated_phone().ring(CALLER);
    let mut check = Check::new(&harness, Bus::Session, TELEPHONY);
    check.map_path(&audio_gateway, PIXEL_AUDIO_GATEWAY);
    check.map_path(&call, PIXEL_CALL);

    check.object(
        "the Telephony manager",
        PIXEL_MANAGER,
        &snapshot.object(PIXEL_MANAGER),
    );
    check.object(
        "the AudioGateway, with its transport",
        &audio_gateway,
        &snapshot.object(PIXEL_AUDIO_GATEWAY),
    );
    check.object("the Call", &call, &snapshot.object(PIXEL_CALL));
    for (method, interface, path) in [
        ("GetManagedObjects", OBJECT_MANAGER, PIXEL_MANAGER),
        ("GetModems", "org.ofono.Manager", PIXEL_MANAGER),
        ("GetManagedObjects", OBJECT_MANAGER, PIXEL_AUDIO_GATEWAY),
        (
            "GetCalls",
            "org.ofono.VoiceCallManager",
            PIXEL_AUDIO_GATEWAY,
        ),
    ] {
        check.method_result(
            interface,
            method,
            path,
            &snapshot.object(path),
            snapshot.method_result(method, path),
        );
    }

    check.passes();
}

#[test]
fn bluez_objects_match_the_pixel() {
    let snapshot = Snapshot::live_call();
    let harness = Harness::start();
    harness.simulated_phone().connect();
    let device = format!(
        "{PIXEL_ADAPTER}/dev_{}",
        harness.phone().address.replace(':', "_")
    );
    let mut check = Check::new(&harness, Bus::System, BLUEZ);

    check.object(
        "the adapter",
        PIXEL_ADAPTER,
        &snapshot.object(PIXEL_ADAPTER),
    );
    check.object("the Phone", &device, &snapshot.object(PIXEL_DEVICE));

    check.passes();
}

#[test]
fn obex_client_matches_the_pixel() {
    let snapshot = Snapshot::read("obex-client.json");
    let harness = Harness::start();
    let mut check = Check::new(&harness, Bus::Session, OBEX);

    check.object("obexd's root", "/", &snapshot.object("/"));
    check.object(
        "obexd's client",
        "/org/bluez/obex",
        &snapshot.object("/org/bluez/obex"),
    );
    check.method_result(
        OBJECT_MANAGER,
        "GetManagedObjects",
        "/",
        &snapshot.object("/"),
        snapshot.method_result("GetManagedObjects", "/"),
    );

    check.passes();
}

#[test]
fn pbap_session_matches_the_pixel() {
    let snapshot = Snapshot::pbap_session();
    let harness = Harness::start();
    harness.simulated_phone().connect();

    // As the app will: open a PBAP session to the Phone, then pick its
    // internal phonebook.
    let session: OwnedObjectPath = harness
        .call_phone_method(
            Bus::Session,
            OBEX,
            "/org/bluez/obex",
            "org.bluez.obex.Client1",
            "CreateSession",
            &(
                harness.phone().address.as_str(),
                HashMap::from([("Target", zbus::Variant::from("pbap"))]),
            ),
        )
        .body()
        .deserialize()
        .expect("CreateSession replies with the session's path");
    let mut check = Check::new(&harness, Bus::Session, OBEX);
    check.object(
        "the PBAP session after CreateSession",
        session.as_str(),
        &snapshot.labelled_object("after CreateSession"),
    );

    let after_select = snapshot.labelled_object("after Select(int, pb)");
    check.call(
        "the PBAP session",
        session.as_str(),
        &after_select,
        "org.bluez.obex.PhonebookAccess1",
        "Select",
        &("int", "pb"),
    );
    check.object(
        "the PBAP session after Select(int, pb)",
        session.as_str(),
        &after_select,
    );

    check.passes();
}

#[test]
fn pbap_pull_matches_the_pixel() {
    let session_snapshot = Snapshot::pbap_session();
    let snapshot = Snapshot::read("obex-transfer.json");
    let harness = Harness::start();
    let phone = harness.simulated_phone();
    phone.set_pbap_folder("pb", &test_harness::vcards::contacts(3, 0));
    // Long enough to look at the transfer before it finishes.
    phone.set_pull_delay(std::time::Duration::from_secs(30));
    phone.connect();

    let session: OwnedObjectPath = harness
        .call_phone_method(
            Bus::Session,
            OBEX,
            "/org/bluez/obex",
            "org.bluez.obex.Client1",
            "CreateSession",
            &(
                harness.phone().address.as_str(),
                HashMap::from([("Target", zbus::Variant::from("pbap"))]),
            ),
        )
        .body()
        .deserialize()
        .expect("CreateSession replies with the session's path");
    let after_select = session_snapshot.labelled_object("after Select(int, pb)");
    let mut check = Check::new(&harness, Bus::Session, OBEX);
    check.call(
        "the PBAP session",
        session.as_str(),
        &after_select,
        PHONEBOOK_ACCESS,
        "Select",
        &("int", "pb"),
    );
    check.call(
        "the PBAP session",
        session.as_str(),
        &after_select,
        PHONEBOOK_ACCESS,
        "GetSize",
        &(),
    );
    let target = harness.directory().join("pb.vcf");
    let reply = check.call(
        "the PBAP session",
        session.as_str(),
        &after_select,
        PHONEBOOK_ACCESS,
        "PullAll",
        &(
            target.to_string_lossy().as_ref(),
            HashMap::from([("Format", zbus::Variant::from("vcard30"))]),
        ),
    );
    let (transfer, properties): (OwnedObjectPath, HashMap<String, OwnedVariant>) = reply
        .body()
        .deserialize()
        .expect("PullAll replies with the transfer and its properties");
    let mut replied: Vec<&str> = properties.keys().map(String::as_str).collect();
    replied.sort_unstable();
    let pixel_replied: Vec<&str> = snapshot.document["PullAll reply properties"]
        .as_array()
        .expect("the snapshot lists PullAll's reply properties")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        replied, pixel_replied,
        "PullAll's reply lists the transfer's properties as the Pixel's did"
    );
    check.object(
        "the transfer during PullAll(pb)",
        transfer.as_str(),
        &snapshot.labelled_object("during PullAll(pb)"),
    );

    check.passes();
}

#[test]
fn introspect_lists_no_children_and_get_modems_has_empty_property_lists() {
    let harness = Harness::start();
    harness.simulated_phone().connect();
    let audio_gateway = harness.simulated_phone().audio_gateway();
    let call = harness.simulated_phone().ring(CALLER);

    for path in [PIXEL_MANAGER, audio_gateway.as_str(), call.as_str()] {
        let xml: String = harness
            .call_phone_method(
                Bus::Session,
                TELEPHONY,
                path,
                INTROSPECTABLE,
                "Introspect",
                &(),
            )
            .body()
            .deserialize()
            .expect("Introspect replies with XML");
        assert_eq!(
            Introspection::parse(&xml).children,
            Vec::<String>::new(),
            "PipeWire's Introspect on {path} lists no child nodes"
        );
    }

    let modems: HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> = harness
        .call_phone_method(
            Bus::Session,
            TELEPHONY,
            PIXEL_MANAGER,
            "org.ofono.Manager",
            "GetModems",
            &(),
        )
        .body()
        .deserialize()
        .expect("GetModems replies with a{oa{sv}}");
    let listed: Vec<(String, usize)> = modems
        .iter()
        .map(|(path, properties)| (path.to_string(), properties.len()))
        .collect();
    assert_eq!(
        listed,
        vec![(audio_gateway.clone(), 0)],
        "GetModems lists the AudioGateway with an empty property list"
    );
}

/// Everything known about the real Pixel, from the committed snapshot.
struct Snapshot {
    document: Value,
}

impl Snapshot {
    fn live_call() -> Self {
        Self::read("live-call.json")
    }

    fn pbap_session() -> Self {
        Self::read("pbap-session.json")
    }

    fn read(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/2026-11-04-dbus-snapshot")
            .join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read the snapshot {}: {error}", path.display()));
        Self {
            document: serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("parse the snapshot {}: {error}", path.display())),
        }
    }

    /// The object at `path` in `live-call.json`.
    fn object(&self, path: &str) -> PixelObject {
        let object = self.document["objects"]
            .as_array()
            .and_then(|objects| objects.iter().find(|object| object["path"] == path))
            .unwrap_or_else(|| panic!("the snapshot has no object at {path}"));
        PixelObject::from_json(object)
    }

    /// The object captured under `label` in `pbap-session.json`.
    fn labelled_object(&self, label: &str) -> PixelObject {
        PixelObject::from_json(&self.document["objects"][label])
    }

    /// What `method` on `path` returned on the Pixel: its type and value.
    fn method_result(&self, method: &str, path: &str) -> &Value {
        let result = &self.document["method_results"][format!("{method}({path})")];
        assert!(
            !result.is_null(),
            "the snapshot has no result for {method}({path})"
        );
        result
    }
}

/// One of the Pixel's objects: its introspection data and the properties it
/// had, by interface, with their types.
struct PixelObject {
    introspection: Introspection,
    properties: BTreeMap<String, BTreeMap<String, String>>,
}

impl PixelObject {
    fn from_json(object: &Value) -> Self {
        let xml = object["introspection_xml"]
            .as_str()
            .expect("each snapshot object has its introspection XML");
        let mut properties = BTreeMap::new();
        if let Some(interfaces) = object["properties"].as_object() {
            for (interface, all) in interfaces {
                let named = all["data"][0]
                    .as_object()
                    .expect("captured properties are an a{sv}")
                    .iter()
                    .map(|(name, value)| (name.clone(), json_type(value)))
                    .collect();
                properties.insert(interface.clone(), named);
            }
        }
        Self {
            introspection: Introspection::parse(xml),
            properties,
        }
    }

    /// The signature of `interface.method`'s out arguments.
    fn reply_signature(&self, interface: &str, method: &str) -> Option<&str> {
        self.introspection
            .reply_signatures
            .get(&format!("{interface}.{method}"))
            .map(String::as_str)
    }
}

/// The type the snapshot recorded for a value: `{"type": "s", "data": ...}`.
fn json_type(value: &Value) -> String {
    value["type"]
        .as_str()
        .expect("each captured value has its type")
        .to_owned()
}

/// Introspection data, flattened into comparable facts.
struct Introspection {
    /// One line per interface and per member, keyed by what it is, such as
    /// `org.ofono.Manager method GetModems` →
    /// `org.ofono.Manager method GetModems(out a{oa{sv}} objects)`.
    facts: BTreeMap<String, String>,
    /// The interfaces that declare properties.
    interfaces_with_properties: Vec<String>,
    /// The signature of each method's out arguments, by `interface.method`.
    reply_signatures: BTreeMap<String, String>,
    /// The names of the child nodes listed.
    children: Vec<String>,
}

impl Introspection {
    fn parse(xml: &str) -> Self {
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let document = roxmltree::Document::parse_with_options(xml, options)
            .unwrap_or_else(|error| panic!("parse introspection XML: {error}\n{xml}"));
        let mut introspection = Self {
            facts: BTreeMap::new(),
            interfaces_with_properties: Vec::new(),
            reply_signatures: BTreeMap::new(),
            children: Vec::new(),
        };
        for element in document
            .root_element()
            .children()
            .filter(roxmltree::Node::is_element)
        {
            let name = element.attribute("name").unwrap_or_default();
            match element.tag_name().name() {
                "node" => introspection.children.push(name.to_owned()),
                "interface" => introspection.add_interface(name, element),
                other => panic!("unexpected <{other}> in introspection XML"),
            }
        }
        introspection
    }

    fn add_interface(&mut self, interface: &str, element: roxmltree::Node<'_, '_>) {
        self.facts
            .insert(interface.to_owned(), format!("interface {interface}"));
        for member in element.children().filter(roxmltree::Node::is_element) {
            let kind = member.tag_name().name();
            let name = member.attribute("name").unwrap_or_default();
            let mut line = format!("{interface} {kind} {name}");
            match kind {
                "method" | "signal" => {
                    let arguments: Vec<String> = children_named(member, "arg")
                        .map(|argument| {
                            let direction = match (kind, argument.attribute("direction")) {
                                // A method argument is "in" unless it says otherwise.
                                ("method", Some("out")) => "out ",
                                ("method", _) => "in ",
                                _ => "",
                            };
                            format!(
                                "{direction}{} {}",
                                argument.attribute("type").unwrap_or_default(),
                                argument.attribute("name").unwrap_or_default()
                            )
                        })
                        .collect();
                    let _ = write!(line, "({})", arguments.join(", "));
                    if kind == "method" {
                        let reply: String = children_named(member, "arg")
                            .filter(|argument| argument.attribute("direction") == Some("out"))
                            .filter_map(|argument| argument.attribute("type"))
                            .collect();
                        self.reply_signatures
                            .insert(format!("{interface}.{name}"), reply);
                    }
                }
                "property" => {
                    let _ = write!(
                        line,
                        " {} {}",
                        member.attribute("type").unwrap_or_default(),
                        member.attribute("access").unwrap_or_default()
                    );
                    if !self
                        .interfaces_with_properties
                        .iter()
                        .any(|known| known == interface)
                    {
                        self.interfaces_with_properties.push(interface.to_owned());
                    }
                }
                "annotation" => {
                    line = format!(
                        "{interface} annotation {name}={}",
                        member.attribute("value").unwrap_or_default()
                    );
                }
                other => panic!("unexpected <{other}> in interface {interface}"),
            }
            for annotation in children_named(member, "annotation") {
                let _ = write!(
                    line,
                    " @{}={}",
                    annotation.attribute("name").unwrap_or_default(),
                    annotation.attribute("value").unwrap_or_default()
                );
            }
            self.facts
                .insert(format!("{interface} {kind} {name}"), line);
        }
    }
}

fn children_named<'a, 'input>(
    element: roxmltree::Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = roxmltree::Node<'a, 'input>> {
    element
        .children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}

/// Compares the Simulated Phone with the Pixel on one bus and service, and
/// collects every difference.
struct Check<'harness> {
    harness: &'harness Harness,
    bus: Bus,
    service: &'static str,
    /// The Simulated Phone's object paths that stand for the Pixel's, such
    /// as its `agN` for the Pixel's `ag1`.
    paths: Vec<(String, String)>,
    differences: Vec<String>,
}

impl<'harness> Check<'harness> {
    fn new(harness: &'harness Harness, bus: Bus, service: &'static str) -> Self {
        Self {
            harness,
            bus,
            service,
            paths: Vec::new(),
            differences: Vec::new(),
        }
    }

    /// Says that the Simulated Phone's `simulated` path stands for the
    /// Pixel's `pixel` path in replies that list objects.
    fn map_path(&mut self, simulated: &str, pixel: &str) {
        self.paths.push((simulated.to_owned(), pixel.to_owned()));
    }

    /// Compares the object at `path` with the Pixel's: introspection data,
    /// then the properties of each interface that declares any.
    fn object(&mut self, what: &str, path: &str, pixel: &PixelObject) {
        let xml: String = self
            .call(what, path, pixel, INTROSPECTABLE, "Introspect", &())
            .body()
            .deserialize()
            .unwrap_or_else(|error| panic!("{what}: Introspect replied: {error}"));
        let simulated = Introspection::parse(&xml);
        self.compare(what, &pixel.introspection.facts, &simulated.facts);

        for interface in &pixel.introspection.interfaces_with_properties {
            let served: HashMap<String, OwnedVariant> = self
                .call(
                    what,
                    path,
                    pixel,
                    PROPERTIES,
                    "GetAll",
                    &(interface.as_str(),),
                )
                .body()
                .deserialize()
                .unwrap_or_else(|error| panic!("{what}: GetAll({interface}) replied: {error}"));
            let pixel_facts = property_facts(
                interface,
                pixel.properties.get(interface).cloned().unwrap_or_default(),
            );
            let simulated_facts = property_facts(
                interface,
                served
                    .iter()
                    .map(|(name, value)| (name.clone(), value.value_signature().to_string()))
                    .collect(),
            );
            self.compare(what, &pixel_facts, &simulated_facts);
        }
    }

    /// Calls `interface.method` on `path` and compares which objects,
    /// interfaces and properties its reply lists with what the Pixel's did.
    fn method_result(
        &mut self,
        interface: &str,
        method: &str,
        path: &str,
        pixel: &PixelObject,
        result: &Value,
    ) {
        let what = format!("{method}({path})");
        let simulated_path = self
            .paths
            .iter()
            .find(|(_, pixel_path)| pixel_path == path)
            .map_or(path, |(simulated, _)| simulated.as_str())
            .to_owned();
        let reply = self.call(&what, &simulated_path, pixel, interface, method, &());
        let signature = reply.body().signature().to_string_no_parens();
        let pixel_type = json_type(result);
        if signature != pixel_type {
            self.differences.push(format!(
                "{what}: the Pixel replied `{pixel_type}`, the Simulated Phone `{signature}`"
            ));
            return;
        }
        let mut pixel_facts = BTreeMap::new();
        let mut simulated_facts = BTreeMap::new();
        match signature.as_str() {
            "a{oa{sv}}" => {
                listed_objects(&mut pixel_facts, &result["data"][0], |properties| {
                    BTreeMap::from([(String::new(), json_properties(properties))])
                });
                let objects: HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> = reply
                    .body()
                    .deserialize()
                    .unwrap_or_else(|error| panic!("{what}: {error}"));
                for (object, properties) in objects {
                    self.list_simulated(
                        &mut simulated_facts,
                        &object,
                        BTreeMap::from([(String::new(), variant_types(&properties))]),
                    );
                }
            }
            "a{oa{sa{sv}}}" => {
                listed_objects(&mut pixel_facts, &result["data"][0], |interfaces| {
                    interfaces
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(interface, properties)| {
                            (interface.clone(), json_properties(properties))
                        })
                        .collect()
                });
                let objects: HashMap<
                    OwnedObjectPath,
                    HashMap<String, HashMap<String, OwnedVariant>>,
                > = reply
                    .body()
                    .deserialize()
                    .unwrap_or_else(|error| panic!("{what}: {error}"));
                for (object, interfaces) in objects {
                    let interfaces = interfaces
                        .iter()
                        .map(|(interface, properties)| {
                            (interface.clone(), variant_types(properties))
                        })
                        .collect();
                    self.list_simulated(&mut simulated_facts, &object, interfaces);
                }
            }
            other => panic!("{what}: no comparison for replies of type {other}"),
        }
        self.compare(&what, &pixel_facts, &simulated_facts);
    }

    fn list_simulated(
        &self,
        facts: &mut BTreeMap<String, String>,
        object: &OwnedObjectPath,
        interfaces: BTreeMap<String, BTreeMap<String, String>>,
    ) {
        let object = self
            .paths
            .iter()
            .find(|(simulated, _)| simulated == object.as_str())
            .map_or(object.as_str(), |(_, pixel)| pixel.as_str());
        add_listed_object(facts, object, interfaces);
    }

    /// Calls a method on the Simulated Phone and checks that the reply's
    /// signature is the one the Pixel declares for that method.
    fn call<B>(
        &mut self,
        what: &str,
        path: &str,
        pixel: &PixelObject,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Message
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType,
    {
        let reply =
            self.harness
                .call_phone_method(self.bus, self.service, path, interface, method, body);
        // zbus holds a body of several arguments as one structure; the wire,
        // like the declared out arguments, has no parentheses around them.
        let signature = reply.body().signature().to_string_no_parens();
        match pixel.reply_signature(interface, method) {
            Some(declared) if declared == signature => {}
            Some(declared) => self.differences.push(format!(
                "{what}: {interface}.{method} replied `{signature}`, but the Pixel declares `{declared}`"
            )),
            None => self.differences.push(format!(
                "{what}: the Simulated Phone answers {interface}.{method}, which the Pixel doesn't declare"
            )),
        }
        reply
    }

    fn compare(
        &mut self,
        what: &str,
        pixel: &BTreeMap<String, String>,
        simulated: &BTreeMap<String, String>,
    ) {
        for (key, pixel_line) in pixel {
            match simulated.get(key) {
                Some(simulated_line) if simulated_line == pixel_line => {}
                Some(simulated_line) => self.differences.push(format!(
                    "{what}: the Pixel has `{pixel_line}`, the Simulated Phone has `{simulated_line}`"
                )),
                None => self.differences.push(format!(
                    "{what}: the Pixel has `{pixel_line}`, the Simulated Phone doesn't"
                )),
            }
        }
        for (key, simulated_line) in simulated {
            if !pixel.contains_key(key) {
                self.differences.push(format!(
                    "{what}: the Simulated Phone has `{simulated_line}`, the Pixel doesn't"
                ));
            }
        }
    }

    /// Fails the scenario if anything differed, listing every difference.
    fn passes(self) {
        assert!(
            self.differences.is_empty(),
            "the Simulated Phone differs from the real Pixel in {} way(s):\n  {}",
            self.differences.len(),
            self.differences.join("\n  ")
        );
    }
}

/// One fact per property: `GetAll(interface) serves Name: type`.
fn property_facts(
    interface: &str,
    properties: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    properties
        .into_iter()
        .map(|(name, signature)| {
            (
                format!("{interface}.{name}"),
                format!("GetAll({interface}) serves {name}: {signature}"),
            )
        })
        .collect()
}

/// Facts for each object a reply lists, from the snapshot's JSON.
fn listed_objects(
    facts: &mut BTreeMap<String, String>,
    objects: &Value,
    interfaces_of: impl Fn(&Value) -> BTreeMap<String, BTreeMap<String, String>>,
) {
    for (object, value) in objects.as_object().into_iter().flatten() {
        add_listed_object(facts, object, interfaces_of(value));
    }
}

/// Facts for one listed object: that it is listed, each interface (unless
/// the reply has no interface level), and each property with its type.
fn add_listed_object(
    facts: &mut BTreeMap<String, String>,
    object: &str,
    interfaces: BTreeMap<String, BTreeMap<String, String>>,
) {
    facts.insert(object.to_owned(), format!("lists {object}"));
    for (interface, properties) in interfaces {
        let prefix = if interface.is_empty() {
            object.to_owned()
        } else {
            facts.insert(
                format!("{object} {interface}"),
                format!("lists {object} with {interface}"),
            );
            format!("{object} {interface}")
        };
        for (name, signature) in properties {
            facts.insert(
                format!("{prefix}.{name}"),
                format!("lists {prefix}.{name}: {signature}"),
            );
        }
    }
}

fn json_properties(properties: &Value) -> BTreeMap<String, String> {
    properties
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.clone(), json_type(value)))
        .collect()
}

fn variant_types(properties: &HashMap<String, OwnedVariant>) -> BTreeMap<String, String> {
    properties
        .iter()
        .map(|(name, value)| (name.clone(), value.value_signature().to_string()))
        .collect()
}
