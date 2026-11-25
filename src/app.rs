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
//! with its next move: from move [`DRAW_MIN_MOVE`] on, it accepts when its search's score
//! says it is clearly worse ([`accepts_draw`]), from what it reported, never from cbtui's own
//! judgement. A resignation
//! or an agreed draw can be taken back (`u`), and the game goes on.
//!
//! ## The engine's turn
//!
//! After every change, [`App`] checks whether the engine plays the side to move. If so, and no
//! search is running, it sends one with the whole game (start FEN and moves, so the engine sees
//! repetitions) and remembers the request id and the game's length. When the answer comes, it
//! is played only if both still match: a takeback or a new game in between makes it stale.

use std::collections::BTreeMap;
use std::fs;
use std::time::Instant;

use cnp::engine::{
    self as proto, EngineCapabilities, EvaluateParams, EvaluateResult, LevelsResult, Score,
    ScoreValue, SearchLimits, SearchParams, SideClock, Strength, Technique,
};
use cnp::{Response, code};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::analysis::{Analysis, DEFAULT_LINES, MAX_LINES};
use crate::clock::{Clocks, TimeControl};
use crate::help::HelpView;
use crate::node::{Engine, NodeEvent};
use crate::review::Review;
use crate::root::RootView;
use crate::rules::{self, Color, Game, Move, Outcome, Piece, Position, Square};
use crate::theme::Theme;
use crate::tree::{TREE_DEPTH, TREE_PLIES, TreeBrowser};

/// How often a search reports progress, in milliseconds, when the engine can.
pub const PROGRESS_MS: u64 = 200;

/// How often an analysis of several lines asks for progress: every report. An engine that
/// throttles its reports with one interval for all its lines sends the best line and drops
/// the others, which follow it within milliseconds; unthrottled, it reports each line once a
/// depth, a few messages a second once the search is deep.
pub const ANALYSIS_PROGRESS_MS: u64 = 0;

/// How often a search reports its root, in milliseconds, when cbtui follows it.
pub const ROOT_MS: u64 = 250;

/// How long the engine thinks about a move by default, in milliseconds.
pub const DEFAULT_MOVETIME_MS: u64 = 1000;

/// The longest the engine may be asked to think about one move, in milliseconds.
pub const MAX_MOVETIME_MS: u64 = 600_000;

/// What cbtui says when asked to use an engine it doesn't have.
const NO_ENGINE: &str =
    "There's no engine. Start cbtui with --engine <path> (or $CBTUI_ENGINE) to play one.";

/// The screen size the help assumes until the event loop says the real one.
const DEFAULT_SCREEN: (u16, u16) = (80, 24);

/// How far Page Up and Page Down move in the techniques.
const TECHNIQUES_PAGE: usize = 10;

/// What the status line says while the promotion picker is open.
const PROMOTE: &str =
    "Promote to: click a piece, or press q, r, b or n (Enter: queen, Esc: put the pawn back).";

/// An engine that can't read a clock gets this share of its time left for each move: as if
/// this many moves were still to play.
pub const CLOCK_FALLBACK_MOVES: u64 = 30;

/// What an engine without a clock keeps in hand on every move, in milliseconds, so the time
/// it is given never quite uses up its clock.
pub const CLOCK_SAFETY_MS: u64 = 50;

/// The engine accepts a draw offer when its own score is at most this, in centipawns: when it
/// is clearly worse, by half a pawn or more.
pub const DRAW_ACCEPT_CP: i32 = -50;

/// The engine considers a draw offer only from this move number on; before it, it declines.
pub const DRAW_MIN_MOVE: u32 = 30;

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
    /// The most lines one search may return (its `maxMultipv`).
    pub max_multipv: u32,
    /// The deepest `engine/tree` search it allows.
    pub max_tree_depth: u32,
    /// The most plies from the root a tree may record.
    pub max_tree_plies: u32,
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
            max_multipv: info.limits.multipv(),
            max_tree_depth: info
                .limits
                .max_tree_depth
                .or(info.limits.max_depth)
                .unwrap_or(u32::MAX),
            max_tree_plies: info.limits.max_tree_plies.unwrap_or(u32::MAX),
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
    /// How many lines an analysis asks for, if the engine can find several.
    pub lines: u32,
    /// Search techniques switched away from their defaults, by id, sent with every search.
    pub techniques: BTreeMap<String, bool>,
    /// The board's colours.
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            engine_plays: [false, true],
            movetime_ms: DEFAULT_MOVETIME_MS,
            level: None,
            fen: None,
            clock: None,
            lines: DEFAULT_LINES,
            techniques: BTreeMap::new(),
            theme: Theme::default(),
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

/// The engine's evaluation of a position, term by term (`engine/evaluate`).
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    /// Which position it is, for the title: `Start`, `After 12. Nf3`.
    pub label: String,
    /// The position evaluated, to write the settling moves in SAN.
    pub position: Position,
    /// What the engine said.
    pub result: EvaluateResult,
}

