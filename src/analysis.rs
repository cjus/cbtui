//! Analysis: the engine searching the position on the board until it is told to stop, with
//! one line or several (MultiPV).
//!
//! ## How it works
//!
//! [`Analysis`] holds what the engine has reported about the position it's analysing: each
//! line's score, depth and moves, and the search's totals. The app ([`crate::app`]) decides
//! when to search: whenever the position on the board changes (a move, a step through a PGN
//! game, a look back), it stops the old search and starts an infinite one on the new
//! position, and the analysis forgets the old lines.
//!
//! Lines arrive one at a time in `engine/progress`, each with its rank (`multipv`), and all
//! together in the search's answer once it stops.
//!
//! ## Pitfalls
//!
//! - Scores arrive from the side to move's point of view; they are kept from White's, like
//!   everything else on screen.
//! - A line's moves are turned into SAN from the position analysed. A report from an older
//!   search, for another position, is dropped by its id before it gets here.
//! - An analysis of several lines asks for every progress report
//!   ([`crate::app::ANALYSIS_PROGRESS_MS`]): an engine that throttles all its lines with one
//!   interval would otherwise report only the best line, which comes first at each depth.
//! - Lines of different depths can be on screen at once: a deeper search reports its best
//!   line first, and the others as it gets to them.

use cnp::engine::{Progress, Score, SearchResult};

use crate::app::white_view;
use crate::rules::{self, Position};

/// How many lines an analysis shows when the engine can show several and nobody said.
pub const DEFAULT_LINES: u32 = 3;

/// The most lines an analysis shows, whatever the engine allows: the engine panel has room
/// for no more.
pub const MAX_LINES: u32 = 7;

/// One line of an analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisLine {
    /// Its score, from White's point of view.
    pub score: Score,
    /// The depth it was searched to.
    pub depth: u32,
    /// Its moves, in SAN.
    pub pv: Vec<String>,
}

/// An analysis in progress, or stopped where it got to.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// The lines found, best first.
    pub lines: Vec<AnalysisLine>,
    /// The depth completed.
    pub depth: u32,
    /// Positions searched.
    pub nodes: u64,
    /// Positions a second, if the engine said.
    pub nps: Option<u64>,
    /// Time spent, in milliseconds.
    pub time_ms: u64,
    /// The running search's request id; `None` when no search runs.
    pub(crate) id: Option<i64>,
    /// What was last sent to be searched (the start FEN and the moves), to tell when the
    /// position on the board has changed.
    pub(crate) key: Option<String>,
    /// Which sides the engine played before the analysis began, to put back after it.
    pub(crate) engine_plays: [bool; 2],
}

impl Analysis {
    /// An analysis that hasn't searched yet. `engine_plays` is put back when it ends.
    pub fn new(engine_plays: [bool; 2]) -> Analysis {
        Analysis {
            lines: Vec::new(),
            depth: 0,
            nodes: 0,
            nps: None,
            time_ms: 0,
            id: None,
            key: None,
            engine_plays,
        }
    }

    /// Whether a search is running.
    pub fn running(&self) -> bool {
        self.id.is_some()
    }

    /// Starts over: the search with request `id` analyses `key`, and the old lines go.
    pub(crate) fn restart(&mut self, id: i64, key: String) {
        *self = Analysis {
            id: Some(id),
            key: Some(key),
            ..Analysis::new(self.engine_plays)
        };
    }

    /// Takes in a progress report on `position`.
    pub fn progress(&mut self, progress: &Progress, position: &Position) {
        let rank = progress.multipv.unwrap_or(1).max(1) as usize;
        self.depth = self.depth.max(progress.depth);
        self.nodes = progress.nodes;
        self.nps = progress.nps;
        self.time_ms = progress.time_ms;
        self.set_line(
            rank,
            AnalysisLine {
                score: white_view(progress.score, position.side_to_move()),
                depth: progress.depth,
                pv: rules::line_to_san(position, &progress.pv),
            },
        );
    }

    /// Takes in the search's answer on `position`: every line as it ended.
    pub fn result(&mut self, result: &SearchResult, position: &Position) {
        self.id = None;
        self.depth = result.depth;
        self.nodes = result.nodes;
        self.time_ms = result.time_ms;
        let side = position.side_to_move();
        if !result.lines.is_empty() {
            self.lines = result
                .lines
                .iter()
                .map(|line| AnalysisLine {
                    score: white_view(line.score, side),
                    depth: line.depth,
                    pv: rules::line_to_san(position, &line.pv),
                })
                .collect();
        } else if let Some(score) = result.score {
            self.set_line(
                1,
                AnalysisLine {
                    score: white_view(score, side),
                    depth: result.depth,
                    pv: rules::line_to_san(position, &result.pv),
                },
            );
        }
    }

    /// Puts `line` at `rank` (from 1), keeping the lines above it.
    fn set_line(&mut self, rank: usize, line: AnalysisLine) {
        let index = rank - 1;
        if index < self.lines.len() {
            self.lines[index] = line;
        } else {
            // A line ranked below ones not reported yet waits at the end until they come.
            self.lines.push(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::score_text;

    fn progress(multipv: u32, depth: u32, cp: i32, pv: &[&str]) -> Progress {
        Progress {
            id: 7.into(),
            depth,
            sel_depth: depth,
            score: Score::cp(cp),
            pv: pv.iter().map(|m| m.to_string()).collect(),
            multipv: Some(multipv),
            nodes: 1000 * u64::from(depth),
            nps: Some(50_000),
            time_ms: 20,
            hash_full: None,
            tb_hits: None,
        }
    }

    #[test]
    fn lines_fill_in_by_rank_from_whites_point_of_view() {
        let position =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
                .unwrap();
        let mut analysis = Analysis::new([false, true]);
        analysis.restart(7, "k".into());
        analysis.progress(&progress(1, 10, 20, &["c7c5", "g1f3"]), &position);
        analysis.progress(&progress(2, 10, -5, &["e7e5"]), &position);
        assert_eq!(analysis.lines.len(), 2);
        assert_eq!(analysis.lines[0].pv, ["c5", "Nf3"]);
        // Black's +0.20 is White's -0.20.
        assert_eq!(score_text(analysis.lines[0].score), "-0.20");
        // A deeper report of the best line replaces it.
        analysis.progress(&progress(1, 11, 30, &["e7e5"]), &position);
        assert_eq!(analysis.lines[0].pv, ["e5"]);
        assert_eq!(analysis.depth, 11);
        assert!(analysis.running());
    }

    #[test]
    fn the_answer_has_every_line_and_ends_the_search() {
        let position = Position::startpos();
        let mut analysis = Analysis::new([false, false]);
        analysis.restart(7, "k".into());
        let result: SearchResult = serde_json::from_value(serde_json::json!({
            "bestMove": "e2e4", "score": {"cp": 30}, "pv": ["e2e4"], "depth": 20,
            "nodes": 99, "timeMs": 5, "stopped": true,
            "lines": [
                {"multipv": 1, "score": {"cp": 30}, "pv": ["e2e4"], "depth": 20},
                {"multipv": 2, "score": {"cp": 25}, "pv": ["d2d4"], "depth": 20}
            ]
        }))
        .unwrap();
        analysis.result(&result, &position);
        assert!(!analysis.running());
        let pvs: Vec<&str> = analysis.lines.iter().map(|l| l.pv[0].as_str()).collect();
        assert_eq!(pvs, ["e4", "d4"]);
        assert_eq!(analysis.engine_plays, [false, false]);
    }
}
