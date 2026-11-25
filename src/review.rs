//! A game read from PGN, with its variations, to step through: cbtui's PGN viewer.
//!
//! [`Review`] turns a [`PgnGame`] into a tree of positions. The root is the starting position;
//! each node is a move and the position after it. A node's first child is the move that
//! follows it in its own line (the main line, or the variation it is in), and its other
//! children are the variations PGN gives as alternatives to that move, in the file's order.
//! So `1. e4 e5 2. Nf3 (2. f4 exf4) Nc6` is e4 → e5 → { Nf3 → Nc6, f4 → exf4 }.
//!
//! A cursor marks the node shown. [`Review::forward`] follows the first child (stays in its
//! line), [`Review::back`] goes to the parent (and so out of a variation at its start),
//! [`Review::enter`] takes one of the alternatives, and [`Review::to_start`] and
//! [`Review::to_end`] jump to the start of the game or the end of the line.
//!
//! [`Review::tokens`] lays the tree out as PGN does, variations in brackets after the move they
//! replace, for the move list to wrap and highlight.
//!
//! Pitfalls: the PGN reader checks every move, so the tree only holds legal ones; it also
//! stops reading variations nested deeper than its limit, which keeps the recursion here
//! shallow. Comments keep their text and lose their commands (`[%clk …]`).

use crate::rules::{self, Color, Game, Move, PgnAnnotations, PgnGame, Position};

/// A move in the tree, or the root (the starting position).
#[derive(Clone, Debug)]
pub struct Node {
    /// The node before, or `None` at the root.
    pub parent: Option<usize>,
    /// The moves from here: the line's next move first, then the alternatives to it.
    pub children: Vec<usize>,
    /// The move, or `None` at the root.
    pub mv: Option<Move>,
    /// The move in SAN (empty at the root).
    pub san: String,
    /// The move's number (`12` in `12… Nf6`).
    pub number: u32,
    /// Who played the move (at the root: who is to move).
    pub by: Color,
    /// Its numeric annotation glyphs (`$1` is `!`).
    pub nags: Vec<u8>,
    /// The comment before the move (a variation's opening comment; at the root, the game's).
    pub before: Option<String>,
    /// The comment after the move.
    pub comment: Option<String>,
    /// The position after the move (at the root, the starting position).
    pub position: Position,
}

/// One entry of the move list: a move with its number and glyphs, and the brackets around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// The move as written: `12. Nf3!`, `12… Nc6` or `Nc6`.
    pub text: String,
    /// The node it stands for.
    pub node: usize,
    /// How many variations open just before it.
    pub open: usize,
    /// How many variations close just after it.
    pub close: usize,
    /// How deep in variations it is: 0 on the main line.
    pub depth: usize,
}

/// A recorded game and where in it the viewer is.
#[derive(Clone, Debug)]
pub struct Review {
    /// What to call it: the file's name.
    pub name: String,
    /// The PGN tags, in order (`White`, `Event`, …).
    pub tags: Vec<(String, String)>,
    /// The result: `1-0`, `0-1`, `1/2-1/2` or `*`.
    pub result: String,
    nodes: Vec<Node>,
    cursor: usize,
}

impl Review {
    /// Reads the first game in PGN `text`, calling it `name`. The cursor starts at the
    /// starting position.
    pub fn from_pgn(text: &str, name: &str) -> Result<Review, String> {
        let pgn = rules::parse_pgn(text)?;
        Ok(Review::from_game(&pgn, name))
    }

    /// The tree of a game already read.
    pub fn from_game(pgn: &PgnGame, name: &str) -> Review {
        let root = Node {
            parent: None,
            children: Vec::new(),
            mv: None,
            san: String::new(),
            number: pgn.start.fullmove_number(),
            by: pgn.start.side_to_move(),
            nags: Vec::new(),
            before: pgn.intro.clone(),
            comment: None,
            position: pgn.start.clone(),
        };
        let mut review = Review {
            name: name.to_string(),
            tags: pgn.tags.clone(),
            result: pgn.result.clone(),
            nodes: vec![root],
            cursor: 0,
        };
        review.add_line(0, None, &pgn.moves, &pgn.comments, &pgn.annotations);
        review
    }

    /// Adds `moves` as a line from node `from`, the alternatives in `annotations` with it.
    fn add_line(
        &mut self,
        from: usize,
        intro: Option<String>,
        moves: &[Move],
        comments: &[Option<String>],
        annotations: &[PgnAnnotations],
    ) {
        let mut current = from;
        for (i, &mv) in moves.iter().enumerate() {
            let before = if i == 0 { intro.clone() } else { None };
            let comment = comments.get(i).cloned().flatten();
            let annotation = annotations.get(i);
            let nags = annotation.map(|a| a.nags.clone()).unwrap_or_default();
            let node = self.push(current, mv, nags, before, comment);
            // The move's alternatives start from the same position: siblings after it.
            for line in annotation.map_or(&[][..], |a| &a.variations) {
                self.add_line(
                    current,
                    line.intro.clone(),
                    &line.moves,
                    &line.comments,
                    &line.annotations,
                );
            }
            current = node;
        }
    }

