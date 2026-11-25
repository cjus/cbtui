//! Drawing the screen.
//!
//! ```text
//! ┌ Board ─────────────────┐┌ cb2026 1.0.0 ────────────────────┐
//! │ 8 ♜ ♞ ♝ ♛ ♚ ♝ ♞ ♜      ││ White: you   Black: cb2026        │
//! │ …                      │├ Moves ────────────────────────────┤
//! │                        ││ 1. e4      e5                     │
//! │                        │├ Engine ───────────────────────────┤
//! │   a b c d e f g h      ││ depth 13/22  +0.18  375k nodes    │
//! └────────────────────────┘└───────────────────────────────────┘
//!  status, or the line being typed
//!  keys
//! ```
//!
//! [`layout`] decides where everything goes and [`board_geometry`] where each square is. The
//! same two functions place the mouse ([`square_at`]), so a click always lands on the square
//! drawn under it.
//!
//! Squares grow with the terminal. With pictures for pieces ([`crate::pieces`]) a square is as
//! wide as it is tall in pixels, as big as the panel allows. With letters or symbols, which
//! are a cell each whatever the square, squares are 7×3 cells, 5×2 or 3×1, whichever fits.
//!
//! Colour: 24-bit when the terminal says it has it (`COLORTERM=truecolor`), or when it shows
//! pictures, else the nearest of the 256 standard colours ([`Palette`]).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as Tint, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui_image::Image;

use crate::app::{App, score_text};
use crate::pieces::{self, Pieces};
use crate::rules::{self, Color, Square};

/// The width the panels on the right need, at least.
const SIDE_MIN_WIDTH: u16 = 32;

/// The smallest terminal cbtui draws in.
pub const MIN_WIDTH: u16 = 8 * 3 + 5 + SIDE_MIN_WIDTH;
/// The smallest terminal cbtui draws in.
pub const MIN_HEIGHT: u16 = 8 + 3 + 2;

/// Square sizes, largest first: (width, height) in cells. Cells are about twice as tall as
/// they are wide, so a square is about twice as wide as it is tall.
const SQUARE_SIZES: [(u16, u16); 3] = [(7, 3), (5, 2), (3, 1)];

/// The tallest square, in cells, when pieces are pictures and squares grow to fill the panel.
const MAX_SQUARE_ROWS: u16 = 16;

/// Where the parts of the screen go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panels {
    /// The board's frame.
    pub board: Rect,
    /// The engine and players.
    pub info: Rect,
    /// The move list.
    pub moves: Rect,
    /// The engine's thinking.
    pub engine: Rect,
    /// The status or input line.
    pub status: Rect,
    /// The keys line.
    pub keys: Rect,
}

/// Where the board's squares are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardGeometry {
    /// The top-left cell of the top-left square.
    pub x: u16,
    /// The top-left cell of the top-left square.
    pub y: u16,
    /// A square's width in cells.
    pub width: u16,
    /// A square's height in cells.
    pub height: u16,
}

/// The size of square that fits in `width` × `height` cells with its labels. `aspect` is a
/// cell's height over its width when squares should be square in pixels (pictures), or
/// `None` for the fixed sizes text pieces use.
fn square_size(width: u16, height: u16, aspect: Option<f32>) -> (u16, u16) {
    let fits = |&(w, h): &(u16, u16)| 8 * w + 3 <= width && 8 * h < height;
    let smallest = SQUARE_SIZES[2];
    match aspect {
        None => SQUARE_SIZES.into_iter().find(fits).unwrap_or(smallest),
        Some(aspect) => (1..=MAX_SQUARE_ROWS)
            .rev()
            .map(|h| ((f32::from(h) * aspect).round().max(2.0) as u16, h))
            .find(fits)
            .unwrap_or(smallest),
    }
}

