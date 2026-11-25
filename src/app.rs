//! The application: the game, whose move it is, what the engine is doing, and what keys,
//! clicks and engine messages do to them.
//!
//! [`App`] holds no terminal and no process. It reacts to input ([`App::handle_key`],
//! [`App::handle_click`]) and to the engine ([`App::handle_node`]), and asks the engine for
//! work through the [`Engine`] trait, so tests drive it with a fake engine and read its state.
//!
//! ## Engine-agnostic by design
//!
//! Everything the engine can do is read from its `initialize` answer ([`NodeSummary`]): levels
//! are offered only when it has the `strength` capability, live thinking only with `progress`.
//! A newer engine with more capabilities works unchanged, and its extra features can be
//! switched on here as cbtui learns them.
//!
//! ## Looking at other positions
//!
//! The board shows [`App::shown_position`], which is not always the game's. As a PGN viewer
//! (`--pgn`, [`App::review`]) it shows the viewer's place in the recorded game, the arrows
//! step through it, and the board is read-only until `:play` turns the position shown into a
//! game. In a game, `[` and `]` look back through its moves ([`App::browse`]); the engine
//! keeps playing meanwhile, and moves can't be made until the board is back at the end.
//!
//! ## Promotions
//!
//! A pawn clicked (or moved with the arrows) to the last rank doesn't promote at once: the
//! picker opens ([`App::promotion`]), offering a queen, rook, bishop and knight on the squares
//! from the promotion square towards the middle. A click on one, or `q` `r` `b` `n`, plays
//! it; Enter is a queen, and Esc or a click elsewhere puts the pawn back. While it's open it
//! takes every key, so `q` is a queen there, not quit. A typed move says its piece (`e8=N`).
//!
//! ## Clocks, resigning and draws
//!
//! With a time control (`--clock 5+3`, `:clock`), [`App::clocks`] runs from the game's first
//! move. Time reaches the app as an input, [`App::tick`], which the event loop calls with the
//! time before each batch of events; a clock that reaches zero ends the game there. An engine
//! with the `clock` capability is sent both clocks and decides its own time; one without it
//! gets a share of its time left ([`CLOCK_FALLBACK_MOVES`]). A takeback puts both clocks back
//! to where they were before the moves taken back.
//!
//! `:resign` resigns the person's side. `:draw` offers the engine a draw, which it answers
//! with its next move: it accepts when its search's score says it isn't better
//! ([`accepts_draw`]), from what it reported, never from cbtui's own judgement. A resignation
//! or an agreed draw can be taken back (`u`), and the game goes on.
//!
//! ## The engine's turn
//!
//! After every change, [`App`] checks whether the engine plays the side to move. If so, and no
//! search is running, it sends one with the whole game (start FEN and moves, so the engine sees
//! repetitions) and remembers the request id and the game's length. When the answer comes, it
//! is played only if both still match: a takeback or a new game in between makes it stale.

use std::fs;
use std::time::Instant;

use cnp::engine::{
    self as proto, EngineCapabilities, LevelsResult, Score, ScoreValue, SearchLimits, SearchParams,
    SideClock, Strength,
};
use cnp::{Response, code};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::clock::{Clocks, TimeControl};
use crate::node::{Engine, NodeEvent};
use crate::review::Review;
use crate::rules::{self, Color, Game, Move, Outcome, Piece, Position, Square};

/// How often a search reports progress, in milliseconds, when the engine can.
pub const PROGRESS_MS: u64 = 200;

/// How long the engine thinks about a move by default, in milliseconds.
pub const DEFAULT_MOVETIME_MS: u64 = 1000;

/// The longest the engine may be asked to think about one move, in milliseconds.
pub const MAX_MOVETIME_MS: u64 = 600_000;

/// What cbtui says when asked to use an engine it doesn't have.
const NO_ENGINE: &str =
    "There's no engine. Start cbtui with --engine <path> (or $CBTUI_ENGINE) to play one.";

/// What the status line says while the promotion picker is open.
const PROMOTE: &str =
    "Promote to: click a piece, or press q, r, b or n (Enter: queen, Esc: put the pawn back).";

/// An engine that can't read a clock gets this share of its time left for each move: as if
/// this many moves were still to play.
pub const CLOCK_FALLBACK_MOVES: u64 = 30;

/// What an engine without a clock keeps in hand on every move, in milliseconds, so the time
/// it is given never quite uses up its clock.
pub const CLOCK_SAFETY_MS: u64 = 50;

/// The engine accepts a draw offer when its own score is at most this, in centipawns: when
/// it doesn't think it's better.
pub const DRAW_ACCEPT_CP: i32 = 10;

/// How many lines of the engine's stderr are kept.
const STDERR_LINES: usize = 50;

/// What the engine said about itself, kept for the screen and for deciding what to offer.
#[derive(Clone, Debug, Default)]
pub struct NodeSummary {
    /// Its name, like `cb2026`.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Its licence (an SPDX expression).
    pub license: String,
    /// What it can do. All `false` if it isn't an engine node.
    pub caps: EngineCapabilities,
}

impl NodeSummary {
    /// The summary when there is no engine (a PGN viewer can do without one).
    pub fn absent() -> NodeSummary {
        NodeSummary {
            name: "no engine".into(),
            ..NodeSummary::default()
        }
    }

    /// The summary of an `initialize` answer.
    pub fn from_info(info: &cnp::lifecycle::InitializeResult) -> NodeSummary {
        NodeSummary {
            name: info.node.name.clone(),
            version: info.node.version.clone(),
            license: info.node.license.clone(),
            caps: info.capabilities.engine.clone().unwrap_or_default(),
        }
    }
}

/// The choices a session starts with (from the command line).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Which sides the engine plays, indexed by [`Color::index`].
    pub engine_plays: [bool; 2],
    /// How long the engine thinks about each move, in milliseconds.
    pub movetime_ms: u64,
    /// The engine's playing level, if it has levels; `None` is full strength.
    pub level: Option<f64>,
    /// The position to start from, if not the usual one.
    pub fen: Option<String>,
    /// The time control, if games are played to a clock.
    pub clock: Option<TimeControl>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            engine_plays: [false, true],
            movetime_ms: DEFAULT_MOVETIME_MS,
            level: None,
            fen: None,
            clock: None,
        }
    }
}

/// The engine's latest report on its current or last search.
#[derive(Clone, Debug, PartialEq)]
pub struct Thinking {
    /// Depth reached, and the deepest line.
    pub depth: u32,
    /// The deepest ply any line reached.
    pub sel_depth: u32,
    /// The score, from White's point of view.
    pub score: Score,
    /// Positions searched.
    pub nodes: u64,
    /// Positions per second, if the engine said.
    pub nps: Option<u64>,
    /// Time spent, in milliseconds.
    pub time_ms: u64,
    /// The best line, in SAN.
    pub pv: Vec<String>,
}

/// A pawn's move to the last rank, waiting for the person to choose its piece.
#[derive(Clone, Debug, PartialEq)]
pub struct Promotion {
    /// Where the pawn promotes.
    pub to: Square,
    /// The legal promotions, queen first ([`rules::PROMOTION_PIECES`]).
    pub choices: Vec<Move>,
}

/// A search the app is waiting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    /// The `engine/search` request's id.
    id: i64,
    /// How many moves the game had when it was sent.
    ply: usize,
}

/// The whole application state.
#[derive(Debug)]
pub struct App {
    /// The game being played.
    pub game: Game,
    /// The session's settings (which sides the engine plays, its time and level).
    pub settings: Settings,
    /// The engine.
    pub node: NodeSummary,
    /// The engine's levels, once it has said them.
    pub levels: Option<LevelsResult>,
    /// Whether Black is at the bottom of the board.
    pub flipped: bool,
    /// Whether the board fills the screen, without the side panels (the `z` key).
    pub zoomed: bool,
    /// The recorded game being viewed, when cbtui is a PGN viewer (`--pgn`).
    pub review: Option<Review>,
    /// In a game, how many of its moves the board shows while looking back (`[`); `None`
    /// shows the game as it stands.
    pub browse: Option<usize>,
    /// Whether there is an engine at all: a PGN viewer starts without one if none is found.
    pub has_engine: bool,
    /// The game's clocks, when it is played to a time control.
    pub clocks: Option<Clocks>,
    /// The time, as the event loop last said ([`App::tick`]).
    pub now: Instant,
    /// Whether the person has offered the engine a draw it hasn't answered yet.
    pub draw_offered: bool,
    /// The keyboard cursor.
    pub cursor: Square,
    /// Whether to show the cursor (after the arrow keys are used, not after a click).
    pub cursor_visible: bool,
    /// The square whose piece is picked up.
    pub selected: Option<Square>,
    /// A promotion waiting for its piece to be chosen.
    pub promotion: Option<Promotion>,
    /// The line being typed: a move, or a command after `:`.
    pub input: Option<String>,
    /// The message under the board.
    pub status: String,
    /// The engine's latest report.
    pub thinking: Option<Thinking>,
    /// Whether the help is showing.
    pub show_help: bool,
    /// The engine's recent stderr lines, oldest first.
    pub stderr: Vec<String>,
    /// Whether the engine has gone.
    pub disconnected: bool,
    /// Set when the user asks to leave.
    pub quit: bool,
    pending: Option<Pending>,
    /// The `engine/levels` request's id, until it is answered.
    levels_request: Option<i64>,
    /// Counts games, to tell the engine when a new one starts (the `game` hint).
    game_number: u32,
    /// With clocks: both sides' time before each move of the game, to put back on a takeback.
    clock_history: Vec<[u64; 2]>,
}