    fn push(
        &mut self,
        parent: usize,
        mv: Move,
        nags: Vec<u8>,
        before: Option<String>,
        comment: Option<String>,
    ) -> usize {
        let mut position = self.nodes[parent].position.clone();
        let san = position.to_san(mv);
        let (number, by) = (position.fullmove_number(), position.side_to_move());
        position.make_move(mv);
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(parent),
            children: Vec::new(),
            mv: Some(mv),
            san,
            number,
            by,
            nags,
            before,
            comment,
            position,
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// The value of tag `name`, unless it is missing or unknown (`?`, `????.??.??`).
    pub fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty() && !v.chars().all(|c| matches!(c, '?' | '.')))
    }

    /// Node `id`.
    pub fn node(&self, id: usize) -> &Node {
        &self.nodes[id]
    }

    /// The node shown.
    pub fn current(&self) -> &Node {
        &self.nodes[self.cursor]
    }

    /// The shown node's id.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The starting position.
    pub fn start(&self) -> &Position {
        &self.nodes[0].position
    }

    /// Goes one move on along the line. False at its end.
    pub fn forward(&mut self) -> bool {
        match self.current().children.first() {
            Some(&next) => {
                self.cursor = next;
                true
            }
            None => false,
        }
    }

    /// Goes one move back. False at the start.
    pub fn back(&mut self) -> bool {
        match self.current().parent {
            Some(parent) => {
                self.cursor = parent;
                true
            }
            None => false,
        }
    }

    /// Goes to the starting position.
    pub fn to_start(&mut self) {
        self.cursor = 0;
    }

    /// Goes to the end of the line the cursor is in.
    pub fn to_end(&mut self) {
        while self.forward() {}
    }

    /// The alternatives to the next move: the variations that start here.
    pub fn alternatives(&self) -> &[usize] {
        self.current().children.get(1..).unwrap_or(&[])
    }

    /// Plays the first move of alternative `n` (1 is the first variation). False if there is
    /// no such alternative.
    pub fn enter(&mut self, n: usize) -> bool {
        match n.checked_sub(1).and_then(|i| self.alternatives().get(i)) {
            Some(&node) => {
                self.cursor = node;
                true
            }
            None => false,
        }
    }

    /// The moves from the start to the cursor, as node ids (the root left out).
    pub fn path(&self) -> Vec<usize> {
        let mut path = Vec::new();
        let mut id = self.cursor;
        while let Some(parent) = self.nodes[id].parent {
            path.push(id);
            id = parent;
        }
        path.reverse();
        path
    }

    /// Where the cursor is on the main line, and the main line's length, in moves; `None` in
    /// a variation.
    pub fn main_line_place(&self) -> Option<(usize, usize)> {
        let path = self.path();
        let main = path.iter().all(|&id| {
            self.nodes[id]
                .parent
                .is_some_and(|p| self.nodes[p].children[0] == id)
        });
        if !main {
            return None;
        }
        let (mut length, mut id) = (0, 0);
        while let Some(&next) = self.nodes[id].children.first() {
            length += 1;
            id = next;
        }
        Some((path.len(), length))
    }

    /// The game up to the cursor, to play on from: its moves and their comments.
    pub fn game_here(&self) -> Game {
        let mut game = Game::from_position(self.start().clone());
        for id in self.path() {
            let node = &self.nodes[id];
            if let Some(mv) = node.mv {
                // Every move in the tree was legal when it was read.
                let _ = game.play_with_comment(mv, node.comment.clone());
            }
        }
        game
    }

    /// Node `id` as the move list writes it: `12. Nf3!`, or for Black `12… Nc6` when
    /// `numbered`, else `Nc6`.
    pub fn label(&self, id: usize, numbered: bool) -> String {
        let node = &self.nodes[id];
        let mut text = match (node.by, numbered) {
            (Color::White, _) => format!("{}. {}", node.number, node.san),
            (Color::Black, true) => format!("{}… {}", node.number, node.san),
            (Color::Black, false) => node.san.clone(),
        };
        for &nag in &node.nags {
            if let Some(symbol) = rules::glyph(nag) {
                // The move's own glyphs (`!`, `?!`) touch it; the position's stand apart.
                if !(1..=6).contains(&nag) {
                    text.push(' ');
                }
                text.push_str(symbol);
            }
        }
        text
    }

    /// The whole tree as the move list shows it: the main line, each variation in brackets
    /// after the move it replaces.
    pub fn tokens(&self) -> Vec<Token> {
        let mut tokens = Vec::new();
        self.line_tokens(0, 0, true, &mut tokens);
        tokens
    }

    fn line_tokens(&self, from: usize, depth: usize, mut numbered: bool, out: &mut Vec<Token>) {
        let mut current = from;
        while let Some(&next) = self.nodes[current].children.first() {
            out.push(self.token(next, numbered, depth));
            numbered = false;
            for &alternative in &self.nodes[current].children[1..] {
                let first = out.len();
                out.push(self.token(alternative, true, depth + 1));
                self.line_tokens(alternative, depth + 1, false, out);
                out[first].open += 1;
                if let Some(last) = out.last_mut() {
                    last.close += 1;
                }
                // After a variation, Black's move needs its number again.
                numbered = true;
            }
            current = next;
        }
    }

    fn token(&self, id: usize, numbered: bool, depth: usize) -> Token {
        Token {
            text: self.label(id, numbered),
            node: id,
            open: 0,
            close: 0,
            depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = r#"[Event "Test"]
[White "Ann"]
[Black "Bo"]
[Result "1-0"]

{An old opening.} 1. e4 e5 2. Nf3 {The usual.} (2. f4 exf4 (2... d5) 3. Nf3) 2... Nc6 $1
3. Bb5 a6 $16 1-0
"#;

    fn review() -> Review {
        Review::from_pgn(GAME, "test.pgn").unwrap()
    }

    fn sans(review: &Review) -> Vec<String> {
        review
            .path()
            .iter()
            .map(|&id| review.node(id).san.clone())
            .collect()
    }

    #[test]
    fn reads_tags_and_starts_at_the_start() {
        let review = review();
        assert_eq!(review.tag("White"), Some("Ann"));
        assert_eq!(review.tag("Site"), None);
        assert_eq!(review.result, "1-0");
        assert_eq!(review.cursor(), 0);
        assert_eq!(review.current().before.as_deref(), Some("An old opening."));
        assert_eq!(review.main_line_place(), Some((0, 6)));
    }

    #[test]
    fn steps_along_the_main_line() {
        let mut review = review();
        assert!(!review.back());
        review.forward();
        review.forward();
        review.forward();
        assert_eq!(sans(&review), ["e4", "e5", "Nf3"]);
        assert_eq!(review.current().comment.as_deref(), Some("The usual."));
        review.to_end();
        assert_eq!(sans(&review), ["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"]);
        assert!(!review.forward());
        assert_eq!(review.main_line_place(), Some((6, 6)));
        review.to_start();
        assert_eq!(review.cursor(), 0);
    }

    #[test]
    fn enters_and_leaves_variations() {
        let mut review = review();
        review.forward();
        review.forward();
        // After 1... e5 the alternatives to 2. Nf3: 2. f4.
        assert_eq!(review.alternatives().len(), 1);
        assert!(!review.enter(2));
        assert!(review.enter(1));
        assert_eq!(sans(&review), ["e4", "e5", "f4"]);
        assert_eq!(review.main_line_place(), None);
        // Inside it, 2... d5 is an alternative to 2... exf4.
        assert!(review.enter(1));
        assert_eq!(sans(&review), ["e4", "e5", "f4", "d5"]);
        review.back();
        review.to_end();
        assert_eq!(sans(&review), ["e4", "e5", "f4", "exf4", "Nf3"]);
        // Back past the variation's start is back on the main line.
        for _ in 0..3 {
            review.back();
        }
        assert_eq!(sans(&review), ["e4", "e5"]);
        assert!(review.main_line_place().is_some());
    }

    #[test]
    fn lays_out_like_pgn() {
        let review = review();
        let text: Vec<String> = review
            .tokens()
            .iter()
            .map(|t| format!("{}{}{}", "(".repeat(t.open), t.text, ")".repeat(t.close)))
            .collect();
        assert_eq!(
            text.join(" "),
            "1. e4 e5 2. Nf3 (2. f4 exf4 (2… d5) 3. Nf3) 2… Nc6! 3. Bb5 a6 ±"
        );
        assert!(review.tokens().iter().any(|t| t.depth == 2));
    }

    #[test]
    fn the_game_here_has_the_moves_to_the_cursor() {
        let mut review = review();
        review.forward();
        review.forward();
        review.enter(1);
        let game = review.game_here();
        let moves: Vec<&str> = game.moves().iter().map(|m| m.san.as_str()).collect();
        assert_eq!(moves, ["e4", "e5", "f4"]);
        assert!(game.outcome().is_none());
    }

    #[test]
    fn a_game_from_a_position_numbers_from_it() {
        let text = "[FEN \"8/8/8/8/8/8/4k3/K7 b - - 0 40\"]\n[SetUp \"1\"]\n\n40... Kd2 41. Ka2 *";
        let review = Review::from_pgn(text, "end.pgn").unwrap();
        let tokens: Vec<String> = review.tokens().into_iter().map(|t| t.text).collect();
        assert_eq!(tokens, ["40… Kd2", "41. Ka2"]);
    }

    #[test]
    fn bad_pgn_is_an_error() {
        assert!(Review::from_pgn("1. e5 e4", "bad.pgn").is_err());
    }
}
