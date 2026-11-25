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
//! A promotion waiting for its piece ([`App::promotion`]) draws the four choices on the
//! board, on squares of their own colour, from the promotion square towards the middle: the
//! squares a click chooses from, so the mouse needs no geometry of its own.
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

use crate::analysis::Analysis;
use crate::app::{App, Overlay, score_text, white_view};
use crate::clock::clock_text;
use crate::help::HelpView;
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

/// Rows of the tree browser's top panel: the search, where the walk is, the node.
const TREE_INFO_PANEL_ROWS: u16 = 10;

/// Rows at the bottom of the root view for the latest re-search.
const ROOT_RESEARCH_ROWS: u16 = 3;

/// Columns for a root move's percentage after its bar.
const ROOT_PERCENT_COLUMNS: usize = 5;

/// How wide the evaluation and techniques overlays are, at most.
const OVERLAY_WIDTH: u16 = 72;

/// The narrowest and widest an evaluation term's name column is.
const EVAL_NAME_MIN: usize = 4;
const EVAL_NAME_MAX: usize = 32;

/// Rows under the techniques for the highlighted one's description and the keys.
const TECHNIQUES_FOOTER_ROWS: u16 = 4;

/// How many moves of the best line the zoomed status line shows while analysing.
const ZOOM_PV_MOVES: usize = 4;

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
    picker: Tint,
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
        picker: Tint::Rgb(125, 135, 160),
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
        picker: Tint::Indexed(103),
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
            _ if app.tree.is_some() => {
                let side = panels.info.union(panels.moves).union(panels.engine);
                draw_tree(frame, side, app);
                "↑ ↓ choose a move · → or Enter into it · ← back · Esc close the tree"
            }
            Some(review) => {
                let side = panels.info.union(panels.moves).union(panels.engine);
                let [info, moves, notes] = split_side(side, REVIEW_INFO_PANEL_ROWS);
                draw_review_info(frame, info, review);
                if app.follow_root {
                    draw_root(frame, moves, app);
                } else {
                    draw_review_moves(frame, moves, review);
                }
                match &app.analysis {
                    Some(analysis) => draw_analysis(frame, notes, analysis, app.analysis_lines()),
                    None => draw_notes(frame, notes, review),
                }
                "← → step · ↑ ↓ start/end · 1-9 variation · i analyse · :play <side> play on · t flip · z zoom · ? help · q quit"
            }
            None if app.promotion.is_some() => {
                draw_info(frame, panels.info, app);
                draw_moves_or_root(frame, panels.moves, app);
                draw_engine(frame, panels.engine, app);
                "promote: click a piece · q queen · r rook · b bishop · n knight · Enter queen · Esc put back"
            }
            None => {
                draw_info(frame, panels.info, app);
                draw_moves_or_root(frame, panels.moves, app);
                draw_engine(frame, panels.engine, app);
                "click or arrows: move · type a move · m engine moves · u undo · n new · i analyse · [ ] look back · t flip · z zoom · : command · ? help · q quit"
            }
        };
        draw_status(frame, panels.status, app);
        frame.render_widget(
            Paragraph::new(keys).style(Style::default().add_modifier(Modifier::DIM)),
            panels.keys,
        );
    }
    match app.overlay {
        Some(Overlay::Evaluation) => draw_evaluation(frame, area, app),
        Some(Overlay::Techniques { cursor }) => draw_techniques(frame, area, app, cursor),
        None => {}
    }
    if app.show_help {
        draw_help(frame, area, app);
    }
}

/// A rectangle `width` by `height`, centred in `area` and no bigger than it.
fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let (width, height) = (width.min(area.width), height.min(area.height));
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Centipawns as pawns with a sign: `+0.30`.
fn pawns(cp: i32) -> String {
    format!("{:+.2}", f64::from(cp) / 100.0)
}

