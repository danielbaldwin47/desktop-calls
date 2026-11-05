//! What Hyprland says about the Desktop's layout, through its request socket
//! (what `hyprctl` uses): `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/
//! .socket.sock`. A client writes one request, such as `j/layers` for
//! `hyprctl layers -j`, and reads the reply until Hyprland closes the socket.
//! Next to it, `.socket2.sock` announces events.

use std::env;
use std::io::{self, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// The layer-shell namespace of Omarchy's notification toasts.
const TOASTS_NAMESPACE: &str = "omarchy-notifications";

/// How long Hyprland may take to answer. It answers in well under a
/// millisecond; this only stops a wedged compositor holding a thread.
const TIMEOUT: Duration = Duration::from_secs(1);

/// Why Hyprland couldn't say.
#[derive(Debug)]
pub enum Unknown {
    /// The Desktop isn't Hyprland.
    NotHyprland,
    /// Hyprland didn't answer.
    Unreachable(io::Error),
    /// The answer wasn't `hyprctl layers -j`'s JSON.
    Unreadable(serde_json::Error),
    /// Omarchy's notification toasts have no layer.
    NoToastLayer,
    /// The answer had no `decoration:rounding`.
    NoRounding,
}

impl Unknown {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotHyprland => "not-hyprland",
            Self::Unreachable(_) => "hyprland-unreachable",
            Self::Unreadable(_) => "hyprland-unreadable",
            Self::NoToastLayer => "no-toast-layer",
            Self::NoRounding => "no-rounding",
        }
    }
}

/// How round Hyprland draws window corners, `decoration:rounding`, in layout
/// pixels. Blocks for as long as Hyprland takes to answer.
pub fn rounding() -> Result<u32, Unknown> {
    let reply = request("j/getoption decoration:rounding")?;
    let reply: serde_json::Value = serde_json::from_slice(&reply).map_err(Unknown::Unreadable)?;
    // {"option": "decoration:rounding", "int": 0, "set": true}
    reply
        .get("int")
        .and_then(serde_json::Value::as_u64)
        .and_then(|rounding| u32::try_from(rounding).ok())
        .ok_or(Unknown::NoRounding)
}

/// Hyprland's event socket, where it announces what happens one line at a
/// time, such as `configreloaded>>` after its configuration is reloaded.
pub fn event_socket() -> Result<PathBuf, Unknown> {
    Ok(instance_directory()?.join(".socket2.sock"))
}

fn instance_directory() -> Result<PathBuf, Unknown> {
    let signature = env::var_os("HYPRLAND_INSTANCE_SIGNATURE").ok_or(Unknown::NotHyprland)?;
    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or(Unknown::NotHyprland)?;
    Ok(PathBuf::from(runtime).join("hypr").join(signature))
}

/// The width, in layout pixels, of Omarchy's notification-toast column: its
/// widest `omarchy-notifications` layer on any output. Blocks for as long as
/// Hyprland takes to answer.
pub fn toast_column_width() -> Result<u32, Unknown> {
    let layers = request("j/layers")?;
    let layers: serde_json::Value = serde_json::from_slice(&layers).map_err(Unknown::Unreadable)?;
    // {"<output>": {"levels": {"<level>": [{"namespace": …, "w": …}, …]}}}
    layers
        .as_object()
        .into_iter()
        .flat_map(|outputs| outputs.values())
        .filter_map(|output| output.get("levels")?.as_object())
        .flat_map(|levels| levels.values())
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter(|layer| {
            layer.get("namespace").and_then(serde_json::Value::as_str) == Some(TOASTS_NAMESPACE)
        })
        .filter_map(|layer| u32::try_from(layer.get("w")?.as_u64()?).ok())
        .max()
        .ok_or(Unknown::NoToastLayer)
}

fn request(request: &str) -> Result<Vec<u8>, Unknown> {
    let socket = instance_directory()?.join(".socket.sock");
    let exchange = || -> io::Result<Vec<u8>> {
        let mut stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        stream.write_all(request.as_bytes())?;
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply)?;
        Ok(reply)
    };
    exchange().map_err(Unknown::Unreachable)
}
