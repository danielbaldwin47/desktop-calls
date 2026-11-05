//! `automation screenshot`: the gallery of every surface in every state,
//! light and dark, stock and under an Omarchy theme.

use std::path::Path;
use std::process::Command;

/// Renders the gallery exactly as `just screenshot` does, into its default
/// directory, `target/screenshots/gallery`. CI keeps that directory as an
/// artifact, so this test is also what makes CI's gallery.
#[test]
fn the_gallery_shows_every_state_in_light_and_dark_stock_and_under_omarchy() {
    let binary = Path::new(env!("CARGO_BIN_EXE_automation"));
    let output = binary
        .parent()
        .and_then(Path::parent)
        .expect("the binary is in target/<profile>/")
        .join("screenshots/gallery");

    let rendered = Command::new(binary)
        .arg("screenshot")
        .output()
        .expect("run automation screenshot");

    assert!(
        rendered.status.success(),
        "automation screenshot failed ({}). Its standard error:\n{}",
        rendered.status,
        String::from_utf8_lossy(&rendered.stderr)
    );
    let index =
        std::fs::read_to_string(output.join("index.html")).expect("the gallery has an index");
    let states = [
        "bare-output",
        "incoming-call-banner",
        "call-window-live",
        "call-window-muted",
        "call-window-mute-unavailable",
        "call-window-ended",
    ];
    let files = states.iter().flat_map(|state| {
        ["light", "dark", "omarchy-light", "omarchy-dark"].map(|look| format!("{state}-{look}.png"))
    });
    for file in files {
        let file = file.as_str();
        assert!(index.contains(file), "the index shows {file}: {index}");
        let png = std::fs::read(output.join(file)).expect("the screenshot exists");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "{file} is a PNG");
        // Width and height from the PNG header: the 3840 by 2160 output.
        assert_eq!(&png[16..24], &[0, 0, 0x0f, 0, 0, 0, 0x08, 0x70], "{file}");
    }
}
