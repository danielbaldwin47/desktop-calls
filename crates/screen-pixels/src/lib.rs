//! The test harness's per-pixel work on 3840 by 2160 screenshots: finding
//! where a colour is, and converting the compositor's frames to RGB.
//!
//! A crate of its own so that the workspace's `Cargo.toml` can optimise it
//! even in debug builds while the harness and the gallery, whose scenarios
//! are most of what CI compiles, build unoptimised: at the harness's old
//! opt-level 1, compiling the scenarios took 45 s of every CI job that ran
//! them, against about 15 s unoptimised. Unoptimised, these loops would
//! take a frame's worth of time a screenshot (ticket #55).

/// A rectangle of pixels: columns `left..right` and rows `top..bottom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
}

/// Decoded pixels: rows of `line_size` bytes, `channels` bytes a pixel, with
/// red, green and blue first.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub channels: usize,
    pub line_size: usize,
}

impl Image<'_> {
    /// How many rows of pixels there are.
    #[must_use]
    pub fn height(&self) -> usize {
        self.data.len() / self.line_size
    }
}

/// The smallest bounds holding every pixel inside `within` whose colour
/// satisfies `matches`, or `None` if none does.
///
/// `matches` is asked once for each run of pixels of one colour, not once a
/// pixel: a screen is mostly runs of flat colour, and the caller's closure,
/// compiled in an unoptimised crate, then costs little. So `matches` must
/// answer the same for the same colour.
#[must_use]
pub fn bounds_where(
    image: &Image<'_>,
    within: Bounds,
    matches: &dyn Fn([u8; 3]) -> bool,
) -> Option<Bounds> {
    let right_edge = within.right.min(image.width);
    let bottom_edge = within.bottom.min(image.height());
    let mut found: Option<Bounds> = None;
    let mut last: Option<([u8; 3], bool)> = None;
    for row in within.top..bottom_edge {
        let line = &image.data[row * image.line_size..][..image.line_size];
        for column in within.left..right_edge {
            let at = column * image.channels;
            let colour = [line[at], line[at + 1], line[at + 2]];
            let counts = match last {
                Some((previous, answer)) if previous == colour => answer,
                _ => {
                    let answer = matches(colour);
                    last = Some((colour, answer));
                    answer
                }
            };
            if counts {
                let bounds = found.get_or_insert(Bounds {
                    left: column,
                    top: row,
                    right: column + 1,
                    bottom: row + 1,
                });
                bounds.left = bounds.left.min(column);
                bounds.right = bounds.right.max(column + 1);
                bounds.bottom = row + 1;
            }
        }
    }
    found
}

/// Converts a frame of 4-byte pixels, `rows` rows of `stride` bytes, into
/// packed RGB, top row first. `offsets` are the bytes of red, green and
/// blue within a pixel; `inverted` says the frame's first row is the
/// bottom one.
#[must_use]
pub fn packed_rgb(
    pixels: &[u8],
    (columns, rows, stride): (usize, usize, usize),
    inverted: bool,
    [red, green, blue]: [usize; 3],
) -> Vec<u8> {
    let mut rgb = vec![0; columns * rows * 3];
    for (row, converted) in rgb.chunks_exact_mut(columns * 3).enumerate() {
        let row = if inverted { rows - 1 - row } else { row };
        let line = &pixels[row * stride..row * stride + columns * 4];
        for (converted, pixel) in converted
            .as_chunks_mut::<3>()
            .0
            .iter_mut()
            .zip(line.as_chunks::<4>().0)
        {
            *converted = [pixel[red], pixel[green], pixel[blue]];
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 3] = [255, 255, 255];
    const RED: [u8; 3] = [255, 0, 0];

    /// A 6 by 4 white image, 4 bytes a pixel, red where `red` says.
    fn image(red: &[(usize, usize)]) -> Vec<u8> {
        let mut data = Vec::new();
        for row in 0..4 {
            for column in 0..6 {
                let colour = if red.contains(&(column, row)) {
                    RED
                } else {
                    WHITE
                };
                data.extend_from_slice(&colour);
                data.push(255);
            }
        }
        data
    }

    fn view(data: &[u8]) -> Image<'_> {
        Image {
            data,
            width: 6,
            channels: 4,
            line_size: 24,
        }
    }

    const EVERYWHERE: Bounds = Bounds {
        left: 0,
        top: 0,
        right: usize::MAX,
        bottom: usize::MAX,
    };

    #[test]
    fn the_bounds_hold_every_matching_pixel_and_no_more() {
        let data = image(&[(1, 1), (4, 2), (2, 3)]);
        let found = bounds_where(&view(&data), EVERYWHERE, &|colour| colour == RED);
        assert_eq!(
            found,
            Some(Bounds {
                left: 1,
                top: 1,
                right: 5,
                bottom: 4
            })
        );
    }

    #[test]
    fn pixels_outside_within_are_not_counted() {
        let data = image(&[(1, 1), (4, 2)]);
        let within = Bounds {
            left: 3,
            top: 0,
            right: 6,
            bottom: 4,
        };
        let found = bounds_where(&view(&data), within, &|colour| colour == RED);
        assert_eq!(
            found,
            Some(Bounds {
                left: 4,
                top: 2,
                right: 5,
                bottom: 3
            })
        );
    }

    #[test]
    fn no_matching_pixel_finds_nothing() {
        let data = image(&[]);
        assert_eq!(
            bounds_where(&view(&data), EVERYWHERE, &|colour| colour == RED),
            None
        );
    }

    #[test]
    fn a_frame_converts_to_rgb_in_its_byte_order_and_upright() {
        // Two rows of two pixels, blue-green-red-unused, with 4 bytes of
        // padding a row, bottom row first.
        let frame = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        let rgb = packed_rgb(&frame, (2, 2, 12), true, [2, 1, 0]);
        assert_eq!(rgb, [9, 8, 7, 12, 11, 10, 3, 2, 1, 6, 5, 4]);
    }
}
