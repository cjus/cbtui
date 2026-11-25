//! How pieces are drawn: as pictures on terminals that can show images, as two letters
//! (`WK`, `BP`) everywhere else.
//!
//! ## Why pictures
//!
//! A character in a terminal is one cell, however big the square around it, so a chess
//! symbol on a large board is a speck. Terminals that speak an image protocol (kitty's, the
//! one Ghostty, kitty and others use; iTerm2's, WezTerm's; or Sixel) can show a picture that
//! fills the square instead. [ratatui-image](https://github.com/ratatui/ratatui-image) finds
//! out which protocol the terminal has, and its font's size in pixels.
//!
//! ## How it works
//!
//! - **The pieces** are Colin M. L. Burnett's drawings, as SVG, built into the binary
//!   (`assets/pieces/`, BSD 3-clause). [resvg](https://github.com/linebender/resvg) draws each
//!   one at the size of a square in pixels, so it is crisp at any size.
//! - **A picture is a whole square.** It is drawn over the square's own colour (light, dark,
//!   last move, selected...), filling every pixel of the square's cells, so no cell around it
//!   shows a slightly different shade. Each (piece, colour, size) is drawn once and kept.
//! - **Squares are square.** With pictures, a square is as many cells wide as makes it as wide
//!   as it is tall in pixels ([`PieceArt::cell_aspect`]), and the board grows to fill its
//!   panel.
//!
//! ## Pitfalls
//!
//! - The font size is asked once, at start. Zooming the terminal's font afterwards (⌘+)
//!   leaves the pictures at the old size until cbtui is restarted.
//! - Inside tmux, image protocols need passthrough set up, and asking tmux about them timed
//!   out often enough to lose a key press (see `query_terminal` in `main.rs`), so cbtui uses
//!   letters there without asking.
//! - With no image protocol, ratatui-image would fall back to drawing pictures out of
//!   half-block characters; at a square's size those are blurry, so cbtui uses letters.

use std::collections::HashMap;

use image::{DynamicImage, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FontSize, Resize};
use resvg::{tiny_skia, usvg};

use crate::rules::{Color, Piece, PieceType};

/// How much of a square a piece's picture fills, across its smaller side.
pub const PIECE_SHARE: f32 = 0.92;

/// The piece drawings, in [`index`]'s order: White's king to pawn, then Black's.
const SVGS: [&[u8]; 12] = [
    include_bytes!("../assets/pieces/wK.svg"),
    include_bytes!("../assets/pieces/wQ.svg"),
    include_bytes!("../assets/pieces/wR.svg"),
    include_bytes!("../assets/pieces/wB.svg"),
    include_bytes!("../assets/pieces/wN.svg"),
    include_bytes!("../assets/pieces/wP.svg"),
    include_bytes!("../assets/pieces/bK.svg"),
    include_bytes!("../assets/pieces/bQ.svg"),
    include_bytes!("../assets/pieces/bR.svg"),
    include_bytes!("../assets/pieces/bB.svg"),
    include_bytes!("../assets/pieces/bN.svg"),
    include_bytes!("../assets/pieces/bP.svg"),
];

/// Where `piece` is in [`SVGS`].
fn index(piece: Piece) -> usize {
    let kind = match piece.piece_type {
        PieceType::King => 0,
        PieceType::Queen => 1,
        PieceType::Rook => 2,
        PieceType::Bishop => 3,
        PieceType::Knight => 4,
        PieceType::Pawn => 5,
    };
    match piece.color {
        Color::White => kind,
        Color::Black => 6 + kind,
    }
}

/// A piece as two letters, side then piece: `WK` is White's king, `BP` a black pawn.
pub fn letters(piece: Piece) -> String {
    let side = match piece.color {
        Color::White => 'W',
        Color::Black => 'B',
    };
    format!("{side}{}", piece.piece_type.to_char().to_ascii_uppercase())
}

/// A piece as a chess symbol: the same solid shape for both sides, told apart by colour.
pub fn symbol(piece: Piece) -> char {
    match piece.piece_type {
        PieceType::King => '♚',
        PieceType::Queen => '♛',
        PieceType::Rook => '♜',
        PieceType::Bishop => '♝',
        PieceType::Knight => '♞',
        PieceType::Pawn => '♟',
    }
}