impl App {
    /// A new session with `node` and `settings`. Fails if the starting FEN can't be read.
    pub fn new(node: NodeSummary, settings: Settings) -> Result<App, String> {
        let game = match &settings.fen {
            Some(fen) => Game::from_fen(fen).map_err(|e| format!("bad FEN: {e:?}"))?,
            None => Game::new(),
        };
        let status = format!(
            "Connected to {} {}. Press ? for help.",
            node.name, node.version
        );
        let clocks = settings.clock.map(Clocks::new);
        Ok(App {
            game,
            settings,
            node,
            levels: None,
            flipped: false,
            zoomed: false,
            review: None,
            browse: None,
            has_engine: true,
            clocks,
            now: Instant::now(),
            draw_offered: false,
            cursor: rules::square(4, 1).expect("e2 is on the board"),
            cursor_visible: false,
            selected: None,
            promotion: None,
            input: None,
            status,
            thinking: None,
            show_help: false,
            stderr: Vec::new(),
            disconnected: false,
            quit: false,
            pending: None,
            levels_request: None,
            game_number: 1,
            clock_history: Vec::new(),
        })
    }

    /// Views `review` as a PGN viewer, from its starting position. The board is read-only and
    /// the engine plays neither side, until `:play` plays on from the position shown.
    pub fn open_review(&mut self, review: Review) {
        self.status = format!(
            "Viewing {}: ← → to step through it, ? for help.",
            review.name
        );
        // Not shown while viewing; `:play` replaces it with the game to the position shown.
        self.game = Game::from_position(review.start().clone());
        self.settings.engine_plays = [false, false];
        self.review = Some(review);
        self.browse = None;
        self.selected = None;
        self.thinking = None;
        // The viewer has no clocks; `:play` starts them, if there's a time control.
        self.clocks = None;
        self.clock_history.clear();
    }

    /// Tells the app the time. A clock that reaches zero ends the game on time.
    pub fn tick(&mut self, now: Instant, engine: &mut dyn Engine) {
        self.now = now;
        let Some(side) = self.clocks.as_ref().and_then(|c| c.flagged(now)) else {
            return;
        };
        if self.game.outcome().is_none() {
            self.game.time_out(side);
        }
        self.game_ended(engine);
    }

    /// Whether a clock is running, so the screen should be redrawn as it counts down.
    pub fn clock_running(&self) -> bool {
        self.clocks.as_ref().is_some_and(|c| c.running().is_some())
    }

    /// New clocks for the game as it stands, from the time control: both at the starting
    /// time, the side to move's running if the game has begun.
    fn reset_clocks(&mut self) {
        self.clocks = self.settings.clock.map(Clocks::new);
        self.clock_history = match self.settings.clock {
            Some(tc) => vec![[tc.base_ms; 2]; self.game.moves().len()],
            None => Vec::new(),
        };
        self.resume_clock();
    }

    /// Runs the side to move's clock if the game has begun and isn't over, else neither.
    fn resume_clock(&mut self) {
        let now = self.now;
        let side = self.game.position().side_to_move();
        let running = !self.game.moves().is_empty() && self.game.outcome().is_none();
        if let Some(clocks) = &mut self.clocks {
            if running {
                clocks.start(side, now);
            } else {
                clocks.stop(now);
            }
        }
    }

    /// Plays `mv` (with a comment for PGN) and presses the clock. False if it can't be played.
    fn play(&mut self, mv: Move, comment: Option<String>) -> bool {
        let mover = self.game.position().side_to_move();
        let before = self.clocks.as_ref().map(|c| c.times(self.now));
        if self.game.play_with_comment(mv, comment).is_err() {
            return false;
        }
        if let (Some(clocks), Some(before)) = (&mut self.clocks, before) {
            self.clock_history.push(before);
            if self.game.outcome().is_some() {
                clocks.stop(self.now);
            } else {
                clocks.press(mover, self.now);
            }
        }
        true
    }

    /// Takes back one move, and puts the clocks back to where they were before it.
    fn take_back(&mut self) -> bool {
        if self.game.undo().is_none() {
            return false;
        }
        if let (Some(clocks), Some(times)) = (&mut self.clocks, self.clock_history.pop()) {
            clocks.set_times(times);
        }
        true
    }

    /// After a game ends off the board (a resignation, a draw, a flag): the search stops,
    /// the clocks stop, and the status says how it ended.
    fn game_ended(&mut self, engine: &mut dyn Engine) {
        self.cancel_search(engine);
        if let Some(clocks) = &mut self.clocks {
            clocks.stop(self.now);
        }
        self.promotion = None;
        self.selected = None;
        self.draw_offered = false;
        if let Some(outcome) = self.game.outcome() {
            self.status = rules::describe_outcome(outcome);
        }
    }

    /// The position on the board: the viewer's, the one looked back at, or the game's.
    pub fn shown_position(&self) -> Position {
        if let Some(review) = &self.review {
            return review.current().position.clone();
        }
        match self.browse {
            Some(ply) => {
                let mut position = self.game.start().clone();
                for played in self.game.moves().iter().take(ply) {
                    position.make_move(played.mv);
                }
                position
            }
            None => self.game.position().clone(),
        }
    }

    /// The move that led to the position on the board, if any.
    pub fn shown_last_move(&self) -> Option<Move> {
        if let Some(review) = &self.review {
            return review.current().mv;
        }
        match self.browse {
            Some(ply) => ply
                .checked_sub(1)
                .and_then(|i| self.game.moves().get(i))
                .map(|played| played.mv),
            None => self.game.last_move().map(|played| played.mv),
        }
    }

    /// The move that led to the position on the board, numbered: `12. Nf3` or `12… Nc6`.
    pub fn shown_move_label(&self) -> Option<String> {
        if let Some(review) = &self.review {
            return (review.cursor() != 0).then(|| review.label(review.cursor(), true));
        }
        let ply = self.browse.unwrap_or(self.game.moves().len());
        let played = self.game.moves().get(ply.checked_sub(1)?)?;
        let start = self.game.start();
        // Plies counted from White's move of the starting position's move number.
        let k = ply - 1 + usize::from(start.side_to_move() == Color::Black);
        let number = start.fullmove_number() as usize + k / 2;
        Some(if k.is_multiple_of(2) {
            format!("{number}. {}", played.san)
        } else {
            format!("{number}… {}", played.san)
        })
    }

    /// Why the board can't be played on now, if it can't: a PGN viewer, or looking back.
    fn read_only(&self) -> Option<String> {
        if let Some(review) = &self.review {
            return Some(format!(
                "{} is read-only here. :play white|black|both|none plays on from this position.",
                review.name
            ));
        }
        self.browse
            .map(|_| "You're looking back. Press ] or End to return to the game.".into())
    }

    /// Starts the session: if the engine plays the side to move (`--play white`), it starts
    /// thinking at once.
    pub fn start(&mut self, engine: &mut dyn Engine) {
        self.maybe_start_engine(engine);
    }

    /// Records that `engine/levels` was sent with this id.
    pub fn expect_levels(&mut self, id: i64) {
        self.levels_request = Some(id);
    }

