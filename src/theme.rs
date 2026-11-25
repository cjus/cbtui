//! Board themes: the colours of the light and dark squares.
//!
//! ## How it works
//!
//! A [`Theme`] is a pair of square colours, each given twice: in 24-bit colour, and as the
//! nearest of the 256 standard colours for terminals without it. Everything else on the board
//! (the last move, the selection, check, the cursor) keeps the same colours in every theme, so
//! it reads the same whichever board is on screen. The palette is built from the theme on
//! every draw ([`crate::ui::Palette::new`]), so `:theme` changes the board at once.
//!
//! ## Pitfalls
//!
//! - Pieces drawn as letters or symbols are text in a colour: white text on a near-white square
//!   would vanish. White pieces on light squares are drawn in the theme's ink instead;
//!   the letter (`WK`) or the hollow symbol still says whose piece it is.
//! - Black text has to read on the dark squares, and white text too, so a dark square is a
//!   mid tone (contrast about 3:1 or more with black, 5:1 with white), not as deep as a
//!   wooden or vinyl board's.
//! - Picture pieces are drawn over their square's colour, so a theme change makes new pictures
//!   (each is made once per piece, colour and size).

use std::fmt;

use ratatui::style::Color as Tint;

/// A board's colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// Cream and brown, the wooden board most chess sites start with.
    #[default]
    Brown,
    /// White and royal blue.
    Blue,
    /// Cream and dark green, the tournament board.
    Green,
    /// Off-white and slate grey.
    Grey,
}

impl Theme {
    /// Every theme, in the order `:theme` goes through them.
    pub const ALL: [Theme; 4] = [Theme::Brown, Theme::Blue, Theme::Green, Theme::Grey];

    /// Its name, as `--theme` and `:theme` take it.
    pub fn name(self) -> &'static str {
        match self {
            Theme::Brown => "brown",
            Theme::Blue => "blue",
            Theme::Green => "green",
            Theme::Grey => "grey",
        }
    }

    /// The theme called `name` (`gray` is `grey`), if there is one.
    pub fn parse(name: &str) -> Option<Theme> {
        match name.to_ascii_lowercase().as_str() {
            "gray" => Some(Theme::Grey),
            name => Theme::ALL.into_iter().find(|t| t.name() == name),
        }
    }

    /// The theme after this one, round to the first.
    pub fn next(self) -> Theme {
        let at = Theme::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Theme::ALL[(at + 1) % Theme::ALL.len()]
    }

    /// The light and dark squares' colours: 24-bit when `true_color`, else the nearest of the
    /// 256 standard colours.
    pub fn squares(self, true_color: bool) -> (Tint, Tint) {
        match (self, true_color) {
            (Theme::Brown, true) => (Tint::Rgb(240, 217, 181), Tint::Rgb(181, 136, 99)),
            (Theme::Brown, false) => (Tint::Indexed(223), Tint::Indexed(137)),
            (Theme::Blue, true) => (Tint::Rgb(228, 232, 238), Tint::Rgb(56, 96, 196)),
            (Theme::Blue, false) => (Tint::Indexed(254), Tint::Indexed(26)),
            (Theme::Green, true) => (Tint::Rgb(236, 228, 200), Tint::Rgb(52, 120, 72)),
            (Theme::Green, false) => (Tint::Indexed(230), Tint::Indexed(28)),
            (Theme::Grey, true) => (Tint::Rgb(232, 232, 230), Tint::Rgb(96, 104, 118)),
            (Theme::Grey, false) => (Tint::Indexed(255), Tint::Indexed(243)),
        }
    }

    /// What white pieces drawn as text are written in on a light square, where white wouldn't
    /// show: a deep shade of the dark squares' colour.
    pub fn ink(self, true_color: bool) -> Tint {
        match (self, true_color) {
            (Theme::Brown, true) => Tint::Rgb(110, 72, 40),
            (Theme::Brown, false) => Tint::Indexed(94),
            (Theme::Blue, true) => Tint::Rgb(30, 54, 132),
            (Theme::Blue, false) => Tint::Indexed(19),
            (Theme::Green, true) => Tint::Rgb(26, 84, 44),
            (Theme::Green, false) => Tint::Indexed(22),
            (Theme::Grey, true) => Tint::Rgb(64, 70, 82),
            (Theme::Grey, false) => Tint::Indexed(238),
        }
    }

    /// The names of every theme, for messages: `brown, blue, green or grey`.
    pub fn names() -> String {
        let names: Vec<&str> = Theme::ALL.iter().map(|t| t.name()).collect();
        let (last, rest) = names.split_last().expect("there are themes");
        format!("{} or {last}", rest.join(", "))
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_have_names_and_go_round() {
        assert_eq!(Theme::parse("blue"), Some(Theme::Blue));
        assert_eq!(Theme::parse("Gray"), Some(Theme::Grey));
        assert_eq!(Theme::parse("pink"), None);
        assert_eq!(Theme::Grey.next(), Theme::Brown);
        assert_eq!(Theme::names(), "brown, blue, green or grey");
        for theme in Theme::ALL {
            assert_eq!(Theme::parse(theme.name()), Some(theme));
        }
    }

    /// WCAG's relative luminance of a 24-bit colour.
    fn luminance(tint: Tint) -> f64 {
        let Tint::Rgb(r, g, b) = tint else {
            unreachable!("24-bit colours here")
        };
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn contrast(a: Tint, b: Tint) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn letter_pieces_read_on_every_square() {
        let white = Tint::Rgb(255, 255, 255);
        let black = Tint::Rgb(20, 20, 20);
        for theme in Theme::ALL {
            let (light, dark) = theme.squares(true);
            // On a dark square: black and white text. On a light one: black, and the ink
            // that white pieces take there.
            assert!(contrast(dark, black) >= 3.0, "{theme}: black on dark");
            assert!(contrast(dark, white) >= 3.0, "{theme}: white on dark");
            assert!(contrast(light, black) >= 7.0, "{theme}: black on light");
            let ink = theme.ink(true);
            assert!(
                contrast(light, ink) >= 4.5,
                "{theme}: white pieces on light"
            );
        }
    }

    #[test]
    fn every_theme_has_a_light_square_lighter_than_its_dark_one() {
        let brightness = |tint: Tint| match tint {
            Tint::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
            _ => unreachable!("24-bit colours here"),
        };
        for theme in Theme::ALL {
            let (light, dark) = theme.squares(true);
            assert!(brightness(light) > brightness(dark) + 200, "{theme}");
            let (light, dark) = theme.squares(false);
            assert_ne!(light, dark, "{theme}");
        }
    }
}