/// What the person asked for on the command line (`--pieces`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PieceChoice {
    /// Pictures if the terminal can show them (and isn't tmux), else letters.
    #[default]
    Auto,
    /// Two letters, `WK`.
    Letters,
    /// Chess symbols, `♚`.
    Symbols,
}

/// How this session draws pieces.
#[derive(Debug)]
pub enum Pieces {
    /// Pictures.
    Images(PieceArt),
    /// Two letters.
    Letters,
    /// Chess symbols.
    Symbols,
}

impl Pieces {
    /// The way to draw pieces given the person's `choice` and, when it was asked, what the
    /// terminal said (`picker`). Must be called before anything else reads the terminal.
    pub fn choose(choice: PieceChoice, picker: Option<Picker>) -> Pieces {
        match (choice, picker) {
            (PieceChoice::Letters, _) => Pieces::Letters,
            (PieceChoice::Symbols, _) => Pieces::Symbols,
            (PieceChoice::Auto, Some(picker)) if picker.tmux_detected() => Pieces::Letters,
            (PieceChoice::Auto, Some(picker)) => match PieceArt::new(picker) {
                Some(art) => Pieces::Images(art),
                None => Pieces::Letters,
            },
            (_, None) => Pieces::Letters,
        }
    }

    /// A few words for the status line, like `pictures (kitty protocol)`.
    pub fn describe(&self) -> String {
        match self {
            Pieces::Images(art) => format!("pictures ({:?} protocol)", art.protocol_type()),
            Pieces::Letters => "letters".into(),
            Pieces::Symbols => "symbols".into(),
        }
    }

    /// How tall a cell is for its width, when squares should be square; `None` for text
    /// pieces, whose squares come in a few fixed sizes.
    pub fn cell_aspect(&self) -> Option<f32> {
        match self {
            Pieces::Images(art) => Some(art.cell_aspect()),
            Pieces::Letters | Pieces::Symbols => None,
        }
    }
}

/// The piece drawings, the terminal's image protocol, and the pictures made so far.
pub struct PieceArt {
    picker: Picker,
    trees: Vec<usvg::Tree>,
    cache: HashMap<(usize, [u8; 3], u16, u16), Protocol>,
}

impl std::fmt::Debug for PieceArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PieceArt")
            .field("protocol", &self.picker.protocol_type())
            .field("font", &self.picker.font_size())
            .field("pictures", &self.cache.len())
            .finish()
    }
}

impl PieceArt {
    /// Art for a terminal that `picker` describes, or `None` if it has no real image
    /// protocol or didn't say its font size.
    pub fn new(picker: Picker) -> Option<PieceArt> {
        let FontSize { width, height } = picker.font_size();
        if picker.protocol_type() == ProtocolType::Halfblocks || width == 0 || height == 0 {
            return None;
        }
        Some(PieceArt {
            picker,
            trees: parse_trees(),
            cache: HashMap::new(),
        })
    }

    /// The terminal's image protocol.
    pub fn protocol_type(&self) -> ProtocolType {
        self.picker.protocol_type()
    }

    /// A cell's height divided by its width, in pixels (about 2).
    pub fn cell_aspect(&self) -> f32 {
        let FontSize { width, height } = self.picker.font_size();
        f32::from(height) / f32::from(width)
    }

    /// The picture of `piece` on a square of `background`, `cells` wide and tall, made the
    /// first time it is asked for.
    pub fn picture(
        &mut self,
        piece: Piece,
        background: [u8; 3],
        cells: (u16, u16),
    ) -> Option<&Protocol> {
        let key = (index(piece), background, cells.0, cells.1);
        if !self.cache.contains_key(&key) {
            let FontSize { width, height } = self.picker.font_size();
            let pixels = (
                u32::from(cells.0) * u32::from(width),
                u32::from(cells.1) * u32::from(height),
            );
            let image = draw_piece(&self.trees[key.0], background, pixels)?;
            let size = Size::new(cells.0, cells.1);
            let protocol = self
                .picker
                .new_protocol(DynamicImage::ImageRgba8(image), size, Resize::Fit(None))
                .ok()?;
            self.cache.insert(key, protocol);
        }
        self.cache.get(&key)
    }
}

