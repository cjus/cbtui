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
//! Zoomed ([`App::zoomed`], the `z` key), the board has the whole screen but the status line:
//! no side panels, no keys line and no frame, and the board is centred. The status line then
//! ends with a short summary (whose move, the engine's score) in place of the panels.
//!
//! Squares grow with the terminal. With pictures for pieces ([`crate::pieces`]) a square is as
//! wide as it is tall in pixels, as big as the panel allows. With letters or symbols, which
//! are a cell each whatever the square, squares come in fixed sizes, from 13×6 cells down to
//! 3×1, the largest that fits.
//!
//! As a PGN viewer ([`App::review`]) the panels on the right show the recorded game: its
//! players and event, its moves with the variations in brackets (the shown move highlighted,
//! the list scrolled to it), and the shown move's comments and the variations that start
//! there. Looking back in a game (`[`), the move list highlights the move shown.
//!
//! Pitfall: a board is usually limited by the terminal's height, not its width, so zooming a
//! wide terminal gains only the rows the frame and keys line took. It gains most in narrow or
//! tall ones, where the side panels' width was what held the board back.
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
use crate::review::Review;
use crate::rules::{self, Color, Position, Square};

/// The width the panels on the right need, at least.
const SIDE_MIN_WIDTH: u16 = 32;

/// The engine-and-players panel's height, frame included: the players, the time and level,
/// and whose move it is (two lines for a long game ending).
const INFO_PANEL_ROWS: u16 = 6;

/// The PGN viewer's top panel's height, frame included: White, Black, the event, and where the
/// board is in the game (two lines in a variation).
const REVIEW_INFO_PANEL_ROWS: u16 = 7;

/// The engine panel's height, frame included, when there is room: its report and a long line.
const ENGINE_PANEL_MAX_ROWS: u16 = 10;

/// The engine panel's height, frame included, when the terminal is short.
const ENGINE_PANEL_MIN_ROWS: u16 = 5;

/// The move list's height, frame included, that the engine panel leaves it if it can.
const MOVES_PANEL_MIN_ROWS: u16 = 5;

/// How many characters of a commit hash an engine's version shows ([`short_version`]).
const SHORT_HASH_LEN: usize = 7;

/// The smallest terminal cbtui draws in.
pub const MIN_WIDTH: u16 = 8 * 3 + 5 + SIDE_MIN_WIDTH;
/// The smallest terminal cbtui draws in.
pub const MIN_HEIGHT: u16 = 8 + 3 + 2;

/// Square sizes for text pieces, largest first: (width, height) in cells. Cells are about
/// twice as tall as they are wide, so a square is about twice as wide as it is tall.
const SQUARE_SIZES: [(u16, u16); 6] = [(13, 6), (11, 5), (9, 4), (7, 3), (5, 2), (3, 1)];

/// The columns the rank labels take, left of the squares.
const RANK_LABEL_COLUMNS: u16 = 2;

/// The columns a board needs besides its squares: the rank labels and one spare.
const BOARD_MARGIN_COLUMNS: u16 = RANK_LABEL_COLUMNS + 1;

/// The tallest square, in cells, when pieces are pictures and squares grow to fill the panel.
const MAX_SQUARE_ROWS: u16 = 16;

