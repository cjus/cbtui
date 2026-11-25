//! The help screen: its text, wrapped to fit the box, and which part of it a screen shows.
//!
//! ## How it works
//!
//! The help is the engine's name, version and licence, then [`HELP`]. [`HelpView`] works out
//! where the box goes on a screen of a given size, wraps the text to the box's width, and
//! says how many lines fit. Both the drawing ([`crate::ui`]) and the scroll keys
//! ([`App::help_scroll`]) use it, so a key never scrolls past what the screen shows.
//!
//! ## Pitfalls
//!
//! - The app has no terminal, so it scrolls against [`App::screen`], the size the event loop
//!   last drew. A resize changes how much fits; the offset is clamped again when drawn.
//! - Wrapped lines keep their indent, plus a little, so a wrapped description stays under its
//!   heading rather than under the keys column.

use ratatui::layout::Rect;

use crate::app::App;

/// The help screen's text, after the engine's section.
pub const HELP: &str = "\
Moves
  Click a piece, then its square. Or use the arrows and Enter.
  Or type a move: e4, Nf3, exd5, O-O, e8=N, or e2e4. Enter plays it.
  A pawn on the last rank offers Q R B N there: click one, or press
  q, r, b or n (Enter: queen, Esc: put it back).

Keys
  m        the engine moves (now, if it is thinking)
  u        take back your last move
  n        new game
  [ ]      look back through the game a move at a time (Home, End)
  t        turn the board around
  z        zoom: the board fills the screen (again for the panels)
  i        analyse: the engine studies the board until i again; you
           move for both sides, and in the PGN viewer it follows you
  v        the engine's evaluation of the board, term by term
  w        follow the search at its root: the move it's on, each
           root move's effort, what it goes back over (w again)
  :        type a command
  ?        this help
  q        quit

PGN viewer (cbtui --pgn <file>)
  ← →      a move back or on (also [ ])
  ↑ ↓      the start of the game, the end of the line (Home, End)
  1-9      into a variation that starts at this move
  :play white|black|both|none   play on from the position shown

Commands
  :play white|black|both|none   which sides the engine plays
  :level <n>|max                the engine's playing level
  :lines <n>                    how many lines an analysis shows
  :techniques                   switch its search techniques
  :tree [depth]                 walk a search tree of the board
  :time <ms>                    how long it thinks a move
  :clock <min>+<sec>|off        play to a clock (5+3)
  :ponder on|off                the engine thinks on your time too
  :resign  :draw                resign, or offer the engine a draw
  :fen <FEN>                    a new game from a position
  :pgn <file>                   save the game
  :theme [name]                 the board: brown, blue, green, grey,
                                dark (alone: the next one)
  :new  :undo  :flip  :zoom  :go  :stop  :quit

Settings are saved in ~/.cbtuirc as they change.

↑ ↓, PgUp, PgDn or the wheel scroll; any other key closes.";

/// How wide the help box is, at most, borders included.
pub const HELP_WIDTH: u16 = 70;

/// How many lines one notch of the mouse wheel scrolls the help.
pub const WHEEL_LINES: isize = 3;

/// How much further than its line a wrapped line's continuation is indented.
const WRAP_INDENT: usize = 2;

/// The help on a screen: where its box goes, its lines wrapped to fit, and how many show.
#[derive(Clone, Debug, PartialEq)]
pub struct HelpView {
    /// The box, borders included, centred on the screen.
    pub rect: Rect,
    /// Every line of the help, wrapped to the box's width.
    pub lines: Vec<String>,
    /// How many lines the box shows at once.
    pub rows: usize,
}

impl HelpView {
    /// The help for `app` on a screen of `area`.
    pub fn new(area: Rect, app: &App) -> HelpView {
        let width = HELP_WIDTH.min(area.width);
        let lines = wrap(&text(app), width.saturating_sub(2) as usize);
        let height = (lines.len() as u16).saturating_add(2).min(area.height);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        HelpView {
            rect,
            lines,
            rows: height.saturating_sub(2) as usize,
        }
    }

    /// The furthest the help scrolls: its last line at the bottom of the box.
    pub fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.rows)
    }
}

/// The whole help: which engine it is (its whole version and licence), then [`HELP`].
pub fn text(app: &App) -> String {
    let engine = if app.has_engine {
        format!(
            "Engine\n  {} {}, licence {}\n\n",
            app.node.name, app.node.version, app.node.license
        )
    } else {
        "Engine\n  None: start cbtui with --engine <path> to play one.\n\n".to_string()
    };
    engine + HELP
}

/// `text`'s lines, each broken at spaces to at most `width` characters (at any character if a
/// word is longer). A continuation keeps its line's indent, plus two spaces.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let mut rest: Vec<char> = line.chars().collect();
        let mut first = true;
        loop {
            let lead = if first { 0 } else { indent + WRAP_INDENT };
            // A continuation too indented for the width loses its indent.
            let lead = if lead >= width { 0 } else { lead };
            let room = width - lead;
            if rest.len() <= room {
                out.push(" ".repeat(lead) + &rest.iter().collect::<String>());
                break;
            }
            // The last space that fits, past the indent; else a hard break.
            let cut = rest[..=room]
                .iter()
                .rposition(|c| *c == ' ')
                .filter(|&at| at > indent.min(room - 1))
                .unwrap_or(room);
            let piece: String = rest[..cut].iter().collect();
            out.push(" ".repeat(lead) + piece.trim_end());
            rest = rest[cut..]
                .iter()
                .copied()
                .skip_while(|c| *c == ' ')
                .collect();
            first = false;
            if rest.is_empty() {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_at_spaces_keeping_their_indent() {
        let lines = wrap(
            "  i        analyse: the engine studies the board until stopped",
            30,
        );
        assert_eq!(
            lines,
            [
                "  i        analyse: the engine",
                "    studies the board until",
                "    stopped"
            ]
        );
        for line in &lines {
            assert!(line.chars().count() <= 30, "{line}");
        }
        // Short lines and blank lines are left alone; a long word is broken.
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "  ef", "  gh", "  ij"]);
    }

    #[test]
    fn every_help_line_fits_a_full_width_box() {
        // Wrapping is for narrow terminals; at full width nothing should need it.
        let inner = HELP_WIDTH as usize - 2;
        for line in HELP.lines() {
            assert!(
                line.chars().count() <= inner,
                "too wide for the help box: {line}"
            );
        }
    }

    #[test]
    fn the_view_fits_the_screen_and_scrolls_the_rest() {
        let app = crate::app::App::new(
            crate::app::NodeSummary::absent(),
            crate::app::Settings::default(),
        )
        .unwrap();
        let tall = HelpView::new(Rect::new(0, 0, 120, 200), &app);
        assert_eq!(tall.max_scroll(), 0, "everything fits");
        assert_eq!(tall.rows, tall.lines.len());
        let short = HelpView::new(Rect::new(0, 0, 61, 20), &app);
        assert_eq!(short.rect.height, 20);
        assert_eq!(short.rows, 18);
        assert_eq!(short.max_scroll(), short.lines.len() - 18);
        // Narrower than the help: lines wrap, none is cut off.
        assert!(short.lines.len() > tall.lines.len());
        assert!(short.lines.iter().all(|l| l.chars().count() <= 59));
    }
}
