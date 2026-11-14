//! What Hyprland says about the Desktop's layout, through its request socket
//! (what `hyprctl` uses): `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/
//! .socket.sock`. A client writes one request, such as
//! `j/getoption decoration:rounding` for `hyprctl getoption -j
//! decoration:rounding`, and reads the reply until Hyprland closes the socket.
//! Next to it, `.socket2.sock` announces events.

use std::env;
use std::io::{self, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

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
    /// The answer wasn't `hyprctl getoption -j`'s JSON.
    Unreadable(serde_json::Error),
    /// The answer had no `decoration:rounding`.
    NoRounding,
    /// The answer had no `general:gaps_out`.
    NoGap,
}

impl Unknown {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotHyprland => "not-hyprland",
            Self::Unreachable(_) => "hyprland-unreachable",
            Self::Unreadable(_) => "hyprland-unreadable",
            Self::NoRounding => "no-rounding",
            Self::NoGap => "no-gap",
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

/// Omarchy's edge gap, in layout pixels: half of Hyprland's
/// `general:gaps_out`, rounded, as Omarchy's shell computes it for its own
/// panels and notifications (`/usr/share/omarchy/shell/Commons/Style.qml`,
/// `applyGapsOutJson`). Blocks for as long as Hyprland takes to answer.
pub fn edge_gap() -> Result<u32, Unknown> {
    let reply = request("j/getoption general:gaps_out")?;
    // Hyprland's answer, in plain text, for an option it doesn't have.
    if reply.trim_ascii() == b"no such option" {
        return Err(Unknown::NoGap);
    }
    let reply: serde_json::Value = serde_json::from_slice(&reply).map_err(Unknown::Unreadable)?;
    // {"option": "general:gaps_out", "css": "10 10 10 10", "set": true}: the
    // top, right, bottom and left gaps, of which Omarchy takes the first.
    let gaps_out = reply
        .get("css")
        .and_then(serde_json::Value::as_str)
        .and_then(|css| css.split_whitespace().next()?.parse::<f64>().ok())
        .or_else(|| reply.get("int")?.as_f64())
        .filter(|gaps_out| gaps_out.is_finite() && *gaps_out >= 0.0)
        .ok_or(Unknown::NoGap)?;
    // Rounded half up, as JavaScript's `Math.round`.
    let half = (gaps_out / 2.0 + 0.5).floor().min(f64::from(u16::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "half is a whole number from 0 to u16::MAX"
    )]
    Ok(half as u32)
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
