//! The halfblocks image encoder, for the surfaces ratatui-image does not
//! run on: the `--dump` frame (both builds) and the browser (the crate's
//! rustix dependency does not compile for wasm32-unknown-unknown, and a
//! dump must not emit a terminal protocol). It is a copy of the crate's
//! own pure-Rust halfblocks path (ratatui-image 11.1.0,
//! `src/protocol/halfblocks/primitive.rs` and `halfblocks.rs`, MIT).
//!
//! Two pixels per cell, upper and lower, become one `▀` or `▄` — a space
//! when the pair is equal — with truecolor fore- and background. The cell
//! aspect is 1:2, the host's 8x16 cell metric, so an image at `size` cells
//! fills the slot.

use image::imageops::FilterType;
use image::DynamicImage;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Rect, Size};
use ratatui::style::Color;

const HALF_UPPER: char = '▀';
const HALF_LOWER: char = '▄';
const SPACE: char = ' ';

#[derive(Clone)]
pub(crate) struct HalfBlock {
    upper: Color,
    lower: Color,
    char: char,
}

/// The image at `size` cells: one halfblock per cell.
pub(crate) fn encode(img: &DynamicImage, size: Size) -> Vec<HalfBlock> {
    let img = img.resize_exact(
        size.width as u32,
        u32::from(size.height) * 2,
        FilterType::Triangle,
    );

    let mut data = vec![
        HalfBlock {
            upper: Color::Rgb(0, 0, 0),
            lower: Color::Rgb(0, 0, 0),
            char: HALF_UPPER,
        };
        usize::from(size.width) * usize::from(size.height)
    ];

    for (y, row) in img.to_rgb8().rows().enumerate() {
        for (x, pixel) in row.enumerate() {
            let position = x + (size.width as usize) * (y / 2);
            if y % 2 == 0 {
                data[position].upper = Color::Rgb(pixel[0], pixel[1], pixel[2]);
            } else {
                data[position].lower = Color::Rgb(pixel[0], pixel[1], pixel[2]);
            }
        }
    }
    for hb in &mut data {
        hb.pick_side();
    }
    data
}

/// The encoded cells into `area`, clipped to it.
pub(crate) fn render(data: &[HalfBlock], size: Size, area: Rect, buf: &mut Buffer) {
    for (i, hb) in data.iter().enumerate() {
        // `i` indexes the cell grid; keep it wide through the divide, so a
        // large image is not aliased back to the top left.
        let x = (i % usize::from(size.width)) as u16;
        let y = (i / usize::from(size.width)) as u16;
        if x >= area.width || y >= area.height {
            continue;
        }
        if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)) {
            hb.set_cell(cell);
        }
    }
}

impl HalfBlock {
    fn set_cell(&self, cell: &mut Cell) {
        cell.set_fg(self.upper)
            .set_bg(self.lower)
            .set_char(self.char);
    }

    fn pick_side(&mut self) {
        if self.upper == self.lower {
            self.char = SPACE;
            return;
        }
        if Self::luminance(Self::rgb(self.lower)) > Self::luminance(Self::rgb(self.upper)) {
            std::mem::swap(&mut self.upper, &mut self.lower);
            self.char = HALF_LOWER;
        }
    }

    fn luminance((r, g, b): (u8, u8, u8)) -> u32 {
        2126 * r as u32 + 7152 * g as u32 + 722 * b as u32
    }

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(i) => Self::indexed_to_rgb(i),
            Color::Black => (0, 0, 0),
            Color::Red => (205, 0, 0),
            Color::Green => (0, 205, 0),
            Color::Yellow => (205, 205, 0),
            Color::Blue => (0, 0, 238),
            Color::Magenta => (205, 0, 205),
            Color::Cyan => (0, 205, 205),
            Color::Gray => (229, 229, 229),
            Color::DarkGray => (127, 127, 127),
            Color::LightRed => (255, 0, 0),
            Color::LightGreen => (0, 255, 0),
            Color::LightYellow => (255, 255, 0),
            Color::LightBlue => (92, 92, 255),
            Color::LightMagenta => (255, 0, 255),
            Color::LightCyan => (0, 255, 255),
            Color::White => (255, 255, 255),
            Color::Reset => (255, 255, 255), // assume a light background
        }
    }

    fn indexed_to_rgb(i: u8) -> (u8, u8, u8) {
        match i {
            0..=15 => match i {
                0 => (0, 0, 0),
                1 => (205, 0, 0),
                2 => (0, 205, 0),
                3 => (205, 205, 0),
                4 => (0, 0, 238),
                5 => (205, 0, 205),
                6 => (0, 205, 205),
                7 => (229, 229, 229),
                8 => (127, 127, 127),
                9 => (255, 0, 0),
                10 => (0, 255, 0),
                11 => (255, 255, 0),
                12 => (92, 92, 255),
                13 => (255, 0, 255),
                14 => (0, 255, 255),
                15 => (255, 255, 255),
                _ => unreachable!(),
            },
            16..=231 => {
                // The 6x6x6 color cube.
                let i = i - 16;
                let r = (i / 36) % 6;
                let g = (i / 6) % 6;
                let b = i % 6;
                let to_val = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
                (to_val(r), to_val(g), to_val(b))
            }
            232..=255 => {
                // The grayscale ramp.
                let gray = 8 + (i - 232) * 10;
                (gray, gray, gray)
            }
        }
    }
}