/// Where the parts of the screen go. Zoomed, the side panels and the keys line are empty
/// rectangles and the board has no frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panels {
    /// The board's frame (the same as `squares` when zoomed, which has no frame).
    pub board: Rect,
    /// Where the squares and their labels go: inside the board's frame.
    pub squares: Rect,
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
    let fits = |&(w, h): &(u16, u16)| 8 * w + BOARD_MARGIN_COLUMNS <= width && 8 * h < height;
    let smallest = SQUARE_SIZES[SQUARE_SIZES.len() - 1];
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
/// when pieces are pictures, else `None`. `zoomed` gives the board everything but the status
/// line.
pub fn layout(area: Rect, aspect: Option<f32>, zoomed: bool) -> Panels {
    if zoomed {
        let [board, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let none = Rect::new(area.x, area.y, 0, 0);
        return Panels {
            board,
            squares: board,
            info: none,
            moves: none,
            engine: none,
            status,
            keys: none,
        };
    }
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
    let board_width = 8 * w + BOARD_MARGIN_COLUMNS + 2;
    let [board, side] =
        Layout::horizontal([Constraint::Length(board_width), Constraint::Min(0)]).areas(main);
    let [info, moves, engine] = split_side(side, INFO_PANEL_ROWS);
    Panels {
        board,
        squares: Block::default().borders(Borders::ALL).inner(board),
        info,
        moves,
        engine,
        status,
        keys,
    }
}

/// The panels on the right, top to bottom, with `info_rows` for the top one. The bottom one
/// grows up to its most, leaving the move list some rows.
fn split_side(side: Rect, info_rows: u16) -> [Rect; 3] {
    let bottom_rows = side
        .height
        .saturating_sub(info_rows + MOVES_PANEL_MIN_ROWS)
        .clamp(ENGINE_PANEL_MIN_ROWS, ENGINE_PANEL_MAX_ROWS);
    Layout::vertical([
        Constraint::Length(info_rows),
        Constraint::Min(0),
        Constraint::Length(bottom_rows),
    ])
    .areas(side)
}

/// Where the squares are in `squares` ([`Panels::squares`]). `aspect` is a cell's height
/// over its width when pieces are pictures, else `None`.
pub fn board_geometry(squares: Rect, aspect: Option<f32>) -> BoardGeometry {
    let (width, height) = square_size(squares.width, squares.height, aspect);
    // Rank labels take the first two columns, file labels the row under the squares; centre
    // what is drawn (it fills the width, and so isn't moved sideways, unless zoomed).
    let (used_x, used_y) = (RANK_LABEL_COLUMNS + 8 * width, 8 * height + 1);
    BoardGeometry {
        x: squares.x + RANK_LABEL_COLUMNS + squares.width.saturating_sub(used_x) / 2,
        y: squares.y + squares.height.saturating_sub(used_y) / 2,
        width,
        height,
    }
}

/// The square drawn at screen column `col` and row `row`, if any. `aspect` is a cell's
/// height over its width when pieces are pictures, else `None`; `zoomed` and `flipped` are
/// [`App::zoomed`] and [`App::flipped`].
pub fn square_at(
    area: Rect,
    aspect: Option<f32>,
    zoomed: bool,
    flipped: bool,
    col: u16,
    row: u16,
) -> Option<Square> {
    let g = board_geometry(layout(area, aspect, zoomed).squares, aspect);
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
    let panels = layout(area, pieces.cell_aspect(), app.zoomed);
    draw_board(frame.buffer_mut(), panels.squares, app, palette, pieces);
    if app.zoomed {
        let summary = zoom_summary(app);
        let [status, summary_area] = Layout::horizontal([
            Constraint::Min(0),
            Constraint::Length(summary.chars().count() as u16),
        ])
        .areas(panels.status);
        draw_status(frame, status, app);
        frame.render_widget(
            Paragraph::new(summary).style(Style::default().add_modifier(Modifier::DIM)),
            summary_area,
        );
    } else {
        let block = Block::default().borders(Borders::ALL).title(" Board ");
        frame.render_widget(block, panels.board);
        let keys = match &app.review {
            Some(review) => {
                let side = panels.info.union(panels.moves).union(panels.engine);
                let [info, moves, notes] = split_side(side, REVIEW_INFO_PANEL_ROWS);
                draw_review_info(frame, info, review);
                draw_review_moves(frame, moves, review);
                draw_notes(frame, notes, review);
                "← → step · ↑ ↓ start/end · 1-9 variation · :play <side> play on · t flip · z zoom · ? help · q quit"
            }
            None => {
                draw_info(frame, panels.info, app);
                draw_moves(frame, panels.moves, app);
                draw_engine(frame, panels.engine, app);
                "click or arrows: move · type a move · m engine moves · u undo · n new · [ ] look back · t flip · z zoom · : command · ? help · q quit"
            }
        };
        draw_status(frame, panels.status, app);
        frame.render_widget(
            Paragraph::new(keys).style(Style::default().add_modifier(Modifier::DIM)),
            panels.keys,
        );
    }
    if app.show_help {
        draw_help(frame, area, app);
    }
}

fn draw_board(buf: &mut Buffer, squares: Rect, app: &App, palette: &Palette, pieces: &mut Pieces) {
    let g = board_geometry(squares, pieces.cell_aspect());
    let shown = app.shown_position();
    let position = &shown;
    let last = app.shown_last_move();
    let targets = app.targets();
    let check_square = position
        .in_check()
        .then(|| position.king_square(position.side_to_move()));
    for line in 0..8 {
        for column in 0..8 {
            let (file, rank) = screen_to_board(column, line, app.flipped);
            let square = rules::square(file, rank).expect("on the board");
            let light = (file + rank) % 2 == 1;
            let is_last =
                last.is_some_and(|mv| mv.from() == square || rules::destination(mv) == square);
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
        buf.set_string(g.x - RANK_LABEL_COLUMNS, y, label, Style::default());
    }
    for column in 0..8 {
        let file = screen_to_board(column, 0, app.flipped).0;
        let label = char::from(b'a' + file as u8).to_string();
        let x = g.x + column as u16 * g.width + g.width / 2;
        buf.set_string(x, g.y + 8 * g.height, label, Style::default());
    }
}

fn draw_info(frame: &mut Frame, area: Rect, app: &App) {
    let title = match short_version(&app.node.version) {
        version if version.is_empty() => format!(" {} ", app.node.name),
        version => format!(" {} {version} ", app.node.name),
    };
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
            state,
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

/// A version with a long commit hash in its build metadata cut to [`SHORT_HASH_LEN`]
/// characters: `1.0.0+f41bbcc85c7a` is `1.0.0+f41bbcc`. Any other version is left as it is.
fn short_version(version: &str) -> String {
    match version.split_once('+') {
        Some((release, build))
            if build.len() > SHORT_HASH_LEN && build.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            format!("{release}+{}", &build[..SHORT_HASH_LEN])
        }
        _ => version.to_string(),
    }
}

fn draw_moves(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Moves ");
    let rows = app.game.numbered_moves();
    let visible = area.height.saturating_sub(2) as usize;
    // Looking back: the row and column (0 White, 1 Black) of the move shown.
    let offset = usize::from(app.game.start().side_to_move() == Color::Black);
    let shown = app
        .browse
        .and_then(|ply| ply.checked_sub(1))
        .map(|i| ((i + offset) / 2, (i + offset) % 2));
    let mut skip = rows.len().saturating_sub(visible);
    if let Some((row, _)) = shown {
        skip = skip.min(row);
    }
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(row, (number, white, black))| {
            let style = |column| {
                if shown == Some((row, column)) {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                }
            };
            Line::from(vec![
                Span::raw(format!("{number:>3}. ")),
                Span::styled(format!("{:<8}", white.unwrap_or("…")), style(0)),
                Span::raw(" "),
                Span::styled(black.unwrap_or("").to_string(), style(1)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The PGN viewer's top panel: the players (with ratings), the event and result, and where
/// the board is in the game (two lines, if it needs them).
fn draw_review_info(frame: &mut Frame, area: Rect, review: &Review) {
    let player = |color: &str| {
        let name = review.tag(color).unwrap_or("?");
        match review.tag(&format!("{color}Elo")) {
            Some(elo) => format!("{name} ({elo})"),
            None => name.to_string(),
        }
    };
    // A date's unknown parts go: `1858.??.??` is `1858`.
    let date = review
        .tag("Date")
        .map(|date| date.trim_end_matches(['?', '.']));
    let mut event: Vec<&str> = [review.tag("Event"), review.tag("Site"), date]
        .into_iter()
        .flatten()
        .collect();
    event.push(&review.result);
    // The players and event are a line each, cut at the edge if long, so the state below
    // always has its two lines.
    let header = vec![
        Line::from(format!("White: {}", player("White"))),
        Line::from(format!("Black: {}", player("Black"))),
        Line::from(event.join(" · ")),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", review.name));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [top, bottom] =
        Layout::vertical([Constraint::Length(header.len() as u16), Constraint::Min(0)])
            .areas(inner);
    frame.render_widget(Paragraph::new(header), top);
    let state = Span::styled(
        review_state(review),
        Style::default().add_modifier(Modifier::BOLD),
    );
    frame.render_widget(
        Paragraph::new(Line::from(state)).wrap(Wrap { trim: true }),
        bottom,
    );
}

/// Where the viewer is: `After 12… Nf6 · White to move`, with `in a variation` off the main
/// line, and checkmate or stalemate when the position is one.
fn review_state(review: &Review) -> String {
    let node = review.current();
    let mut parts = vec![match review.cursor() {
        0 => "Start".to_string(),
        id => format!("After {}", review.label(id, true)),
    }];
    parts.push(side_or_ending(&node.position));
    if review.main_line_place().is_none() {
        parts.push("in a variation".into());
    }
    parts.join(" · ")
}

/// `White to move` (`, in check`), or `Checkmate` or `Stalemate` when there are no moves.
fn side_or_ending(position: &Position) -> String {
    let side = position.side_to_move();
    match (position.legal_moves().is_empty(), position.in_check()) {
        (true, true) => format!("Checkmate: {} wins", !side),
        (true, false) => "Stalemate".into(),
        (false, true) => format!("{side} to move, in check"),
        (false, false) => format!("{side} to move"),
    }
}

/// The PGN viewer's move list: the moves as PGN writes them, variations in brackets and
/// dimmed, wrapped to the panel, the shown move highlighted and kept in view.
fn draw_review_moves(frame: &mut Frame, area: Rect, review: &Review) {
    let block = Block::default().borders(Borders::ALL).title(" Moves ");
    let width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Vec<Span>> = vec![Vec::new()];
    let (mut used, mut current_line) = (0, 0);
    for token in review.tokens() {
        let (open, close) = ("(".repeat(token.open), ")".repeat(token.close));
        let length = open.chars().count() + token.text.chars().count() + close.chars().count();
        if used > 0 && used + 1 + length > width {
            lines.push(Vec::new());
            used = 0;
        }
        let line = lines.last_mut().expect("never empty");
        if used > 0 {
            line.push(Span::raw(" "));
            used += 1;
        }
        let dim = Style::default().add_modifier(Modifier::DIM);
        let style = if token.node == review.cursor() {
            current_line = lines.len() - 1;
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else if token.depth > 0 {
            dim
        } else {
            Style::default()
        };
        let line = lines.last_mut().expect("never empty");
        line.push(Span::styled(open, dim));
        line.push(Span::styled(token.text, style));
        line.push(Span::styled(close, dim));
        used += length;
    }
    let visible = area.height.saturating_sub(2) as usize;
    let skip = current_line
        .saturating_sub(visible / 2)
        .min(lines.len().saturating_sub(visible));
    let lines: Vec<Line> = lines.into_iter().skip(skip).map(Line::from).collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The PGN viewer's bottom panel: the shown move's comments, and the variations that start
/// here, numbered for the keys that enter them.
fn draw_notes(frame: &mut Frame, area: Rect, review: &Review) {
    let node = review.current();
    let mut lines: Vec<Line> = [&node.before, &node.comment]
        .into_iter()
        .flatten()
        .map(|comment| rules::comment_text(comment))
        .filter(|text| !text.is_empty())
        .map(Line::from)
        .collect();
    let alternatives = review.alternatives();
    if !alternatives.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let main = node.children[0];
        lines.push(Line::from(format!(
            "Next: {} · or press",
            review.label(main, true)
        )));
        for (i, &id) in alternatives.iter().enumerate().take(9) {
            lines.push(Line::from(format!(
                "  {}  {}",
                i + 1,
                review.label(id, true)
            )));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No comment here.",
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    let block = Block::default().borders(Borders::ALL).title(" Notes ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
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

/// What the zoomed status line ends with, in place of the side panels: the last move, whose
/// move it is (or how the game ended), and the engine's latest score and depth.
fn zoom_summary(app: &App) -> String {
    let mut parts = Vec::new();
    parts.extend(app.shown_move_label());
    if let Some(review) = &app.review {
        parts.push(side_or_ending(&review.current().position));
        if review.main_line_place().is_none() {
            parts.push("in a variation".into());
        }
    } else if app.browse.is_some() {
        parts.push("looking back · ] to return".into());
    } else {
        parts.push(match app.game.outcome() {
            Some(outcome) => rules::describe_outcome(outcome),
            None if app.searching() => format!("{} is thinking…", app.node.name),
            None => format!("{} to move", app.game.position().side_to_move()),
        });
        if let Some(t) = &app.thinking {
            parts.push(format!("{} d{}", score_text(t.score), t.depth));
        }
    }
    parts.push("z panels ".into());
    parts.join(" · ")
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
  [ ]      look back through the game a move at a time (Home, End)
  t        turn the board around
  z        zoom: the board fills the screen (again for the panels)
  :        type a command
  ?        this help
  q        quit

PGN viewer (cbtui --pgn <file>)
  ← →      a move back or on (also [ ])
  ↑ ↓      the start of the game, the end of the line (also Home, End)
  1-9      into a variation that starts at this move
  :play white|black|both|none   play on from the position shown

Commands
  :play white|black|both|none   which sides the engine plays
  :level <n>|max                the engine's playing level
  :time <ms>                    how long it thinks a move
  :fen <FEN>                    a new game from a position
  :pgn <file>                   save the game
  :new  :undo  :flip  :zoom  :go  :stop  :quit

Press any key to close.";

/// The help screen: which engine it is (its whole version, and its licence), then [`HELP`].
fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let engine = if app.has_engine {
        format!(
            "Engine\n  {} {}, licence {}\n\n",
            app.node.name, app.node.version, app.node.license
        )
    } else {
        "Engine\n  None: start cbtui with --engine <path> to play one.\n\n".to_string()
    };
    let text = engine + HELP;
    let width = 70.min(area.width);
    let height = (text.lines().count() as u16 + 2).min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" Help ");
    frame.render_widget(Paragraph::new(text).block(block), rect);
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
        assert!(
            !text.contains("MIT"),
            "the licence is on the help screen: {text}"
        );
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
        let g = board_geometry(
            layout(Rect::new(0, 0, 160, 50), Some(2.0), false).squares,
            Some(2.0),
        );
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
            for (aspect, zoomed) in [None, Some(2.0), Some(2.2)]
                .into_iter()
                .flat_map(|aspect| [(aspect, false), (aspect, true)])
            {
                let area = Rect::new(0, 0, width, height);
                let g = board_geometry(layout(area, aspect, zoomed).squares, aspect);
                for flipped in [false, true] {
                    // The middle of the bottom-left square is a1, or h8 when flipped.
                    let col = g.x + g.width / 2;
                    let row = g.y + 7 * g.height + g.height / 2;
                    let expected = if flipped { "h8" } else { "a1" };
                    let at = |col, row| square_at(area, aspect, zoomed, flipped, col, row);
                    let square = at(col, row).unwrap();
                    let case = format!("{width}x{height} {aspect:?} zoomed {zoomed}");
                    assert_eq!(square.to_string(), expected, "{case}");
                    // Just off the board, either side, is nothing.
                    assert_eq!(at(g.x + 8 * g.width, row), None, "{case}");
                    assert_eq!(at(g.x - 1, row), None, "{case}");
                }
            }
        }
    }

    #[test]
    fn the_board_grows_with_the_terminal() {
        let geometry =
            |w, h| board_geometry(layout(Rect::new(0, 0, w, h), None, false).squares, None);
        let (small, large) = (geometry(MIN_WIDTH, MIN_HEIGHT), geometry(160, 50));
        assert_eq!((small.width, small.height), (3, 1));
        assert_eq!((large.width, large.height), (11, 5));
    }

    #[test]
    fn zoomed_the_board_has_the_screen_and_the_panels_go() {
        let mut app = app();
        app.zoomed = true;
        let text = screen(&app, 100, 32);
        assert!(text.contains("WK") && text.contains("BP"), "{text}");
        assert!(
            !text.contains("Moves") && !text.contains("Licence"),
            "{text}"
        );
        assert!(!text.contains("q quit"), "{text}");
        // The status line keeps the message and ends with the summary.
        let status = text.lines().last().unwrap();
        assert!(status.starts_with("Connected to eng 1.2"), "{status}");
        assert!(
            status.trim_end().ends_with("White to move · z panels"),
            "{status}"
        );
    }

    #[test]
    fn zooming_a_narrow_terminal_makes_the_board_bigger_and_centres_it() {
        let size = |zoomed, aspect| {
            let area = Rect::new(0, 0, 100, 60);
            board_geometry(layout(area, aspect, zoomed).squares, aspect)
        };
        for aspect in [None, Some(2.0)] {
            let (panels, zoomed) = (size(false, aspect), size(true, aspect));
            assert!(
                zoomed.height > panels.height,
                "{aspect:?}: {panels:?} {zoomed:?}"
            );
            // Centred: as much room left of the labels as right of the board.
            let right = 100 - (zoomed.x + 8 * zoomed.width);
            assert!((zoomed.x - 2).abs_diff(right) <= 1, "{zoomed:?}");
        }
    }

    #[test]
    fn the_help_says_the_engine_s_whole_version_and_licence() {
        let mut app = app();
        app.node.version = "1.0.0+f41bbcc85c7a".into();
        let text = screen(&app, 100, 40);
        assert!(text.contains("eng 1.0.0+f41bbcc "), "{text}");
        assert!(!text.contains("f41bbcc85c7a"), "{text}");
        app.show_help = true;
        let text = screen(&app, 100, 40);
        assert!(
            text.contains("eng 1.0.0+f41bbcc85c7a, licence MIT"),
            "{text}"
        );
    }

    #[test]
    fn versions_keep_a_short_hash() {
        assert_eq!(short_version("1.0.0+f41bbcc85c7a"), "1.0.0+f41bbcc");
        assert_eq!(short_version("1.0.0+f41bbcc"), "1.0.0+f41bbcc");
        assert_eq!(short_version("1.0.0+build.5"), "1.0.0+build.5");
        assert_eq!(short_version("2.1"), "2.1");
    }

    #[test]
    fn the_engine_panel_grows_but_leaves_the_move_list_room() {
        let panels = |h| layout(Rect::new(0, 0, 100, h), None, false);
        assert_eq!(panels(50).engine.height, ENGINE_PANEL_MAX_ROWS);
        let short = panels(20);
        assert!(short.engine.height >= ENGINE_PANEL_MIN_ROWS, "{short:?}");
        assert!(short.moves.height >= MOVES_PANEL_MIN_ROWS, "{short:?}");
    }

    fn viewer() -> App {
        let pgn = "[Event \"Club match\"]\n[Date \"2026.11.24\"]\n[White \"Ann\"]\n\
            [WhiteElo \"1900\"]\n[Black \"Bo\"]\n[Result \"1-0\"]\n\n\
            1. e4 e5 2. Nf3 {The usual.} (2. f4 exf4) Nc6 $1 1-0";
        let mut app = app();
        app.open_review(Review::from_pgn(pgn, "club.pgn").unwrap());
        app
    }

    #[test]
    fn the_viewer_shows_the_tags_the_moves_and_the_notes() {
        let mut app = viewer();
        let text = screen(&app, 100, 32);
        assert!(text.contains(" club.pgn "), "{text}");
        assert!(text.contains("White: Ann (1900)"), "{text}");
        assert!(text.contains("Black: Bo"), "{text}");
        assert!(text.contains("Club match · 2026.11.24 · 1-0"), "{text}");
        assert!(text.contains("Start · White to move"), "{text}");
        assert!(
            text.contains("1. e4 e5 2. Nf3 (2. f4 exf4) 2… Nc6!"),
            "{text}"
        );
        assert!(text.contains("← → step"), "{text}");
        for _ in 0..2 {
            app.review.as_mut().unwrap().forward();
        }
        let text = screen(&app, 100, 32);
        assert!(text.contains("After 1… e5 · White to move"), "{text}");
        assert!(text.contains("Next: 2. Nf3 · or press"), "{text}");
        assert!(text.contains("1  2. f4"), "{text}");
        app.review.as_mut().unwrap().forward();
        let text = screen(&app, 100, 32);
        assert!(text.contains("The usual."), "{text}");
        app.review.as_mut().unwrap().back();
        app.review.as_mut().unwrap().enter(1);
        let text = screen(&app, 100, 32);
        // The state wraps onto the panel's last line.
        assert!(
            text.contains("After 2. f4 · Black to move · in a"),
            "{text}"
        );
        assert!(text.contains("│variation"), "{text}");
    }

    #[test]
    fn the_viewer_s_move_list_keeps_the_shown_move_in_view() {
        // A long game in a short terminal: the shown move is on screen wherever it is.
        let moves: Vec<String> = (0..60)
            .map(|i| match i % 4 {
                0 => "Nf3",
                1 => "Nf6",
                2 => "Ng1",
                _ => "Ng8",
            })
            .enumerate()
            .map(|(i, m)| {
                if i % 2 == 0 {
                    format!("{}. {m}", i / 2 + 1)
                } else {
                    m.into()
                }
            })
            .collect();
        let mut app = app();
        app.open_review(Review::from_pgn(&moves.join(" "), "long.pgn").unwrap());
        app.review.as_mut().unwrap().to_end();
        let text = screen(&app, MIN_WIDTH + 10, 24);
        assert!(text.contains("30… Ng8"), "{text}");
    }

    #[test]
    fn looking_back_highlights_the_move_in_the_list() {
        let mut app = app();
        let mut engine = crate::node::NoEngine;
        app.settings.engine_plays = [false, false];
        for c in ["e4", "e5"] {
            app.input = Some(c.into());
            app.handle_key(
                ratatui::crossterm::event::KeyEvent::from(
                    ratatui::crossterm::event::KeyCode::Enter,
                ),
                &mut engine,
            );
        }
        app.browse = Some(1);
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        terminal
            .draw(|frame| draw(frame, &app, &Palette::TRUE_COLOR, &mut Pieces::Letters))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // Find "e4" in the move list: it is the highlighted one.
        let (x, y) = (0..32)
            .flat_map(|y| (0..99).map(move |x| (x, y)))
            .find(|&(x, y)| {
                buffer[(x, y)].symbol() == "e"
                    && buffer[(x + 1, y)].symbol() == "4"
                    && buffer[(x - 1, y)].symbol() == " "
                    && buffer[(x - 2, y)].symbol() == "."
            })
            .expect("e4 in the move list");
        assert!(buffer[(x, y)].modifier.contains(Modifier::REVERSED));
        let zoomed = {
            app.zoomed = true;
            screen(&app, 100, 32)
        };
        assert!(zoomed.contains("1. e4 · looking back"), "{zoomed}");
    }

    #[test]
    fn counts_get_suffixes() {
        assert_eq!(count_text(950), "950");
        assert_eq!(count_text(12_345), "12.3k");
        assert_eq!(count_text(4_500_000), "4.5M");
    }
}
