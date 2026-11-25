//! Browsing a search's tree (`engine/tree`, `engine/treeNode`): which moves the search looked
//! at, what each scored, and what it decided about them.
//!
//! ## How it works
//!
//! `:tree` asks the engine to search the position on the board and keep the tree near the root
//! ([`TreeParams::plies`]). Its answer is a handle, the search's result, and the root with its
//! children: the first level of a [`TreeBrowser`]. Going into a child asks `engine/treeNode`
//! for it by the moves that lead there, and its answer is the next level. Going back pops a
//! level, so nothing is asked again on the way up.
//!
//! Everything is shown from White's point of view, like the rest of the screen: the protocol
//! gives a node's score and window from its own side to move, so a Black node's window
//! `[alpha, beta]` is White's `[-beta, -alpha]`, and its bounds turn round.
//!
//! ## Pitfalls
//!
//! - A tree is recorded only near the root, but the search goes on below it: a node's score
//!   comes from the whole search, even when the tree holds none of its children.
//! - The handle expires (`expiresMs` after the last request, or sooner). Then the engine
//!   answers `Expired`, and the tree has to be searched again.
//! - A node may have been searched more than once; it is shown as its last search.

use cnp::engine::{
    Bound, Decision, DecisionKind, ResearchReason, Score, TreeNode, TreeNodeResult, TreeParams,
    TreeResult,
};

use crate::app::{score_text, white_view};
use crate::rules::{self, Color, Position};

/// How deep `:tree` searches when no depth is given (or the engine's most, if less).
pub const TREE_DEPTH: u32 = 8;

/// How many plies from the root a tree records (or the engine's most, if less).
pub const TREE_PLIES: u32 = 3;

/// One level of the walk: a node, its children, and the child highlighted.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeLevel {
    /// The moves from the root to it, in coordinate notation.
    pub path: Vec<String>,
    /// The position there.
    pub position: Position,
    /// The node.
    pub node: TreeNode,
    /// Its children, in the order the search first tried them.
    pub children: Vec<TreeNode>,
    /// The highlighted child.
    pub cursor: usize,
}

impl TreeLevel {
    /// The highlighted child, if there are any.
    pub fn selected(&self) -> Option<&TreeNode> {
        self.children.get(self.cursor)
    }
}

/// A search tree being walked.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeBrowser {
    /// The engine's handle for the tree.
    pub handle: String,
    /// The search's result: depth, nodes, time, best move and score.
    pub depth: u32,
    /// Nodes searched.
    pub nodes: u64,
    /// The best move, in SAN.
    pub best: Option<String>,
    /// The best move's score, from White's point of view.
    pub score: Option<Score>,
    /// How many plies the tree records, and whether the engine stopped recording early.
    pub plies: u32,
    /// Whether the recording stopped before the search ended.
    pub truncated: bool,
    /// The levels walked, the root first; the last is the one shown.
    pub levels: Vec<TreeLevel>,
    /// The `engine/treeNode` request on its way, into the highlighted child.
    pub(crate) request: Option<i64>,
}

impl TreeBrowser {
    /// The browser for an `engine/tree` answer about `position`.
    pub fn new(result: TreeResult, position: Position) -> TreeBrowser {
        let side = position.side_to_move();
        let best = result.best_move.as_deref().map(|mv| san(&position, mv));
        TreeBrowser {
            handle: result.tree,
            depth: result.depth,
            nodes: result.nodes,
            best,
            score: result.score.map(|s| white_view(s, side)),
            plies: result.recorded.plies,
            truncated: result.recorded.truncated,
            levels: vec![TreeLevel {
                path: Vec::new(),
                position,
                node: result.root,
                children: result.children,
                cursor: 0,
            }],
            request: None,
        }
    }

    /// The level shown.
    pub fn level(&self) -> &TreeLevel {
        self.levels.last().expect("a tree has its root level")
    }

    /// Moves the highlight among the children by `by` (up is negative), within them.
    pub fn move_cursor(&mut self, by: isize) {
        let level = self.levels.last_mut().expect("a tree has its root level");
        let last = level.children.len().saturating_sub(1) as isize;
        level.cursor = (level.cursor as isize + by).clamp(0, last.max(0)) as usize;
    }

    /// Highlights child `index`, or the last if there are fewer.
    pub fn select(&mut self, index: usize) {
        let level = self.levels.last_mut().expect("a tree has its root level");
        level.cursor = index.min(level.children.len().saturating_sub(1));
    }

    /// The path to the highlighted child, to ask `engine/treeNode` for, if it has children
    /// recorded.
    pub fn path_into(&self) -> Option<Vec<String>> {
        let level = self.level();
        let child = level.selected()?;
        if child.child_count == 0 {
            return None;
        }
        let mut path = level.path.clone();
        path.push(child.mv.clone()?);
        Some(path)
    }