/// Where the parts of the screen go in `area`. `aspect` is a cell's height over its width
/// when pieces are pictures, else `None`.
pub fn layout(area: Rect, aspect: Option<f32>) -> Panels {
    let [main, status, keys] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    // Borders take 2 cells each way; the side panels need their own width.
    let (w, _) = square_size(
        main.width.saturating_sub(2 + SIDE_MIN_WIDTH),
        main.height.saturating_sub(2),
        aspect,
    );
    let board_width = 8 * w + 3 + 2;
    let [board, side] =
        Layout::horizontal([Constraint::Length(board_width), Constraint::Min(0)]).areas(main);
    let [info, moves, engine] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(3),
        Constraint::Length(5),
    ])
    .areas(side);
    Panels {
        board,
        info,
        moves,
        engine,
        status,
        keys,
    }
}

/// Where the squares are in the board's frame `board`. `aspect` is a cell's height over its
/// width when pieces are pictures, else `None`.
pub fn board_geometry(board: Rect, aspect: Option<f32>) -> BoardGeometry {
    let inner = Block::default().borders(Borders::ALL).inner(board);
    let (width, height) = square_size(inner.width, inner.height, aspect);
    // Rank labels take the first two columns; centre the rest vertically.
    let used = 8 * height + 1;
    BoardGeometry {
        x: inner.x + 2,
        y: inner.y + inner.height.saturating_sub(used) / 2,
        width,
        height,
    }
}

/// The square drawn at screen column `col` and row `row`, if any. `aspect` is a cell's
/// height over its width when pieces are pictures, else `None`.
pub fn square_at(
    area: Rect,
    aspect: Option<f32>,
    flipped: bool,
    col: u16,
    row: u16,
) -> Option<Square> {
    let g = board_geometry(layout(area, aspect).board, aspect);
    if col < g.x || row < g.y {
        return None;
    }
    let (column, line) = ((col - g.x) / g.width, (row - g.y) / g.height);
    if column > 7 || line > 7 {
        return None;
    }
    let (file, rank) = screen_to_board(column as usize, line as usize, flipped);
    rules::square(file, rank)
}

/// The board file and rank shown at screen column `column` and line `line` (0 at top-left).
fn screen_to_board(column: usize, line: usize, flipped: bool) -> (usize, usize) {
    if flipped {
        (7 - column, line)
    } else {
        (column, 7 - line)
    }
}

/// The colours the board uses.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    light: Tint,
    dark: Tint,
    last_light: Tint,
    last_dark: Tint,
    selected: Tint,
    check: Tint,
    cursor: Tint,
    capture: Tint,
    white_piece: Tint,
    black_piece: Tint,
    target: Tint,
}

impl Palette {
    /// 24-bit colours.
    pub const TRUE_COLOR: Palette = Palette {
        light: Tint::Rgb(240, 217, 181),
        dark: Tint::Rgb(181, 136, 99),
        last_light: Tint::Rgb(206, 210, 107),
        last_dark: Tint::Rgb(170, 162, 58),
        selected: Tint::Rgb(120, 160, 110),
        check: Tint::Rgb(220, 70, 60),
        cursor: Tint::Rgb(110, 160, 210),
        capture: Tint::Rgb(214, 120, 96),
        white_piece: Tint::Rgb(255, 255, 255),
        black_piece: Tint::Rgb(20, 20, 20),
        target: Tint::Rgb(60, 90, 50),
    };

    /// The nearest of the 256 standard colours.
    pub const INDEXED: Palette = Palette {
        light: Tint::Indexed(223),
        dark: Tint::Indexed(137),
        last_light: Tint::Indexed(186),
        last_dark: Tint::Indexed(142),
        selected: Tint::Indexed(108),
        check: Tint::Indexed(167),
        cursor: Tint::Indexed(74),
        capture: Tint::Indexed(173),
        white_piece: Tint::Indexed(231),
        black_piece: Tint::Indexed(16),
        target: Tint::Indexed(22),
    };

    /// The palette for this terminal, from `COLORTERM`.
    pub fn detect() -> Palette {
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor" | "24bit") => Palette::TRUE_COLOR,
            _ => Palette::INDEXED,
        }
    }
}