/// The evaluation overlay: the score, then each term for White and Black and the difference,
/// in pawns.
fn draw_evaluation(frame: &mut Frame, area: Rect, app: &App) {
    let Some(evaluation) = &app.evaluation else {
        return;
    };
    let result = &evaluation.result;
    let side = evaluation.position.side_to_move();
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("Score {}", score_text(white_view(result.score, side))),
            bold,
        ),
        Span::raw("  for White, the position as it stands, without a search"),
    ])];
    if let Some(search) = result.search_score {
        lines.push(Line::from(format!(
            "The search evaluates it {}: it uses another evaluation.",
            score_text(white_view(search, side))
        )));
    }
    if let Some(quiet) = result.quiet.as_ref().filter(|moves| !moves.is_empty()) {
        lines.push(Line::from(format!(
            "Evaluated after the captures settle: {}",
            rules::line_to_san(&evaluation.position, quiet).join(" ")
        )));
    }
    lines.push(Line::from(""));
    let name_width = result
        .terms
        .iter()
        .map(|t| t.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(EVAL_NAME_MIN, EVAL_NAME_MAX);
    lines.push(Line::from(Span::styled(
        format!(
            "{:<name_width$} {:>8} {:>8} {:>8}",
            "Term", "White", "Black", "Net"
        ),
        bold,
    )));
    for term in &result.terms {
        let name: String = term.name.chars().take(name_width).collect();
        lines.push(Line::from(format!(
            "{name:<name_width$} {:>8} {:>8} {:>8}",
            pawns(term.white),
            pawns(term.black),
            pawns(term.white - term.black)
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "In pawns; Net is White's less Black's. Any key closes.",
        Style::default().add_modifier(Modifier::DIM),
    )));
    let rect = centred(area, OVERLAY_WIDTH, lines.len() as u16 + 2);
    frame.render_widget(Clear, rect);
    let title = format!(" Evaluation · {} ", evaluation.label);
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The techniques overlay: each with a box for on or off (`*` when switched from its
/// default), the highlighted one's description below, and the keys.
fn draw_techniques(frame: &mut Frame, area: Rect, app: &App, cursor: usize) {
    let Some(techniques) = &app.techniques else {
        return;
    };
    let height =
        (techniques.len() as u16 + TECHNIQUES_FOOTER_ROWS + 2).min(area.height.saturating_sub(2));
    let rect = centred(area, OVERLAY_WIDTH, height);
    frame.render_widget(Clear, rect);
    let title = match app.settings.techniques.len() {
        0 => " Techniques · all at their defaults ".to_string(),
        n => format!(" Techniques · {n} switched "),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let [list, about] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(TECHNIQUES_FOOTER_ROWS),
    ])
    .areas(inner);
    let rows = list.height as usize;
    // A name more than one technique has (an engine might call several "Evaluation") says
    // too little, so those show their descriptions.
    let label = |technique: &cnp::engine::Technique| {
        let shared = techniques
            .iter()
            .filter(|t| t.name == technique.name)
            .count()
            > 1;
        if shared && !technique.description.is_empty() {
            technique.description.clone()
        } else {
            technique.name.clone()
        }
    };
    // Keeps the highlighted one in view, a little above the bottom when it can.
    let skip = cursor.saturating_sub(rows.saturating_sub(1));
    let lines: Vec<Line> = techniques
        .iter()
        .enumerate()
        .skip(skip)
        .take(rows)
        .map(|(i, technique)| {
            let on = app.technique_on(technique);
            let switched = app.settings.techniques.contains_key(&technique.id);
            let text = format!(
                "[{}]{} {}",
                if on { "x" } else { " " },
                if switched { "*" } else { " " },
                label(technique)
            );
            let style = if i == cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Line::from(Span::styled(text, style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
    let mut about_lines = Vec::new();
    if let Some(technique) = techniques.get(cursor) {
        about_lines.push(Line::from(technique.description.clone()));
        let mut facts = format!(
            "id {}, {} by default",
            technique.id,
            if technique.default { "on" } else { "off" }
        );
        if !technique.requires.is_empty() {
            facts.push_str(&format!("; needs {}", technique.requires.join(", ")));
        }
        about_lines.push(Line::from(Span::styled(
            facts,
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    let [description, keys] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(about);
    frame.render_widget(
        Paragraph::new(about_lines).wrap(Wrap { trim: true }),
        description,
    );
    frame.render_widget(
        Paragraph::new("↑ ↓ choose · Space switch · d default · Esc close")
            .style(Style::default().add_modifier(Modifier::DIM)),
        keys,
    );
}

fn draw_board(buf: &mut Buffer, squares: Rect, app: &App, palette: &Palette, pieces: &mut Pieces) {
    let g = board_geometry(squares, pieces.cell_aspect());
    let shown = app.shown_position();
    let position = &shown;
    let last = app.shown_last_move();
    let targets = app.targets();
    let picker = app.promotion_squares();
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
            let offered = picker.iter().find(|(at, _)| *at == square).map(|&(_, p)| p);
            let piece = offered.or_else(|| position.piece_at(square));
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
            if offered.is_some() {
                bg = palette.picker;
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
    let mut strength = match &app.clocks {
        Some(clocks) if app.node.caps.clock => format!("{} clock", clocks.control()),
        // The engine can't read a clock: cbtui gives it a share of its time each move.
        Some(clocks) => format!("{} clock (cbtui times {})", clocks.control(), app.node.name),
        None => format!("{} ms a move", app.settings.movetime_ms),
    };
    if !app.settings.techniques.is_empty() {
        strength.push_str(&format!(
            " · {} technique{} switched",
            app.settings.techniques.len(),
            if app.settings.techniques.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
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
            let offer = if app.draw_offered {
                " · draw offered"
            } else {
                ""
            };
            format!("{side} to move{check}{thinking}{offer}")
        }
    };
    let mut players = vec![Span::raw(format!("White: {}", player(Color::White)))];
    players.extend(clock_span(app, Color::White));
    players.push(Span::raw(format!("   Black: {}", player(Color::Black))));
    players.extend(clock_span(app, Color::Black));
    let lines = vec![
        Line::from(players),
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

/// `color`'s clock, after a space, highlighted while it runs; nothing without clocks.
fn clock_span(app: &App, color: Color) -> Option<Span<'static>> {
    let clocks = app.clocks.as_ref()?;
    let text = format!(" {}", clock_text(clocks.left(color, app.now)));
    let style = if clocks.running() == Some(color) {
        Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
    };
    Some(Span::styled(text, style))
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

/// A search tree being walked, in place of the side panels: the search and the node shown,
/// the node's children with their scores, and the highlighted child's decisions in full.
fn draw_tree(frame: &mut Frame, area: Rect, app: &App) {
    let Some(tree) = &app.tree else {
        return;
    };
    let level = tree.level();
    let side = level.position.side_to_move();
    let [info, children, about] = split_side(area, TREE_INFO_PANEL_ROWS);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let technique = |id: &str| {
        app.techniques
            .as_ref()
            .and_then(|all| all.iter().find(|t| t.id == id))
            .map_or(id.to_string(), |t| t.name.clone())
    };

    // The search, and where the walk is.
    let mut head = format!(
        "Depth {} · {} nodes · {} plies recorded",
        tree.depth,
        count_text(tree.nodes),
        tree.plies
    );
    if tree.truncated {
        head.push_str(" (cut short)");
    }
    let best = match (&tree.best, tree.score) {
        (Some(best), Some(score)) => format!("Best {best} {}", score_text(score)),
        (Some(best), None) => format!("Best {best}"),
        _ => "No legal move".into(),
    };
    let path = tree.path_san();
    let here = if path.is_empty() {
        format!("At the root · {side} to move")
    } else {
        format!("After {} · {side} to move", path.join(" "))
    };
    let mut node_line = format!("Searched {} plies", level.node.depth);
    if let (Some(low), Some(high)) = crate::tree::window(&level.node, side) {
        node_line.push_str(&format!(
            " · window {} to {}",
            score_text(low),
            score_text(high)
        ));
    }
    if let Some(score) = crate::tree::node_score(&level.node, side) {
        node_line.push_str(&format!(" · {}", crate::tree::bound_text(score)));
    }
    let mut lines = vec![
        Line::from(Span::styled(head, dim)),
        Line::from(best),
        Line::from(Span::styled(here, bold)),
        Line::from(node_line),
    ];
    if let Some(parent) = tree.levels.iter().rev().nth(1) {
        for decision in &level.node.decisions {
            lines.push(Line::from(Span::styled(
                crate::tree::describe(decision, &parent.position, &technique),
                dim,
            )));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Search tree ");
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        info,
    );

    // The children, the highlighted one kept in view.
    let title = format!(
        " {} move{} searched here ",
        level.children.len(),
        if level.children.len() == 1 { "" } else { "s" }
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let rows = children.height.saturating_sub(2) as usize;
    let skip = level.cursor.saturating_sub(rows.saturating_sub(1));
    let child_lines: Vec<Line> = level
        .children
        .iter()
        .enumerate()
        .skip(skip)
        .take(rows)
        .map(|(i, child)| {
            let mv = child.mv.as_deref().unwrap_or("");
            let san = rules::line_to_san(&level.position, &[mv.to_string()])
                .pop()
                .unwrap_or_else(|| mv.to_string());
            let score = crate::tree::node_score(child, !side)
                .map_or("…".to_string(), crate::tree::bound_text);
            // A child's decisions name moves from the child's position.
            let mut after = level.position.clone();
            if let Some(parsed) = after.parse_uci_move(mv) {
                after.make_move(parsed);
            }
            let briefs: Vec<String> = child
                .decisions
                .iter()
                .map(|d| crate::tree::brief(d, &after))
                .collect();
            let below = if child.child_count > 0 { "›" } else { " " };
            let text = format!(
                "{san:<7} {score:>9}  d{:<2} {below} {}",
                child.depth,
                briefs.join(", ")
            );
            let style = if i == level.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Line::from(Span::styled(text, style))
        })
        .collect();
    frame.render_widget(Paragraph::new(child_lines).block(block), children);

    // The highlighted child's decisions in full.
    let mut about_lines = Vec::new();
    if let Some(child) = level.selected() {
        let mut position = level.position.clone();
        if let Some(parsed) = child
            .mv
            .as_deref()
            .and_then(|mv| position.parse_uci_move(mv))
        {
            position.make_move(parsed);
        }
        if child.decisions.is_empty() {
            about_lines.push(Line::from(Span::styled(
                "Searched in full, nothing cut short.",
                dim,
            )));
        }
        for decision in &child.decisions {
            about_lines.push(Line::from(crate::tree::describe(
                decision, &position, &technique,
            )));
        }
        let below = match child.subtree {
            0 => "Nothing recorded below it.".to_string(),
            n if child.child_count > 0 => {
                format!("{} nodes recorded below it: → to look.", count_text(n))
            }
            n => format!("{} nodes recorded below it.", count_text(n)),
        };
        about_lines.push(Line::from(Span::styled(below, dim)));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" What the search decided ");
    frame.render_widget(
        Paragraph::new(about_lines)
            .block(block)
            .wrap(Wrap { trim: true }),
        about,
    );
}

/// The move list, or while following the root (`w`), the root.
fn draw_moves_or_root(frame: &mut Frame, area: Rect, app: &App) {
    if app.follow_root {
        draw_root(frame, area, app);
    } else {
        draw_moves(frame, area, app);
    }
}

/// The search at its root: the move it's on and the best so far, a bar for each root move's
/// share of the effort, biggest first, and the latest thing it went back over.
fn draw_root(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Root · w stops ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(root) = &app.root else {
        let text =
            Paragraph::new("Waiting for a search to report its root: the engine's turn, or i.")
                .style(Style::default().add_modifier(Modifier::DIM))
                .wrap(Wrap { trim: true });
        frame.render_widget(text, inner);
        return;
    };
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut head = format!(
        "depth {} · move {} of {}: {}",
        root.depth, root.number, root.of, root.current
    );
    if let Some(best) = &root.best {
        head.push_str(&format!(" · best {best}"));
    }
    let mut lines = vec![
        Line::from(Span::styled(head, bold)),
        Line::from(Span::styled(
            format!(
                "{:.1}s · {} nodes · effort by root move:",
                root.time_ms as f64 / 1000.0,
                count_text(root.nodes)
            ),
            Style::default().add_modifier(Modifier::DIM),
        )),
    ];
    // The re-search takes the last rows, wrapped; the bars get the rest.
    let research_rows = if root.research.is_some() {
        ROOT_RESEARCH_ROWS
    } else {
        0
    };
    let bar_rows = (inner.height as usize).saturating_sub(lines.len() + research_rows as usize);
    let label_width = root
        .efforts
        .iter()
        .map(|e| e.san.chars().count())
        .max()
        .unwrap_or(0);
    let bar_width = (inner.width as usize).saturating_sub(label_width + ROOT_PERCENT_COLUMNS + 1);
    for effort in root.efforts.iter().take(bar_rows) {
        let filled = (effort.share * bar_width as f64).round() as usize;
        let style = if root.best.as_deref() == Some(effort.san.as_str()) {
            bold
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<label_width$} ", effort.san), style),
            Span::styled("█".repeat(filled), style),
            Span::raw(format!(
                "{:>width$}",
                format!("{:.0}%", effort.share * 100.0),
                width = ROOT_PERCENT_COLUMNS + bar_width - filled
            )),
        ]));
    }
    let [bars, research] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(research_rows)]).areas(inner);
    frame.render_widget(Paragraph::new(lines), bars);
    if let Some((text, at)) = &root.research {
        let text = format!("At {:.1}s: {text}", *at as f64 / 1000.0);
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().add_modifier(Modifier::DIM))
                .wrap(Wrap { trim: true }),
            research,
        );
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

/// The analysis in the engine panel: the search's totals, then each line on a row of its
/// own, best first: its score, depth and moves (cut off at the panel's edge).
fn draw_analysis(frame: &mut Frame, area: Rect, analysis: &Analysis, wanted: u32) {
    let title = format!(
        " Analysis · {wanted} line{} · i stops ",
        if wanted == 1 { "" } else { "s" }
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let mut head = format!(
        "depth {}  {} nodes",
        analysis.depth,
        count_text(analysis.nodes)
    );
    if let Some(nps) = analysis.nps {
        head.push_str(&format!("  {}/s", count_text(nps)));
    }
    head.push_str(&format!("  {:.1}s", analysis.time_ms as f64 / 1000.0));
    if !analysis.running() {
        head.push_str("  (stopped)");
    }
    let mut lines = vec![Line::from(Span::styled(
        head,
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    if analysis.lines.is_empty() {
        lines.push(Line::from("Thinking…"));
    }
    for line in &analysis.lines {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>6} ", score_text(line.score)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("d{:<3}", line.depth),
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::raw(line.pv.join(" ")),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_engine(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(analysis) = &app.analysis {
        draw_analysis(frame, area, analysis, app.analysis_lines());
        return;
    }
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
    } else if app.analysis.is_some() {
        parts.push(format!("{} to move", app.game.position().side_to_move()));
    } else {
        parts.push(match app.game.outcome() {
            Some(outcome) => rules::describe_outcome(outcome),
            None if app.searching() => format!("{} is thinking…", app.node.name),
            None => format!("{} to move", app.game.position().side_to_move()),
        });
        if let Some(t) = &app.thinking {
            parts.push(format!("{} d{}", score_text(t.score), t.depth));
        }
        if let Some(clocks) = &app.clocks {
            parts.push(format!(
                "W {} B {}",
                clock_text(clocks.left(Color::White, app.now)),
                clock_text(clocks.left(Color::Black, app.now))
            ));
        }
    }
    if let Some(best) = app.analysis.as_ref().and_then(|a| a.lines.first()) {
        let moves: Vec<&str> = best
            .pv
            .iter()
            .take(ZOOM_PV_MOVES)
            .map(String::as_str)
            .collect();
        parts.push(format!(
            "analysis {} d{} {}",
            score_text(best.score),
            best.depth,
            moves.join(" ")
        ));
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

/// The help screen, over everything: the part of [`crate::help`]'s text that fits, from
/// [`App::help_scroll`], with where it is and how to scroll on the bottom edge when it doesn't
/// all fit.
fn draw_help(frame: &mut Frame, area: Rect, app: &App) {
    let view = HelpView::new(area, app);
    let scroll = app.help_scroll.min(view.max_scroll());
    let shown: Vec<Line> = view
        .lines
        .iter()
        .skip(scroll)
        .take(view.rows)
        .map(|line| Line::from(line.as_str()))
        .collect();
    frame.render_widget(Clear, view.rect);
    let mut block = Block::default().borders(Borders::ALL).title(" Help ");
    if view.max_scroll() > 0 {
        let last = (scroll + view.rows).min(view.lines.len());
        block = block.title_bottom(format!(
            " {}–{last} of {} · ↑ ↓ PgUp PgDn scroll · Esc closes ",
            scroll + 1,
            view.lines.len()
        ));
    }
    frame.render_widget(Paragraph::new(shown).block(block), view.rect);
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
            max_multipv: 1,
            max_tree_depth: 24,
            max_tree_plies: 6,
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
    fn the_promotion_picker_shows_its_pieces_on_the_board() {
        let node = NodeSummary {
            name: "eng".into(),
            ..NodeSummary::default()
        };
        let settings = Settings {
            engine_plays: [false, false],
            fen: Some("8/4P3/8/8/8/8/8/K6k w - - 0 1".into()),
            ..Settings::default()
        };
        let mut app = App::new(node, settings).unwrap();
        app.selected = Some("e7".parse().unwrap());
        app.promotion = Some(crate::app::Promotion {
            to: "e8".parse().unwrap(),
            choices: rules::promotions(
                app.game.position(),
                "e7".parse().unwrap(),
                "e8".parse().unwrap(),
            ),
        });
        let text = screen(&app, 100, 32);
        for piece in ["WQ", "WR", "WB", "WN"] {
            assert!(text.contains(piece), "{piece}: {text}");
        }
        // Down the e-file: the queen's row is above the rook's.
        let row = |piece: &str| text.lines().position(|l| l.contains(piece)).unwrap();
        assert!(row("WQ") < row("WR") && row("WR") < row("WB") && row("WB") < row("WN"));
        assert!(text.contains("q queen"), "{text}");
    }

    #[test]
    fn clocks_stand_beside_the_players_and_in_the_zoomed_summary() {
        let node = NodeSummary {
            name: "eng".into(),
            ..NodeSummary::default()
        };
        let settings = Settings {
            clock: Some(crate::clock::TimeControl::parse("5+3").unwrap()),
            ..Settings::default()
        };
        let mut app = App::new(node, settings).unwrap();
        let text = screen(&app, 100, 32);
        assert!(text.contains("White: you 5:00   Black: eng 5:00"), "{text}");
        // This engine can't read a clock, so the panel says cbtui times it.
        assert!(text.contains("5+3 clock (cbtui times eng)"), "{text}");
        app.draw_offered = true;
        assert!(screen(&app, 100, 32).contains("draw offered"));
        app.zoomed = true;
        let text = screen(&app, 100, 32);
        assert!(text.contains("W 5:00 B 5:00"), "{text}");
    }

    #[test]
    fn an_analysis_shows_each_line_on_a_row_of_its_own() {
        use crate::analysis::{Analysis, AnalysisLine};
        use cnp::engine::Score;
        let mut app = app();
        let mut analysis = Analysis::new([false, true]);
        analysis.depth = 18;
        analysis.nodes = 2_500_000;
        for (cp, pv) in [(35, "e4 e5 Nf3"), (28, "d4 d5"), (-10, "f3 e5")] {
            analysis.lines.push(AnalysisLine {
                score: Score::cp(cp),
                depth: 18,
                pv: pv.split(' ').map(String::from).collect(),
            });
        }
        app.analysis = Some(analysis);
        let text = screen(&app, 100, 32);
        assert!(text.contains("Analysis · 1 line"), "{text}");
        assert!(text.contains("depth 18  2.5M nodes"), "{text}");
        for row in ["+0.35 d18 e4 e5 Nf3", "+0.28 d18 d4 d5", "-0.10 d18 f3 e5"] {
            assert!(text.contains(row), "{row}: {text}");
        }
        assert!(text.contains("i analyse"), "{text}");
    }

    #[test]
    fn the_evaluation_shows_each_term_and_the_difference_in_pawns() {
        let mut app = app();
        app.evaluation = Some(crate::app::Evaluation {
            label: "After 1. e4".into(),
            position: "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
                .parse::<rules::Position>()
                .unwrap(),
            result: serde_json::from_value(serde_json::json!({
                "score": {"cp": -33}, "searchScore": {"cp": -11}, "quiet": [],
                "terms": [
                    {"id": "material", "name": "Material", "white": 3940, "black": 3940},
                    {"id": "mobility", "name": "Mobility", "white": -89, "black": -117}
                ]
            }))
            .unwrap(),
        });
        app.overlay = Some(Overlay::Evaluation);
        let text = screen(&app, 100, 32);
        assert!(text.contains("Evaluation · After 1. e4"), "{text}");
        // Black to move scored -0.33: +0.33 for White.
        assert!(text.contains("Score +0.33"), "{text}");
        assert!(text.contains("The search evaluates it +0.11"), "{text}");
        assert!(
            text.contains("Material   +39.40   +39.40    +0.00"),
            "{text}"
        );
        assert!(
            text.contains("Mobility    -0.89    -1.17    +0.28"),
            "{text}"
        );
    }

    #[test]
    fn the_techniques_show_on_off_and_what_was_switched() {
        let mut app = app();
        app.techniques = Some(
            serde_json::from_value(serde_json::json!([
                {"id": "alphabeta", "name": "Alpha-beta pruning", "description": "Cuts branches.", "default": true},
                {"id": "probcut", "name": "ProbCut", "description": "Guesses cuts.", "default": false, "requires": ["alphabeta"]},
                {"id": "mobility", "name": "Evaluation", "description": "Evaluation: mobility", "default": true},
                {"id": "pawns", "name": "Evaluation", "description": "Evaluation: pawns", "default": true}
            ]))
            .unwrap(),
        );
        app.settings.techniques.insert("probcut".into(), true);
        app.overlay = Some(Overlay::Techniques { cursor: 1 });
        let text = screen(&app, 100, 32);
        assert!(text.contains("Techniques · 1 switched"), "{text}");
        assert!(text.contains("[x]  Alpha-beta pruning"), "{text}");
        assert!(text.contains("[x]* ProbCut"), "{text}");
        assert!(text.contains("Guesses cuts."), "{text}");
        assert!(
            text.contains("id probcut, off by default; needs alphabeta"),
            "{text}"
        );
        assert!(text.contains("1 technique switched"), "{text}");
        // Two called "Evaluation" show what tells them apart.
        assert!(text.contains("[x]  Evaluation: mobility"), "{text}");
        assert!(text.contains("[x]  Evaluation: pawns"), "{text}");
    }

    #[test]
    fn the_root_shows_the_move_searched_and_bars_of_effort() {
        let mut app = app();
        app.follow_root = true;
        let text = screen(&app, 100, 32);
        assert!(text.contains("Waiting for a search"), "{text}");
        let report: cnp::engine::RootParams = serde_json::from_value(serde_json::json!({
            "id": 2, "depth": 14, "move": "d2d4", "number": 2, "of": 20, "best": "e2e4",
            "moves": [{"move": "e2e4", "nodes": 750}, {"move": "d2d4", "nodes": 250}],
            "nodes": 372_437, "timeMs": 251,
            "research": {"kind": "better", "move": "d2d4"}
        }))
        .unwrap();
        app.root = Some(crate::root::RootView::update(
            &report,
            &rules::Position::startpos(),
            None,
        ));
        let text = screen(&app, 100, 32);
        assert!(
            text.contains("depth 14 · move 2 of 20: d4 · best e4"),
            "{text}"
        );
        let bar = |san: &str| {
            text.lines()
                .find(|l| l.contains(&format!("│{san} ")))
                .map(|l| l.matches('█').count())
                .unwrap_or(0)
        };
        assert!(bar("e4") > 2 * bar("d4") && bar("d4") > 0, "{text}");
        assert!(text.contains("75%") && text.contains("25%"), "{text}");
        assert!(text.contains("At 0.3s: d4 looked better"), "{text}");
    }

    #[test]
    fn the_tree_shows_the_node_its_moves_and_what_was_decided() {
        let mut app = app();
        let result: cnp::engine::TreeResult = serde_json::from_value(serde_json::json!({
            "tree": "t1", "expiresMs": 300000, "depth": 8, "nodes": 34636, "timeMs": 31,
            "bestMove": "f1b5", "score": {"cp": 36}, "pv": ["f1b5"],
            "recorded": {"plies": 3, "nodes": 342, "decisions": 1033},
            "root": {"depth": 8, "alpha": {"cp": 14}, "beta": {"cp": 64}, "score": {"cp": 36},
                     "childCount": 2, "subtree": 341},
            "children": [
                {"move": "d2d4", "depth": 7, "score": {"cp": -14, "bound": "lower"},
                 "childCount": 1, "subtree": 38,
                 "decisions": [{"kind": "cutoff", "move": "e5d4", "order": 1, "technique": "alphabeta"}]},
                {"move": "f1b5", "depth": 7, "score": {"cp": -36}, "childCount": 30, "subtree": 108}
            ]
        }))
        .unwrap();
        let position: rules::Position =
            "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3"
                .parse()
                .unwrap();
        app.tree = Some(crate::tree::TreeBrowser::new(result, position));
        let text = screen(&app, 110, 36);
        assert!(
            text.contains("Depth 8 · 34.6k nodes · 3 plies recorded"),
            "{text}"
        );
        assert!(text.contains("Best Bb5 +0.36"), "{text}");
        assert!(text.contains("window +0.14 to +0.64"), "{text}");
        assert!(text.contains("2 moves searched here"), "{text}");
        assert!(
            text.contains("d4        ≤ +0.14  d7  › refuted by exd4"),
            "{text}"
        );
        assert!(text.contains("Bb5         +0.36  d7  ›"), "{text}");
        assert!(
            text.contains("exd4 refuted the move that led here"),
            "{text}"
        );
        assert!(text.contains("Esc close the tree"), "{text}");
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
    fn a_short_terminal_scrolls_the_help_and_says_where_it_is() {
        let mut app = app();
        app.show_help = true;
        let all = crate::help::HelpView::new(Rect::new(0, 0, 100, 200), &app).lines;
        let text = screen(&app, 80, 24);
        assert!(text.contains("Engine"), "{text}");
        assert!(text.contains(&format!("1–22 of {}", all.len())), "{text}");
        assert!(text.contains("PgUp PgDn scroll"), "{text}");
        assert!(
            !text.contains("any other key closes"),
            "the end is below: {text}"
        );
        // Scrolled to the end, the last line shows and the first doesn't.
        app.screen = Rect::new(0, 0, 80, 24);
        app.scroll_help(isize::MAX);
        let text = screen(&app, 80, 24);
        assert!(text.contains("any other key closes"), "{text}");
        assert!(!text.contains("Engine"), "{text}");
        // A tall terminal shows it all, with no scroll note.
        let text = screen(&app, 100, 80);
        assert!(
            text.contains("Engine") && text.contains("any other key closes"),
            "{text}"
        );
        assert!(!text.contains("PgUp PgDn scroll"), "{text}");
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
