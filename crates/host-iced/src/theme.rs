//! This host's iced theme: the web page's exact palette, so the windowed
//! host matches the static web host.
//!
//! The web page (`web/index.html`) uses the Tokyo Night palette —
//! `#1a1b26` background, `#c0caf5` text, `#24283b` buttons. Iced ships a
//! built-in `Theme::TokyoNight` whose text and primary colors differ from
//! the page's, so the page's exact values go into a custom theme instead.
//!
//! iced 0.14 custom themes are palette-only, so the button chrome (the
//! page's `.waui-button` fill, border, and radius) is a style function
//! applied per widget in `map_tree`, not part of the `Theme`.

use iced::theme::{Palette, Style};
use iced::widget::button;
use iced::{border, Background, Border, Color, Shadow, Theme};

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

/// The widget theme; the palette's dark tones.
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

/// The web page's button chrome (`.waui-button` in `web/index.html`):
/// a dark fill, a subtle border, and a 6px radius. The page's hover and
/// pressed fills; the border color doubles as the pressed fill.
pub fn button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(0x2f, 0x35, 0x49),
        button::Status::Pressed => Color::from_rgb8(0x41, 0x48, 0x68),
        _ => Color::from_rgb8(0x24, 0x28, 0x3b),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: palette().text,
        border: Border {
            color: Color::from_rgb8(0x41, 0x48, 0x68),
            width: 1.0,
            radius: border::radius(6.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The chrome is pinned to the web page's values (web/index.html,
    // `.waui-button`): the baseline the windowed host matches.
    #[test]
    fn button_style_matches_web_baseline() {
        let theme = theme();
        let base = button_style(&theme, button::Status::Active);
        assert_eq!(
            base.background,
            Some(Background::Color(Color::from_rgb8(0x24, 0x28, 0x3b)))
        );
        assert_eq!(base.border.width, 1.0);
        assert_eq!(base.border.color, Color::from_rgb8(0x41, 0x48, 0x68));
        assert_eq!(base.border.radius, border::radius(6.0),);

        let hovered = button_style(&theme, button::Status::Hovered);
        assert_eq!(
            hovered.background,
            Some(Background::Color(Color::from_rgb8(0x2f, 0x35, 0x49)))
        );
        let pressed = button_style(&theme, button::Status::Pressed);
        assert_eq!(
            pressed.background,
            Some(Background::Color(Color::from_rgb8(0x41, 0x48, 0x68)))
        );
    }
}