/// Draws the whole screen, with pieces drawn as `pieces` says.
pub fn draw(frame: &mut Frame, app: &App, palette: &Palette, pieces: &mut Pieces) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let text = format!(
            "The terminal is {}×{}; cbtui needs at least {MIN_WIDTH}×{MIN_HEIGHT}.",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), area);
        return;
    }
    let panels = layout(area, pieces.cell_aspect());
    let block = Block::default().borders(Borders::ALL).title(" Board ");
    frame.render_widget(block, panels.board);
    draw_board(frame.buffer_mut(), panels.board, app, palette, pieces);
    draw_info(frame, panels.info, app);
    draw_moves(frame, panels.moves, app);
    draw_engine(frame, panels.engine, app);
    draw_status(frame, panels.status, app);
    let keys = "click or arrows: move · type a move · m engine moves · u undo · n new · t flip · : command · ? help · q quit";
    frame.render_widget(
        Paragraph::new(keys).style(Style::default().add_modifier(Modifier::DIM)),
        panels.keys,
    );
    if app.show_help {
        draw_help(frame, area);
    }
}

fn draw_board(buf: &mut Buffer, board: Rect, app: &App, palette: &Palette, pieces: &mut Pieces) {
    let g = board_geometry(board, pieces.cell_aspect());
    let position = app.game.position();
    let last = app.game.last_move().map(|played| played.mv);
    let targets = app.targets();
    let check_square = position
        .in_check()
        .then(|| position.king_square(position.side_to_move()));
    for line in 0..8 {
        for column in 0..8 {
            let (file, rank) = screen_to_board(column, line, app.flipped);
            let square = rules::square(file, rank).expect("on the board");
            let light = (file + rank) % 2 == 1;
            let is_last = last.is_some_and(|mv| mv.from() == square || mv.to() == square);
            let piece = position.piece_at(square);
            let mut bg = match (light, is_last) {
                (true, false) => palette.light,
                (false, false) => palette.dark,
                (true, true) => palette.last_light,
                (false, true) => palette.last_dark,
            };
            if targets.contains(&square) && piece.is_some() {
                bg = palette.capture;
            }
            if check_square == Some(square) {
                bg = palette.check;
            }
            if app.selected == Some(square) {
                bg = palette.selected;
            }
            if app.cursor_visible && app.cursor == square {
                bg = palette.cursor;
            }
            let x = g.x + column as u16 * g.width;
            let y = g.y + line as u16 * g.height;
            let rect = Rect::new(x, y, g.width, g.height);
            buf.set_style(rect, Style::default().bg(bg));
            let (cx, cy) = (x + g.width / 2, y + g.height / 2);
            if let Some(piece) = piece {
                // A picture fills the square, over its colour (which a picture needs as RGB:
                // a terminal that shows pictures has 24-bit colour, so the palette is RGB).
                if let Pieces::Images(art) = pieces
                    && let Tint::Rgb(r, g_, b) = bg
                    && let Some(picture) = art.picture(piece, [r, g_, b], (g.width, g.height))
                {
                    Image::new(picture).render(rect, buf);
                    continue;
                }
                let fg = match piece.color {
                    Color::White => palette.white_piece,
                    Color::Black => palette.black_piece,
                };
                let style = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
                let text = match pieces {
                    Pieces::Symbols => pieces::symbol(piece).to_string(),
                    // Letters, and the fallback if a picture couldn't be made.
                    Pieces::Letters | Pieces::Images(_) => pieces::letters(piece),
                };
                let start = x + g.width.saturating_sub(text.chars().count() as u16) / 2;
                buf.set_string(start, cy, text, style);
            } else if targets.contains(&square) {
                buf.set_string(cx, cy, "•", Style::default().fg(palette.target).bg(bg));
            }
        }
        let rank = screen_to_board(0, line, app.flipped).1;
        let label = char::from(b'1' + rank as u8).to_string();
        let y = g.y + line as u16 * g.height + g.height / 2;
        buf.set_string(g.x - 2, y, label, Style::default());
    }
    for column in 0..8 {
        let file = screen_to_board(column, 0, app.flipped).0;
        let label = char::from(b'a' + file as u8).to_string();
        let x = g.x + column as u16 * g.width + g.width / 2;
        buf.set_string(x, g.y + 8 * g.height, label, Style::default());
    }
}