    /// Takes in the answer for the highlighted child: the next level.
    pub fn enter(&mut self, answer: TreeNodeResult) {
        let level = self.level();
        let Some(mv) = level.selected().and_then(|c| c.mv.clone()) else {
            return;
        };
        let mut position = level.position.clone();
        let Some(parsed) = position.parse_uci_move(&mv) else {
            return;
        };
        position.make_move(parsed);
        let mut path = level.path.clone();
        path.push(mv);
        self.levels.push(TreeLevel {
            path,
            position,
            node: answer.node,
            children: answer.children,
            cursor: 0,
        });
    }

    /// Goes back up a level. False at the root.
    pub fn back(&mut self) -> bool {
        if self.levels.len() > 1 {
            self.levels.pop();
            true
        } else {
            false
        }
    }

    /// The moves from the root to the level shown, in SAN.
    pub fn path_san(&self) -> Vec<String> {
        rules::line_to_san(&self.levels[0].position, &self.level().path)
    }
}

/// The parameters for a tree of the game up to `position` (start FEN and moves).
pub fn params(position: cnp::engine::Position, depth: u32, plies: u32) -> TreeParams {
    TreeParams {
        plies: Some(plies),
        ..TreeParams::new(position, depth)
    }
}

/// A move in coordinate notation as SAN, from `position`.
fn san(position: &Position, mv: &str) -> String {
    rules::line_to_san(position, &[mv.to_string()])
        .pop()
        .unwrap_or_else(|| mv.to_string())
}

/// A score from White's point of view with its bound: `+0.36`, `≥ -0.14`, `≤ +0.10`.
pub fn bound_text(score: Score) -> String {
    let text = score_text(score);
    match score.bound {
        Bound::Exact => text,
        Bound::Lower => format!("≥ {text}"),
        Bound::Upper => format!("≤ {text}"),
    }
}

/// A node's score from White's point of view, `side` being its side to move.
pub fn node_score(node: &TreeNode, side: Color) -> Option<Score> {
    node.score.map(|s| white_view(s, side))
}

/// A node's window from White's point of view, low edge first: `side` being its side to move.
/// `None` for an edge it doesn't have.
pub fn window(node: &TreeNode, side: Color) -> (Option<Score>, Option<Score>) {
    let alpha = node.alpha.map(|s| white_view(s, side));
    let beta = node.beta.map(|s| white_view(s, side));
    match side {
        Color::White => (alpha, beta),
        Color::Black => (beta, alpha),
    }
}

/// One decision in words. `position` is the node's, which its moves are from; `technique`
/// names a technique by its id.
pub fn describe(
    decision: &Decision,
    position: &Position,
    technique: &dyn Fn(&str) -> String,
) -> String {
    let text = match &decision.kind {
        DecisionKind::Cutoff { mv, order } => format!(
            "{} refuted the move that led here: the {} move searched, it beat the other side's \
             best, so the rest weren't searched.",
            san(position, mv),
            ordinal(*order)
        ),
        DecisionKind::Table => {
            "An earlier search's result for this position settled it, from the table.".into()
        }
        DecisionKind::Tablebase => "Endgame tablebases settled it.".into(),
        DecisionKind::NullMove => {
            "Even passing kept it good enough for the side to move, so it wasn't searched \
             further."
                .into()
        }
        DecisionKind::NodePruned { mv: None } => {
            "Settled before any move was searched, by an estimate.".into()
        }
        DecisionKind::NodePruned { mv: Some(mv) } => format!(
            "Settled before any move was searched: a shallower search of {} proved it.",
            san(position, mv)
        ),
        DecisionKind::MovePruned { mv } => format!(
            "{} was left out without a search, as unable to change the outcome.",
            san(position, mv)
        ),
        DecisionKind::Reduced { plies } => format!(
            "Searched {} shallower, its move judged unlikely to matter.",
            plies_text(*plies)
        ),
        DecisionKind::Extended { plies } => format!(
            "Searched {} deeper, its move judged to matter more than most.",
            plies_text(*plies)
        ),
        DecisionKind::Research {
            reason: ResearchReason::Depth,
        } => "Searched again at full depth: the shallower search looked good.".into(),
        DecisionKind::Research {
            reason: ResearchReason::Window,
        } => "Searched again with the full window, to find its exact score.".into(),
        DecisionKind::Research {
            reason: ResearchReason::Unknown,
        } => "Searched again.".into(),
        DecisionKind::Unknown => "A decision this cbtui doesn't know.".into(),
    };
    match &decision.technique {
        Some(id) => format!("{text} ({})", technique(id)),
        None => text,
    }
}