/// The twelve drawings, parsed.
fn parse_trees() -> Vec<usvg::Tree> {
    SVGS.iter()
        .map(|svg| {
            usvg::Tree::from_data(svg, &usvg::Options::default())
                .expect("the piece SVGs built into cbtui are valid")
        })
        .collect()
}

/// `tree` drawn over `background`, centred in a picture of `pixels` (width, height), filling
/// [`PIECE_SHARE`] of its smaller side. Every pixel is opaque.
pub fn draw_piece(
    tree: &usvg::Tree,
    background: [u8; 3],
    (width, height): (u32, u32),
) -> Option<RgbaImage> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let [r, g, b] = background;
    pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));
    let size = tree.size();
    let side = width.min(height) as f32 * PIECE_SHARE;
    let scale = side / size.width().max(size.height());
    let dx = (width as f32 - size.width() * scale) / 2.0;
    let dy = (height as f32 - size.height() * scale) / 2.0;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    // The background is opaque, so tiny-skia's premultiplied pixels are plain RGBA.
    RgbaImage::from_raw(width, height, pixmap.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_letters_name_side_and_piece() {
        assert_eq!(letters(Piece::new(Color::White, PieceType::King)), "WK");
        assert_eq!(letters(Piece::new(Color::Black, PieceType::Pawn)), "BP");
        assert_eq!(letters(Piece::new(Color::Black, PieceType::Knight)), "BN");
    }

    #[test]
    fn every_drawing_parses_and_draws_to_size() {
        let trees = parse_trees();
        assert_eq!(trees.len(), 12);
        for tree in &trees {
            let image = draw_piece(tree, [240, 217, 181], (70, 64)).unwrap();
            assert_eq!(image.dimensions(), (70, 64));
            // The corner is the square's colour; the centre is the piece.
            assert_eq!(image.get_pixel(0, 0).0, [240, 217, 181, 255]);
            assert!(image.pixels().all(|p| p.0[3] == 255), "opaque");
        }
    }

    #[test]
    fn white_and_black_pieces_differ() {
        let trees = parse_trees();
        let white = draw_piece(&trees[0], [0, 0, 255], (48, 48)).unwrap();
        let black = draw_piece(&trees[6], [0, 0, 255], (48, 48)).unwrap();
        assert_ne!(white, black);
        // A white king has white in it; a black king's body is black.
        assert!(white.pixels().any(|p| p.0 == [255, 255, 255, 255]));
        assert!(black.pixels().any(|p| p.0 == [0, 0, 0, 255]));
    }

    #[test]
    fn a_picture_is_made_once_per_piece_colour_and_size() {
        // A picker that claims kitty's protocol with a 10x20 font, as a real terminal would.
        // A test has no terminal to ask, so it says the font size itself.
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(FontSize::new(10, 20));
        picker.set_protocol_type(ProtocolType::Kitty);
        let mut art = PieceArt::new(picker).unwrap();
        assert_eq!(art.cell_aspect(), 2.0);
        let king = Piece::new(Color::White, PieceType::King);
        assert!(art.picture(king, [1, 2, 3], (6, 3)).is_some());
        assert!(art.picture(king, [1, 2, 3], (6, 3)).is_some());
        assert!(art.picture(king, [9, 9, 9], (6, 3)).is_some());
        assert_eq!(art.cache.len(), 2);
    }

    #[test]
    fn no_image_protocol_means_letters() {
        assert!(PieceArt::new(Picker::halfblocks()).is_none());
        assert!(matches!(
            Pieces::choose(PieceChoice::Auto, Some(Picker::halfblocks())),
            Pieces::Letters
        ));
        assert!(matches!(
            Pieces::choose(PieceChoice::Auto, None),
            Pieces::Letters
        ));
        assert!(matches!(
            Pieces::choose(PieceChoice::Symbols, None),
            Pieces::Symbols
        ));
    }
}