fn draw_info(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" {} {} ", app.node.name, app.node.version);
    let player = |color: Color| {
        if app.settings.engine_plays[color.index()] {
            app.node.name.clone()
        } else {
            "you".to_string()
        }
    };
    let mut strength = format!("{} ms a move", app.settings.movetime_ms);
    if let Some(level) = app.level_text() {
        strength = format!("{level} · {strength}");
    }
    let state = match app.game.outcome() {
        Some(outcome) => rules::describe_outcome(outcome),
        None => {
            let side = app.game.position().side_to_move();
            let check = if app.game.position().in_check() {
                ", in check"
            } else {
                ""
            };
            let thinking = if app.searching() {
                format!(" · {} is thinking…", app.node.name)
            } else {
                String::new()
            };
            format!("{side} to move{check}{thinking}")
        }
    };
    let lines = vec![
        Line::from(format!(
            "White: {}   Black: {}",
            player(Color::White),
            player(Color::Black)
        )),
        Line::from(strength),
        Line::from(Span::styled(
            format!("Licence: {}", app.node.license),
            Style::default().add_modifier(Modifier::DIM),
        )),
        Line::from(Span::styled(
            state,
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_moves(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Moves ");
    let rows = app.game.numbered_moves();
    let visible = area.height.saturating_sub(2) as usize;
    let skip = rows.len().saturating_sub(visible);
    let lines: Vec<Line> = rows
        .iter()
        .skip(skip)
        .map(|(number, white, black)| {
            Line::from(format!(
                "{:>3}. {:<8} {}",
                number,
                white.unwrap_or("…"),
                black.unwrap_or("")
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// A count with a suffix: `950`, `12.3k`, `4.5M`.
fn count_text(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}G", n as f64 / 1e9),
    }
}

fn draw_engine(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Engine ");
    let lines = match &app.thinking {
        Some(t) => {
            let mut head = format!(
                "depth {}/{}  {}  {} nodes",
                t.depth,
                t.sel_depth,
                score_text(t.score),
                count_text(t.nodes)
            );
            if let Some(nps) = t.nps {
                head.push_str(&format!("  {}/s", count_text(nps)));
            }
            head.push_str(&format!("  {:.1}s", t.time_ms as f64 / 1000.0));
            vec![
                Line::from(Span::styled(
                    head,
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(t.pv.join(" ")),
            ]
        }
        None if app.searching() => vec![Line::from("Thinking…")],
        None => vec![Line::from(Span::styled(
            "Waiting for the engine's turn.",
            Style::default().add_modifier(Modifier::DIM),
        ))],
    };
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    match &app.input {
        Some(input) => {
            let line = Line::from(vec![
                Span::styled("> ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(input.as_str()),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            frame.set_cursor_position((area.x + 2 + input.chars().count() as u16, area.y));
        }
        None => frame.render_widget(Paragraph::new(app.status.as_str()), area),
    }
}

/// The help screen's text.
pub const HELP: &str = "\
Moves
  Click a piece, then its square. Or use the arrows and Enter.
  Or type a move: e4, Nf3, exd5, O-O, e8=N, or e2e4. Enter plays it.

Keys
  m        the engine moves (now, if it is thinking)
  u        take back your last move
  n        new game
  t        turn the board around
  :        type a command
  ?        this help
  q        quit

Commands
  :play white|black|both|none   which sides the engine plays
  :level <n>|max                the engine's playing level
  :time <ms>                    how long it thinks a move
  :fen <FEN>                    a new game from a position
  :pgn <file>                   save the game
  :new  :undo  :flip  :go  :stop  :quit

Press any key to close.";

fn draw_help(frame: &mut Frame, area: Rect) {
    let width = 70.min(area.width);
    let height = (HELP.lines().count() as u16 + 2).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" Help ");
    frame.render_widget(Paragraph::new(HELP).block(block), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{NodeSummary, Settings};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> App {
        let node = NodeSummary {
            name: "eng".into(),
            version: "1.2".into(),
            license: "MIT".into(),
            caps: Default::default(),
        };
        App::new(node, Settings::default()).unwrap()
    }

    fn screen(app: &App, width: u16, height: u16) -> String {
        screen_with(app, width, height, &mut Pieces::Letters)
    }

    fn screen_with(app: &App, width: u16, height: u16, pieces: &mut Pieces) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, app, &Palette::TRUE_COLOR, pieces))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn draws_the_board_the_engine_and_the_moves() {
        let text = screen(&app(), 100, 32);
        assert!(text.contains("eng 1.2"), "{text}");
        assert!(text.contains("Licence: MIT"), "{text}");
        assert!(text.contains("White: you   Black: eng"), "{text}");
        assert!(text.contains("WK") && text.contains("BP"), "{text}");
        assert!(text.contains("White to move"), "{text}");
    }

    #[test]
    fn symbols_are_still_there_if_asked_for() {
        let text = screen_with(&app(), 100, 32, &mut Pieces::Symbols);
        assert!(text.contains('♚'), "{text}");
    }

    #[test]
    fn pictures_fill_square_squares_and_the_panel() {
        use ratatui_image::FontSize;
        use ratatui_image::picker::{Picker, ProtocolType};
        // A terminal that speaks kitty's image protocol, with a 10×20-pixel font.
        // A test has no terminal to ask, so it says the font size itself.
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(FontSize::new(10, 20));
        picker.set_protocol_type(ProtocolType::Kitty);
        let mut pieces = Pieces::choose(crate::pieces::PieceChoice::Auto, Some(picker));
        assert!(matches!(pieces, Pieces::Images(_)));
        let text = screen_with(&app(), 160, 50, &mut pieces);
        // No letters: the pieces are pictures.
        assert!(!text.contains("WK"), "{text}");
        let g = board_geometry(layout(Rect::new(0, 0, 160, 50), Some(2.0)).board, Some(2.0));
        // Twice as wide as tall in cells is square in pixels, and bigger than text's 7×3.
        assert_eq!(g.width, 2 * g.height);
        assert!(g.height > 3, "{g:?}");
    }

    #[test]
    fn a_small_terminal_says_so() {
        let text = screen(&app(), MIN_WIDTH - 1, 10);
        assert!(text.contains("needs at least"), "{text}");
    }

    #[test]
    fn clicks_land_on_the_square_drawn_under_them() {
        let sizes = [
            (100, 32),
            (80, 24),
            (MIN_WIDTH, MIN_HEIGHT),
            (160, 50),
            (240, 70),
        ];
        for (width, height) in sizes {
            for aspect in [None, Some(2.0), Some(2.2)] {
                let area = Rect::new(0, 0, width, height);
                let g = board_geometry(layout(area, aspect).board, aspect);
                for flipped in [false, true] {
                    // The middle of the bottom-left square is a1, or h8 when flipped.
                    let col = g.x + g.width / 2;
                    let row = g.y + 7 * g.height + g.height / 2;
                    let expected = if flipped { "h8" } else { "a1" };
                    let square = square_at(area, aspect, flipped, col, row).unwrap();
                    assert_eq!(square.to_string(), expected, "{width}x{height} {aspect:?}");
                    // Just off the board is nothing.
                    let off = square_at(area, aspect, flipped, g.x + 8 * g.width, row);
                    assert_eq!(off, None);
                }
            }
        }
    }

    #[test]
    fn the_board_grows_with_the_terminal() {
        let geometry = |w, h| board_geometry(layout(Rect::new(0, 0, w, h), None).board, None);
        let (small, large) = (geometry(MIN_WIDTH, MIN_HEIGHT), geometry(160, 50));
        assert_eq!((small.width, small.height), (3, 1));
        assert_eq!((large.width, large.height), (7, 3));
    }

    #[test]
    fn counts_get_suffixes() {
        assert_eq!(count_text(950), "950");
        assert_eq!(count_text(12_345), "12.3k");
        assert_eq!(count_text(4_500_000), "4.5M");
    }
}