/// A decision in a word or two, for a row of the children: `refuted by Nf6`, `null move`.
pub fn brief(decision: &Decision, position: &Position) -> String {
    match &decision.kind {
        DecisionKind::Cutoff { mv, .. } => format!("refuted by {}", san(position, mv)),
        DecisionKind::Table => "from the table".into(),
        DecisionKind::Tablebase => "tablebases".into(),
        DecisionKind::NullMove => "null move".into(),
        DecisionKind::NodePruned { .. } => "pruned".into(),
        DecisionKind::MovePruned { mv } => format!("left out {}", san(position, mv)),
        DecisionKind::Reduced { plies } => format!("{} shallower", plies_text(*plies)),
        DecisionKind::Extended { plies } => format!("{} deeper", plies_text(*plies)),
        DecisionKind::Research { .. } => "searched again".into(),
        DecisionKind::Unknown => "?".into(),
    }
}

/// `1 ply`, `2 plies`.
fn plies_text(n: u32) -> String {
    if n == 1 {
        "1 ply".into()
    } else {
        format!("{n} plies")
    }
}

/// `1st`, `2nd`, `3rd`, `11th`.
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position() -> Position {
        Position::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3")
            .unwrap()
    }

    fn tree() -> TreeBrowser {
        let result: TreeResult = serde_json::from_value(serde_json::json!({
            "tree": "t1", "expiresMs": 300000, "depth": 8, "nodes": 34636, "timeMs": 31,
            "bestMove": "f1b5", "score": {"cp": 36}, "pv": ["f1b5"],
            "recorded": {"plies": 3, "nodes": 342, "decisions": 1033},
            "root": {"depth": 8, "alpha": {"cp": 14}, "beta": {"cp": 64}, "score": {"cp": 36},
                     "childCount": 2, "subtree": 341},
            "children": [
                {"move": "d2d4", "depth": 7, "alpha": {"cp": -64}, "beta": {"cp": -14},
                 "score": {"cp": -14, "bound": "lower"}, "childCount": 1, "subtree": 38,
                 "decisions": [{"kind": "cutoff", "move": "e5d4", "order": 1, "technique": "alphabeta"}]},
                {"move": "e1e2", "depth": 7, "score": {"cp": -1, "bound": "lower"},
                 "childCount": 0, "subtree": 0, "decisions": [{"kind": "nullMove"}]}
            ]
        }))
        .unwrap();
        TreeBrowser::new(result, position())
    }

    #[test]
    fn a_tree_walks_down_and_back_up() {
        let mut tree = tree();
        assert_eq!(tree.best.as_deref(), Some("Bb5"));
        assert_eq!(tree.path_into(), Some(vec!["d2d4".to_string()]));
        // Ke2 has nothing recorded below it.
        tree.move_cursor(5);
        assert_eq!(tree.level().cursor, 1);
        assert_eq!(tree.path_into(), None);
        tree.move_cursor(-1);
        let answer: TreeNodeResult = serde_json::from_value(serde_json::json!({
            "node": {"move": "d2d4", "depth": 7, "childCount": 1, "subtree": 38},
            "children": [{"move": "e5d4", "depth": 6, "childCount": 0, "subtree": 0}]
        }))
        .unwrap();
        tree.enter(answer);
        assert_eq!(tree.levels.len(), 2);
        assert_eq!(tree.path_san(), ["d4"]);
        assert_eq!(tree.level().position.side_to_move(), Color::Black);
        assert!(tree.back());
        assert!(!tree.back(), "the root has no parent");
    }

    #[test]
    fn scores_and_windows_read_from_whites_side() {
        let tree = tree();
        let child = &tree.level().children[0];
        // d4 is a Black node: its -0.14 or more for Black is +0.14 or less for White.
        let score = node_score(child, Color::Black).unwrap();
        assert_eq!(bound_text(score), "≤ +0.14");
        let (low, high) = window(child, Color::Black);
        assert_eq!(score_text(low.unwrap()), "+0.14");
        assert_eq!(score_text(high.unwrap()), "+0.64");
    }

    #[test]
    fn decisions_read_as_sentences_and_briefly() {
        let tree = tree();
        let child = &tree.level().children[0];
        let mut at = position();
        at.make_move(at.parse_uci_move("d2d4").unwrap());
        let name = |id: &str| format!("by {id}");
        let text = describe(&child.decisions[0], &at, &name);
        assert!(
            text.starts_with("exd4 refuted the move that led here: the 1st move searched"),
            "{text}"
        );
        assert!(text.ends_with("(by alphabeta)"), "{text}");
        assert_eq!(brief(&child.decisions[0], &at), "refuted by exd4");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(ordinal(13), "13th");
        let extended = Decision::new(DecisionKind::Extended { plies: 1 });
        assert_eq!(brief(&extended, &at), "1 ply deeper");
    }
}