    /// Whether a search is running.
    pub fn searching(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the engine plays the side to move.
    pub fn engine_to_move(&self) -> bool {
        self.settings.engine_plays[self.game.position().side_to_move().index()]
    }

    /// The legal destinations of the picked-up piece.
    pub fn targets(&self) -> Vec<Square> {
        match self.selected {
            Some(from) => rules::moves_from(self.game.position(), from)
                .iter()
                .map(|mv| rules::destination(*mv))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The promotion picker's squares and what each offers: the promotion square, then the
    /// squares behind it towards the middle of the board, queen first.
    pub fn promotion_squares(&self) -> Vec<(Square, Piece)> {
        let Some(promotion) = &self.promotion else {
            return Vec::new();
        };
        let color = self.game.position().side_to_move();
        let file = promotion.to.file().index();
        let rank = promotion.to.rank().index();
        promotion
            .choices
            .iter()
            .enumerate()
            .filter_map(|(i, mv)| {
                let rank = match color {
                    Color::White => rank.checked_sub(i)?,
                    Color::Black => rank + i,
                };
                let square = rules::square(file, rank)?;
                Some((square, Piece::new(color, mv.promotion()?)))
            })
            .collect()
    }

    /// A short description of a level, like `level 10 (1600)`.
    pub fn level_text(&self) -> Option<String> {
        let level = self.settings.level?;
        let rating = self.levels.as_ref().and_then(|levels| {
            levels
                .levels
                .iter()
                .find(|l| f64::from(l.level) == level)
                .map(|l| l.rating)
        });
        Some(match rating {
            Some(rating) => format!("level {level} ({rating})"),
            None => format!("level {level}"),
        })
    }

    // ---- input -------------------------------------------------------------------------

    /// Reacts to a key.
    pub fn handle_key(&mut self, key: KeyEvent, engine: &mut dyn Engine) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.input.is_some() {
            self.handle_input_key(key, engine);
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.promotion.is_some() {
            self.promotion_key(key.code, engine);
            return;
        }
        let stepped = if self.review.is_some() {
            self.review_key(key.code)
        } else {
            self.browse_key(key.code)
        };
        if stepped {
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char(':') => self.input = Some(":".into()),
            KeyCode::Char('n') => self.new_game(None, engine),
            KeyCode::Char('u') => self.undo(engine),
            KeyCode::Char('t') => self.flipped = !self.flipped,
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            // `m`: the engine moves (now, if it is thinking already).
            KeyCode::Char('m') if self.searching() => self.move_now(engine),
            KeyCode::Char('m') => self.go(engine),
            // The first letter of a move in SAN or coordinates starts typing it.
            KeyCode::Char(c @ ('a'..='h' | 'N' | 'B' | 'R' | 'Q' | 'K' | 'O' | '0')) => {
                self.input = Some(c.to_string());
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                self.move_cursor(key.code);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.cursor_visible = true;
                self.handle_square(self.cursor, engine);
            }
            KeyCode::Esc => self.selected = None,
            _ => {}
        }
    }

    /// The promotion picker's keys: a piece's letter (`q`, `r`, `b`, `n`), Enter or Space for a
    /// queen, Esc to put the pawn back. Other keys do nothing while it's open.
    fn promotion_key(&mut self, code: KeyCode, engine: &mut dyn Engine) {
        let kind = match code {
            KeyCode::Char('q') | KeyCode::Enter | KeyCode::Char(' ') => rules::PieceType::Queen,
            KeyCode::Char('r') => rules::PieceType::Rook,
            KeyCode::Char('b') => rules::PieceType::Bishop,
            KeyCode::Char('n') => rules::PieceType::Knight,
            KeyCode::Esc => {
                self.promotion = None;
                self.selected = None;
                self.status = "Promotion cancelled.".into();
                return;
            }
            _ => return,
        };
        self.promote(kind, engine);
    }

    /// Plays the waiting promotion to `kind`, if it's one of the choices.
    fn promote(&mut self, kind: rules::PieceType, engine: &mut dyn Engine) {
        let Some(promotion) = self.promotion.take() else {
            return;
        };
        match promotion
            .choices
            .iter()
            .find(|mv| mv.promotion() == Some(kind))
        {
            Some(&mv) => {
                self.selected = None;
                self.human_move(mv, engine);
            }
            None => self.promotion = Some(promotion),
        }
    }

    /// The PGN viewer's keys: the arrows (and `[`, `]`, Home, End) step through the game, and
    /// a digit enters a variation. True if the key was one of them.
    fn review_key(&mut self, code: KeyCode) -> bool {
        let Some(review) = &mut self.review else {
            return false;
        };
        match code {
            KeyCode::Left | KeyCode::Char('[') => {
                review.back();
            }
            KeyCode::Right | KeyCode::Char(']') => {
                review.forward();
            }
            KeyCode::Up | KeyCode::Home => review.to_start(),
            KeyCode::Down | KeyCode::End => review.to_end(),
            KeyCode::Char(c @ '1'..='9') => {
                let n = c as usize - '0' as usize;
                if !review.enter(n) {
                    self.status = format!("There's no variation {n} at this move.");
                }
            }
            _ => return false,
        }
        true
    }

    /// Looking back through a game: `[` and `]` step, Home and End jump to its start and back
    /// to where it stands. True if the key was one of them.
    fn browse_key(&mut self, code: KeyCode) -> bool {
        let length = self.game.moves().len();
        let shown = self.browse.unwrap_or(length).min(length);
        let to = match code {
            KeyCode::Char('[') => shown.saturating_sub(1),
            KeyCode::Char(']') => (shown + 1).min(length),
            KeyCode::Home => 0,
            KeyCode::End => length,
            _ => return false,
        };
        self.browse = (to < length).then_some(to);
        self.selected = None;
        true
    }

    fn handle_input_key(&mut self, key: KeyEvent, engine: &mut dyn Engine) {
        let Some(input) = &mut self.input else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.input = None,
            KeyCode::Backspace => {
                input.pop();
                if input.is_empty() {
                    self.input = None;
                }
            }
            KeyCode::Enter => {
                let line = self.input.take().unwrap_or_default();
                match line.strip_prefix(':') {
                    Some(command) => self.command(command, engine),
                    None => self.typed_move(line.trim(), engine),
                }
            }
            KeyCode::Char(c) => input.push(c),
            _ => {}
        }
    }

    fn move_cursor(&mut self, code: KeyCode) {
        self.cursor_visible = true;
        let (mut file, mut rank) = (
            self.cursor.file().index() as i32,
            self.cursor.rank().index() as i32,
        );
        // On the screen, "up" is towards Black's side unless the board is flipped.
        let sign = if self.flipped { -1 } else { 1 };
        match code {
            KeyCode::Left => file -= sign,
            KeyCode::Right => file += sign,
            KeyCode::Up => rank += sign,
            KeyCode::Down => rank -= sign,
            _ => {}
        }
        let clamp = |v: i32| v.clamp(0, 7) as usize;
        self.cursor = rules::square(clamp(file), clamp(rank)).expect("clamped onto the board");
    }

    /// Reacts to a click on `square`.
    pub fn handle_click(&mut self, square: Square, engine: &mut dyn Engine) {
        self.cursor = square;
        self.cursor_visible = false;
        self.handle_square(square, engine);
    }

    /// Picks up a piece on `square`, or puts the picked-up one down there.
    fn handle_square(&mut self, square: Square, engine: &mut dyn Engine) {
        if let Some(reason) = self.read_only() {
            self.status = reason;
            return;
        }
        if self.promotion.is_some() {
            // A click on the picker chooses; anywhere else puts the pawn back.
            let chosen = self
                .promotion_squares()
                .into_iter()
                .find(|(at, _)| *at == square);
            match chosen {
                Some((_, piece)) => self.promote(piece.piece_type, engine),
                None => {
                    self.promotion = None;
                    self.selected = None;
                    self.status = "Promotion cancelled.".into();
                }
            }
            return;
        }
        let position = self.game.position().clone();
        if let Some(from) = self.selected {
            let choices = rules::promotions(&position, from, square);
            if choices.len() > 1 {
                self.promotion = Some(Promotion {
                    to: square,
                    choices,
                });
                self.status = PROMOTE.into();
                return;
            }
        }
        if let Some(from) = self.selected
            && let Some(mv) = rules::move_between(&position, from, square)
        {
            self.selected = None;
            self.human_move(mv, engine);
            return;
        }
        let own_piece = position
            .piece_at(square)
            .is_some_and(|piece| piece.color == position.side_to_move());
        if own_piece && self.selected != Some(square) {
            self.selected = Some(square);
        } else {
            self.selected = None;
        }
    }

    fn typed_move(&mut self, text: &str, engine: &mut dyn Engine) {
        if text.is_empty() {
            return;
        }
        if let Some(reason) = self.read_only() {
            self.status = reason;
            return;
        }
        match self.game.position().parse_move(text) {
            Ok(mv) => self.human_move(mv, engine),
            Err(e) => self.status = format!("Can't play `{text}`: {e}"),
        }
    }

    /// Plays a person's move, if it's their turn.
    fn human_move(&mut self, mv: rules::Move, engine: &mut dyn Engine) {
        if let Some(reason) = self.read_only() {
            self.status = reason;
            return;
        }
        if let Some(outcome) = self.game.outcome() {
            self.status = format!("The game is over. {}", rules::describe_outcome(outcome));
            return;
        }
        if self.engine_to_move() {
            self.status = format!(
                "It's {}'s move. Press m to make it move now, or :play none to take over.",
                self.node.name
            );
            return;
        }
        if self.searching() {
            // A search started with `g` for this side: the person moved instead.
            self.cancel_search(engine);
        }
        let san = self.game.position().to_san(mv);
        if self.play(mv, None) {
            self.selected = None;
            self.status = format!("You played {san}.");
            self.after_move(engine);
        }
    }

    // ---- commands ----------------------------------------------------------------------

    /// Runs a `:` command.
    pub fn command(&mut self, line: &str, engine: &mut dyn Engine) {
        let mut words = line.split_whitespace();
        let name = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        match (name, rest.as_slice()) {
            ("" | "help" | "h", _) => self.show_help = true,
            ("q" | "quit" | "exit", _) => self.quit = true,
            ("new", _) => self.new_game(None, engine),
            ("fen", fen) if !fen.is_empty() => self.new_game(Some(fen.join(" ")), engine),
            ("flip", _) => self.flipped = !self.flipped,
            ("zoom", _) => self.zoomed = !self.zoomed,
            ("undo", _) => self.undo(engine),
            ("go", _) => self.go(engine),
            ("stop", _) => self.move_now(engine),
            ("play", [side]) => self.set_play(side, engine),
            ("level", [level]) => self.set_level(level),
            ("level", []) => {
                self.status = match self.level_text() {
                    Some(text) => format!("Playing at {text}."),
                    None => "Playing at full strength.".into(),
                }
            }
            ("time" | "movetime", [ms]) => match ms.parse::<u64>() {
                Ok(ms) if (1..=MAX_MOVETIME_MS).contains(&ms) => {
                    self.settings.movetime_ms = ms;
                    self.status = format!("The engine now thinks {ms} ms a move.");
                }
                _ => self.status = format!("Usage: :time <1-{MAX_MOVETIME_MS}> (milliseconds)"),
            },
            ("resign", _) => self.resign(engine),
            ("draw", _) => self.offer_draw(engine),
            ("clock", [text]) => self.set_clock(text),
            ("clock", []) => {
                self.status = match self.settings.clock {
                    Some(tc) => format!("Playing to a {tc} clock. :clock off for none."),
                    None => format!(
                        "No clock: the engine thinks {} ms a move. :clock 5+3 sets one.",
                        self.settings.movetime_ms
                    ),
                }
            }
            ("pgn", [_]) if self.review.is_some() => {
                self.status = "This game is already a PGN file.".into();
            }
            ("pgn", [path]) => self.save_pgn(path),
            _ => self.status = format!("Unknown command `:{line}`. Press ? for help."),
        }
    }

    /// Sets which sides the engine plays. In the PGN viewer, this also turns the position
    /// shown into a game, the moves to it included, and leaves the viewer.
    fn set_play(&mut self, side: &str, engine: &mut dyn Engine) {
        let sides = match side {
            "white" | "w" => [true, false],
            "black" | "b" => [false, true],
            "both" => [true, true],
            "none" => [false, false],
            _ => {
                self.status = "Usage: :play white|black|both|none".into();
                return;
            }
        };
        if sides.contains(&true) && !self.has_engine {
            self.status = NO_ENGINE.into();
            return;
        }
        let mut status = match sides {
            [false, false] => "You play both sides.".to_string(),
            _ => format!("{} plays {side}.", self.node.name),
        };
        if let Some(review) = self.review.take() {
            self.cancel_search(engine);
            self.game = review.game_here();
            self.game_number += 1;
            self.selected = None;
            self.thinking = None;
            self.reset_clocks();
            status = format!("Playing on from {}. {status}", review.name);
        }
        self.browse = None;
        self.settings.engine_plays = sides;
        self.status = status;
        self.maybe_start_engine(engine);
    }

    /// Sets the time control (`5+3`), or takes it away (`off`). The game's clocks start
    /// again from it.
    fn set_clock(&mut self, text: &str) {
        if matches!(text, "off" | "none") {
            self.settings.clock = None;
            self.clocks = None;
            self.clock_history.clear();
            self.status = format!(
                "No clock now: the engine thinks {} ms a move.",
                self.settings.movetime_ms
            );
            return;
        }
        let tc = match TimeControl::parse(text) {
            Ok(tc) => tc,
            Err(e) => {
                self.status = format!("Usage: :clock <minutes>+<seconds>|off ({e})");
                return;
            }
        };
        self.settings.clock = Some(tc);
        if self.review.is_none() {
            self.reset_clocks();
        }
        self.status = format!(
            "Playing to a {tc} clock: each side has {}, plus {} s a move.",
            crate::clock::clock_text(tc.base_ms),
            tc.increment_ms as f64 / 1000.0
        );
        if self.has_engine && !self.node.caps.clock {
            self.status.push_str(&format!(
                " {} can't read a clock, so cbtui gives it a share of its time each move.",
                self.node.name
            ));
        }
    }

    /// The person resigns: their side if the engine plays the other, else the side to move.
    fn resign(&mut self, engine: &mut dyn Engine) {
        if let Some(reason) = self.review.is_some().then(|| self.read_only()).flatten() {
            self.status = reason;
            return;
        }
        if let Some(outcome) = self.game.outcome() {
            self.status = format!("The game is over. {}", rules::describe_outcome(outcome));
            return;
        }
        let humans: Vec<Color> = [Color::White, Color::Black]
            .into_iter()
            .filter(|c| !self.settings.engine_plays[c.index()])
            .collect();
        let loser = match humans.as_slice() {
            [one] => *one,
            _ => self.game.position().side_to_move(),
        };
        self.game.resign(loser);
        self.game_ended(engine);
    }

    /// Offers a draw. Against the engine, it answers with its next move ([`accepts_draw`]);
    /// with no engine side, or two, the draw is agreed at once.
    fn offer_draw(&mut self, engine: &mut dyn Engine) {
        if let Some(reason) = self.review.is_some().then(|| self.read_only()).flatten() {
            self.status = reason;
            return;
        }
        if let Some(outcome) = self.game.outcome() {
            self.status = format!("The game is over. {}", rules::describe_outcome(outcome));
            return;
        }
        let engine_sides = self.settings.engine_plays.iter().filter(|&&p| p).count();
        if engine_sides == 1 {
            self.draw_offered = true;
            self.status = format!(
                "You offer a draw. {} answers with its next move.",
                self.node.name
            );
            return;
        }
        self.game.agree_draw();
        self.game_ended(engine);
    }

    fn set_level(&mut self, text: &str) {
        if !self.node.caps.strength {
            self.status = format!("{} has no playing levels.", self.node.name);
            return;
        }
        if matches!(text, "max" | "full" | "off") {
            self.settings.level = None;
            self.status = "Playing at full strength.".into();
            return;
        }
        let Ok(level) = text.parse::<f64>() else {
            self.status = "Usage: :level <n>|max".into();
            return;
        };
        if let Some(levels) = &self.levels {
            let min = levels.levels.first().map_or(0, |l| l.level);
            let max = levels.levels.last().map_or(0, |l| l.level);
            let whole = level.fract() == 0.0;
            if level < f64::from(min) || level > f64::from(max) || !(whole || levels.fractional) {
                self.status = format!("Levels go from {min} to {max}.");
                return;
            }
        }
        self.settings.level = Some(level);
        self.status = format!(
            "Playing at {}.",
            self.level_text()
                .unwrap_or_else(|| format!("level {level}"))
        );
    }

    fn save_pgn(&mut self, path: &str) {
        let mut pgn = self.game.to_pgn();
        let engine = format!("{} {}", self.node.name, self.node.version);
        let name = |color: Color| {
            if self.settings.engine_plays[color.index()] {
                engine.clone()
            } else {
                "Human".to_string()
            }
        };
        pgn.set_tag("Event", "cbtui game");
        pgn.set_tag("White", &name(Color::White));
        pgn.set_tag("Black", &name(Color::Black));
        if let Some(tc) = self.settings.clock {
            pgn.set_tag("TimeControl", &tc.pgn_tag());
        }
        self.status = match fs::write(path, pgn.to_pgn()) {
            Ok(()) => format!("Saved the game to {path}."),
            Err(e) => format!("Couldn't save {path}: {e}"),
        };
    }

    fn new_game(&mut self, fen: Option<String>, engine: &mut dyn Engine) {
        let game = match &fen {
            Some(fen) => match Game::from_fen(fen) {
                Ok(game) => game,
                Err(e) => {
                    self.status = format!("Bad FEN: {e:?}");
                    return;
                }
            },
            None => Game::new(),
        };
        self.cancel_search(engine);
        self.game = game;
        self.review = None;
        self.browse = None;
        self.game_number += 1;
        self.selected = None;
        self.promotion = None;
        self.draw_offered = false;
        self.thinking = None;
        self.reset_clocks();
        self.status = "New game.".into();
        self.maybe_start_engine(engine);
    }

    /// Takes back the last move, and the engine's reply before it, so it's the person's turn.
    fn undo(&mut self, engine: &mut dyn Engine) {
        if let Some(reason) = self.review.is_some().then(|| self.read_only()).flatten() {
            self.status = reason;
            return;
        }
        self.browse = None;
        self.cancel_search(engine);
        self.draw_offered = false;
        self.selected = None;
        self.promotion = None;
        // A resignation or an agreed draw is taken back on its own: the moves stay.
        if matches!(
            self.game.outcome(),
            Some(Outcome::Resignation { .. } | Outcome::DrawAgreed)
        ) {
            if let Some(played) = self.game.undo() {
                let _ = self.game.play_with_comment(played.mv, played.comment);
            }
            self.resume_clock();
            self.status = "Took that back: the game goes on.".into();
            self.maybe_start_engine(engine);
            return;
        }
        if !self.take_back() {
            self.status = "No move to take back.".into();
            return;
        }
        let one_engine = self.settings.engine_plays.iter().filter(|&&p| p).count() == 1;
        if one_engine && self.engine_to_move() {
            self.take_back();
        }
        self.resume_clock();
        self.thinking = None;
        self.status = "Took back.".into();
        self.maybe_start_engine(engine);
    }

    /// Makes the engine play the side to move now, whoever plays it.
    fn go(&mut self, engine: &mut dyn Engine) {
        if let Some(reason) = self.review.is_some().then(|| self.read_only()).flatten() {
            self.status = reason;
            return;
        }
        self.browse = None;
        if self.searching() {
            self.status = format!("{} is already thinking.", self.node.name);
            return;
        }
        self.start_search(engine);
    }

    /// Asks a running search to stop; its best move so far is then played.
    fn move_now(&mut self, engine: &mut dyn Engine) {
        match self.pending {
            Some(pending) => {
                if let Err(e) = engine.stop(pending.id) {
                    self.status = e;
                }
            }
            None => self.status = format!("{} isn't thinking.", self.node.name),
        }
    }

    // ---- the engine --------------------------------------------------------------------

    fn after_move(&mut self, engine: &mut dyn Engine) {
        if let Some(outcome) = self.game.outcome() {
            self.status = rules::describe_outcome(outcome);
            return;
        }
        self.maybe_start_engine(engine);
    }

    /// Starts a search if the engine plays the side to move and isn't searching already.
    fn maybe_start_engine(&mut self, engine: &mut dyn Engine) {
        if self.engine_to_move() && !self.searching() && self.game.outcome().is_none() {
            self.start_search(engine);
        }
    }

    fn start_search(&mut self, engine: &mut dyn Engine) {
        if !self.has_engine {
            self.status = NO_ENGINE.into();
            return;
        }
        if self.disconnected {
            self.status = "The engine has gone. Restart cbtui.".into();
            return;
        }
        if let Some(outcome) = self.game.outcome() {
            self.status = format!("The game is over. {}", rules::describe_outcome(outcome));
            return;
        }
        match engine.search(&self.search_params()) {
            Ok(id) => {
                self.pending = Some(Pending {
                    id,
                    ply: self.game.moves().len(),
                });
                self.thinking = None;
            }
            Err(e) => self.status = e,
        }
    }

    /// The search for the current position: the whole game, the time per move, the level
    /// and progress reports if the engine can do them.
    pub fn search_params(&self) -> SearchParams {
        let position = proto::Position::fen(self.game.start().to_fen())
            .with_moves(rules::game_moves_uci(&self.game));
        let mut limits = SearchLimits::default();
        let tc_side = |clocks: &Clocks, color: Color| SideClock {
            time_ms: clocks.left(color, self.now),
            increment_ms: clocks.control().increment_ms,
        };
        match &self.clocks {
            Some(clocks) if self.node.caps.clock => {
                limits.clock = Some(proto::Clock {
                    white: tc_side(clocks, Color::White),
                    black: tc_side(clocks, Color::Black),
                    moves_to_go: None,
                });
            }
            // An engine that can't read a clock gets a share of what's left, and the increment.
            Some(clocks) => {
                let left = clocks.left(self.game.position().side_to_move(), self.now);
                let share = left / CLOCK_FALLBACK_MOVES + clocks.control().increment_ms;
                limits.time_ms = Some(share.min(left.saturating_sub(CLOCK_SAFETY_MS)).max(1));
            }
            None => limits.time_ms = Some(self.settings.movetime_ms),
        }
        let mut params = SearchParams::new(position).limits(limits);
        if self.node.caps.strength
            && let Some(level) = self.settings.level
        {
            params.strength = Some(Strength {
                level: Some(level),
                elo: None,
            });
        }
        if self.node.caps.progress {
            params.progress_ms = Some(PROGRESS_MS);
        }
        // Lets the engine keep what it learned (its hash table) from move to move.
        params.game = Some(format!("cbtui-{}", self.game_number));
        params
    }

    /// Stops the running search, if any, and forgets it: its answer will be ignored.
    fn cancel_search(&mut self, engine: &mut dyn Engine) {
        if let Some(pending) = self.pending.take() {
            let _ = engine.stop(pending.id);
        }
    }

    /// Reacts to something from the engine.
    pub fn handle_node(&mut self, event: NodeEvent, engine: &mut dyn Engine) {
        match event {
            NodeEvent::Progress(progress) => {
                let current = self
                    .pending
                    .is_some_and(|pending| progress.id == pending.id.into());
                if current && progress.multipv.unwrap_or(1) == 1 {
                    let position = self.game.position();
                    self.thinking = Some(Thinking {
                        depth: progress.depth,
                        sel_depth: progress.sel_depth,
                        score: white_view(progress.score, position.side_to_move()),
                        nodes: progress.nodes,
                        nps: progress.nps,
                        time_ms: progress.time_ms,
                        pv: rules::line_to_san(position, &progress.pv),
                    });
                }
            }
            NodeEvent::Response(response) => self.handle_response(response, engine),
            NodeEvent::Stderr(line) => {
                if self.stderr.len() == STDERR_LINES {
                    self.stderr.remove(0);
                }
                self.status = format!("{}: {line}", self.node.name);
                self.stderr.push(line);
            }
            NodeEvent::Closed(reason) => {
                self.disconnected = true;
                self.pending = None;
                self.status = format!("Engine disconnected: {reason}");
            }
        }
    }

    fn handle_response(&mut self, response: Response, engine: &mut dyn Engine) {
        let Some(cnp::Id::Number(id)) = response.id else {
            return;
        };
        if self.levels_request == Some(id) {
            self.levels_request = None;
            if let Ok(levels) = response.result::<proto::Levels>() {
                self.levels = Some(levels);
            }
            return;
        }
        let Some(pending) = self.pending.filter(|pending| pending.id == id) else {
            // A stale search, or a stop's acknowledgement.
            return;
        };
        self.pending = None;
        let result = match response.result::<proto::Search>() {
            Ok(result) => result,
            Err(e) if e.code == code::CANCELLED => return,
            Err(e) => {
                self.status = format!("{} couldn't search: {}", self.node.name, e.message);
                return;
            }
        };
        if self.game.moves().len() != pending.ply {
            return;
        }
        let position = self.game.position().clone();
        let Some(mv) = result
            .best_move
            .as_deref()
            .and_then(|text| position.parse_uci_move(text))
        else {
            self.status = format!("{} found no move to play.", self.node.name);
            return;
        };
        let score = result
            .score
            .map(|score| white_view(score, position.side_to_move()));
        if let Some(score) = score {
            self.thinking = Some(Thinking {
                depth: result.depth,
                sel_depth: result.sel_depth,
                score,
                nodes: result.nodes,
                nps: (result.time_ms > 0).then(|| result.nodes * 1000 / result.time_ms),
                time_ms: result.time_ms,
                pv: rules::line_to_san(&position, &result.pv),
            });
        }
        // A PGN comment with the score and depth, as engines' games usually have (`+0.35/18`).
        let comment = score.map(|score| format!("{}/{}", score_text(score), result.depth));
        // A draw offer is answered with the engine's move: it accepts if its own score says
        // it isn't better, and the move isn't played.
        let engine_side = self.settings.engine_plays[position.side_to_move().index()];
        let answering = engine_side && std::mem::take(&mut self.draw_offered);
        if answering && result.score.is_some_and(accepts_draw) {
            self.game.agree_draw();
            self.game_ended(engine);
            self.status = format!("{} accepts the draw. {}", self.node.name, self.status);
            return;
        }
        let san = position.to_san(mv);
        if self.play(mv, comment) {
            // A picker opened for the side the engine just moved for is stale now.
            self.promotion = None;
            self.selected = None;
            self.status = if answering {
                format!("{} declines the draw and plays {san}.", self.node.name)
            } else {
                format!("{} played {san}.", self.node.name)
            };
            self.after_move(engine);
        }
    }
}

/// Whether an engine whose search scored `score` (from its own point of view) accepts a draw:
/// when it doesn't think it's better ([`DRAW_ACCEPT_CP`]), or it is being mated.
pub fn accepts_draw(score: Score) -> bool {
    match score.value {
        ScoreValue::Centipawns(cp) => cp <= DRAW_ACCEPT_CP,
        ScoreValue::Mate(n) => n < 0,
    }
}

/// A score from the side to move's point of view, turned to White's.
pub fn white_view(score: Score, side_to_move: Color) -> Score {
    if side_to_move == Color::White {
        return score;
    }
    let value = match score.value {
        ScoreValue::Centipawns(cp) => ScoreValue::Centipawns(-cp),
        ScoreValue::Mate(n) => ScoreValue::Mate(-n),
    };
    let bound = match score.bound {
        proto::Bound::Lower => proto::Bound::Upper,
        proto::Bound::Upper => proto::Bound::Lower,
        proto::Bound::Exact => proto::Bound::Exact,
    };
    Score { value, bound }
}

/// A score as people write it: `+0.35`, `-1.20`, `#3` (White mates in 3), `-#2`.
pub fn score_text(score: Score) -> String {
    match score.value {
        ScoreValue::Centipawns(cp) => format!("{:+.2}", f64::from(cp) / 100.0),
        ScoreValue::Mate(n) if n >= 0 => format!("#{n}"),
        ScoreValue::Mate(n) => format!("-#{}", -n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cnp::engine::{Progress, SearchResult};

    /// Records what the app asks of the engine.
    #[derive(Default)]
    struct FakeEngine {
        searches: Vec<SearchParams>,
        stops: Vec<i64>,
    }

    impl Engine for FakeEngine {
        fn search(&mut self, params: &SearchParams) -> Result<i64, String> {
            self.searches.push(params.clone());
            Ok(100 + self.searches.len() as i64)
        }
        fn stop(&mut self, id: i64) -> Result<(), String> {
            self.stops.push(id);
            Ok(())
        }
    }

    fn node() -> NodeSummary {
        NodeSummary {
            name: "eng".into(),
            version: "1".into(),
            license: "MIT".into(),
            caps: EngineCapabilities {
                progress: true,
                strength: true,
                ..EngineCapabilities::default()
            },
        }
    }

    fn app() -> App {
        App::new(node(), Settings::default()).unwrap()
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::from(KeyCode::Char(c))
    }

    fn type_line(app: &mut App, engine: &mut FakeEngine, text: &str) {
        for c in text.chars() {
            app.handle_key(key(c), engine);
        }
        app.handle_key(KeyEvent::from(KeyCode::Enter), engine);
    }

    fn answer(id: i64, best: &str) -> NodeEvent {
        NodeEvent::Response(Response::ok::<proto::Search>(
            id.into(),
            &serde_json::from_value::<SearchResult>(serde_json::json!({
                "bestMove": best, "score": {"cp": -30}, "pv": [best],
                "depth": 9, "selDepth": 12, "nodes": 1000, "timeMs": 10, "stopped": false
            }))
            .unwrap(),
        ))
    }

    fn sq(name: &str) -> Square {
        name.parse().unwrap()
    }

    #[test]
    fn a_typed_move_starts_the_engine_and_its_answer_is_played() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        assert_eq!(engine.searches.len(), 1);
        let params = &engine.searches[0];
        assert_eq!(params.position.moves, ["e2e4"]);
        assert_eq!(params.limits.time_ms, Some(DEFAULT_MOVETIME_MS));
        assert_eq!(params.progress_ms, Some(PROGRESS_MS));
        assert!(app.searching());

        app.handle_node(answer(101, "e7e5"), &mut engine);
        assert!(!app.searching());
        let sans: Vec<&str> = app.game.moves().iter().map(|m| m.san.as_str()).collect();
        assert_eq!(sans, ["e4", "e5"]);
        // Black's -0.30 is White's +0.30, and goes into the PGN comment.
        assert_eq!(app.game.moves()[1].comment.as_deref(), Some("+0.30/9"));
    }

    #[test]
    fn z_and_zoom_toggle_the_zoomed_board() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        app.handle_key(key('z'), &mut engine);
        assert!(app.zoomed);
        type_line(&mut app, &mut engine, ":zoom");
        assert!(!app.zoomed);
        // While a line is being typed, `z` is a letter in it.
        app.handle_key(key(':'), &mut engine);
        app.handle_key(key('z'), &mut engine);
        assert!(!app.zoomed);
        assert_eq!(app.input.as_deref(), Some(":z"));
    }

    const PGN: &str = "[White \"Ann\"]\n[Black \"Bo\"]\n\n1. e4 e5 2. Nf3 (2. f4 exf4) Nc6 *";

    fn viewer(has_engine: bool) -> App {
        let mut app = app();
        app.has_engine = has_engine;
        app.open_review(Review::from_pgn(PGN, "g.pgn").unwrap());
        app
    }

    fn press(app: &mut App, engine: &mut FakeEngine, code: KeyCode) {
        app.handle_key(KeyEvent::from(code), engine);
    }

    #[test]
    fn the_viewer_steps_with_the_arrows_and_enters_variations() {
        let (mut app, mut engine) = (viewer(true), FakeEngine::default());
        assert_eq!(app.shown_position(), *app.review.as_ref().unwrap().start());
        press(&mut app, &mut engine, KeyCode::Right);
        press(&mut app, &mut engine, KeyCode::Right);
        assert_eq!(app.shown_move_label().as_deref(), Some("1… e5"));
        app.handle_key(key('1'), &mut engine);
        assert_eq!(app.shown_move_label().as_deref(), Some("2. f4"));
        app.handle_key(key('2'), &mut engine);
        assert!(app.status.contains("no variation 2"), "{}", app.status);
        press(&mut app, &mut engine, KeyCode::Left);
        press(&mut app, &mut engine, KeyCode::Down);
        assert_eq!(app.shown_move_label().as_deref(), Some("2… Nc6"));
        press(&mut app, &mut engine, KeyCode::Up);
        assert_eq!(app.shown_move_label(), None);
        // Nothing went to the engine.
        assert!(engine.searches.is_empty());
    }

    #[test]
    fn the_viewer_is_read_only_until_play_plays_on() {
        let (mut app, mut engine) = (viewer(true), FakeEngine::default());
        press(&mut app, &mut engine, KeyCode::Right);
        app.handle_click(sq("e7"), &mut engine);
        assert_eq!(app.selected, None);
        assert!(app.status.contains("read-only"), "{}", app.status);
        type_line(&mut app, &mut engine, "e5");
        assert!(app.status.contains("read-only"), "{}", app.status);
        app.handle_key(key('u'), &mut engine);
        assert!(app.status.contains("read-only"), "{}", app.status);

        // :play black: a game of the moves so far, the engine to move as Black.
        type_line(&mut app, &mut engine, ":play black");
        assert!(app.review.is_none());
        let sans: Vec<&str> = app.game.moves().iter().map(|m| m.san.as_str()).collect();
        assert_eq!(sans, ["e4"]);
        assert_eq!(engine.searches.len(), 1);
        assert!(
            app.status.contains("Playing on from g.pgn"),
            "{}",
            app.status
        );
    }

    #[test]
    fn without_an_engine_the_viewer_plays_on_only_as_both_sides() {
        let (mut app, mut engine) = (viewer(false), FakeEngine::default());
        type_line(&mut app, &mut engine, ":play black");
        assert!(app.review.is_some());
        assert!(app.status.contains("no engine"), "{}", app.status);
        type_line(&mut app, &mut engine, ":play none");
        assert!(app.review.is_none());
        type_line(&mut app, &mut engine, "e4");
        assert_eq!(app.game.moves().len(), 1);
        app.handle_key(key('m'), &mut engine);
        assert!(app.status.contains("no engine"), "{}", app.status);
        assert!(engine.searches.is_empty());
    }

    #[test]
    fn brackets_look_back_through_a_game_and_block_moves_meanwhile() {
        let settings = Settings {
            engine_plays: [false, false],
            ..Settings::default()
        };
        let (mut app, mut engine) = (App::new(node(), settings).unwrap(), FakeEngine::default());
        for mv in ["e4", "e5", "Nf3"] {
            type_line(&mut app, &mut engine, mv);
        }
        let live = app.shown_position();
        app.handle_key(key('['), &mut engine);
        assert_eq!(app.browse, Some(2));
        assert_eq!(app.shown_move_label().as_deref(), Some("1… e5"));
        assert_eq!(app.shown_last_move(), Some(app.game.moves()[1].mv));
        // A move while looking back is refused.
        type_line(&mut app, &mut engine, "Nc6");
        assert_eq!(app.game.moves().len(), 3);
        assert!(app.status.contains("looking back"), "{}", app.status);
        press(&mut app, &mut engine, KeyCode::Home);
        assert_eq!(app.browse, Some(0));
        assert_eq!(app.shown_position(), *app.game.start());
        press(&mut app, &mut engine, KeyCode::End);
        assert_eq!(app.browse, None);
        assert_eq!(app.shown_position(), live);
        // The arrows still move the board's cursor in a game.
        press(&mut app, &mut engine, KeyCode::Left);
        assert_eq!(app.browse, None);
        assert_eq!(app.cursor, sq("d2"));
    }

    use crate::clock::TimeControl;
    use std::time::Duration;

    fn timed(engine_plays: [bool; 2], clock: &str, caps: EngineCapabilities) -> App {
        let settings = Settings {
            engine_plays,
            clock: Some(TimeControl::parse(clock).unwrap()),
            ..Settings::default()
        };
        let mut n = node();
        n.caps = caps;
        App::new(n, settings).unwrap()
    }

    fn after(app: &App, ms: u64) -> Instant {
        app.now + Duration::from_millis(ms)
    }

    #[test]
    fn clocks_start_with_the_first_move_and_go_to_an_engine_that_reads_them() {
        let caps = EngineCapabilities {
            clock: true,
            ..EngineCapabilities::default()
        };
        let (mut app, mut engine) = (timed([false, true], "1+2", caps), FakeEngine::default());
        assert!(!app.clock_running());
        let t = after(&app, 5000);
        app.tick(t, &mut engine);
        type_line(&mut app, &mut engine, "e4");
        let clocks = app.clocks.clone().unwrap();
        // The first move is free, and earns the increment.
        assert_eq!(clocks.times(t), [62_000, 60_000]);
        assert_eq!(clocks.running(), Some(Color::Black));
        let limits = &engine.searches[0].limits;
        assert_eq!(limits.time_ms, None);
        let clock = limits.clock.clone().unwrap();
        assert_eq!((clock.white.time_ms, clock.black.time_ms), (62_000, 60_000));
        assert_eq!(clock.black.increment_ms, 2000);

        // The engine takes 3 s: charged to it, plus its increment.
        let t = after(&app, 3000);
        app.tick(t, &mut engine);
        app.handle_node(answer(101, "e7e5"), &mut engine);
        assert_eq!(app.clocks.as_ref().unwrap().times(t), [62_000, 59_000]);
        assert_eq!(app.clocks.as_ref().unwrap().running(), Some(Color::White));
    }

    #[test]
    fn an_engine_without_a_clock_gets_a_share_of_its_time() {
        let caps = EngineCapabilities::default();
        let (mut app, mut engine) = (timed([false, true], "5+3", caps), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        let limits = &engine.searches[0].limits;
        assert_eq!(limits.clock, None);
        assert_eq!(limits.time_ms, Some(300_000 / CLOCK_FALLBACK_MOVES + 3000));
    }

    #[test]
    fn a_flag_falls_and_the_search_stops() {
        let (mut app, mut engine) = (
            timed([false, true], "0.5", EngineCapabilities::default()),
            FakeEngine::default(),
        );
        type_line(&mut app, &mut engine, "e4");
        app.tick(after(&app, 29_000), &mut engine);
        assert_eq!(app.game.outcome(), None);
        app.tick(after(&app, 1000), &mut engine);
        assert_eq!(
            app.game.outcome(),
            Some(Outcome::Timeout {
                winner: Color::White
            })
        );
        assert_eq!(engine.stops, [101]);
        assert!(!app.clock_running());
        assert!(app.status.starts_with("Time out"), "{}", app.status);
        // Its late answer isn't played.
        app.handle_node(answer(101, "e7e5"), &mut engine);
        assert_eq!(app.game.moves().len(), 1);
    }

    #[test]
    fn a_takeback_puts_the_clocks_back() {
        let (mut app, mut engine) = (
            timed([false, true], "1", EngineCapabilities::default()),
            FakeEngine::default(),
        );
        type_line(&mut app, &mut engine, "e4");
        app.tick(after(&app, 4000), &mut engine);
        app.handle_node(answer(101, "e7e5"), &mut engine);
        app.tick(after(&app, 7000), &mut engine);
        type_line(&mut app, &mut engine, "Nf3");
        app.tick(after(&app, 1000), &mut engine);
        // Takes back Nf3, the engine still thinking: the clocks are as they were when it
        // was played (White's 7 s are spent), and White's runs again.
        app.handle_key(key('u'), &mut engine);
        assert_eq!(app.game.moves().len(), 2);
        let clocks = app.clocks.clone().unwrap();
        assert_eq!(clocks.times(app.now), [53_000, 56_000]);
        assert_eq!(clocks.running(), Some(Color::White));
        // Then e5 and e4: back to the start, where no clock runs.
        app.handle_key(key('u'), &mut engine);
        assert_eq!(app.game.moves().len(), 0);
        let clocks = app.clocks.clone().unwrap();
        assert_eq!(clocks.times(app.now), [60_000, 60_000]);
        assert_eq!(clocks.running(), None);
    }

    #[test]
    fn resigning_ends_the_game_for_the_persons_side_and_can_be_taken_back() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, ":resign");
        assert_eq!(
            app.game.outcome(),
            Some(Outcome::Resignation {
                winner: Color::Black
            })
        );
        assert_eq!(engine.stops, [101], "the engine stops thinking");
        assert!(app.status.contains("Black wins"), "{}", app.status);
        app.handle_key(key('u'), &mut engine);
        assert_eq!(app.game.outcome(), None);
        assert_eq!(app.game.moves().len(), 1, "the moves stay");
        assert!(app.searching(), "and the engine's turn goes on");
    }

    #[test]
    fn the_engine_answers_a_draw_offer_from_its_score() {
        let answer_scored = |id: i64, cp: i32| {
            NodeEvent::Response(Response::ok::<proto::Search>(
                id.into(),
                &serde_json::from_value::<SearchResult>(serde_json::json!({
                    "bestMove": "e7e5", "score": {"cp": cp}, "pv": ["e7e5"],
                    "depth": 9, "selDepth": 12, "nodes": 1000, "timeMs": 10, "stopped": false
                }))
                .unwrap(),
            ))
        };
        // Ahead by 0.40 for itself: it declines and plays on.
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, ":draw");
        assert!(app.draw_offered);
        app.handle_node(answer_scored(101, 40), &mut engine);
        assert!(!app.draw_offered);
        assert_eq!(app.game.moves().len(), 2);
        assert!(app.status.contains("declines"), "{}", app.status);

        // Level for itself: it accepts, and its move isn't played.
        let (mut app, mut engine) = (self::app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":draw");
        type_line(&mut app, &mut engine, "e4");
        app.handle_node(answer_scored(101, 0), &mut engine);
        assert_eq!(app.game.outcome(), Some(Outcome::DrawAgreed));
        assert_eq!(app.game.moves().len(), 1);
        assert!(app.status.starts_with("eng accepts"), "{}", app.status);
    }

