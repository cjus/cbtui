//! Board themes: the colours of the squares, the pieces and the last move.
//!
//! ## How it works
//!
//! A [`Theme`] gives its [`Colours`] twice: in 24-bit colour, and as the nearest of the 256
//! standard colours for terminals without it. The other highlights (the selection, check, the
//! cursor, captures) are the same in every theme, so they read the same whichever board is on
//! screen. The palette is built from the theme on every draw ([`crate::ui::Palette::new`]), so
//! `:theme` changes the board at once.
//!
//! The dark theme is for dark terminals: dim squares, white pieces muted to a soft grey, and a
//! dull last-move tint, so nothing on the board glares. Picture pieces are muted with it: their
//! colours are multiplied by the theme's white ([`crate::pieces::draw_piece`]).
//!
//! ## Pitfalls
//!
//! - Pieces drawn as letters or symbols are text in a colour: white text on a near-white square
//!   would vanish. White pieces on light squares are drawn in the theme's ink instead; the
//!   letter (`WK`) or the hollow symbol still says whose piece it is.
//! - Black text has to read on the dark squares, and white text too, so a dark square is a mid
//!   tone, not as deep as a wooden or vinyl board's. A test checks every piece's text against
//!   every square it can stand on.
//! - Picture pieces are drawn over their square's colour, so a theme change makes new pictures
//!   (each is made once per piece, colour, tint and size).

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
    /// Dim slate squares and muted white pieces, for dark terminals.
    Dark,
}

/// The colours a theme sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colours {
    /// The light squares.
    pub light: Tint,
    /// The dark squares.
    pub dark: Tint,
    /// A light square the last move left or reached.
    pub last_light: Tint,
    /// A dark square the last move left or reached.
    pub last_dark: Tint,
    /// White's pieces, as text, and the tint picture pieces are multiplied by.
    pub white: Tint,
    /// Black's pieces, as text.
    pub black: Tint,
    /// White's pieces as text on a light square, where white might not show.
    pub ink: Tint,
}

impl Theme {
    /// Every theme, in the order `:theme` goes through them.
    pub const ALL: [Theme; 5] = [
        Theme::Brown,
        Theme::Blue,
        Theme::Green,
        Theme::Grey,
        Theme::Dark,
    ];

    /// Its name, as `--theme` and `:theme` take it.
    pub fn name(self) -> &'static str {
        match self {
            Theme::Brown => "brown",
            Theme::Blue => "blue",
            Theme::Green => "green",
            Theme::Grey => "grey",
            Theme::Dark => "dark",
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

    /// The theme's colours: 24-bit when `true_color`, else the nearest of the 256 standard
    /// colours.
    pub fn colours(self, true_color: bool) -> Colours {
        if true_color {
            self.rgb()
        } else {
            self.indexed()
        }
    }

    fn rgb(self) -> Colours {
        let rgb = Tint::Rgb;
        // The light themes share their pieces and last-move tints.
        let light_theme = |light, dark, ink| Colours {
            light,
            dark,
            last_light: rgb(206, 210, 107),
            last_dark: rgb(170, 162, 58),
            white: rgb(255, 255, 255),
            black: rgb(20, 20, 20),
            ink,
        };
        match self {
            Theme::Brown => light_theme(rgb(240, 217, 181), rgb(181, 136, 99), rgb(110, 72, 40)),
            Theme::Blue => light_theme(rgb(228, 232, 238), rgb(56, 96, 196), rgb(30, 54, 132)),
            Theme::Green => light_theme(rgb(236, 228, 200), rgb(52, 120, 72), rgb(26, 84, 44)),
            Theme::Grey => light_theme(rgb(232, 232, 230), rgb(96, 104, 118), rgb(64, 70, 82)),
            Theme::Dark => Colours {
                light: rgb(110, 118, 132),
                dark: rgb(78, 86, 100),
                last_light: rgb(116, 120, 72),
                last_dark: rgb(100, 104, 60),
                white: rgb(200, 200, 192),
                black: rgb(14, 14, 14),
                // Muted white reads on these squares, so it stays.
                ink: rgb(200, 200, 192),
            },
        }
    }

    fn indexed(self) -> Colours {
        let ix = Tint::Indexed;
        let light_theme = |light, dark, ink| Colours {
            light,
            dark,
            last_light: ix(186),
            last_dark: ix(142),
            white: ix(231),
            black: ix(16),
            ink,
        };
        match self {
            Theme::Brown => light_theme(ix(223), ix(137), ix(94)),
            Theme::Blue => light_theme(ix(254), ix(26), ix(19)),
            Theme::Green => light_theme(ix(230), ix(28), ix(22)),
            Theme::Grey => light_theme(ix(255), ix(243), ix(238)),
            Theme::Dark => Colours {
                light: ix(243),
                dark: ix(240),
                last_light: ix(101),
                last_dark: ix(58),
                white: ix(251),
                black: ix(16),
                ink: ix(251),
            },
        }
    }

    /// The names of every theme, for messages: `brown, blue, green, grey or dark`.
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
        assert_eq!(Theme::Grey.next(), Theme::Dark);
        assert_eq!(Theme::Dark.next(), Theme::Brown);
        assert_eq!(Theme::names(), "brown, blue, green, grey or dark");
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
        for theme in Theme::ALL {
            let c = theme.colours(true);
            // Each piece's text on each square it can stand on, as the board draws it: white
            // takes the ink on light squares.
            let pairs = [
                ("white on dark", c.white, c.dark),
                ("white on the last move's dark", c.white, c.last_dark),
                ("white on light", c.ink, c.light),
                ("white on the last move's light", c.ink, c.last_light),
                ("black on dark", c.black, c.dark),
                ("black on light", c.black, c.light),
                ("black on the last move's dark", c.black, c.last_dark),
            ];
            for (what, text, square) in pairs {
                let ratio = contrast(text, square);
                assert!(ratio >= 2.5, "{theme}: {what} is {ratio:.2}");
            }
        }
    }

    #[test]
    fn the_light_squares_are_lighter_and_the_dark_theme_is_dim() {
        for theme in Theme::ALL {
            let c = theme.colours(true);
            assert!(luminance(c.light) > luminance(c.dark), "{theme}");
            assert_ne!(theme.colours(false).light, theme.colours(false).dark);
        }
        let dark = Theme::Dark.colours(true);
        assert!(luminance(dark.light) < 0.25, "no bright squares");
        assert!(luminance(dark.white) < 0.65, "white pieces muted");
    }
}
