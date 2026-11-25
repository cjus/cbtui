//! Following a search at its root (`engine/root`): which root move it is on, the best so far,
//! where its effort has gone, and what it goes back over.
//!
//! ## How it works
//!
//! A search sent with `rootMs` (capability `root`) reports its root at most that often, and at
//! once when it goes back over something ([`RootResearch`]). [`RootView::update`] turns each
//! report into what the screen shows: the moves in SAN, each root move's share of the effort,
//! biggest first, and the latest re-search in words. A re-search is kept until the next one,
//! since the reports after it usually have none.
//!
//! ## Pitfalls
//!
//! - A node may follow its root on one thread only, so the nodes per move are that thread's:
//!   good for shares, not for totals. The total is the search's own count.
//! - Moves arrive in coordinate notation for the position searched; a report from an older
//!   search is dropped by its id before it gets here.

use cnp::engine::{RootParams, RootResearch};

use crate::app::{score_text, white_view};
use crate::rules::{self, Position};

/// One root move and its share of the search's effort.
#[derive(Clone, Debug, PartialEq)]
pub struct Effort {
    /// The move, in SAN.
    pub san: String,
    /// The nodes searched below it.
    pub nodes: u64,
    /// Its share of the nodes below every root move, from 0 to 1.
    pub share: f64,
}

/// What the screen shows of a search's root.
#[derive(Clone, Debug, PartialEq)]
pub struct RootView {
    /// The depth being searched.
    pub depth: u32,
    /// The root move being searched now, in SAN.
    pub current: String,
    /// Its place in this depth's order, from 1.
    pub number: u32,
    /// How many root moves this depth searches.
    pub of: u32,
    /// The best root move so far, in SAN.
    pub best: Option<String>,
    /// Every root move searched so far, the most effort first.
    pub efforts: Vec<Effort>,
    /// Nodes searched, every thread together.
    pub nodes: u64,
    /// Time so far, in milliseconds.
    pub time_ms: u64,
    /// The latest thing the search went back over, in words, and when (milliseconds in).
    pub research: Option<(String, u64)>,
}

impl RootView {
    /// The view after `report` on `position`, keeping `previous`'s re-search if this report
    /// has none.
    pub fn update(report: &RootParams, position: &Position, previous: Option<&RootView>) -> Self {
        let san = |mv: &str| {
            rules::line_to_san(position, &[mv.to_string()])
                .pop()
                .unwrap_or_else(|| mv.to_string())
        };
        let total: u64 = report.moves.iter().map(|m| m.nodes).sum();
        let mut efforts: Vec<Effort> = report
            .moves
            .iter()
            .map(|m| Effort {
                san: san(&m.mv),
                nodes: m.nodes,
                share: if total == 0 {
                    0.0
                } else {
                    m.nodes as f64 / total as f64
                },
            })
            .collect();
        efforts.sort_by(|a, b| b.nodes.cmp(&a.nodes).then_with(|| a.san.cmp(&b.san)));
        let research = match &report.research {
            Some(research) => Some((describe(research, position, &san), report.time_ms)),
            None => previous.and_then(|p| p.research.clone()),
        };
        RootView {
            depth: report.depth,
            current: san(&report.mv),
            number: report.number,
            of: report.of,
            best: report.best.as_deref().map(san),
            efforts,
            nodes: report.nodes,
            time_ms: report.time_ms,
            research,
        }
    }
}

/// A re-search in words. Scores are turned to White's point of view, like every score shown.
fn describe(research: &RootResearch, position: &Position, san: &dyn Fn(&str) -> String) -> String {
    let side = position.side_to_move();
    match research {
        RootResearch::Better { mv } => format!(
            "{} looked better than the best on a quick search: searching it again in full.",
            san(mv)
        ),
        RootResearch::FailHigh { score } => format!(
            "The depth came out better than expected ({} for White or more): searching it \
             again with a wider window.",
            score_text(white_view(*score, side))
        ),
        RootResearch::FailLow { score } => format!(
            "The depth came out worse than expected ({} for White or less): searching it \
             again with a wider window.",
            score_text(white_view(*score, side))
        ),
        RootResearch::Unknown => "The search went back over something.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(research: Option<serde_json::Value>) -> RootParams {
        let mut json = serde_json::json!({
            "id": 2, "depth": 14, "move": "d2d4", "number": 2, "of": 27, "best": "f1b5",
            "moves": [
                {"move": "f1c4", "nodes": 100},
                {"move": "f1b5", "nodes": 600},
                {"move": "d2d4", "nodes": 300}
            ],
            "nodes": 372_437, "timeMs": 251
        });
        if let Some(research) = research {
            json["research"] = research;
        }
        serde_json::from_value(json).unwrap()
    }

    fn position() -> Position {
        Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
            .unwrap()
    }

    #[test]
    fn efforts_are_shares_biggest_first_in_san() {
        let view = RootView::update(&report(None), &position(), None);
        assert_eq!(view.current, "d4");
        assert_eq!(view.best.as_deref(), Some("Bb5"));
        let efforts: Vec<(&str, u32)> = view
            .efforts
            .iter()
            .map(|e| (e.san.as_str(), (e.share * 100.0).round() as u32))
            .collect();
        assert_eq!(efforts, [("Bb5", 60), ("d4", 30), ("Bc4", 10)]);
        assert_eq!(view.research, None);
    }

    #[test]
    fn a_research_is_described_and_kept_until_the_next() {
        let better = serde_json::json!({"kind": "better", "move": "d2d4"});
        let first = RootView::update(&report(Some(better)), &position(), None);
        let (text, at) = first.research.clone().unwrap();
        assert!(text.starts_with("d4 looked better"), "{text}");
        assert_eq!(at, 251);
        let quiet = RootView::update(&report(None), &position(), Some(&first));
        assert_eq!(quiet.research, first.research);

        let low = serde_json::json!({"kind": "failLow", "score": {"cp": -40, "bound": "upper"}});
        let view = RootView::update(&report(Some(low)), &position(), Some(&first));
        let text = view.research.unwrap().0;
        assert!(text.contains("worse than expected (-0.40"), "{text}");
    }
}