    #[test]
    fn with_no_engine_side_a_draw_is_agreed_at_once() {
        let settings = Settings {
            engine_plays: [false, false],
            ..Settings::default()
        };
        let (mut app, mut engine) = (App::new(node(), settings).unwrap(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":draw");
        assert_eq!(app.game.outcome(), Some(Outcome::DrawAgreed));
        assert!(accepts_draw(Score::mate(-3)) && !accepts_draw(Score::mate(2)));
    }

    #[test]
    fn clock_sets_and_clears_the_time_control() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, ":clock 3+2");
        let clocks = app.clocks.clone().unwrap();
        assert_eq!(clocks.times(app.now), [180_000, 180_000]);
        // The game has begun, so the side to move's clock runs.
        assert_eq!(clocks.running(), Some(Color::Black));
        assert!(app.status.contains("3+2"), "{}", app.status);
        type_line(&mut app, &mut engine, ":clock fast");
        assert!(app.status.starts_with("Usage"), "{}", app.status);
        type_line(&mut app, &mut engine, ":clock off");
        assert_eq!(app.clocks, None);
        assert_eq!(app.settings.clock, None);
    }

    #[test]
    fn clicks_pick_up_and_put_down() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        app.handle_click(sq("g1"), &mut engine);
        assert_eq!(app.selected, Some(sq("g1")));
        assert_eq!(app.targets().len(), 2);
        app.handle_click(sq("f3"), &mut engine);
        assert_eq!(app.game.moves()[0].san, "Nf3");
        assert_eq!(app.selected, None);
    }

    #[test]
    fn clicking_the_king_then_g1_castles() {
        let mut engine = FakeEngine::default();
        let settings = Settings {
            fen: Some("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1".into()),
            ..Settings::default()
        };
        let mut app = App::new(node(), settings).unwrap();
        app.handle_click(sq("e1"), &mut engine);
        // The dots are where the king lands, not on the rooks.
        let targets = app.targets();
        assert!(
            targets.contains(&sq("g1")) && targets.contains(&sq("c1")),
            "{targets:?}"
        );
        assert!(!targets.contains(&sq("h1")), "{targets:?}");
        app.handle_click(sq("g1"), &mut engine);
        assert_eq!(app.game.moves()[0].san, "O-O");
        assert_eq!(engine.searches[0].position.moves, ["e1g1"]);
    }

    fn promoting_app() -> App {
        let settings = Settings {
            engine_plays: [false, false],
            fen: Some("8/4P3/8/8/8/8/8/K6k w - - 0 1".into()),
            ..Settings::default()
        };
        App::new(node(), settings).unwrap()
    }

    #[test]
    fn a_promotion_opens_a_picker_and_a_click_on_it_chooses() {
        let (mut app, mut engine) = (promoting_app(), FakeEngine::default());
        app.handle_click(sq("e7"), &mut engine);
        app.handle_click(sq("e8"), &mut engine);
        assert!(
            app.game.moves().is_empty(),
            "nothing played until a piece is chosen"
        );
        let picker: Vec<(Square, char)> = app
            .promotion_squares()
            .into_iter()
            .map(|(at, piece)| (at, piece.to_fen_char()))
            .collect();
        assert_eq!(
            picker,
            [
                (sq("e8"), 'Q'),
                (sq("e7"), 'R'),
                (sq("e6"), 'B'),
                (sq("e5"), 'N')
            ]
        );
        app.handle_click(sq("e5"), &mut engine);
        assert_eq!(app.game.moves()[0].san, "e8=N");
        assert_eq!(app.promotion, None);
    }

    #[test]
    fn the_picker_takes_a_letter_enter_for_a_queen_or_esc() {
        let (mut app, mut engine) = (promoting_app(), FakeEngine::default());
        app.handle_click(sq("e7"), &mut engine);
        app.handle_click(sq("e8"), &mut engine);
        press(&mut app, &mut engine, KeyCode::Esc);
        assert_eq!(app.promotion, None);
        assert!(app.game.moves().is_empty());

        // A letter chooses; other keys wait.
        app.handle_click(sq("e7"), &mut engine);
        app.handle_click(sq("e8"), &mut engine);
        app.handle_key(key('x'), &mut engine);
        assert!(app.promotion.is_some());
        app.handle_key(key('r'), &mut engine);
        assert_eq!(app.game.moves()[0].san, "e8=R");

        // With the keyboard: Enter on e8 opens the picker, and Enter again is a queen.
        let (mut app, mut engine) = (promoting_app(), FakeEngine::default());
        app.cursor = sq("e7");
        press(&mut app, &mut engine, KeyCode::Enter);
        press(&mut app, &mut engine, KeyCode::Up);
        press(&mut app, &mut engine, KeyCode::Enter);
        assert!(app.promotion.is_some());
        press(&mut app, &mut engine, KeyCode::Enter);
        assert_eq!(app.game.moves()[0].san, "e8=Q");

        // `q` is a queen here, not quit.
        let (mut app, mut engine) = (promoting_app(), FakeEngine::default());
        app.handle_click(sq("e7"), &mut engine);
        app.handle_click(sq("e8"), &mut engine);
        app.handle_key(key('q'), &mut engine);
        assert!(!app.quit);
        assert_eq!(app.game.moves()[0].san, "e8=Q");
    }

    #[test]
    fn blacks_picker_runs_up_the_board_from_the_first_rank() {
        let settings = Settings {
            engine_plays: [false, false],
            fen: Some("K6k/8/8/8/8/8/3p4/8 b - - 0 1".into()),
            ..Settings::default()
        };
        let (mut app, mut engine) = (App::new(node(), settings).unwrap(), FakeEngine::default());
        app.handle_click(sq("d2"), &mut engine);
        app.handle_click(sq("d1"), &mut engine);
        let squares: Vec<Square> = app.promotion_squares().iter().map(|(at, _)| *at).collect();
        assert_eq!(squares, [sq("d1"), sq("d2"), sq("d3"), sq("d4")]);
        app.handle_click(sq("d3"), &mut engine);
        assert_eq!(app.game.moves()[0].san, "d1=B");
    }

    #[test]
    fn a_click_off_the_picker_puts_the_pawn_back() {
        let (mut app, mut engine) = (promoting_app(), FakeEngine::default());
        app.handle_click(sq("e7"), &mut engine);
        app.handle_click(sq("e8"), &mut engine);
        app.handle_click(sq("a8"), &mut engine);
        assert_eq!(app.promotion, None);
        assert_eq!(app.selected, None);
        assert!(app.game.moves().is_empty());
        assert!(app.status.contains("cancelled"), "{}", app.status);
    }

    #[test]
    fn clicking_the_opponents_piece_picks_nothing_up() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        app.handle_click(sq("e7"), &mut engine);
        assert_eq!(app.selected, None);
    }

    #[test]
    fn the_person_cant_move_for_the_engine() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, "e5");
        assert_eq!(app.game.moves().len(), 1);
        assert!(app.status.contains("eng's move"), "{}", app.status);
    }

    #[test]
    fn a_takeback_during_a_search_makes_its_answer_stale() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        app.handle_key(key('u'), &mut engine);
        assert_eq!(engine.stops, [101]);
        assert_eq!(app.game.moves().len(), 0);
        app.handle_node(answer(101, "e7e5"), &mut engine);
        assert_eq!(app.game.moves().len(), 0);
    }

    #[test]
    fn undo_takes_back_the_engines_reply_too() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        app.handle_node(answer(101, "e7e5"), &mut engine);
        app.handle_key(key('u'), &mut engine);
        assert_eq!(app.game.moves().len(), 0);
        assert!(!app.searching());
    }

    #[test]
    fn an_engine_playing_white_opens_the_game() {
        let mut engine = FakeEngine::default();
        let settings = Settings {
            engine_plays: [true, false],
            ..Settings::default()
        };
        let mut app = App::new(node(), settings).unwrap();
        assert!(engine.searches.is_empty());
        app.start(&mut engine);
        assert_eq!(engine.searches.len(), 1);
        app.handle_node(answer(101, "d2d4"), &mut engine);
        assert_eq!(app.game.moves()[0].san, "d4");
        assert!(!app.searching(), "then it's the person's turn");
    }

    #[test]
    fn the_engine_can_play_white() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":play white");
        assert_eq!(engine.searches.len(), 1);
        assert!(engine.searches[0].position.moves.is_empty());
    }

    #[test]
    fn levels_go_to_the_engine_only_when_it_has_them() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":level 5");
        type_line(&mut app, &mut engine, "d4");
        let strength = engine.searches[0].strength.clone().unwrap();
        assert_eq!(strength.level, Some(5.0));

        let mut weak = node();
        weak.caps = EngineCapabilities::default();
        let mut app = App::new(weak, Settings::default()).unwrap();
        type_line(&mut app, &mut engine, ":level 5");
        assert!(app.status.contains("no playing levels"));
        assert_eq!(app.settings.level, None);
    }

    #[test]
    fn progress_shows_the_line_in_san_from_whites_view() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        let progress = Progress {
            id: 101.into(),
            depth: 5,
            sel_depth: 8,
            score: Score::cp(25),
            pv: vec!["e7e5".into(), "g1f3".into()],
            multipv: None,
            nodes: 5000,
            nps: Some(100_000),
            time_ms: 50,
            hash_full: None,
            tb_hits: None,
        };
        app.handle_node(NodeEvent::Progress(progress), &mut engine);
        let thinking = app.thinking.clone().unwrap();
        assert_eq!(thinking.pv, ["e5", "Nf3"]);
        assert_eq!(score_text(thinking.score), "-0.25");
    }

    #[test]
    fn a_bad_move_says_why() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e5");
        assert!(app.status.starts_with("Can't play `e5`"), "{}", app.status);
        assert!(engine.searches.is_empty());
    }

    #[test]
    fn a_new_game_from_a_fen_tells_the_engine_its_a_new_game() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":fen 4k3/8/8/8/8/8/8/4K2R b K - 0 1");
        assert_eq!(engine.searches.len(), 1);
        let params = &engine.searches[0];
        assert_eq!(params.position.fen, "4k3/8/8/8/8/8/8/4K2R b K - 0 1");
        assert_eq!(params.game.as_deref(), Some("cbtui-2"));
    }

    #[test]
    fn scores_read_as_people_write_them() {
        assert_eq!(score_text(Score::cp(35)), "+0.35");
        assert_eq!(score_text(Score::cp(-120)), "-1.20");
        assert_eq!(score_text(Score::mate(3)), "#3");
        assert_eq!(score_text(Score::mate(-2)), "-#2");
        assert_eq!(score_text(white_view(Score::mate(2), Color::Black)), "-#2");
    }

    #[test]
    fn saves_a_pgn() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        app.handle_node(answer(101, "c7c5"), &mut engine);
        let dir = std::env::temp_dir().join(format!("cbtui-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.pgn");
        app.command(&format!("pgn {}", path.display()), &mut engine);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[Black \"eng 1\"]"), "{text}");
        assert!(text.contains("1. e4 c5"), "{text}");
        fs::remove_dir_all(dir).unwrap();
    }
}