/// A window over the board that takes the keys until it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The evaluation, term by term ([`App::evaluation`]).
    Evaluation,
    /// The search techniques, to switch on and off; the highlighted one's index.
    Techniques {
        /// The highlighted technique.
        cursor: usize,
    },
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
    /// The engine analysing the position on the board (`i`), when it is.
    pub analysis: Option<Analysis>,
    /// The latest evaluation the engine sent (`v`).
    pub evaluation: Option<Evaluation>,
    /// The search techniques the engine can switch, once it has said them.
    pub techniques: Option<Vec<Technique>>,
    /// The window open over the board, if any.
    pub overlay: Option<Overlay>,
    /// Whether to follow searches at their root (`w`), when the engine can.
    pub follow_root: bool,
    /// The latest report of the current search's root.
    pub root: Option<RootView>,
    /// A search tree being walked (`:tree`).
    pub tree: Option<TreeBrowser>,
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
    /// How many lines the help is scrolled down.
    pub help_scroll: usize,
    /// The screen's size when it was last drawn, as the event loop says: what the help
    /// scrolls against.
    pub screen: Rect,
    /// The engine's recent stderr lines, oldest first.
    pub stderr: Vec<String>,
    /// Whether the engine has gone.
    pub disconnected: bool,
    /// Set when the user asks to leave.
    pub quit: bool,
    pending: Option<Pending>,
    /// The `engine/levels` request's id, until it is answered.
    levels_request: Option<i64>,
    /// The `engine/techniques` request's id, until it is answered.
    techniques_request: Option<i64>,
    /// The `engine/tree` request's id and the position it searches, until it is answered.
    tree_request: Option<(i64, Position)>,
    /// The `engine/evaluate` request's id and what it evaluates, until it is answered.
    evaluate_request: Option<(i64, String, Position)>,
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
            analysis: None,
            evaluation: None,
            techniques: None,
            overlay: None,
            follow_root: false,
            root: None,
            tree: None,
            cursor: rules::square(4, 1).expect("e2 is on the board"),
            cursor_visible: false,
            selected: None,
            promotion: None,
            input: None,
            status,
            thinking: None,
            show_help: false,
            help_scroll: 0,
            screen: Rect::new(0, 0, DEFAULT_SCREEN.0, DEFAULT_SCREEN.1),
            stderr: Vec::new(),
            disconnected: false,
            quit: false,
            pending: None,
            levels_request: None,
            techniques_request: None,
            evaluate_request: None,
            tree_request: None,
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
        let running = !self.game.moves().is_empty()
            && self.game.outcome().is_none()
            && self.analysis.is_none();
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
            if self.game.outcome().is_some() || self.analysis.is_some() {
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
        if let Some(tree) = &self.tree {
            return tree.level().position.clone();
        }
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

    /// The game up to the position on the board: what an analysis searches, so the engine
    /// sees the moves that led there (and any repetition).
    pub fn shown_game(&self) -> Game {
        if let Some(review) = &self.review {
            return review.game_here();
        }
        match self.browse {
            Some(ply) => {
                let mut game = Game::from_position(self.game.start().clone());
                for played in self.game.moves().iter().take(ply) {
                    let _ = game.play_with_comment(played.mv, played.comment.clone());
                }
                game
            }
            None => self.game.clone(),
        }
    }

    /// The move that led to the position on the board, if any.
    pub fn shown_last_move(&self) -> Option<Move> {
        // In a tree, the highlighted child's move: the one being looked at.
        if let Some(tree) = &self.tree {
            let level = tree.level();
            return level
                .selected()
                .and_then(|child| child.mv.as_deref())
                .and_then(|mv| level.position.parse_uci_move(mv));
        }
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
        if self.tree.is_some() {
            return None;
        }
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

    /// Records that `engine/techniques` was sent with this id.
    pub fn expect_techniques(&mut self, id: i64) {
        self.techniques_request = Some(id);
    }

    /// Whether technique `id` is on in searches: as switched, else its default.
    pub fn technique_on(&self, technique: &Technique) -> bool {
        self.settings
            .techniques
            .get(&technique.id)
            .copied()
            .unwrap_or(technique.default)
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
        self.key(key, engine);
        self.sync_analysis(engine);
    }

    fn key(&mut self, key: KeyEvent, engine: &mut dyn Engine) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.input.is_some() {
            self.handle_input_key(key, engine);
            return;
        }
        if self.show_help {
            self.help_key(key.code);
            return;
        }
        if let Some(overlay) = self.overlay {
            self.overlay_key(overlay, key.code);
            return;
        }
        if self.tree.is_some() {
            self.tree_key(key.code, engine);
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
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.input = Some(":".into()),
            KeyCode::Char('n') => self.new_game(None, engine),
            KeyCode::Char('u') => self.undo(engine),
            KeyCode::Char('t') => self.flipped = !self.flipped,
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
            KeyCode::Char('i') => self.toggle_analysis(engine),
            KeyCode::Char('v') => self.evaluate(engine),
            KeyCode::Char('w') => self.toggle_root(),
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

    /// An overlay's keys. The evaluation closes on any key. The techniques move with the
    /// arrows (and Page Up, Page Down, Home, End), switch with Space or Enter, go back to
    /// their default with `d`, and close with Esc or `q`.
    fn overlay_key(&mut self, overlay: Overlay, code: KeyCode) {
        let Overlay::Techniques { cursor } = overlay else {
            self.overlay = None;
            return;
        };
        let count = self.techniques.as_ref().map_or(0, Vec::len);
        let last = count.saturating_sub(1);
        let moved = match code {
            KeyCode::Up | KeyCode::Char('k') => cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => (cursor + 1).min(last),
            KeyCode::PageUp => cursor.saturating_sub(TECHNIQUES_PAGE),
            KeyCode::PageDown => (cursor + TECHNIQUES_PAGE).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            KeyCode::Char(' ') | KeyCode::Enter => {
                self.switch_technique(cursor, false);
                cursor
            }
            KeyCode::Char('d') => {
                self.switch_technique(cursor, true);
                cursor
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlay = None;
                return;
            }
            _ => cursor,
        };
        self.overlay = Some(Overlay::Techniques { cursor: moved });
    }

    /// Switches technique number `index`: flips it, or with `to_default`, puts it back to its
    /// default. Only switches away from the defaults are kept, and sent.
    fn switch_technique(&mut self, index: usize, to_default: bool) {
        let Some(technique) = self.techniques.as_ref().and_then(|t| t.get(index)).cloned() else {
            return;
        };
        let on = if to_default {
            technique.default
        } else {
            !self.technique_on(&technique)
        };
        if on == technique.default {
            self.settings.techniques.remove(&technique.id);
        } else {
            self.settings.techniques.insert(technique.id.clone(), on);
        }
        self.status = format!(
            "{} {} for the next search{}.",
            technique.name,
            if on { "on" } else { "off" },
            match self.settings.techniques.len() {
                0 => " (every technique at its default)".to_string(),
                n => format!(" ({n} switched from the defaults)"),
            }
        );
    }

    /// Starts or stops following searches at their root: the next search reports it, and a
    /// running analysis starts again to.
    fn toggle_root(&mut self) {
        if !self.node.caps.root {
            self.status = format!("{} can't report its search's root.", self.node.name);
            return;
        }
        self.follow_root = !self.follow_root;
        self.root = None;
        self.status = if self.follow_root {
            "Following the search at its root, from the next search on. w stops.".into()
        } else {
            "Not following the root any more.".into()
        };
    }

    /// Whether searches should report their root.
    fn wants_root(&self) -> bool {
        self.follow_root && self.node.caps.root
    }

    /// Asks the engine for a search tree of the position on the board, `depth` plies deep (or
    /// [`TREE_DEPTH`]), recording [`TREE_PLIES`] plies; it opens when the answer comes.
    fn open_tree(&mut self, depth: Option<u32>, engine: &mut dyn Engine) {
        if !self.has_engine {
            self.status = NO_ENGINE.into();
            return;
        }
        if !self.node.caps.tree {
            self.status = format!("{} can't record a search's tree.", self.node.name);
            return;
        }
        let most = self.node.max_tree_depth;
        let depth = depth.unwrap_or(TREE_DEPTH).min(most);
        let plies = TREE_PLIES.min(self.node.max_tree_plies).max(1);
        let game = self.shown_game();
        let position =
            proto::Position::fen(game.start().to_fen()).with_moves(rules::game_moves_uci(&game));
        match engine.tree(&crate::tree::params(position, depth, plies)) {
            Ok(id) => {
                self.tree_request = Some((id, game.position().clone()));
                self.status = format!("Searching {depth} plies deep and keeping the tree…");
            }
            Err(e) => self.status = e,
        }
    }

    /// The tree's keys: the arrows choose a child (Page Up, Page Down, Home, End too), → or
    /// Enter goes into it, ← or Backspace back up, Esc or `q` closes the tree.
    fn tree_key(&mut self, code: KeyCode, engine: &mut dyn Engine) {
        let Some(tree) = &mut self.tree else {
            return;
        };
        match code {
            KeyCode::Up | KeyCode::Char('k') => tree.move_cursor(-1),
            KeyCode::Down | KeyCode::Char('j') => tree.move_cursor(1),
            KeyCode::PageUp => tree.move_cursor(-(TECHNIQUES_PAGE as isize)),
            KeyCode::PageDown => tree.move_cursor(TECHNIQUES_PAGE as isize),
            KeyCode::Home => tree.select(0),
            KeyCode::End => tree.select(usize::MAX),
            KeyCode::Right | KeyCode::Enter => match tree.path_into() {
                Some(path) => {
                    let params = cnp::engine::TreeNodeParams::new(tree.handle.clone(), path);
                    match engine.tree_node(&params) {
                        Ok(id) => tree.request = Some(id),
                        Err(e) => self.status = e,
                    }
                }
                None => self.status = "The tree holds nothing below that move.".into(),
            },
            KeyCode::Left | KeyCode::Backspace => {
                if !tree.back() {
                    self.status = "This is the root. Esc closes the tree.".into();
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.tree = None;
                self.status = "Tree closed.".into();
            }
            _ => {}
        }
    }

    /// Opens the techniques, if the engine has them.
    fn open_techniques(&mut self) {
        if !self.node.caps.techniques {
            self.status = format!("{} has no techniques to switch.", self.node.name);
            return;
        }
        if self.techniques.is_none() {
            self.status = format!("{} hasn't said its techniques yet.", self.node.name);
            return;
        }
        self.overlay = Some(Overlay::Techniques { cursor: 0 });
    }

    /// Asks the engine to evaluate the position on the board, term by term.
    fn evaluate(&mut self, engine: &mut dyn Engine) {
        if !self.has_engine {
            self.status = NO_ENGINE.into();
            return;
        }
        if !self.node.caps.evaluate {
            self.status = format!("{} can't break its evaluation into terms.", self.node.name);
            return;
        }
        let game = self.shown_game();
        let mut params = EvaluateParams::new(
            proto::Position::fen(game.start().to_fen()).with_moves(rules::game_moves_uci(&game)),
        );
        // Settle the captures first, when it can, so a piece about to be taken isn't counted.
        params.quiet = self.node.caps.quiet;
        let label = self
            .shown_move_label()
            .map_or("Start".to_string(), |label| format!("After {label}"));
        match engine.evaluate(&params) {
            Ok(id) => {
                self.evaluate_request = Some((id, label, game.position().clone()));
                self.status = "Evaluating…".into();
            }
            Err(e) => self.status = e,
        }
    }

    /// Colours the board with `theme`.
    fn set_theme(&mut self, theme: Theme) {
        self.settings.theme = theme;
        self.status = format!(
            "The {theme} board. :theme goes on to the next ({}).",
            theme.next()
        );
    }

    /// Shows the help, from its top.
    fn open_help(&mut self) {
        self.show_help = true;
        self.help_scroll = 0;
    }

    /// The help's keys: ↑ ↓ (or `k` `j`) a line, Page Up and Page Down (or `b`, Space, `f`) a
    /// page, Home and End (or `g`, `G`) the top and bottom; any other key closes it.
    fn help_key(&mut self, code: KeyCode) {
        let view = HelpView::new(self.screen, self);
        let page = view.rows.max(1) as isize;
        let by = match code {
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::PageUp | KeyCode::Char('b') => -page,
            KeyCode::PageDown | KeyCode::Char(' ' | 'f') => page,
            KeyCode::Home | KeyCode::Char('g') => isize::MIN,
            KeyCode::End | KeyCode::Char('G') => isize::MAX,
            _ => {
                self.show_help = false;
                return;
            }
        };
        self.scroll_help(by);
    }

    /// Scrolls the help `by` lines (up is negative), within what the screen leaves hidden.
    pub fn scroll_help(&mut self, by: isize) {
        let max = HelpView::new(self.screen, self).max_scroll();
        let at = self.help_scroll.min(max);
        self.help_scroll = at.saturating_add_signed(by).min(max);
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
        if self.overlay.take().is_some() {
            return;
        }
        if self.tree.is_some() {
            self.status = "The board shows the tree: Esc closes it.".into();
            return;
        }
        self.cursor = square;
        self.cursor_visible = false;
        self.handle_square(square, engine);
        self.sync_analysis(engine);
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
            ("" | "help" | "h", _) => self.open_help(),
            ("q" | "quit" | "exit", _) => self.quit = true,
            ("new", _) => self.new_game(None, engine),
            ("fen", fen) if !fen.is_empty() => self.new_game(Some(fen.join(" ")), engine),
            ("flip", _) => self.flipped = !self.flipped,
            ("zoom", _) => self.zoomed = !self.zoomed,
            ("theme", []) => self.set_theme(self.settings.theme.next()),
            ("theme", [name]) => match Theme::parse(name) {
                Some(theme) => self.set_theme(theme),
                None => {
                    self.status = format!("Usage: :theme {} (alone: the next one)", Theme::names())
                }
            },
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
            ("analyse" | "analyze" | "an", _) => self.toggle_analysis(engine),
            ("lines", [n]) => self.set_lines(n),
            ("eval" | "evaluate", _) => self.evaluate(engine),
            ("techniques" | "tech", _) => self.open_techniques(),
            ("root", _) => self.toggle_root(),
            ("tree", []) => self.open_tree(None, engine),
            ("tree", [depth]) => match depth.parse::<u32>() {
                Ok(depth) if depth >= 1 => self.open_tree(Some(depth), engine),
                _ => self.status = "Usage: :tree [depth]".into(),
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
        // Playing ends an analysis: the engine can't do both at once.
        self.end_analysis(engine);
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
        if let Some(reason) = self.analysing() {
            self.status = reason;
            return;
        }
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
        if let Some(reason) = self.analysing() {
            self.status = reason;
            return;
        }
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
            if self.game.position().fullmove_number() < DRAW_MIN_MOVE {
                self.status.push_str(&format!(
                    " (It considers draws only from move {DRAW_MIN_MOVE}.)"
                ));
            }
            return;
        }
        self.game.agree_draw();
        self.game_ended(engine);
    }

    /// Starts or stops analysing. While it analyses, the engine plays neither side, the clocks
    /// stop, and moves can be made for both sides; stopping puts back who played what.
    fn toggle_analysis(&mut self, engine: &mut dyn Engine) {
        if let Some(analysis) = self.end_analysis(engine) {
            self.settings.engine_plays = analysis.engine_plays;
            self.status = "Analysis stopped.".into();
            self.resume_clock();
            if self.review.is_none() && self.browse.is_none() {
                self.maybe_start_engine(engine);
            }
            return;
        }
        if !self.has_engine {
            self.status = NO_ENGINE.into();
            return;
        }
        if !self.node.caps.progress {
            self.status = format!(
                "{} doesn't report its search as it goes, so cbtui can't show an analysis.",
                self.node.name
            );
            return;
        }
        self.cancel_search(engine);
        self.draw_offered = false;
        self.analysis = Some(Analysis::new(self.settings.engine_plays));
        self.settings.engine_plays = [false, false];
        if let Some(clocks) = &mut self.clocks {
            clocks.stop(self.now);
        }
        let lines = self.analysis_lines();
        self.status = format!(
            "Analysing the position on the board, {lines} line{}. i stops it.",
            if lines == 1 { "" } else { "s" }
        );
        self.sync_analysis(engine);
    }

    /// Stops the analysis, if there is one, and returns it.
    fn end_analysis(&mut self, engine: &mut dyn Engine) -> Option<Analysis> {
        let analysis = self.analysis.take()?;
        if let Some(id) = analysis.id {
            let _ = engine.stop(id);
        }
        Some(analysis)
    }

    /// How many lines an analysis asks for: as many as asked, if the engine can find them.
    pub fn analysis_lines(&self) -> u32 {
        if self.node.caps.multipv {
            self.settings.lines.min(self.node.max_multipv).max(1)
        } else {
            1
        }
    }

    /// Sets how many lines an analysis shows.
    fn set_lines(&mut self, text: &str) {
        if !self.node.caps.multipv {
            self.status = format!("{} finds one line at a time.", self.node.name);
            return;
        }
        let most = MAX_LINES.min(self.node.max_multipv);
        match text.parse::<u32>() {
            Ok(n) if (1..=most).contains(&n) => {
                self.settings.lines = n;
                self.status = format!("Analysis shows {n} line{}.", if n == 1 { "" } else { "s" });
                // Searched again with the new count.
                if let Some(analysis) = &mut self.analysis {
                    analysis.key = None;
                }
            }
            _ => self.status = format!("Usage: :lines <1-{most}>"),
        }
    }

    /// When analysing, makes sure the engine is searching the position on the board: if it
    /// has changed since the last search began, stops that search and starts another.
    fn sync_analysis(&mut self, engine: &mut dyn Engine) {
        let Some(analysis) = &self.analysis else {
            return;
        };
        let game = self.shown_game();
        let start = game.start().to_fen();
        let moves = rules::game_moves_uci(&game);
        // Switching a technique searches again too.
        let key = format!(
            "{start} {} {} {:?} {}",
            moves.join(" "),
            self.analysis_lines(),
            self.settings.techniques,
            self.wants_root()
        );
        if analysis.key.as_deref() == Some(key.as_str()) {
            return;
        }
        if let Some(id) = analysis.id {
            let _ = engine.stop(id);
        }
        let mut params = SearchParams::new(proto::Position::fen(start).with_moves(moves))
            .limits(SearchLimits::infinite());
        let lines = self.analysis_lines();
        if lines > 1 {
            params.multipv = Some(lines);
        }
        params.progress_ms = Some(if lines > 1 {
            ANALYSIS_PROGRESS_MS
        } else {
            PROGRESS_MS
        });
        params.techniques = self.settings.techniques.clone();
        if self.wants_root() {
            params.root_ms = Some(ROOT_MS);
        }
        params.game = Some(format!("cbtui-{}", self.game_number));
        match engine.search(&params) {
            Ok(id) => {
                self.root = None;
                if let Some(analysis) = &mut self.analysis {
                    analysis.restart(id, key);
                }
            }
            Err(e) => {
                self.status = e;
                if let Some(analysis) = &mut self.analysis {
                    // Not tried again until the position changes.
                    analysis.key = Some(key);
                    analysis.id = None;
                }
            }
        }
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
        self.tree = None;
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

    /// Why the engine can't be asked to play now: it's analysing.
    fn analysing(&self) -> Option<String> {
        self.analysis
            .is_some()
            .then(|| "You're analysing: press i to stop, then play on.".to_string())
    }

    /// Makes the engine play the side to move now, whoever plays it.
    fn go(&mut self, engine: &mut dyn Engine) {
        if let Some(reason) = self.analysing() {
            self.status = reason;
            return;
        }
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
                self.root = None;
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
        params.techniques = self.settings.techniques.clone();
        if self.wants_root() {
            params.root_ms = Some(ROOT_MS);
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
                let position = self.shown_position();
                if let Some(analysis) = &mut self.analysis
                    && analysis.id.is_some_and(|id| progress.id == id.into())
                {
                    analysis.progress(&progress, &position);
                    return;
                }
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
            NodeEvent::Root(report) => {
                let analysing = self.analysis.as_ref().and_then(|a| a.id);
                let position = if analysing.is_some_and(|id| report.id == id.into()) {
                    self.shown_position()
                } else if self.pending.is_some_and(|p| report.id == p.id.into()) {
                    self.game.position().clone()
                } else {
                    return;
                };
                self.root = Some(RootView::update(&report, &position, self.root.as_ref()));
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
                if let Some(analysis) = &mut self.analysis {
                    analysis.id = None;
                }
                self.status = format!("Engine disconnected: {reason}");
            }
        }
    }

    fn handle_response(&mut self, response: Response, engine: &mut dyn Engine) {
        let Some(cnp::Id::Number(id)) = response.id else {
            return;
        };
        if let Some((_, position)) = self.tree_request.take_if(|(at, _)| *at == id) {
            match response.result::<proto::Tree>() {
                Ok(result) => {
                    let tree = TreeBrowser::new(result, position);
                    self.status = format!(
                        "The search's tree, {} plies recorded: ↑ ↓ choose, → into, ← back, Esc closes.",
                        tree.plies
                    );
                    self.selected = None;
                    self.tree = Some(tree);
                }
                Err(e) => {
                    self.status = format!("{} couldn't make a tree: {}", self.node.name, e.message);
                }
            }
            return;
        }
        if let Some(tree) = &mut self.tree
            && tree.request == Some(id)
        {
            tree.request = None;
            match response.result::<proto::TreeNodePage>() {
                Ok(answer) => tree.enter(answer),
                Err(e) if e.code == code::EXPIRED => {
                    self.tree = None;
                    self.status = format!(
                        "{} has let the tree go: :tree searches again.",
                        self.node.name
                    );
                }
                Err(e) => {
                    self.status =
                        format!("{} couldn't read the tree: {}", self.node.name, e.message);
                }
            }
            return;
        }
        if self.techniques_request == Some(id) {
            self.techniques_request = None;
            if let Ok(result) = response.result::<proto::Techniques>() {
                self.techniques = Some(result.techniques);
            }
            return;
        }
        if let Some((_, label, position)) = self.evaluate_request.take_if(|(at, ..)| *at == id) {
            match response.result::<proto::Evaluate>() {
                Ok(result) => {
                    self.evaluation = Some(Evaluation {
                        label,
                        position,
                        result,
                    });
                    self.overlay = Some(Overlay::Evaluation);
                    self.status = "Any key closes the evaluation.".into();
                }
                Err(e) => {
                    self.status = format!("{} couldn't evaluate: {}", self.node.name, e.message);
                }
            }
            return;
        }
        if self.levels_request == Some(id) {
            self.levels_request = None;
            if let Ok(levels) = response.result::<proto::Levels>() {
                self.levels = Some(levels);
            }
            return;
        }
        if self.analysis.as_ref().is_some_and(|a| a.id == Some(id)) {
            let position = self.shown_position();
            match response.result::<proto::Search>() {
                Ok(result) => {
                    if let Some(analysis) = &mut self.analysis {
                        analysis.result(&result, &position);
                    }
                }
                Err(e) => {
                    if let Some(analysis) = &mut self.analysis {
                        analysis.id = None;
                    }
                    if e.code != code::CANCELLED {
                        self.status = format!("{} couldn't analyse: {}", self.node.name, e.message);
                    }
                }
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
        // A draw offer is answered with the engine's move: it accepts if it's late enough and
        // its own score says it's clearly worse, and then the move isn't played.
        let engine_side = self.settings.engine_plays[position.side_to_move().index()];
        let answering = engine_side && std::mem::take(&mut self.draw_offered);
        let move_number = position.fullmove_number();
        if answering
            && result
                .score
                .is_some_and(|score| accepts_draw(score, move_number))
        {
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
            self.status = if answering && move_number < DRAW_MIN_MOVE {
                format!(
                    "{} declines the draw (it considers one from move {DRAW_MIN_MOVE}) and plays {san}.",
                    self.node.name
                )
            } else if answering {
                format!("{} declines the draw and plays {san}.", self.node.name)
            } else {
                format!("{} played {san}.", self.node.name)
            };
            self.after_move(engine);
        }
    }
}

/// Whether an engine whose search scored `score` (from its own point of view) at move
/// `move_number` accepts a draw: from [`DRAW_MIN_MOVE`] on, when it is clearly worse
/// ([`DRAW_ACCEPT_CP`]) or being mated.
pub fn accepts_draw(score: Score, move_number: u32) -> bool {
    if move_number < DRAW_MIN_MOVE {
        return false;
    }
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
        evaluations: Vec<EvaluateParams>,
        trees: Vec<cnp::engine::TreeParams>,
        tree_nodes: Vec<cnp::engine::TreeNodeParams>,
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
        fn evaluate(&mut self, params: &EvaluateParams) -> Result<i64, String> {
            self.evaluations.push(params.clone());
            Ok(500 + self.evaluations.len() as i64)
        }
        fn tree(&mut self, params: &cnp::engine::TreeParams) -> Result<i64, String> {
            self.trees.push(params.clone());
            Ok(700 + self.trees.len() as i64)
        }
        fn tree_node(&mut self, params: &cnp::engine::TreeNodeParams) -> Result<i64, String> {
            self.tree_nodes.push(params.clone());
            Ok(800 + self.tree_nodes.len() as i64)
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
                multipv: true,
                ..EngineCapabilities::default()
            },
            max_multipv: 4,
            max_tree_depth: 24,
            max_tree_plies: 6,
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
        // Move 1: too early, however bad it thinks things are.
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, ":draw");
        assert!(app.draw_offered);
        assert!(app.status.contains("from move 30"), "{}", app.status);
        app.handle_node(answer_scored(101, -300), &mut engine);
        assert!(!app.draw_offered);
        assert_eq!(app.game.moves().len(), 2);
        assert!(
            app.status.contains("declines the draw (it considers"),
            "{}",
            app.status
        );

        // Move 40, a pawn up for White (the person): Black, the engine, is clearly worse.
        let late = |cp: i32| {
            let settings = Settings {
                fen: Some("4k3/4p3/8/8/8/8/3PP3/4K3 w - - 0 40".into()),
                ..Settings::default()
            };
            let (mut app, mut engine) =
                (App::new(node(), settings).unwrap(), FakeEngine::default());
            type_line(&mut app, &mut engine, ":draw");
            type_line(&mut app, &mut engine, "d4");
            app.handle_node(answer_scored(101, cp), &mut engine);
            app
        };
        let app = late(-60);
        assert_eq!(app.game.outcome(), Some(Outcome::DrawAgreed));
        assert_eq!(app.game.moves().len(), 1, "its move isn't played");
        assert!(app.status.starts_with("eng accepts"), "{}", app.status);
        // Only a little worse: it plays on.
        let app = late(-20);
        assert_eq!(app.game.outcome(), None);
        assert!(
            app.status.starts_with("eng declines the draw and plays"),
            "{}",
            app.status
        );
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
        assert!(accepts_draw(Score::mate(-3), 40) && !accepts_draw(Score::mate(2), 40));
        assert!(!accepts_draw(Score::mate(-3), 29), "not before move 30");
        assert!(accepts_draw(Score::cp(-50), 30) && !accepts_draw(Score::cp(-49), 30));
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

    fn analysis_progress(id: i64, multipv: u32, pv: &[&str]) -> NodeEvent {
        NodeEvent::Progress(Progress {
            id: id.into(),
            depth: 12,
            sel_depth: 15,
            score: Score::cp(20),
            pv: pv.iter().map(|m| m.to_string()).collect(),
            multipv: Some(multipv),
            nodes: 9000,
            nps: Some(90_000),
            time_ms: 100,
            hash_full: None,
            tb_hits: None,
        })
    }

    #[test]
    fn analysis_searches_the_board_until_stopped_and_follows_each_move() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        app.handle_key(key('i'), &mut engine);
        assert_eq!(engine.searches.len(), 1);
        let params = &engine.searches[0];
        assert!(params.limits.infinite);
        assert_eq!(params.multipv, Some(DEFAULT_LINES));
        // Every report, so each line is heard, not just the best.
        assert_eq!(params.progress_ms, Some(ANALYSIS_PROGRESS_MS));
        app.handle_node(analysis_progress(101, 1, &["e2e4", "e7e5"]), &mut engine);
        app.handle_node(analysis_progress(101, 2, &["d2d4"]), &mut engine);
        let lines = &app.analysis.as_ref().unwrap().lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].pv, ["e4", "e5"]);

        // A move: the engine plays neither side now, and the analysis starts again.
        type_line(&mut app, &mut engine, "e4");
        assert_eq!(app.game.moves().len(), 1);
        assert_eq!(engine.stops, [101]);
        assert_eq!(engine.searches.len(), 2);
        assert_eq!(engine.searches[1].position.moves, ["e2e4"]);
        assert!(app.analysis.as_ref().unwrap().lines.is_empty());
        // A late report from the old search is dropped.
        app.handle_node(analysis_progress(101, 1, &["e2e4"]), &mut engine);
        assert!(app.analysis.as_ref().unwrap().lines.is_empty());
        // Black can move too.
        type_line(&mut app, &mut engine, "c5");
        assert_eq!(app.game.moves().len(), 2);

        // Stopping puts the engine back on Black, and it's White's move.
        app.handle_key(key('i'), &mut engine);
        assert_eq!(app.analysis, None);
        assert_eq!(engine.stops, [101, 102, 103]);
        assert_eq!(app.settings.engine_plays, [false, true]);
        assert!(!app.searching());
    }

    #[test]
    fn analysis_follows_the_pgn_viewer() {
        let (mut app, mut engine) = (viewer(true), FakeEngine::default());
        app.handle_key(key('i'), &mut engine);
        press(&mut app, &mut engine, KeyCode::Right);
        press(&mut app, &mut engine, KeyCode::Right);
        assert_eq!(engine.searches.len(), 3);
        assert_eq!(engine.searches[2].position.moves, ["e2e4", "e7e5"]);
        // A key that doesn't move the board doesn't start the search again.
        app.handle_key(key('t'), &mut engine);
        assert_eq!(engine.searches.len(), 3);
    }

    #[test]
    fn lines_sets_how_many_and_the_engine_decides_if_it_can() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":lines 9");
        assert!(app.status.contains("1-4"), "{}", app.status);
        type_line(&mut app, &mut engine, ":lines 1");
        app.handle_key(key('i'), &mut engine);
        assert_eq!(engine.searches[0].multipv, None);
        type_line(&mut app, &mut engine, ":lines 2");
        assert_eq!(engine.searches.len(), 2, "searched again for two lines");
        assert_eq!(engine.searches[1].multipv, Some(2));

        // An engine with no MultiPV: one line, and it says so.
        let mut single = node();
        single.caps.multipv = false;
        let mut app = App::new(single, Settings::default()).unwrap();
        type_line(&mut app, &mut engine, ":lines 3");
        assert!(app.status.contains("one line"), "{}", app.status);
        assert_eq!(app.analysis_lines(), 1);
    }

    #[test]
    fn analysis_needs_an_engine_that_reports_progress() {
        let mut quiet = node();
        quiet.caps.progress = false;
        let (mut app, mut engine) = (
            App::new(quiet, Settings::default()).unwrap(),
            FakeEngine::default(),
        );
        app.handle_key(key('i'), &mut engine);
        assert_eq!(app.analysis, None);
        assert!(app.status.contains("doesn't report"), "{}", app.status);
        assert!(engine.searches.is_empty());
    }

    fn evaluating_node() -> NodeSummary {
        let mut n = node();
        n.caps.evaluate = true;
        n.caps.quiet = true;
        n.caps.techniques = true;
        n
    }

    fn evaluation_answer(id: i64) -> NodeEvent {
        NodeEvent::Response(Response::ok::<proto::Evaluate>(
            id.into(),
            &serde_json::from_value::<EvaluateResult>(serde_json::json!({
                "score": {"cp": -33}, "quiet": [],
                "terms": [
                    {"id": "material", "name": "Material", "white": 3940, "black": 3940},
                    {"id": "mobility", "name": "Mobility", "white": -89, "black": -117}
                ]
            }))
            .unwrap(),
        ))
    }

    #[test]
    fn v_evaluates_the_board_and_any_key_closes_it() {
        let settings = Settings {
            engine_plays: [false, false],
            ..Settings::default()
        };
        let (mut app, mut engine) = (
            App::new(evaluating_node(), settings).unwrap(),
            FakeEngine::default(),
        );
        type_line(&mut app, &mut engine, "e4");
        app.handle_key(key('v'), &mut engine);
        assert_eq!(engine.evaluations.len(), 1);
        let params = &engine.evaluations[0];
        assert_eq!(params.position.moves, ["e2e4"]);
        assert!(params.quiet, "it settles captures when it can");
        app.handle_node(evaluation_answer(501), &mut engine);
        assert_eq!(app.overlay, Some(Overlay::Evaluation));
        let evaluation = app.evaluation.clone().unwrap();
        assert_eq!(evaluation.label, "After 1. e4");
        assert_eq!(evaluation.result.terms.len(), 2);
        // Any key closes it, and does nothing else: `q` doesn't quit.
        app.handle_key(key('q'), &mut engine);
        assert_eq!(app.overlay, None);
        assert!(!app.quit);

        // An engine that can't: it says so.
        let (mut app, mut engine) = (self::app(), FakeEngine::default());
        app.handle_key(key('v'), &mut engine);
        assert!(engine.evaluations.is_empty());
        assert!(app.status.contains("can't break"), "{}", app.status);
    }

    fn techniques_answer(id: i64) -> NodeEvent {
        NodeEvent::Response(Response::ok::<proto::Techniques>(
            id.into(),
            &serde_json::from_value::<proto::TechniquesResult>(serde_json::json!({
                "techniques": [
                    {"id": "alphabeta", "name": "Alpha-beta pruning", "description": "Cuts.", "default": true},
                    {"id": "nullmove", "name": "Null-move pruning", "description": "Passes.", "default": true},
                    {"id": "probcut", "name": "ProbCut", "description": "Guesses.", "default": false}
                ]
            }))
            .unwrap(),
        ))
    }

    #[test]
    fn techniques_switch_on_and_off_and_go_with_every_search() {
        let (mut app, mut engine) = (
            App::new(evaluating_node(), Settings::default()).unwrap(),
            FakeEngine::default(),
        );
        type_line(&mut app, &mut engine, ":techniques");
        assert!(app.status.contains("hasn't said"), "{}", app.status);
        app.expect_techniques(7);
        app.handle_node(techniques_answer(7), &mut engine);
        type_line(&mut app, &mut engine, ":techniques");
        assert_eq!(app.overlay, Some(Overlay::Techniques { cursor: 0 }));
        // Null-move pruning off, ProbCut on.
        press(&mut app, &mut engine, KeyCode::Down);
        app.handle_key(key(' '), &mut engine);
        press(&mut app, &mut engine, KeyCode::End);
        press(&mut app, &mut engine, KeyCode::Enter);
        let switched: Vec<(&str, bool)> = app
            .settings
            .techniques
            .iter()
            .map(|(id, on)| (id.as_str(), *on))
            .collect();
        assert_eq!(switched, [("nullmove", false), ("probcut", true)]);
        // `d` puts ProbCut back to its default; Esc closes.
        app.handle_key(key('d'), &mut engine);
        press(&mut app, &mut engine, KeyCode::Esc);
        assert_eq!(app.overlay, None);
        type_line(&mut app, &mut engine, "e4");
        let sent = &engine.searches[0].techniques;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent.get("nullmove"), Some(&false));

        // Switching one while analysing searches again with it.
        let (mut app, mut engine) = (
            App::new(evaluating_node(), Settings::default()).unwrap(),
            FakeEngine::default(),
        );
        app.expect_techniques(7);
        app.handle_node(techniques_answer(7), &mut engine);
        app.handle_key(key('i'), &mut engine);
        type_line(&mut app, &mut engine, ":techniques");
        app.handle_key(key(' '), &mut engine);
        assert_eq!(engine.searches.len(), 2);
        assert_eq!(engine.searches[1].techniques.get("alphabeta"), Some(&false));
    }

    fn root_report(id: i64, mv: &str) -> NodeEvent {
        NodeEvent::Root(
            serde_json::from_value(serde_json::json!({
                "id": id, "depth": 9, "move": mv, "number": 1, "of": 20,
                "moves": [{"move": mv, "nodes": 50}], "nodes": 500, "timeMs": 30
            }))
            .unwrap(),
        )
    }

    #[test]
    fn w_follows_the_root_of_the_engines_searches_and_analyses() {
        let mut n = node();
        n.caps.root = true;
        let (mut app, mut engine) = (
            App::new(n, Settings::default()).unwrap(),
            FakeEngine::default(),
        );
        app.handle_key(key('w'), &mut engine);
        assert!(app.follow_root);
        type_line(&mut app, &mut engine, "e4");
        assert_eq!(engine.searches[0].root_ms, Some(ROOT_MS));
        app.handle_node(root_report(101, "e7e5"), &mut engine);
        assert_eq!(app.root.as_ref().unwrap().current, "e5");
        // Another search's report is dropped.
        app.handle_node(root_report(99, "c7c5"), &mut engine);
        assert_eq!(app.root.as_ref().unwrap().current, "e5");
        app.handle_node(answer(101, "e7e5"), &mut engine);

        // An analysis follows it too, from the position on the board.
        app.handle_key(key('i'), &mut engine);
        assert_eq!(engine.searches[1].root_ms, Some(ROOT_MS));
        assert_eq!(app.root, None, "a new search starts afresh");
        app.handle_node(root_report(102, "g1f3"), &mut engine);
        assert_eq!(app.root.as_ref().unwrap().current, "Nf3");
        // Stopping following starts the analysis again without it.
        app.handle_key(key('w'), &mut engine);
        assert_eq!(engine.searches.len(), 3);
        assert_eq!(engine.searches[2].root_ms, None);

        // An engine that can't: it says so.
        let (mut app, mut engine) = (self::app(), FakeEngine::default());
        app.handle_key(key('w'), &mut engine);
        assert!(!app.follow_root);
        assert!(app.status.contains("can't report"), "{}", app.status);
    }

    fn tree_answer(id: i64) -> NodeEvent {
        NodeEvent::Response(Response::ok::<proto::Tree>(
            id.into(),
            &serde_json::from_value::<proto::TreeResult>(serde_json::json!({
                "tree": "t1", "expiresMs": 300000, "depth": 8, "nodes": 30000, "timeMs": 30,
                "bestMove": "e7e5", "score": {"cp": -20}, "pv": ["e7e5"],
                "recorded": {"plies": 3, "nodes": 50, "decisions": 90},
                "root": {"depth": 8, "score": {"cp": -20}, "childCount": 2, "subtree": 49},
                "children": [
                    {"move": "e7e5", "depth": 7, "score": {"cp": 20}, "childCount": 3, "subtree": 20},
                    {"move": "a7a6", "depth": 7, "score": {"cp": 60, "bound": "lower"},
                     "childCount": 0, "subtree": 0,
                     "decisions": [{"kind": "cutoff", "move": "d2d4", "order": 1}]}
                ]
            }))
            .unwrap(),
        ))
    }

    #[test]
    fn tree_searches_the_board_and_walks_its_tree() {
        let mut n = node();
        n.caps.tree = true;
        n.max_tree_depth = 24;
        n.max_tree_plies = 6;
        let settings = Settings {
            engine_plays: [false, false],
            ..Settings::default()
        };
        let (mut app, mut engine) = (App::new(n, settings).unwrap(), FakeEngine::default());
        type_line(&mut app, &mut engine, "e4");
        type_line(&mut app, &mut engine, ":tree 30");
        let params = &engine.trees[0];
        assert_eq!(params.position.moves, ["e2e4"]);
        assert_eq!(
            (params.depth, params.plies),
            (24, Some(TREE_PLIES)),
            "within the engine's limits"
        );
        app.handle_node(tree_answer(701), &mut engine);
        let tree = app.tree.as_ref().unwrap();
        assert_eq!(tree.best.as_deref(), Some("e5"));
        // The board shows the tree's position, and the highlighted child's move.
        assert_eq!(
            app.shown_last_move(),
            app.shown_position().parse_uci_move("e7e5")
        );

        // a6 has nothing below it; e5 has.
        press(&mut app, &mut engine, KeyCode::Down);
        press(&mut app, &mut engine, KeyCode::Right);
        assert!(engine.tree_nodes.is_empty());
        assert!(app.status.contains("nothing below"), "{}", app.status);
        press(&mut app, &mut engine, KeyCode::Up);
        press(&mut app, &mut engine, KeyCode::Enter);
        assert_eq!(engine.tree_nodes[0].path, ["e7e5"]);
        assert_eq!(engine.tree_nodes[0].tree, "t1");
        app.handle_node(
            NodeEvent::Response(Response::ok::<proto::TreeNodePage>(
                801.into(),
                &serde_json::from_value::<proto::TreeNodeResult>(serde_json::json!({
                    "node": {"move": "e7e5", "depth": 7, "childCount": 1, "subtree": 20},
                    "children": [{"move": "g1f3", "depth": 6, "childCount": 0, "subtree": 0}]
                }))
                .unwrap(),
            )),
            &mut engine,
        );
        assert_eq!(app.tree.as_ref().unwrap().path_san(), ["e5"]);
        assert_eq!(app.shown_position().side_to_move(), Color::White);
        // Clicks don't move pieces meanwhile; ← goes back, Esc closes, and q isn't quit.
        app.handle_click(sq("g1"), &mut engine);
        assert_eq!(app.selected, None);
        press(&mut app, &mut engine, KeyCode::Left);
        assert_eq!(app.tree.as_ref().unwrap().levels.len(), 1);
        app.handle_key(key('q'), &mut engine);
        assert!(app.tree.is_none() && !app.quit);
        assert_eq!(app.shown_position(), *app.game.position());
    }

    #[test]
    fn an_expired_tree_closes_and_says_to_search_again() {
        let mut n = node();
        n.caps.tree = true;
        let (mut app, mut engine) = (
            App::new(n, Settings::default()).unwrap(),
            FakeEngine::default(),
        );
        type_line(&mut app, &mut engine, ":tree");
        app.handle_node(tree_answer(701), &mut engine);
        press(&mut app, &mut engine, KeyCode::Enter);
        app.handle_node(
            NodeEvent::Response(Response::error(
                Some(801.into()),
                cnp::ResponseError::new(code::EXPIRED, "expired"),
            )),
            &mut engine,
        );
        assert!(app.tree.is_none());
        assert!(
            app.status.contains(":tree searches again"),
            "{}",
            app.status
        );

        // Without the capability it explains.
        let (mut app, mut engine) = (self::app(), FakeEngine::default());
        type_line(&mut app, &mut engine, ":tree");
        assert!(engine.trees.is_empty());
        assert!(app.status.contains("can't record"), "{}", app.status);
    }

    #[test]
    fn the_help_scrolls_within_what_the_screen_hides_and_other_keys_close_it() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        app.screen = Rect::new(0, 0, 80, 20);
        let max = HelpView::new(app.screen, &app).max_scroll();
        assert!(max > 0, "the help doesn't fit 20 rows");
        app.handle_key(key('?'), &mut engine);
        press(&mut app, &mut engine, KeyCode::Down);
        press(&mut app, &mut engine, KeyCode::Down);
        assert_eq!(app.help_scroll, 2);
        press(&mut app, &mut engine, KeyCode::Up);
        assert_eq!(app.help_scroll, 1);
        press(&mut app, &mut engine, KeyCode::PageDown);
        assert_eq!(
            app.help_scroll,
            (1 + 18).min(max),
            "a page is the box's 18 rows"
        );
        press(&mut app, &mut engine, KeyCode::End);
        assert_eq!(app.help_scroll, max);
        press(&mut app, &mut engine, KeyCode::Down);
        assert_eq!(app.help_scroll, max, "never past the end");
        app.scroll_help(-3);
        assert_eq!(app.help_scroll, max - 3);
        press(&mut app, &mut engine, KeyCode::Home);
        assert_eq!(app.help_scroll, 0);
        assert!(app.show_help);
        // `q` closes the help; it doesn't quit.
        app.handle_key(key('q'), &mut engine);
        assert!(!app.show_help && !app.quit);
        // It opens at the top again.
        app.help_scroll = 5;
        type_line(&mut app, &mut engine, ":help");
        assert_eq!(app.help_scroll, 0);
    }

    #[test]
    fn theme_names_one_or_goes_on_to_the_next() {
        let (mut app, mut engine) = (app(), FakeEngine::default());
        assert_eq!(app.settings.theme, Theme::Brown);
        type_line(&mut app, &mut engine, ":theme");
        assert_eq!(app.settings.theme, Theme::Blue);
        type_line(&mut app, &mut engine, ":theme grey");
        assert_eq!(app.settings.theme, Theme::Grey);
        assert!(app.status.contains("grey board"), "{}", app.status);
        type_line(&mut app, &mut engine, ":theme pink");
        assert_eq!(app.settings.theme, Theme::Grey);
        assert!(
            app.status.contains("brown, blue, green, grey or dark"),
            "{}",
            app.status
        );
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
