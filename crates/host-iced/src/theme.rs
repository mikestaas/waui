//! This host's iced theme: the web page's palette, so the windowed host
//! reads like a sibling of the static web host.
//!
//! The web page (`web/index.html`) is built on the Tokyo Night palette —
//! `#1a1b26` background, `#c0caf5` text, `#24283b` buttons. Iced ships a
//! built-in `Theme::TokyoNight` that is close, but its text and primary
//! colors differ from the page's, so we feed the page's exact values into
//! a custom theme instead. The component's semantic tokens then land on
//! the same colors in the window as in the browser.

use iced::theme::{Palette, Style};
use iced::{Color, Theme};

/// The palette, matching `web/index.html`.
fn palette() -> Palette {
    Palette {
        background: Color::from_rgb8(0x1a, 0x1b, 0x26),
        text: Color::from_rgb8(0xc0, 0xca, 0xf5),
        primary: Color::from_rgb8(0x7a, 0xa2, 0xf7),
        success: Color::from_rgb8(0x9e, 0xce, 0x6a),
        warning: Color::from_rgb8(0xe0, 0xaf, 0x68),
        danger: Color::from_rgb8(0xf7, 0x76, 0x8e),
    }
}

/// The widget theme; button chrome and the like derive from this palette.
pub fn theme() -> Theme {
    Theme::custom("waui", palette())
}

/// The window's base style: background color and default text color.
pub fn style() -> Style {
    let palette = palette();
    Style {
        background_color: palette.background,
        text_color: palette.text,
    }
}
