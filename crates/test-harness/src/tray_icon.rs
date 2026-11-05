//! A tray item's icon drawn as Omarchy's bar draws it, for the gallery.
//!
//! Omarchy 4.5's tray (`Tray.qml`) looks an icon name up in `IconThemePath`
//! when its icon theme lacks it, draws it [`ICON_SIZE`] logical pixels
//! square, and paints a `-symbolic` icon in the bar's foreground through
//! its alpha mask, keeping only pixels at least half opaque
//! (`maskThresholdMin: 0.5`). This draws the SVG the app wrote with
//! `rsvg-convert` (librsvg, which GTK already needs) the same way, on a
//! strip of the bar's background.

use std::fs;
use std::io::{BufWriter, Cursor};
use std::path::Path;
use std::process::Command;

use crate::stand_ins::TrayItem;

/// The tray icon's size in Omarchy's bar, in logical pixels:
/// `Style.bar.iconSize`, at most 12.
const ICON_SIZE: u32 = 12;
/// The bar's height in logical pixels. Keep in step with [`crate::BAR_HEIGHT`].
const BAR_HEIGHT: u32 = 26;
/// How wide a strip of bar to draw, in logical pixels.
const STRIP_WIDTH: u32 = 48;
/// Pixels to the logical pixel: a 4K output at Omarchy's 2× scaling.
const SCALE: u32 = 2;

/// Draws `item`'s icon as Omarchy's bar shows it, on a strip of bar in
/// `background`, painted in `foreground`, into the PNG at `output`.
///
/// # Errors
///
/// If the item names no icon file, `rsvg-convert` can't draw it, or the
/// PNG can't be written.
pub fn draw_tray_icon(
    item: &TrayItem,
    background: [u8; 3],
    foreground: [u8; 3],
    output: &Path,
) -> Result<(), String> {
    let (Some(directory), Some(name)) = (&item.icon_theme_path, &item.icon_name) else {
        return Err(format!("{item:?} names no icon file"));
    };
    if !name.ends_with("-symbolic") {
        return Err(format!(
            "{name} isn't symbolic, so Omarchy wouldn't recolour it"
        ));
    }
    let svg = Path::new(directory).join(format!("{name}.svg"));
    let size = ICON_SIZE * SCALE;
    let rendered = Command::new("rsvg-convert")
        .args(["--width", &size.to_string(), "--height", &size.to_string()])
        .arg(&svg)
        .output()
        .map_err(|error| format!("run rsvg-convert: {error}"))?;
    if !rendered.status.success() {
        return Err(format!(
            "rsvg-convert {}: {}",
            svg.display(),
            String::from_utf8_lossy(&rendered.stderr)
        ));
    }
    let alpha = alpha_of(&rendered.stdout, size)?;

    let (width, height) = (STRIP_WIDTH * SCALE, BAR_HEIGHT * SCALE);
    let (left, top) = ((width - size) / 2, (height - size) / 2);
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let inside = (left..left + size).contains(&x) && (top..top + size).contains(&y);
            let opaque = inside
                && alpha
                    .get(usize::try_from((y - top) * size + (x - left)).unwrap_or(usize::MAX))
                    .is_some_and(|alpha| *alpha >= 128);
            pixels.extend_from_slice(if opaque { &foreground } else { &background });
        }
    }
    write_png(output, width, height, &pixels)
}

/// The alpha channel of the `size`-square RGBA PNG `png`.
fn alpha_of(png: &[u8], size: u32) -> Result<Vec<u8>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("read rsvg-convert's PNG: {error}"))?;
    let mut data = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let frame = reader
        .next_frame(&mut data)
        .map_err(|error| format!("decode rsvg-convert's PNG: {error}"))?;
    if frame.width != size || frame.height != size || frame.color_type != png::ColorType::Rgba {
        return Err(format!("rsvg-convert drew {frame:?}, not {size}² RGBA"));
    }
    data.truncate(frame.buffer_size());
    let (pixels, _) = data.as_chunks::<4>();
    Ok(pixels.iter().map(|[_, _, _, alpha]| *alpha).collect())
}

fn write_png(output: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let file =
        fs::File::create(output).map_err(|error| format!("{}: {error}", output.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("{}: {error}", output.display()))?;
    writer
        .write_image_data(pixels)
        .and_then(|()| writer.finish())
        .map_err(|error| format!("{}: {error}", output.display()))
}
